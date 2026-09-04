//! Strategy T: inventory-aware three-stablecoin router over the Rumi 3pool
//! (icUSD/ckUSDT/ckUSDC) and three ICPSwap direct-stable pools. Dry-run
//! only — see the plan's Global Constraints for why this module never
//! calls a fund-moving canister method.
//!
//! This file is split pure-first: everything above the `// ─── Live quoting
//! ───` marker (added in later tasks) has no `ic_cdk` dependency and is
//! covered by `tests/strategy_t_math.rs` with zero network access.

/// One of the three par-valued ($1) stablecoins Strategy T routes between.
/// Rumi 3pool coin index is fixed by the pool's own token ordering
/// (verified live 2026-09-02/03): IcUsd=0, CkUsdt=1, CkUsdc=2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StableToken {
    IcUsd,
    CkUsdt,
    CkUsdc,
}

impl StableToken {
    pub const ALL: [StableToken; 3] = [StableToken::IcUsd, StableToken::CkUsdt, StableToken::CkUsdc];

    pub fn decimals(self) -> u8 {
        match self {
            StableToken::IcUsd => 8,
            StableToken::CkUsdt | StableToken::CkUsdc => 6,
        }
    }

    /// Ledger transfer fee, native decimals. icUSD 0.001 (100_000 raw, 8
    /// dec); ckUSDT/ckUSDC 0.01 (10_000 raw, 6 dec) — verified live
    /// 2026-09-02/03 `icrc1_fee` queries, matches the existing
    /// `ICUSD_FEE`/`CKUSDT_FEE`/`CKUSDC_FEE` constants in `arb.rs`.
    pub fn ledger_fee(self) -> u64 {
        match self {
            StableToken::IcUsd => 100_000,
            StableToken::CkUsdt | StableToken::CkUsdc => 10_000,
        }
    }

    pub fn rumi_coin_index(self) -> u8 {
        match self {
            StableToken::IcUsd => 0,
            StableToken::CkUsdt => 1,
            StableToken::CkUsdc => 2,
        }
    }
}

/// Which ICPSwap pool connects a given unordered pair of Strategy T
/// stablecoins. Each of the three pairs has exactly one pool (verified
/// live 2026-09-02/03 `metadata` calls).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClosingPool {
    /// eb25l-dyaaa-aaaar-qb4lq-cai — token0=icUSD, token1=ckUSDC.
    IcusdCkusdc,
    /// jogrm-gqaaa-aaaar-qcg2a-cai — token0=ckUSDT, token1=icUSD.
    IcusdCkusdt,
    /// heq6n-fyaaa-aaaag-qkcpq-cai — token0=ckUSDT, token1=ckUSDC.
    CkusdtCkusdc,
}

impl ClosingPool {
    /// The pool connecting an unordered pair, or `None` if `a == b`.
    pub fn for_pair(a: StableToken, b: StableToken) -> Option<ClosingPool> {
        use StableToken::*;
        match (a, b) {
            (IcUsd, CkUsdc) | (CkUsdc, IcUsd) => Some(ClosingPool::IcusdCkusdc),
            (IcUsd, CkUsdt) | (CkUsdt, IcUsd) => Some(ClosingPool::IcusdCkusdt),
            (CkUsdt, CkUsdc) | (CkUsdc, CkUsdt) => Some(ClosingPool::CkusdtCkusdc),
            _ => None,
        }
    }

    /// `zeroForOne` for a swap FROM `from` on this pool, given each pool's
    /// live-verified token0/token1 ordering. Panics-free: returns `false`
    /// for a `(pool, from)` combination that can't occur for a valid route
    /// (i.e. `from` not one of the pool's two tokens) — callers only ever
    /// invoke this with `from` equal to a route's `rumi_out` or `start`,
    /// which `for_pair` already guarantees is one of the pool's tokens.
    pub fn zero_for_one_from(self, from: StableToken) -> bool {
        use StableToken::*;
        matches!(
            (self, from),
            (ClosingPool::IcusdCkusdc, IcUsd)
                | (ClosingPool::IcusdCkusdt, CkUsdt)
                | (ClosingPool::CkusdtCkusdc, CkUsdt)
        )
    }
}

/// One of the twelve candidates: a directed Rumi 3pool leg (`start` →
/// `rumi_out`), optionally closed back to `start` via the matching ICPSwap
/// pool. `closing: None` is a one-leg inventory conversion.
#[derive(Clone, Copy, Debug)]
pub struct RouteDescriptor {
    pub start: StableToken,
    pub rumi_out: StableToken,
    pub closing: Option<ClosingPool>,
}

/// All twelve candidates: six directed Rumi legs × {stop, close}.
pub fn all_routes() -> Vec<RouteDescriptor> {
    let mut routes = Vec::with_capacity(12);
    for &start in StableToken::ALL.iter() {
        for &rumi_out in StableToken::ALL.iter() {
            if start == rumi_out {
                continue;
            }
            routes.push(RouteDescriptor { start, rumi_out, closing: None });
            if let Some(pool) = ClosingPool::for_pair(rumi_out, start) {
                routes.push(RouteDescriptor { start, rumi_out, closing: Some(pool) });
            }
        }
    }
    routes
}

/// Converts a native-decimal token amount to 6-decimal par-valued USD (each
/// of the three stables is scored at exactly $1 — an explicit operator
/// policy, not a market fact; see the plan's Global Constraints).
pub fn par_usd_6dec(amount_native: u64, token: StableToken) -> i64 {
    let decimals = token.decimals() as u32;
    if decimals >= 6 {
        (amount_native / 10u64.pow(decimals - 6)) as i64
    } else {
        (amount_native * 10u64.pow(6 - decimals)) as i64
    }
}

/// Net profit (6-dec USD) for a one-leg stop: `start` → `rumi_out` via
/// Rumi, no closing leg. `rumi_gross_out` is `calc_swap`'s raw returned
/// amount (native decimals of `rumi_out`). Two fee events apply: the start
/// token's own ledger fee to enter the Rumi leg (a real capital outflow on
/// top of the notional sent, per the four-ledger-movement accounting), and
/// `rumi_out`'s ledger fee on Rumi's own output transfer. There is no
/// closing-leg fee because there is no closing leg.
pub fn one_leg_net_profit_usd(
    start: StableToken,
    start_amount_native: u64,
    rumi_out: StableToken,
    rumi_gross_out: u64,
) -> i64 {
    let net_received = rumi_gross_out.saturating_sub(rumi_out.ledger_fee());
    par_usd_6dec(net_received, rumi_out) - par_usd_6dec(start_amount_native, start) - par_usd_6dec(start.ledger_fee(), start)
}

/// The exact native amount to feed into the closing-leg `quoteForAll` call
/// for a two-leg candidate: Rumi's gross output minus two ledger-fee events
/// on the intermediate token (Rumi's own send-side fee, then our send-side
/// fee into the closing pool) — part of the four-ledger-movement
/// accounting in the plan's Global Constraints.
pub fn closing_leg_input(rumi_out: StableToken, rumi_gross_out: u64) -> u64 {
    rumi_gross_out.saturating_sub(2 * rumi_out.ledger_fee())
}

/// Net profit (6-dec USD) for a two-leg round trip, given the closing
/// pool's raw `quoteForAll` gross output (native decimals of `start`).
/// Three fee events beyond `closing_leg_input`'s two: the start token's
/// entry-leg fee (see `one_leg_net_profit_usd` doc) and the closing pool's
/// own output-transfer fee on the start token — four total across the
/// round trip, matching the audited fixtures in `tests/strategy_t_math.rs`
/// exactly.
pub fn two_leg_net_profit_usd(
    start: StableToken,
    start_amount_native: u64,
    closing_gross_out: u64,
) -> i64 {
    let net_received = closing_gross_out.saturating_sub(start.ledger_fee());
    par_usd_6dec(net_received, start) - par_usd_6dec(start_amount_native, start) - par_usd_6dec(start.ledger_fee(), start)
}

// ─── Live quoting (async — calls Rumi 3pool `calc_swap` and ICPSwap
// `quoteForAll`; both are read-only query-style calls, no fund movement) ───

use candid::Principal;

/// The three configured ICPSwap closing-pool principals, resolved from
/// `BotConfig` by the caller (Task 6) — kept out of `state::` here so this
/// module's async layer stays testable against arbitrary principals.
#[derive(Clone, Copy, Debug)]
pub struct PoolPrincipals {
    pub icusd_ckusdc: Principal,
    pub icusd_ckusdt: Principal,
    pub ckusdt_ckusdc: Principal,
}

impl PoolPrincipals {
    fn resolve(self, pool: ClosingPool) -> Principal {
        match pool {
            ClosingPool::IcusdCkusdc => self.icusd_ckusdc,
            ClosingPool::IcusdCkusdt => self.icusd_ckusdt,
            ClosingPool::CkusdtCkusdc => self.ckusdt_ckusdc,
        }
    }
}

/// Why a candidate's economic profit could or couldn't be computed / relied
/// on. `FullyQuoted` is the only status a candidate may be ranked on.
#[derive(Clone, Debug)]
pub enum FillStatus {
    /// Both legs (or the one leg, for a stop) quoted successfully with a
    /// full-fill guarantee (`quoteForAll` for ICPSwap; Rumi's `calc_swap`
    /// has no partial-fill mode).
    FullyQuoted,
    /// Rumi's `calc_swap` call failed or errored.
    RumiQuoteFailed(String),
    /// The closing pool rejected the sizing (partial-fill boundary hit, or
    /// any other `quoteForAll` error) — the candidate is not fillable at
    /// this size right now, not merely "less profitable."
    ClosingQuoteRejected(String),
}

#[derive(Clone, Debug)]
pub struct CandidateQuote {
    pub route: RouteDescriptor,
    pub start_amount_native: u64,
    pub economic_profit_usd: i64, // meaningless unless fill_status is FullyQuoted
    pub fill_status: FillStatus,
    /// The exact native-decimal amount (in `route.rumi_out`'s decimals)
    /// fed into the closing pool's `quoteForAll` call — `None` for a
    /// one-leg candidate, or when the Rumi leg itself failed. This is the
    /// exact figure `check_allowance` needs as its required-allowance
    /// amount; it must never be independently recomputed elsewhere, since
    /// recomputing it from `start_amount_native` mixes decimals across
    /// tokens (see the fix history for this field).
    pub closing_leg_input_native: Option<u64>,
    /// The candidate's actual native-decimal end-token amount net of all
    /// fees — for a one-leg stop, `rumi_gross_out` minus `rumi_out`'s own
    /// ledger fee; for a two-leg close, `closing_gross_out` minus
    /// `start`'s ledger fee (the same `net_received` value each of Task
    /// 2's profit functions computes internally, just also surfaced here).
    /// Zero when the candidate's fill_status isn't `FullyQuoted`. This
    /// must be used for any native-decimal inventory/balance check — never
    /// reconstructed from `economic_profit_usd`, which is USD-denominated
    /// and mixes decimals across tokens exactly like the bug Task 5's fix
    /// round corrected.
    pub net_end_amount_native: u64,
}

/// Quotes all twelve candidates for the given per-start-token trade size.
/// Never calls `swap`, `depositFromAndSwap`, `icrc2_approve`, or any other
/// fund-moving method — every call here is `calc_swap` (Rumi, read-only)
/// or `quoteForAll` (ICPSwap, read-only query).
pub async fn quote_all_candidates(
    rumi_3pool: Principal,
    pools: PoolPrincipals,
    start_amount_native: impl Fn(StableToken) -> u64,
) -> Vec<CandidateQuote> {
    let mut results = Vec::with_capacity(12);
    for route in all_routes() {
        let amount_in = start_amount_native(route.start);
        let rumi_result = crate::swaps::pool_calc_swap(
            rumi_3pool,
            route.start.rumi_coin_index(),
            route.rumi_out.rumi_coin_index(),
            amount_in,
        )
        .await;

        let (economic_profit_usd, fill_status, closing_leg_input_native, net_end_amount_native) =
            match (rumi_result, route.closing) {
                (Err(e), _) => (0, FillStatus::RumiQuoteFailed(e), None, 0),
                (Ok(rumi_gross), None) => {
                    let profit = one_leg_net_profit_usd(route.start, amount_in, route.rumi_out, rumi_gross);
                    let net_end = rumi_gross.saturating_sub(route.rumi_out.ledger_fee());
                    (profit, FillStatus::FullyQuoted, None, net_end)
                }
                (Ok(rumi_gross), Some(closing_pool)) => {
                    let leg2_input = closing_leg_input(route.rumi_out, rumi_gross);
                    let zero_for_one = closing_pool.zero_for_one_from(route.rumi_out);
                    let closing_result = crate::prices::fetch_icpswap_quote_for_all(
                        pools.resolve(closing_pool),
                        leg2_input,
                        zero_for_one,
                    )
                    .await;
                    match closing_result {
                        Err(e) => (0, FillStatus::ClosingQuoteRejected(e), Some(leg2_input), 0),
                        Ok(closing_gross) => {
                            let profit = two_leg_net_profit_usd(route.start, amount_in, closing_gross);
                            let net_end = closing_gross.saturating_sub(route.start.ledger_fee());
                            (profit, FillStatus::FullyQuoted, Some(leg2_input), net_end)
                        }
                    }
                }
            };

        results.push(CandidateQuote {
            route,
            start_amount_native: amount_in,
            economic_profit_usd,
            fill_status,
            closing_leg_input_native,
            net_end_amount_native,
        });
    }
    results
}

// ─── Eligibility: allowance (read-only) and inventory bands (pure) ───

/// Whether a candidate's required ICPSwap-side allowance is currently
/// sufficient. This module only ever *reads* allowances — see
/// `swaps::query_allowance` — never grants or modifies one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AllowanceStatus {
    /// No closing leg — nothing to approve (every Rumi 3pool approval
    /// already exists, verified live 2026-09-02/03; a one-leg candidate is
    /// never allowance-blocked on that basis).
    NotRequired,
    /// Allowance covers the candidate's required input.
    Sufficient,
    /// Allowance exists but is smaller than required, or is exactly zero.
    Insufficient { allowance: u64, required: u64 },
}

/// Checks the allowance a two-leg candidate's closing leg needs: the
/// intermediate token approved to spend into the closing ICPSwap pool.
/// Returns `NotRequired` for a one-leg candidate (no closing leg) or when
/// the Rumi quote itself failed (no actual closing amount to allocate).
pub async fn check_allowance(
    candidate: &CandidateQuote,
    token_ledger: Principal,
    spender: Principal,
    this_canister: Principal,
) -> AllowanceStatus {
    let required = match candidate.closing_leg_input_native {
        None => return AllowanceStatus::NotRequired,
        Some(r) => r,
    };
    let allowance_result = crate::swaps::query_allowance(token_ledger, this_canister, spender).await;
    allowance_status_for(required, allowance_result)
}

/// Pure comparison logic for allowance sufficiency. Split out from
/// `check_allowance` so this logic is unit-testable without a network call.
pub fn allowance_status_for(required: u64, allowance_result: Result<(u64, Option<u64>), String>) -> AllowanceStatus {
    match allowance_result {
        Ok((allowance, _expires_at)) if allowance >= required => AllowanceStatus::Sufficient,
        Ok((allowance, _expires_at)) => AllowanceStatus::Insufficient { allowance, required },
        Err(_) => AllowanceStatus::Insufficient { allowance: 0, required },
    }
}

/// Native-decimal balances/bands for all three tokens, keyed by
/// `StableToken`. A small fixed-size struct rather than a `HashMap` — there
/// are exactly three tokens and this is on the hot path of every dry-run.
#[derive(Clone, Copy, Debug)]
pub struct TokenAmounts {
    pub icusd: u64,
    pub ckusdt: u64,
    pub ckusdc: u64,
}

impl TokenAmounts {
    pub fn get(self, token: StableToken) -> u64 {
        match token {
            StableToken::IcUsd => self.icusd,
            StableToken::CkUsdt => self.ckusdt,
            StableToken::CkUsdc => self.ckusdc,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct InventoryCheck {
    /// False if spending `start_amount` would take the start token's
    /// balance below its configured floor.
    pub start_ok: bool,
    /// False if receiving the expected end amount would take the end
    /// token's balance above its configured ceiling.
    pub end_ok: bool,
}

impl InventoryCheck {
    pub fn eligible(self) -> bool {
        self.start_ok && self.end_ok
    }
}

/// Pure inventory-band check — balances and bands are passed in (fetched
/// by the caller in Task 6) so this stays synchronously unit-testable.
/// `end_token`/`expected_end_amount` are the route's actual output token
/// (== `rumi_out` for a one-leg stop, == `start` for a two-leg close).
pub fn check_inventory_bands(
    start_token: StableToken,
    start_amount: u64,
    end_token: StableToken,
    expected_end_amount: u64,
    balances: TokenAmounts,
    floors: TokenAmounts,
    ceilings: TokenAmounts,
) -> InventoryCheck {
    let start_balance = balances.get(start_token);
    let start_floor = floors.get(start_token);
    let start_ok = start_balance.saturating_sub(start_amount) >= start_floor;

    let end_balance = balances.get(end_token);
    let end_ceiling = ceilings.get(end_token);
    let end_ok = end_balance.saturating_add(expected_end_amount) <= end_ceiling;

    InventoryCheck { start_ok, end_ok }
}

// ─── Candid-facing conversions (dashboard reporting reuses Task 1's
// state::StrategyTToken/state::StrategyTPool — no third enum pair) ───

impl From<StableToken> for crate::state::StrategyTToken {
    fn from(t: StableToken) -> Self {
        match t {
            StableToken::IcUsd => crate::state::StrategyTToken::IcUsd,
            StableToken::CkUsdt => crate::state::StrategyTToken::CkUsdt,
            StableToken::CkUsdc => crate::state::StrategyTToken::CkUsdc,
        }
    }
}

impl From<ClosingPool> for crate::state::StrategyTPool {
    fn from(p: ClosingPool) -> Self {
        match p {
            ClosingPool::IcusdCkusdc => crate::state::StrategyTPool::IcusdCkusdc,
            ClosingPool::IcusdCkusdt => crate::state::StrategyTPool::IcusdCkusdt,
            ClosingPool::CkusdtCkusdc => crate::state::StrategyTPool::CkusdtCkusdc,
        }
    }
}

// ─── Reporting and ranking ───

use candid::CandidType;

#[derive(CandidType, Clone, Debug)]
pub struct CandidateReport {
    pub start: crate::state::StrategyTToken,
    pub rumi_out: crate::state::StrategyTToken,
    pub closing: Option<crate::state::StrategyTPool>,
    pub start_amount_native: u64,
    pub economic_profit_usd: i64,
    pub meets_profit_threshold: bool,
    pub allowance_status: String, // Display of AllowanceStatus — candid-simple, dashboard-friendly
    pub inventory_eligible: bool,
    pub fill_ok: bool,
    pub fill_note: String, // empty if fill_ok, else the FillStatus error text
}

#[derive(CandidType, Clone, Debug)]
pub struct StrategyTDryRunResult {
    pub candidates: Vec<CandidateReport>,
    /// Highest `economic_profit_usd` among fully-quoted candidates that
    /// clear the profit threshold — regardless of allowance/inventory
    /// eligibility. A profitable route must never disappear from this
    /// list just because it isn't executable today.
    pub best_economic: Option<CandidateReport>,
    /// Highest `economic_profit_usd` among candidates that ALSO have
    /// `allowance_status == Sufficient/NotRequired` AND `inventory_eligible`.
    /// `None` if no candidate is both profitable and currently executable.
    pub best_executable: Option<CandidateReport>,
}

/// Ranks a fully-assembled, fully-checked candidate list. Pure — no
/// network calls; Task 6 Step 3's `evaluate()` does the async assembly and
/// calls this at the end.
pub fn rank_candidates(reports: Vec<CandidateReport>) -> StrategyTDryRunResult {
    let best_economic = reports
        .iter()
        .filter(|r| r.fill_ok && r.meets_profit_threshold)
        .max_by_key(|r| r.economic_profit_usd)
        .cloned();

    let best_executable = reports
        .iter()
        .filter(|r| {
            r.fill_ok
                && r.meets_profit_threshold
                && r.inventory_eligible
                && matches!(r.allowance_status.as_str(), "NotRequired" | "Sufficient")
        })
        .max_by_key(|r| r.economic_profit_usd)
        .cloned();

    StrategyTDryRunResult { candidates: reports, best_economic, best_executable }
}

/// Full Strategy T dry-run evaluation: quotes all twelve candidates,
/// checks allowance and inventory eligibility for each, and ranks them.
/// Every call this function makes is read-only (see Task 4/5 doc comments
/// for the exhaustive list). Requires all three closing pools to be
/// configured (non-anonymous) — returns an empty result otherwise.
pub async fn evaluate(
    rumi_3pool: Principal,
    pools: PoolPrincipals,
    this_canister: Principal,
    ledgers: TokenLedgers,
    start_amount_native: impl Fn(StableToken) -> u64 + Copy,
    min_profit_usd: i64,
    min_profit_bps: u32,
    balances: TokenAmounts,
    floors: TokenAmounts,
    ceilings: TokenAmounts,
) -> StrategyTDryRunResult {
    let quotes = quote_all_candidates(rumi_3pool, pools, start_amount_native).await;

    let mut reports = Vec::with_capacity(quotes.len());
    for q in quotes {
        let start_amount = q.start_amount_native;
        let bps_profit = if start_amount == 0 {
            0i64
        } else {
            (q.economic_profit_usd as i128 * 10_000 / par_usd_6dec(start_amount, q.route.start).max(1) as i128) as i64
        };
        let meets_profit_threshold =
            q.economic_profit_usd >= min_profit_usd && bps_profit >= min_profit_bps as i64;

        let allowance_status = match q.route.closing {
            None => AllowanceStatus::NotRequired,
            Some(pool) => {
                let token_ledger = ledgers.get(q.route.rumi_out);
                let spender = pools.resolve(pool);
                check_allowance(&q, token_ledger, spender, this_canister).await
            }
        };

        let end_token = q.route.closing.map(|_| q.route.start).unwrap_or(q.route.rumi_out);
        let expected_end_amount = q.net_end_amount_native;
        let inventory = check_inventory_bands(
            q.route.start, start_amount, end_token, expected_end_amount, balances, floors, ceilings,
        );

        let (fill_ok, fill_note) = match &q.fill_status {
            FillStatus::FullyQuoted => (true, String::new()),
            FillStatus::RumiQuoteFailed(e) => (false, format!("Rumi quote failed: {e}")),
            FillStatus::ClosingQuoteRejected(e) => (false, format!("Closing quote rejected: {e}")),
        };

        reports.push(CandidateReport {
            start: q.route.start.into(),
            rumi_out: q.route.rumi_out.into(),
            closing: q.route.closing.map(Into::into),
            start_amount_native: start_amount,
            economic_profit_usd: q.economic_profit_usd,
            meets_profit_threshold,
            allowance_status: format!("{:?}", allowance_status).split(' ').next().unwrap_or("").trim_end_matches('{').to_string(),
            inventory_eligible: inventory.eligible(),
            fill_ok,
            fill_note,
        });
    }

    rank_candidates(reports)
}

/// The three stablecoin ledger principals, resolved by the caller from
/// `BotConfig` (`icusd_ledger`, `ckusdt_ledger`, `ckusdc_ledger`).
#[derive(Clone, Copy, Debug)]
pub struct TokenLedgers {
    pub icusd: Principal,
    pub ckusdt: Principal,
    pub ckusdc: Principal,
}

impl TokenLedgers {
    fn get(self, token: StableToken) -> Principal {
        match token {
            StableToken::IcUsd => self.icusd,
            StableToken::CkUsdt => self.ckusdt,
            StableToken::CkUsdc => self.ckusdc,
        }
    }
}
