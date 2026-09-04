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
