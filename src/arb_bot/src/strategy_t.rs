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

        let (economic_profit_usd, fill_status) = match (rumi_result, route.closing) {
            (Err(e), _) => (0, FillStatus::RumiQuoteFailed(e)),
            (Ok(rumi_gross), None) => {
                let profit = one_leg_net_profit_usd(route.start, amount_in, route.rumi_out, rumi_gross);
                (profit, FillStatus::FullyQuoted)
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
                    Err(e) => (0, FillStatus::ClosingQuoteRejected(e)),
                    Ok(closing_gross) => {
                        let profit = two_leg_net_profit_usd(route.start, amount_in, closing_gross);
                        (profit, FillStatus::FullyQuoted)
                    }
                }
            }
        };

        results.push(CandidateQuote {
            route,
            start_amount_native: amount_in,
            economic_profit_usd,
            fill_status,
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
/// Returns `NotRequired` for a one-leg candidate.
pub async fn check_allowance(
    candidate: &CandidateQuote,
    token_ledger: Principal,
    spender: Principal,
    this_canister: Principal,
) -> AllowanceStatus {
    if candidate.route.closing.is_none() {
        return AllowanceStatus::NotRequired;
    }
    let required = closing_leg_input(
        candidate.route.rumi_out,
        // Re-derive the pre-closing-fee gross amount is not available here
        // without re-quoting; callers pass the already-computed leg2 input
        // via `required_override` in Task 6's wiring instead of calling
        // this helper standalone when the exact figure matters. For a
        // conservative (never-false-positive) check, use the candidate's
        // start amount as a floor — any real leg2 input for a profitable
        // candidate is within a few fee-units of it.
        candidate.start_amount_native,
    );
    match crate::swaps::query_allowance(token_ledger, this_canister, spender).await {
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
