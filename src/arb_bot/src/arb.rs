use candid::{CandidType, Nat, Principal};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::prices::{self, PriceData, nat_to_u64};
use crate::state::{self, Direction, Token};
use crate::swaps;
use crate::partydex;
use crate::volume;

const ICP_FEE: u64 = 10_000;        // 0.0001 ICP
const CKUSDC_FEE: u64 = 10_000;      // 0.01 ckUSDC
const CKUSDT_FEE: u64 = 10_000;      // 0.01 ckUSDT (6 decimals)
const ICUSD_FEE: u64 = 100_000;      // 0.001 icUSD (8 decimals)
// Note: 3USD has no transfer fee (0)

// Pool ID is deterministic: sorted principals joined by "_"
const RUMI_POOL_ID: &str = "fohh4-yyaaa-aaaap-qtkpa-cai_ryjl3-tyaaa-aaaaa-aaaba-cai";

/// Slippage tolerance in basis points, read from `state.config.slippage_bps`
/// at the top of each execute_/drain flow. Clamped to [0, 10_000] so the
/// `(10_000 - slippage)` math never underflows.
#[inline]
fn slippage_bps_clamped(config: &state::BotConfig) -> u64 {
    config.slippage_bps.min(10_000)
}

/// Number of candidate trade sizes to evaluate
const NUM_CANDIDATES: u64 = 4;

/// VP precision (1e18)
const VP_PRECISION: u128 = 1_000_000_000_000_000_000;

// ─── Venue dispatch (PR2a) ───
//
// Thin per-venue dispatch. No target sets `venue: state::Venue::PartyDex` in
// PR2a (that's PR2b's job) — the `Icpswap` arm below is byte-identical to
// what each rerouted call site did before this dispatch existed, so existing
// strategies A/B/C/D/F are behavior-preserving.

/// Price of 1 ICP in the pool's stable native units.
async fn venue_price_icp(pool: Principal, icp_is_token0: bool, venue: state::Venue, fee_pips: u32) -> Result<u64, String> {
    match venue {
        state::Venue::Icpswap => prices::fetch_icpswap_price(pool, icp_is_token0).await,
        state::Venue::PartyDex => partydex::price_icp(pool, fee_pips).await,
    }
}

/// Quote buying ICP by spending `amount` of the stable token.
async fn venue_quote_stable_to_icp(pool: Principal, icp_is_token0: bool, venue: state::Venue, fee_pips: u32, amount: u64) -> Result<u64, String> {
    match venue {
        state::Venue::Icpswap => prices::fetch_icpswap_quote_for_amount(pool, amount, !icp_is_token0).await,
        state::Venue::PartyDex => partydex::quote_stable_to_icp(pool, fee_pips, amount).await,
    }
}

/// Quote selling `amount` ICP for the stable token.
async fn venue_quote_icp_to_stable(pool: Principal, icp_is_token0: bool, venue: state::Venue, fee_pips: u32, amount: u64) -> Result<u64, String> {
    match venue {
        state::Venue::Icpswap => prices::fetch_icpswap_quote_for_amount(pool, amount, icp_is_token0).await,
        state::Venue::PartyDex => partydex::quote_icp_to_stable(pool, fee_pips, amount).await,
    }
}

/// Execute a stable→ICP swap on the given venue.
#[allow(clippy::too_many_arguments)]
async fn venue_swap_stable_to_icp(
    pool: Principal, icp_is_token0: bool, venue: state::Venue, fee_pips: u32,
    amount: u64, min_out: u64, stable_ledger: Principal, stable_fee: u64, icp_ledger: Principal,
) -> Result<u64, swaps::SwapError> {
    match venue {
        state::Venue::Icpswap => swaps::icpswap_swap(pool, amount, !icp_is_token0, min_out, stable_fee, ICP_FEE).await,
        state::Venue::PartyDex => partydex::swap(pool, fee_pips, partydex::Side::Buy, amount, min_out, stable_ledger, icp_ledger, false).await,
    }
}

/// Execute an ICP→stable swap on the given venue.
#[allow(clippy::too_many_arguments)]
async fn venue_swap_icp_to_stable(
    pool: Principal, icp_is_token0: bool, venue: state::Venue, fee_pips: u32,
    amount: u64, min_out: u64, icp_ledger: Principal, stable_ledger: Principal, stable_fee: u64,
) -> Result<u64, swaps::SwapError> {
    match venue {
        state::Venue::Icpswap => swaps::icpswap_swap(pool, amount, icp_is_token0, min_out, ICP_FEE, stable_fee).await,
        state::Venue::PartyDex => partydex::swap(pool, fee_pips, partydex::Side::Sell, amount, min_out, icp_ledger, stable_ledger, true).await,
    }
}

/// Per-venue minimum trade floor, in 6-dec USD. ICPSwap keeps the existing
/// ~$0.01 floor (byte-identical to pre-PR2a behavior); PartyDEX rejects
/// trades below ~$0.10, so its leg needs a higher floor (decision #3).
fn min_trade_floor_usd(venue: state::Venue) -> u64 {
    match venue {
        state::Venue::Icpswap => 10_000,   // $0.01 — unchanged from pre-PR2a
        state::Venue::PartyDex => 100_000, // $0.10
    }
}

/// Extra ledger-fee cost (6-dec USD) of routing one leg through PartyDEX
/// instead of ICPSwap: PartyDEX's deposit/withdraw custody flow charges one
/// extra ledger transfer fee on the way in (deposit of the input token) and
/// one on the way out (withdraw of the output token) versus ICPSwap's single
/// depositFromAndSwap call (decision #2). Zero for the Icpswap venue, so
/// existing strategies' profit math is unaffected.
///
/// `icp_leg_effective_rate` prices the ICP-side extra ledger fee (`ICP_FEE`,
/// which is denominated in ICP, not USD) using the candidate's own realized
/// exchange rate (stable-out / icp-in, in native stable units per native ICP
/// unit) rather than a separate live ICP/USD price lookup — self-consistent
/// with the rest of each profit computation, which already works entirely in
/// each candidate's own quoted amounts.
fn partydex_extra_fee_usd(
    venue: state::Venue,
    stable_fee: u64,
    stable_decimals: u8,
    stable_vp: u64,
    icp_amount: u64,
    icp_leg_stable_amount: u64,
) -> i64 {
    if !matches!(venue, state::Venue::PartyDex) {
        return 0;
    }
    // Extra ledger fee on the stable-token leg of the deposit/withdraw pair.
    let stable_leg_fee_usd = stable_to_usd_6dec_vp(stable_fee, stable_decimals, stable_vp);
    // Extra ledger fee on the ICP leg: reprice ICP_FEE (denominated in ICP) into
    // native stable units using this candidate's own realized rate (stable/ICP),
    // then convert that stable-native amount to USD like any other stable amount.
    let icp_fee_in_stable_native = if icp_amount > 0 {
        (ICP_FEE as u128 * icp_leg_stable_amount as u128 / icp_amount as u128) as u64
    } else {
        0
    };
    let icp_leg_fee_usd = stable_to_usd_6dec_vp(icp_fee_in_stable_native, stable_decimals, stable_vp);
    stable_leg_fee_usd + icp_leg_fee_usd
}

thread_local! {
    static CYCLE_IN_PROGRESS: Cell<bool> = Cell::new(false);
    /// Per-cycle cache for balances and virtual price. Cleared at the start
    /// of each `run_arb_cycle`. Lets multiple strategies share inter-canister
    /// reads (3USD balance, ICP balance, virtual price) instead of refetching.
    /// Safe within a single cycle: dry-run computation is read-only and runs
    /// before any swap that would mutate balances.
    static CYCLE_BALANCE_CACHE: RefCell<HashMap<Principal, u64>> = RefCell::new(HashMap::new());
    static CYCLE_VP_CACHE: Cell<Option<u64>> = Cell::new(None);
}

pub fn is_cycle_in_progress() -> bool {
    CYCLE_IN_PROGRESS.with(|c| c.get())
}

/// Admin escape hatch for a wedged cycle lock. `CYCLE_IN_PROGRESS` is normally
/// released by a `Guard` Drop at the end of `run_arb_cycle` /
/// `run_specific_strategy`, but a wasm trap unwinds without running Drop — so a
/// trap after the flag commits would leave it stuck `true`, silently rejecting
/// every future cycle and manual strategy run until the next upgrade. Clears the
/// flag (and the per-cycle caches it guards) and reports the prior state.
pub fn force_clear_cycle_lock() -> bool {
    let was_locked = CYCLE_IN_PROGRESS.with(|c| c.replace(false));
    clear_cycle_cache();
    was_locked
}

fn clear_cycle_cache() {
    CYCLE_BALANCE_CACHE.with(|c| c.borrow_mut().clear());
    CYCLE_VP_CACHE.with(|c| c.set(None));
}

/// Cached balance fetch for the current arb cycle. First call hits the ledger;
/// subsequent calls within the same cycle return the cached value.
pub async fn fetch_balance_cached(ledger: Principal) -> Result<u64, String> {
    if let Some(v) = CYCLE_BALANCE_CACHE.with(|c| c.borrow().get(&ledger).copied()) {
        return Ok(v);
    }
    let v = fetch_balance(ledger).await?;
    CYCLE_BALANCE_CACHE.with(|c| { c.borrow_mut().insert(ledger, v); });
    Ok(v)
}

/// Cached virtual price fetch for the current arb cycle.
pub async fn fetch_virtual_price_cached(rumi_3pool: Principal) -> Result<u64, String> {
    if let Some(v) = CYCLE_VP_CACHE.with(|c| c.get()) {
        return Ok(v);
    }
    let v = prices::fetch_virtual_price(rumi_3pool).await?;
    CYCLE_VP_CACHE.with(|c| c.set(Some(v)));
    Ok(v)
}

// ─── Strategy S reference pricing helpers ───
//
// USD routing for Strategy S's band-edge legs (top-up and stranded-BOB skim)
// and its ICP↔USD reference mark. Candidate set intentionally mirrors design
// decision #4: every configured non-PartyDEX stable/ICP pool (ICPSwap
// ckUSDC/icUSD/ckUSDT, Rumi AMM VP-adjusted) — the same venues
// `drain_residual_icp` considers, MINUS the ICPSwap 3USD/ICP pool (a
// volume-bot-only venue redundant with Rumi AMM's 3USD/ICP quote). This
// deliberately duplicates `drain_residual_icp`'s per-pool decimal/VP math
// rather than refactoring it, per the behavior-preservation constraint on
// existing strategies A–R.

/// Quote every configured stable/ICP candidate pool for selling
/// `icp_amount_e8s`, returning (pool, usd_out_6dec) per successful quote.
/// Shared by the max- and median-based consumers below.
async fn stable_usd_per_icp_candidates(
    config: &state::BotConfig,
    icp_amount_e8s: u64,
) -> Vec<(state::Pool, u64)> {
    let vp = fetch_virtual_price_cached(config.rumi_3pool).await
        .unwrap_or(1_000_000_000_000_000_000);

    let has_icusd_pool = config.icpswap_icusd_pool != Principal::anonymous();
    let icusd_resolved = state::read_state(|s| s.icusd_token_ordering_resolved);
    let has_ckusdt_pool = config.icpswap_ckusdt_pool != Principal::anonymous();
    let ckusdt_resolved = state::read_state(|s| s.ckusdt_token_ordering_resolved);

    let rumi_fut = prices::fetch_rumi_quote_for_amount(config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, icp_amount_e8s);
    let ckusdc_fut = prices::fetch_icpswap_quote_for_amount(config.icpswap_pool, icp_amount_e8s, config.icpswap_icp_is_token0);
    let (rumi_res, ckusdc_res) = futures::future::join(rumi_fut, ckusdc_fut).await;

    let icusd_res: Result<u64, String> = if has_icusd_pool && icusd_resolved {
        prices::fetch_icpswap_quote_for_amount(config.icpswap_icusd_pool, icp_amount_e8s, config.icpswap_icusd_icp_is_token0).await
    } else {
        Err("icUSD pool unavailable".to_string())
    };
    let ckusdt_res: Result<u64, String> = if has_ckusdt_pool && ckusdt_resolved {
        prices::fetch_icpswap_quote_for_amount(config.icpswap_ckusdt_pool, icp_amount_e8s, config.icpswap_ckusdt_icp_is_token0).await
    } else {
        Err("ckUSDT pool unavailable".to_string())
    };

    // (pool, usd_out_6dec) — same per-pool decimal/VP math as drain_residual_icp's candidate block.
    let mut candidates: Vec<(state::Pool, u64)> = Vec::new();
    if let Ok(out_3usd) = rumi_res {
        // 3USD is 8 dec, worth VP/1e18 USD each.
        candidates.push((state::Pool::RumiThreeUsd, stable_to_usd_6dec_vp(out_3usd, 8, vp).max(0) as u64));
    }
    if let Ok(out_ck) = ckusdc_res {
        // ckUSDC is 6 dec ≈ $1.
        candidates.push((state::Pool::IcpswapCkusdc, stable_to_usd_6dec(out_ck, 6).max(0) as u64));
    }
    if let Ok(out_icusd) = icusd_res {
        // icUSD is 8 dec ≈ $1.
        candidates.push((state::Pool::IcpswapIcusd, stable_to_usd_6dec(out_icusd, 8).max(0) as u64));
    }
    if let Ok(out_ckusdt) = ckusdt_res {
        // ckUSDT is 6 dec ≈ $1.
        candidates.push((state::Pool::IcpswapCkusdt, stable_to_usd_6dec(out_ckusdt, 6).max(0) as u64));
    }
    candidates
}

/// Best USD-per-ICP quote across every configured stable/ICP pool, selling
/// `icp_amount_e8s`. Returns `None` if every candidate pool is unconfigured
/// or every quote call failed.
/// allow(dead_code): Strategy S's reference/marks moved to
/// `median_stable_usd_per_icp` (manipulation hardening); this max-based
/// variant remains available for best-execution routing.
#[allow(dead_code)]
pub struct StableQuote {
    pub pool: state::Pool,
    pub usd_out_6dec: u64,
    pub usd_per_icp_6dec: u64,
}

#[allow(dead_code)]
async fn best_stable_usd_per_icp(config: &state::BotConfig, icp_amount_e8s: u64) -> Option<StableQuote> {
    let candidates = stable_usd_per_icp_candidates(config, icp_amount_e8s).await;
    candidates.into_iter().max_by_key(|(_, usd_out)| *usd_out).map(|(pool, usd_out_6dec)| {
        let usd_per_icp_6dec = if icp_amount_e8s > 0 {
            (usd_out_6dec as u128 * 100_000_000 / icp_amount_e8s as u128) as u64
        } else {
            0
        };
        StableQuote { pool, usd_out_6dec, usd_per_icp_6dec }
    })
}

/// Median USD-per-ICP rate across the same candidate set (6-dec USD per
/// 1 ICP). Strategy S uses this — not the max — for its reference price and
/// USD marks: taking the best quote lets an attacker cheaply push a single
/// thin pool (e.g. the ~$3.5K-TVL icUSD/ICP pool) to become the outlier max
/// and skew the S reference; the median requires moving half the candidate
/// set. Even count → mean of the middle two; single candidate → itself;
/// degenerate zero-rate quotes are excluded.
pub(crate) async fn median_stable_usd_per_icp(config: &state::BotConfig, icp_amount_e8s: u64) -> Option<u64> {
    if icp_amount_e8s == 0 {
        return None;
    }
    let mut rates: Vec<u64> = stable_usd_per_icp_candidates(config, icp_amount_e8s).await
        .into_iter()
        .map(|(_, usd_out_6dec)| (usd_out_6dec as u128 * 100_000_000 / icp_amount_e8s as u128) as u64)
        .filter(|&r| r > 0)
        .collect();
    if rates.is_empty() {
        return None;
    }
    rates.sort_unstable();
    let n = rates.len();
    Some(if n % 2 == 1 {
        rates[n / 2]
    } else {
        // Mean of the middle two, without u64 overflow.
        ((rates[n / 2 - 1] as u128 + rates[n / 2] as u128) / 2) as u64
    })
}

/// Mirror of `best_stable_usd_per_icp` for the reverse direction: spend
/// `usd_6dec` worth of whichever configured stable gives the most ICP back.
/// Used by Strategy S's ICP inventory top-up leg (a later task).
pub struct TopUpQuote {
    pub pool: state::Pool,
    pub stable_in_amount: u64,
    pub icp_out_e8s: u64,
}

async fn best_stable_icp_per_usd(config: &state::BotConfig, usd_6dec: u64) -> Option<TopUpQuote> {
    let vp = fetch_virtual_price_cached(config.rumi_3pool).await
        .unwrap_or(1_000_000_000_000_000_000);

    let has_icusd_pool = config.icpswap_icusd_pool != Principal::anonymous();
    let icusd_resolved = state::read_state(|s| s.icusd_token_ordering_resolved);
    let has_ckusdt_pool = config.icpswap_ckusdt_pool != Principal::anonymous();
    let ckusdt_resolved = state::read_state(|s| s.ckusdt_token_ordering_resolved);

    let rumi_in = usd_6dec_to_stable_vp(usd_6dec, 8, vp);
    let ckusdc_in = usd_6dec_to_stable(usd_6dec, 6);
    let icusd_in = usd_6dec_to_stable(usd_6dec, 8);
    let ckusdt_in = usd_6dec_to_stable(usd_6dec, 6);

    let rumi_fut = prices::fetch_rumi_quote_for_amount(config.rumi_amm, RUMI_POOL_ID, config.three_usd_ledger, rumi_in);
    let ckusdc_fut = prices::fetch_icpswap_quote_for_amount(config.icpswap_pool, ckusdc_in, !config.icpswap_icp_is_token0);
    let (rumi_res, ckusdc_res) = futures::future::join(rumi_fut, ckusdc_fut).await;

    let icusd_res: Result<u64, String> = if has_icusd_pool && icusd_resolved {
        prices::fetch_icpswap_quote_for_amount(config.icpswap_icusd_pool, icusd_in, !config.icpswap_icusd_icp_is_token0).await
    } else {
        Err("icUSD pool unavailable".to_string())
    };
    let ckusdt_res: Result<u64, String> = if has_ckusdt_pool && ckusdt_resolved {
        prices::fetch_icpswap_quote_for_amount(config.icpswap_ckusdt_pool, ckusdt_in, !config.icpswap_ckusdt_icp_is_token0).await
    } else {
        Err("ckUSDT pool unavailable".to_string())
    };

    // (pool, stable_in_amount, icp_out_e8s)
    let mut candidates: Vec<(state::Pool, u64, u64)> = Vec::new();
    if let Ok(icp_out) = rumi_res {
        candidates.push((state::Pool::RumiThreeUsd, rumi_in, icp_out));
    }
    if let Ok(icp_out) = ckusdc_res {
        candidates.push((state::Pool::IcpswapCkusdc, ckusdc_in, icp_out));
    }
    if let Ok(icp_out) = icusd_res {
        candidates.push((state::Pool::IcpswapIcusd, icusd_in, icp_out));
    }
    if let Ok(icp_out) = ckusdt_res {
        candidates.push((state::Pool::IcpswapCkusdt, ckusdt_in, icp_out));
    }

    candidates.into_iter().max_by_key(|(_, _, icp_out)| *icp_out)
        .map(|(pool, stable_in_amount, icp_out_e8s)| TopUpQuote { pool, stable_in_amount, icp_out_e8s })
}

/// USD mark (6-dec) for an ICP-denominated leg, given a USD-per-ICP rate
/// (6-dec USD per 1 ICP, i.e. per 100_000_000 e8s). Used to book Strategy S's
/// ICP legs at the reference USD quote fetched at trade time (design decision
/// #3) — unlike strategies A–R, which keep `sold_usd_value = 0` for ICP legs.
fn mark_icp_usd(icp_e8s: u64, usd_per_icp_6dec: u64) -> i64 {
    (icp_e8s as u128 * usd_per_icp_6dec as u128 / 100_000_000) as i64
}

// ─── Dry Run Result ───

#[derive(CandidType, Clone, Debug)]
pub struct DryRunResult {
    pub should_trade: bool,
    pub direction: Option<Direction>,
    pub spread_bps: i32,
    pub rumi_price_usd: u64,      // 6 decimals
    pub icpswap_price_usd: u64,   // 6 decimals
    pub virtual_price: u64,
    pub optimal_input_amount: u64,
    pub optimal_input_token: Option<Token>,
    pub expected_icp_amount: u64,
    pub expected_output_amount: u64,
    pub expected_output_token: Option<Token>,
    pub expected_profit_usd: i64, // 6 decimals
    pub candidates_evaluated: Vec<CandidateResult>,
    pub balance_3usd: u64,
    pub balance_ckusdc: u64,
    pub message: String,
}

#[derive(CandidType, Clone, Debug)]
pub struct CandidateResult {
    pub input_amount: u64,
    pub icp_amount: u64,
    pub output_amount: u64,
    pub profit_usd: i64, // 6 decimals
}

impl Default for DryRunResult {
    fn default() -> Self {
        Self {
            should_trade: false,
            direction: None,
            spread_bps: 0,
            rumi_price_usd: 0,
            icpswap_price_usd: 0,
            virtual_price: 0,
            optimal_input_amount: 0,
            optimal_input_token: None,
            expected_icp_amount: 0,
            expected_output_amount: 0,
            expected_output_token: None,
            expected_profit_usd: 0,
            candidates_evaluated: Vec::new(),
            balance_3usd: 0,
            balance_ckusdc: 0,
            message: String::new(),
        }
    }
}

// ─── Main Arb Cycle ───

pub async fn run_arb_cycle() {
    let already_running = CYCLE_IN_PROGRESS.with(|c| {
        if c.get() { true } else { c.set(true); false }
    });
    if already_running { return; }

    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            CYCLE_IN_PROGRESS.with(|c| c.set(false));
            clear_cycle_cache();
        }
    }
    let _guard = Guard;
    clear_cycle_cache();

    // Resolve ICPSwap token ordering on first cycle (Strategy A: ckUSDC/ICP pool)
    let resolved = state::read_state(|s| s.token_ordering_resolved);
    if !resolved {
        let (icpswap_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(icpswap_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_icp_is_token0 = icp_is_token0;
                    s.token_ordering_resolved = true;
                });
            }
            Err(e) => {
                log_error(&format!("Failed to resolve token ordering: {}. Retrying.", e));
                return;
            }
        }
    }

    // Resolve ICPSwap token ordering for Strategy B: icUSD/ICP pool
    let (icusd_resolved, has_icusd_pool) = state::read_state(|s| {
        (s.icusd_token_ordering_resolved, s.config.icpswap_icusd_pool != Principal::anonymous())
    });
    if has_icusd_pool && !icusd_resolved {
        let (icusd_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_icusd_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(icusd_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_icusd_icp_is_token0 = icp_is_token0;
                    s.icusd_token_ordering_resolved = true;
                });
            }
            Err(e) => {
                log_error(&format!("Failed to resolve icUSD pool token ordering: {}. Retrying.", e));
                // Don't return — Strategy A can still run
            }
        }
    }

    // Resolve ICPSwap token ordering for Strategy C: ckUSDT/ICP pool
    let (ckusdt_resolved, has_ckusdt_pool) = state::read_state(|s| {
        (s.ckusdt_token_ordering_resolved, s.config.icpswap_ckusdt_pool != Principal::anonymous())
    });
    if has_ckusdt_pool && !ckusdt_resolved {
        let (ckusdt_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_ckusdt_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(ckusdt_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_ckusdt_icp_is_token0 = icp_is_token0;
                    s.ckusdt_token_ordering_resolved = true;
                });
            }
            Err(e) => {
                log_error(&format!("Failed to resolve ckUSDT pool token ordering: {}. Retrying.", e));
                // Don't return — other strategies can still run
            }
        }
    }

    // Resolve ICPSwap token ordering for the 3USD/ICP pool (shared by the
    // volume bot and the residual-ICP drain). No arb strategy uses it, but
    // the drain's 3USD candidate needs the ordering resolved to quote it.
    let (three_usd_resolved, has_3usd_pool) = state::read_state(|s| {
        (s.icpswap_3usd_token_ordering_resolved, s.config.icpswap_3usd_pool != Principal::anonymous())
    });
    if has_3usd_pool && !three_usd_resolved {
        let (three_usd_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_3usd_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(three_usd_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_3usd_icp_is_token0 = icp_is_token0;
                    s.icpswap_3usd_token_ordering_resolved = true;
                });
            }
            Err(e) => {
                log_error(&format!("Failed to resolve 3USD ICPSwap pool token ordering: {}. Retrying.", e));
                // Don't return — other strategies can still run
            }
        }
    }

    // Resolve ICPSwap token ordering for Strategy S: BOB/ICP pool (probed
    // with icp_ledger — this pool's other leg is ICP).
    let (bob_icp_resolved, has_bob_icp_pool) = state::read_state(|s| {
        (s.bob_icp_ordering_resolved, s.config.icpswap_bob_icp_pool != Principal::anonymous())
    });
    if has_bob_icp_pool && !bob_icp_resolved {
        let (bob_icp_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_bob_icp_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(bob_icp_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_bob_icp_icp_is_token0 = icp_is_token0;
                    s.bob_icp_ordering_resolved = true;
                });
            }
            Err(e) => {
                log_error(&format!("Failed to resolve BOB/ICP pool token ordering: {}. Retrying.", e));
                // Don't return — other strategies can still run
            }
        }
    }

    // Resolve ICPSwap token ordering for Strategy S: icUSD/BOB pool (probed
    // with icusd_ledger — this pool's other leg is icUSD, not ICP).
    let (icusd_bob_resolved, has_icusd_bob_pool) = state::read_state(|s| {
        (s.icusd_bob_ordering_resolved, s.config.icpswap_icusd_bob_pool != Principal::anonymous())
    });
    if has_icusd_bob_pool && !icusd_bob_resolved {
        let (icusd_bob_pool, icusd_ledger) = state::read_state(|s| (s.config.icpswap_icusd_bob_pool, s.config.icusd_ledger));
        match prices::fetch_icpswap_token_ordering(icusd_bob_pool, icusd_ledger).await {
            Ok(icusd_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_icusd_bob_icusd_is_token0 = icusd_is_token0;
                    s.icusd_bob_ordering_resolved = true;
                });
            }
            Err(e) => {
                log_error(&format!("Failed to resolve icUSD/BOB pool token ordering: {}. Retrying.", e));
                // Don't return — other strategies can still run
            }
        }
    }

    let config = state::read_state(|s| s.config.clone());
    if config.paused { return; }

    if let Err(e) = drain_residual_icp(&config).await {
        log_error(&format!("Drain residual ICP failed: {}", e));
    }

    // Strategy S stranded-BOB recovery. Recovery of the bot's OWN stranded
    // BOB (pending_bob_exit set) must always run — but sweeping loose BOB
    // while the strategy is disabled would move funds the bot didn't acquire,
    // which is surprising in dry-run mode. So beyond a pending exit, the
    // drain only runs once S is fully live (pool configured AND execution
    // enabled). An inert deploy therefore adds zero standing balance queries.
    let bob_drain_relevant = state::read_state(|s| s.pending_bob_exit.is_some())
        || (config.icpswap_icusd_bob_pool != Principal::anonymous() && config.bob_execution_enabled);
    if bob_drain_relevant {
        if let Err(e) = drain_residual_bob(&config).await {
            log_error(&format!("Drain residual BOB failed: {}", e));
        }
    }

    // Build IcpswapTarget for strategy A (ckUSDC/ICP) — always present
    let target_a = IcpswapTarget {
        pool: config.icpswap_pool,
        icp_is_token0: config.icpswap_icp_is_token0,
        label: "ICPSwap",
        strategy_tag: "A",
        stable_token_name: "ckUSDC",
        stable_fee: CKUSDC_FEE,
        stable_ledger: config.ckusdc_ledger,
        pool_enum: state::Pool::IcpswapCkusdc,
        stable_decimals: 6,
        uses_vp: false,
        venue: state::Venue::Icpswap,
        fee_pips: 0,
    };

    // Rumi AMM (3USD/ICP) kill switch — Strategies A/C/D/Q/R all trade
    // against `config.rumi_amm`; skip them with zero network calls while
    // this is set (e.g. Rumi pool liquidity constraints).
    let rumi_amm_paused = config.rumi_amm_paused;

    // Evaluate Strategy A (Rumi vs ICPSwap ckUSDC/ICP)
    let dry_run_a = if !rumi_amm_paused {
        match compute_optimal_trade(&config, &target_a).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy A computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Build cross-pool targets for B and F
    let icusd_resolved = state::read_state(|s| s.icusd_token_ordering_resolved);
    let icusd_side = CrossPoolSide {
        pool: config.icpswap_icusd_pool,
        icp_is_token0: config.icpswap_icusd_icp_is_token0,
        stable_token_name: "icUSD",
        stable_fee: ICUSD_FEE,
        stable_ledger: config.icusd_ledger,
        stable_decimals: 8,
        pool_enum: state::Pool::IcpswapIcusd,
        dex_label: "ICPSwap-icUSD",
        uses_vp: false,
        venue: state::Venue::Icpswap,
        fee_pips: 0,
    };
    let ckusdc_side = CrossPoolSide {
        pool: config.icpswap_pool,
        icp_is_token0: config.icpswap_icp_is_token0,
        stable_token_name: "ckUSDC",
        stable_fee: CKUSDC_FEE,
        stable_ledger: config.ckusdc_ledger,
        stable_decimals: 6,
        pool_enum: state::Pool::IcpswapCkusdc,
        dex_label: "ICPSwap",
        uses_vp: false,
        venue: state::Venue::Icpswap,
        fee_pips: 0,
    };
    let ckusdt_side = CrossPoolSide {
        pool: config.icpswap_ckusdt_pool,
        icp_is_token0: config.icpswap_ckusdt_icp_is_token0,
        stable_token_name: "ckUSDT",
        stable_fee: CKUSDT_FEE,
        stable_ledger: config.ckusdt_ledger,
        stable_decimals: 6,
        pool_enum: state::Pool::IcpswapCkusdt,
        dex_label: "ICPSwap-ckUSDT",
        uses_vp: false,
        venue: state::Venue::Icpswap,
        fee_pips: 0,
    };
    let cross_b = CrossPoolTarget { strategy_tag: "B", buy_side: icusd_side, sell_side: ckusdc_side };
    let cross_f = CrossPoolTarget { strategy_tag: "F", buy_side: icusd_side, sell_side: ckusdt_side };

    // PartyDEX sides for strategies K–R (PR2b). icp_is_token0 is irrelevant for
    // PartyDex (ICP is always `base`), set true per plan.
    let partydex_ckusdc_side = CrossPoolSide {
        pool: config.partydex_ckusdc_pool,
        icp_is_token0: true,
        stable_token_name: "ckUSDC",
        stable_fee: CKUSDC_FEE,
        stable_ledger: config.ckusdc_ledger,
        stable_decimals: 6,
        pool_enum: state::Pool::PartyDexIcpCkusdc,
        dex_label: "PartyDEX-ckUSDC",
        uses_vp: false,
        venue: state::Venue::PartyDex,
        fee_pips: config.partydex_ckusdc_fee_pips,
    };
    let partydex_ckusdt_side = CrossPoolSide {
        pool: config.partydex_ckusdt_pool,
        icp_is_token0: true,
        stable_token_name: "ckUSDT",
        stable_fee: CKUSDT_FEE,
        stable_ledger: config.ckusdt_ledger,
        stable_decimals: 6,
        pool_enum: state::Pool::PartyDexIcpCkusdt,
        dex_label: "PartyDEX-ckUSDT",
        uses_vp: false,
        venue: state::Venue::PartyDex,
        fee_pips: config.partydex_ckusdt_fee_pips,
    };
    let cross_k = CrossPoolTarget { strategy_tag: "K", buy_side: partydex_ckusdc_side, sell_side: ckusdc_side };
    let cross_l = CrossPoolTarget { strategy_tag: "L", buy_side: partydex_ckusdc_side, sell_side: ckusdt_side };
    let cross_m = CrossPoolTarget { strategy_tag: "M", buy_side: partydex_ckusdc_side, sell_side: icusd_side };
    let cross_n = CrossPoolTarget { strategy_tag: "N", buy_side: partydex_ckusdt_side, sell_side: ckusdc_side };
    let cross_o = CrossPoolTarget { strategy_tag: "O", buy_side: partydex_ckusdt_side, sell_side: ckusdt_side };
    let cross_p = CrossPoolTarget { strategy_tag: "P", buy_side: partydex_ckusdt_side, sell_side: icusd_side };

    // IcpswapTarget-shaped Rumi-vs-PartyDEX targets for strategies Q/R.
    let target_q = IcpswapTarget {
        pool: config.partydex_ckusdc_pool,
        icp_is_token0: true,
        label: "PartyDEX-ckUSDC",
        strategy_tag: "Q",
        stable_token_name: "ckUSDC",
        stable_fee: CKUSDC_FEE,
        stable_ledger: config.ckusdc_ledger,
        pool_enum: state::Pool::PartyDexIcpCkusdc,
        stable_decimals: 6,
        uses_vp: false,
        venue: state::Venue::PartyDex,
        fee_pips: config.partydex_ckusdc_fee_pips,
    };
    let target_r = IcpswapTarget {
        pool: config.partydex_ckusdt_pool,
        icp_is_token0: true,
        label: "PartyDEX-ckUSDT",
        strategy_tag: "R",
        stable_token_name: "ckUSDT",
        stable_fee: CKUSDT_FEE,
        stable_ledger: config.ckusdt_ledger,
        pool_enum: state::Pool::PartyDexIcpCkusdt,
        stable_decimals: 6,
        uses_vp: false,
        venue: state::Venue::PartyDex,
        fee_pips: config.partydex_ckusdt_fee_pips,
    };
    let has_partydex_ckusdc = config.partydex_ckusdc_pool != Principal::anonymous();
    let has_partydex_ckusdt = config.partydex_ckusdt_pool != Principal::anonymous();

    // Evaluate Strategy B (ICPSwap icUSD/ICP vs ICPSwap ckUSDC/ICP)
    let dry_run_b = if has_icusd_pool && icusd_resolved {
        match compute_optimal_cross_pool_trade(&config, &cross_b).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy B computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Evaluate Strategy C (Rumi vs ICPSwap ckUSDT/ICP) — same shape as A
    let ckusdt_resolved = state::read_state(|s| s.ckusdt_token_ordering_resolved);
    let target_c = IcpswapTarget {
        pool: config.icpswap_ckusdt_pool,
        icp_is_token0: config.icpswap_ckusdt_icp_is_token0,
        label: "ICPSwap-ckUSDT",
        strategy_tag: "C",
        stable_token_name: "ckUSDT",
        stable_fee: CKUSDT_FEE,
        stable_ledger: config.ckusdt_ledger,
        pool_enum: state::Pool::IcpswapCkusdt,
        stable_decimals: 6,
        uses_vp: false,
        venue: state::Venue::Icpswap,
        fee_pips: 0,
    };
    let dry_run_c = if has_ckusdt_pool && ckusdt_resolved && !rumi_amm_paused {
        match compute_optimal_trade(&config, &target_c).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy C computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Evaluate Strategy D (Rumi vs ICPSwap icUSD/ICP) — same shape as A but 8-dec stable
    let target_d = IcpswapTarget {
        pool: config.icpswap_icusd_pool,
        icp_is_token0: config.icpswap_icusd_icp_is_token0,
        label: "ICPSwap-icUSD",
        strategy_tag: "D",
        stable_token_name: "icUSD",
        stable_fee: ICUSD_FEE,
        stable_ledger: config.icusd_ledger,
        pool_enum: state::Pool::IcpswapIcusd,
        stable_decimals: 8,
        uses_vp: false,
        venue: state::Venue::Icpswap,
        fee_pips: 0,
    };
    let dry_run_d = if has_icusd_pool && icusd_resolved && !rumi_amm_paused {
        match compute_optimal_trade(&config, &target_d).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy D computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Evaluate Strategy F (ICPSwap icUSD/ICP vs ICPSwap ckUSDT/ICP)
    let dry_run_f = if has_icusd_pool && icusd_resolved && has_ckusdt_pool && ckusdt_resolved {
        match compute_optimal_cross_pool_trade(&config, &cross_f).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy F computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Evaluate Strategy K (PartyDEX ckUSDC vs ICPSwap ckUSDC/ICP). icpswap_pool's
    // ordering is guaranteed resolved by this point (the cycle returned above if
    // resolution failed), so only the PartyDEX pool needs a presence guard.
    let dry_run_k = if has_partydex_ckusdc {
        match compute_optimal_cross_pool_trade(&config, &cross_k).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy K computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Evaluate Strategy L (PartyDEX ckUSDC vs ICPSwap ckUSDT/ICP)
    let dry_run_l = if has_partydex_ckusdc && has_ckusdt_pool && ckusdt_resolved {
        match compute_optimal_cross_pool_trade(&config, &cross_l).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy L computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Evaluate Strategy M (PartyDEX ckUSDC vs ICPSwap icUSD/ICP)
    let dry_run_m = if has_partydex_ckusdc && has_icusd_pool && icusd_resolved {
        match compute_optimal_cross_pool_trade(&config, &cross_m).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy M computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Evaluate Strategy N (PartyDEX ckUSDT vs ICPSwap ckUSDC/ICP)
    let dry_run_n = if has_partydex_ckusdt {
        match compute_optimal_cross_pool_trade(&config, &cross_n).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy N computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Evaluate Strategy O (PartyDEX ckUSDT vs ICPSwap ckUSDT/ICP)
    let dry_run_o = if has_partydex_ckusdt && has_ckusdt_pool && ckusdt_resolved {
        match compute_optimal_cross_pool_trade(&config, &cross_o).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy O computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Evaluate Strategy P (PartyDEX ckUSDT vs ICPSwap icUSD/ICP)
    let dry_run_p = if has_partydex_ckusdt && has_icusd_pool && icusd_resolved {
        match compute_optimal_cross_pool_trade(&config, &cross_p).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy P computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Evaluate Strategy Q (Rumi vs PartyDEX ckUSDC) — same shape as A/C/D
    let dry_run_q = if has_partydex_ckusdc && !rumi_amm_paused {
        match compute_optimal_trade(&config, &target_q).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy Q computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Evaluate Strategy R (Rumi vs PartyDEX ckUSDT) — same shape as A/C/D
    let dry_run_r = if has_partydex_ckusdt && !rumi_amm_paused {
        match compute_optimal_trade(&config, &target_r).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy R computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Evaluate Strategy S (icUSD/BOB triangular). Dry-run + snapshot whenever
    // both BOB pools are configured and their orderings resolved; joining the
    // best-strategy selection additionally requires bob_execution_enabled
    // (checked at the profit collection below) — dry-run-first, spec §5.
    let (bob_icp_resolved, icusd_bob_resolved) = state::read_state(|s| {
        (s.bob_icp_ordering_resolved, s.icusd_bob_ordering_resolved)
    });
    let has_bob = config.icpswap_icusd_bob_pool != Principal::anonymous()
        && config.icpswap_bob_icp_pool != Principal::anonymous()
        && bob_icp_resolved
        && icusd_bob_resolved;
    let target_s = BobTarget {
        icusd_bob_pool: config.icpswap_icusd_bob_pool,
        icusd_is_token0: config.icpswap_icusd_bob_icusd_is_token0,
        bob_icp_pool: config.icpswap_bob_icp_pool,
        bob_icp_icp_is_token0: config.icpswap_bob_icp_icp_is_token0,
        bob_ledger: config.bob_ledger,
        bob_fee: config.bob_ledger_fee,
        icusd_ledger: config.icusd_ledger,
        icusd_fee: ICUSD_FEE,
        icp_ledger: config.icp_ledger,
    };
    let dry_run_s = if has_bob {
        match find_optimal_bob(&config, &target_s).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy S computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Fetch extra balances for snapshot (ICP, icUSD, ckUSDT, BOB) — dry runs already have 3USD/ckUSDC/ckUSDT.
    // Still need ICP and icUSD here; ckUSDT balance falls back to dry_run_c if available.
    // BOB is only queried once Strategy S is live (zero standing cost while inert).
    let ckusdt_fallback_ledger = if config.ckusdt_ledger != Principal::anonymous() {
        config.ckusdt_ledger
    } else {
        candid::Principal::from_text("cngnf-vqaaa-aaaar-qag4q-cai").unwrap()
    };
    let (bal_icp, bal_icusd, bal_ckusdt, bal_bob) = futures::future::join4(
        fetch_balance_cached(config.icp_ledger),
        async {
            if has_icusd_pool { fetch_balance_cached(config.icusd_ledger).await } else { Ok(0) }
        },
        fetch_balance_cached(ckusdt_fallback_ledger),
        async {
            if has_bob { fetch_balance_cached(config.bob_ledger).await } else { Ok(0) }
        },
    ).await;

    // Build snapshot from dry run data
    let mut snapshot = state::CycleSnapshot {
        timestamp: ic_cdk::api::time(),
        // Recover 3USD/ICP (8 dec) from USD price: usd_6dec * 100 * 1e18 / vp
        rumi_icp_price_3usd: dry_run_a.as_ref().map(|d| {
            if d.virtual_price > 0 {
                (d.rumi_price_usd as u128 * 100 * VP_PRECISION / d.virtual_price as u128) as u64
            } else { 0 }
        }).unwrap_or(0),
        rumi_icp_price_usd: dry_run_a.as_ref().map(|d| d.rumi_price_usd).unwrap_or(0),
        icpswap_icp_price_ckusdc: dry_run_a.as_ref().map(|d| d.icpswap_price_usd).filter(|&v| v > 0)
            .or_else(|| dry_run_k.as_ref().map(|d| d.icpswap_price_usd).filter(|&v| v > 0))
            .or_else(|| dry_run_n.as_ref().map(|d| d.icpswap_price_usd).filter(|&v| v > 0))
            .or_else(|| dry_run_b.as_ref().map(|d| d.icpswap_price_usd).filter(|&v| v > 0))
            .unwrap_or(0),
        virtual_price: dry_run_a.as_ref().map(|d| d.virtual_price).unwrap_or(0),
        spread_a_bps: dry_run_a.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        // icUSD/ICP native (8 dec): dry_run_b stores 6-dec USD, multiply by 100
        icpswap_icp_price_icusd: dry_run_b.as_ref().map(|d| d.rumi_price_usd * 100).unwrap_or(0),
        spread_b_bps: dry_run_b.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        balance_icp: bal_icp.unwrap_or(0),
        balance_3usd: dry_run_a.as_ref().map(|d| d.balance_3usd).unwrap_or(0),
        balance_ckusdc: dry_run_a.as_ref().map(|d| d.balance_ckusdc).unwrap_or(0),
        balance_ckusdt: bal_ckusdt.unwrap_or(0),
        balance_icusd: bal_icusd.unwrap_or(0),
        icpswap_icp_price_ckusdt: dry_run_c.as_ref().map(|d| d.icpswap_price_usd).filter(|&v| v > 0)
            .or_else(|| dry_run_l.as_ref().map(|d| d.icpswap_price_usd).filter(|&v| v > 0))
            .or_else(|| dry_run_o.as_ref().map(|d| d.icpswap_price_usd).filter(|&v| v > 0))
            .or_else(|| dry_run_f.as_ref().map(|d| d.icpswap_price_usd).filter(|&v| v > 0))
            .unwrap_or(0),
        spread_c_bps: dry_run_c.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        partydex_icp_price_ckusdc: dry_run_k.as_ref().map(|d| d.rumi_price_usd).filter(|&v| v > 0)
            .or_else(|| dry_run_l.as_ref().map(|d| d.rumi_price_usd).filter(|&v| v > 0))
            .or_else(|| dry_run_m.as_ref().map(|d| d.rumi_price_usd).filter(|&v| v > 0))
            .or_else(|| dry_run_q.as_ref().map(|d| d.icpswap_price_usd).filter(|&v| v > 0))
            .unwrap_or(0),
        partydex_icp_price_ckusdt: dry_run_n.as_ref().map(|d| d.rumi_price_usd).filter(|&v| v > 0)
            .or_else(|| dry_run_o.as_ref().map(|d| d.rumi_price_usd).filter(|&v| v > 0))
            .or_else(|| dry_run_p.as_ref().map(|d| d.rumi_price_usd).filter(|&v| v > 0))
            .or_else(|| dry_run_r.as_ref().map(|d| d.icpswap_price_usd).filter(|&v| v > 0))
            .unwrap_or(0),
        spread_d_bps: dry_run_d.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        spread_f_bps: dry_run_f.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        spread_k_bps: dry_run_k.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        spread_l_bps: dry_run_l.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        spread_m_bps: dry_run_m.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        spread_n_bps: dry_run_n.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        spread_o_bps: dry_run_o.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        spread_p_bps: dry_run_p.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        spread_q_bps: dry_run_q.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        spread_r_bps: dry_run_r.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        bob_pool_price_icusd_per_bob: dry_run_s.as_ref().map(|d| d.pool_price_icusd_per_bob_8dec).unwrap_or(0),
        bob_ref_price_icusd_per_bob: dry_run_s.as_ref().map(|d| d.ref_price_icusd_per_bob_8dec).unwrap_or(0),
        spread_s_bps: dry_run_s.as_ref().map(|d| d.spread_bps as i64).unwrap_or(0),
        balance_bob: bal_bob.unwrap_or(0),
        traded: false,
        strategy_used: String::new(),
    };

    // Pick the best strategy across all
    let profit_a = dry_run_a.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_b = dry_run_b.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_c = dry_run_c.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_d = dry_run_d.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_f = dry_run_f.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_k = dry_run_k.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_l = dry_run_l.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_m = dry_run_m.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_n = dry_run_n.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_o = dry_run_o.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_p = dry_run_p.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_q = dry_run_q.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_r = dry_run_r.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    // Strategy S joins the selection ONLY when execution is enabled; while
    // bob_execution_enabled is false it stays dry-run log + snapshot only.
    let profit_s = if config.bob_execution_enabled {
        dry_run_s.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0)
    } else {
        0
    };

    let all_profits = [
        profit_a, profit_b, profit_c, profit_d, profit_f,
        profit_k, profit_l, profit_m, profit_n, profit_o, profit_p, profit_q, profit_r,
        profit_s,
    ];
    if all_profits.iter().all(|&p| p <= 0) {
        // Nothing profitable
        let mut parts: Vec<String> = Vec::new();
        if let Some(a) = &dry_run_a { parts.push(format!("A: {}", a.message)); }
        if let Some(b) = &dry_run_b { parts.push(format!("B: {}", b.message)); }
        if let Some(c) = &dry_run_c { parts.push(format!("C: {}", c.message)); }
        if let Some(d) = &dry_run_d { parts.push(format!("D: {}", d.message)); }
        if let Some(f) = &dry_run_f { parts.push(format!("F: {}", f.message)); }
        if let Some(k) = &dry_run_k { parts.push(format!("K: {}", k.message)); }
        if let Some(l) = &dry_run_l { parts.push(format!("L: {}", l.message)); }
        if let Some(m) = &dry_run_m { parts.push(format!("M: {}", m.message)); }
        if let Some(n) = &dry_run_n { parts.push(format!("N: {}", n.message)); }
        if let Some(o) = &dry_run_o { parts.push(format!("O: {}", o.message)); }
        if let Some(p) = &dry_run_p { parts.push(format!("P: {}", p.message)); }
        if let Some(q) = &dry_run_q { parts.push(format!("Q: {}", q.message)); }
        if let Some(r) = &dry_run_r { parts.push(format!("R: {}", r.message)); }
        let msg = if parts.is_empty() { "All strategies failed".to_string() } else { parts.join(" | ") };
        state::log_activity("arb_skip", &msg);
        state::append_snapshot(snapshot);
        return;
    }

    // Check minimum profit threshold against the best
    let best_profit = *all_profits.iter().max().unwrap();
    if config.min_profit_usd > 0 && best_profit < config.min_profit_usd {
        let msg = format!("Best profit ${:.2} < min ${:.2}",
            best_profit as f64 / 1e6, config.min_profit_usd as f64 / 1e6);
        state::log_activity("arb_skip", &msg);
        state::append_snapshot(snapshot);
        return;
    }

    // Dispatch to the strategy with the highest profit
    let profits = [
        ("A", profit_a), ("B", profit_b), ("C", profit_c),
        ("D", profit_d), ("F", profit_f),
        ("K", profit_k), ("L", profit_l), ("M", profit_m),
        ("N", profit_n), ("O", profit_o), ("P", profit_p),
        ("Q", profit_q), ("R", profit_r), ("S", profit_s),
    ];
    let winner = profits.iter()
        .max_by_key(|(_, p)| *p)
        .map(|(tag, _)| *tag)
        .unwrap();
    snapshot.traded = true;
    snapshot.strategy_used = winner.to_string();
    let log_start = |tag: &str, dr: &DryRunResult| {
        state::log_activity("arb_start", &format!(
            "[{}] Starting {:?} trade: {} {:?} → est {} ICP → est {} {:?} (spread: {} bps, est profit: {})",
            tag,
            dr.direction.as_ref().unwrap(),
            dr.optimal_input_amount,
            dr.optimal_input_token.as_ref().unwrap(),
            dr.expected_icp_amount,
            dr.expected_output_amount,
            dr.expected_output_token.as_ref().unwrap(),
            dr.spread_bps,
            dr.expected_profit_usd,
        ));
    };
    match winner {
        "A" => {
            let dry_run = dry_run_a.unwrap();
            log_start("A", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_rumi_to_icpswap(&config, &target_a, &dry_run).await,
                Direction::IcpswapToRumi => execute_icpswap_to_rumi(&config, &target_a, &dry_run).await,
            }
        }
        "C" => {
            let dry_run = dry_run_c.unwrap();
            log_start("C", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_rumi_to_icpswap(&config, &target_c, &dry_run).await,
                Direction::IcpswapToRumi => execute_icpswap_to_rumi(&config, &target_c, &dry_run).await,
            }
        }
        "D" => {
            let dry_run = dry_run_d.unwrap();
            log_start("D", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_rumi_to_icpswap(&config, &target_d, &dry_run).await,
                Direction::IcpswapToRumi => execute_icpswap_to_rumi(&config, &target_d, &dry_run).await,
            }
        }
        "B" => {
            let dry_run = dry_run_b.unwrap();
            log_start("B", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_cross_pool_forward(&cross_b, &dry_run).await,
                Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross_b, &dry_run).await,
            }
        }
        "F" => {
            let dry_run = dry_run_f.unwrap();
            log_start("F", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_cross_pool_forward(&cross_f, &dry_run).await,
                Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross_f, &dry_run).await,
            }
        }
        "K" => {
            let dry_run = dry_run_k.unwrap();
            log_start("K", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_cross_pool_forward(&cross_k, &dry_run).await,
                Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross_k, &dry_run).await,
            }
        }
        "L" => {
            let dry_run = dry_run_l.unwrap();
            log_start("L", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_cross_pool_forward(&cross_l, &dry_run).await,
                Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross_l, &dry_run).await,
            }
        }
        "M" => {
            let dry_run = dry_run_m.unwrap();
            log_start("M", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_cross_pool_forward(&cross_m, &dry_run).await,
                Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross_m, &dry_run).await,
            }
        }
        "N" => {
            let dry_run = dry_run_n.unwrap();
            log_start("N", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_cross_pool_forward(&cross_n, &dry_run).await,
                Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross_n, &dry_run).await,
            }
        }
        "O" => {
            let dry_run = dry_run_o.unwrap();
            log_start("O", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_cross_pool_forward(&cross_o, &dry_run).await,
                Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross_o, &dry_run).await,
            }
        }
        "P" => {
            let dry_run = dry_run_p.unwrap();
            log_start("P", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_cross_pool_forward(&cross_p, &dry_run).await,
                Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross_p, &dry_run).await,
            }
        }
        "Q" => {
            let dry_run = dry_run_q.unwrap();
            log_start("Q", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_rumi_to_icpswap(&config, &target_q, &dry_run).await,
                Direction::IcpswapToRumi => execute_icpswap_to_rumi(&config, &target_q, &dry_run).await,
            }
        }
        "S" => {
            // Winner "S" implies bob_execution_enabled, should_trade, and a
            // set direction (profit_s stays 0 otherwise). BobDryRun has its
            // own shape, so it doesn't go through log_start.
            let dry_run = dry_run_s.unwrap();
            let (in_tok, out_tok) = match dry_run.direction.unwrap() {
                BobDirection::Forward => ("icUSD", "ICP"),
                BobDirection::Reverse => ("ICP", "icUSD"),
            };
            state::log_activity("arb_start", &format!(
                "[S] Starting {:?} trade: {} {} → est {} BOB → est {} {} (spread: {} bps, est profit: {})",
                dry_run.direction.unwrap(), dry_run.input_amount, in_tok,
                dry_run.bob_amount, dry_run.output_amount, out_tok,
                dry_run.spread_bps, dry_run.expected_profit_usd,
            ));
            execute_bob(&config, &target_s, &dry_run).await;
        }
        _ => {
            let dry_run = dry_run_r.unwrap();
            log_start("R", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_rumi_to_icpswap(&config, &target_r, &dry_run).await,
                Direction::IcpswapToRumi => execute_icpswap_to_rumi(&config, &target_r, &dry_run).await,
            }
        }
    }

    // Record snapshot after trade execution
    state::append_snapshot(snapshot);
}

/// Force-execute a specific strategy by tag ("A", "B", "C", "D", "F").
/// Skips profit-vs-other-strategies comparison; executes whatever the computation returns
/// as long as there is a valid direction. Bypasses `should_trade` threshold check.
pub async fn run_specific_strategy(strategy_tag: &str) {
    let already_running = CYCLE_IN_PROGRESS.with(|c| {
        if c.get() { true } else { c.set(true); false }
    });
    if already_running {
        state::log_activity("arb_skip", &format!("[{}] Cycle already in progress", strategy_tag));
        return;
    }

    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            CYCLE_IN_PROGRESS.with(|c| c.set(false));
            clear_cycle_cache();
        }
    }
    let _guard = Guard;
    clear_cycle_cache();

    let config = state::read_state(|s| s.config.clone());
    if config.paused {
        state::log_activity("arb_skip", &format!("[{}] Bot is paused", strategy_tag));
        return;
    }
    if config.rumi_amm_paused && matches!(strategy_tag, "A" | "C" | "D" | "Q" | "R") {
        state::log_activity("arb_skip", &format!("[{}] Rumi AMM paused (liquidity constraints)", strategy_tag));
        return;
    }

    match strategy_tag {
        "A" => {
            let resolved = state::read_state(|s| s.token_ordering_resolved);
            if !resolved {
                let (pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_pool, s.config.icp_ledger));
                match prices::fetch_icpswap_token_ordering(pool, icp_ledger).await {
                    Ok(icp_is_token0) => state::mutate_state(|s| { s.config.icpswap_icp_is_token0 = icp_is_token0; s.token_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[A] Token ordering failed: {}", e)); return; }
                }
            }
            let config = state::read_state(|s| s.config.clone());
            let target = IcpswapTarget {
                pool: config.icpswap_pool, icp_is_token0: config.icpswap_icp_is_token0,
                label: "ICPSwap", strategy_tag: "A", stable_token_name: "ckUSDC",
                stable_fee: CKUSDC_FEE, stable_ledger: config.ckusdc_ledger,
                pool_enum: state::Pool::IcpswapCkusdc, stable_decimals: 6, uses_vp: false,
                venue: state::Venue::Icpswap,
                fee_pips: 0,
            };
            match compute_optimal_trade(&config, &target).await {
                Ok(dr) => {
                    if dr.direction.is_none() { state::log_activity("arb_skip", &format!("[A] No direction: {}", dr.message)); return; }
                    state::log_activity("arb_start", &format!("[A] Force-execute {:?} spread {} bps est profit {}", dr.direction.as_ref().unwrap(), dr.spread_bps, dr.expected_profit_usd));
                    match dr.direction.as_ref().unwrap() {
                        Direction::RumiToIcpswap => execute_rumi_to_icpswap(&config, &target, &dr).await,
                        Direction::IcpswapToRumi => execute_icpswap_to_rumi(&config, &target, &dr).await,
                    }
                }
                Err(e) => log_error(&format!("[A] Computation failed: {}", e)),
            }
        }
        "B" => {
            let resolved = state::read_state(|s| s.token_ordering_resolved);
            if !resolved {
                let (pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_pool, s.config.icp_ledger));
                if let Ok(icp_is_token0) = prices::fetch_icpswap_token_ordering(pool, icp_ledger).await {
                    state::mutate_state(|s| { s.config.icpswap_icp_is_token0 = icp_is_token0; s.token_ordering_resolved = true; });
                }
            }
            let (icusd_resolved, has_icusd_pool) = state::read_state(|s| (s.icusd_token_ordering_resolved, s.config.icpswap_icusd_pool != Principal::anonymous()));
            if !has_icusd_pool { state::log_activity("arb_skip", "[B] No icUSD pool configured"); return; }
            if !icusd_resolved {
                let (pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_icusd_pool, s.config.icp_ledger));
                match prices::fetch_icpswap_token_ordering(pool, icp_ledger).await {
                    Ok(icp_is_token0) => state::mutate_state(|s| { s.config.icpswap_icusd_icp_is_token0 = icp_is_token0; s.icusd_token_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[B] icUSD ordering failed: {}", e)); return; }
                }
            }
            let config = state::read_state(|s| s.config.clone());
            let cross = CrossPoolTarget {
                strategy_tag: "B",
                buy_side: CrossPoolSide { pool: config.icpswap_icusd_pool, icp_is_token0: config.icpswap_icusd_icp_is_token0, stable_token_name: "icUSD", stable_fee: ICUSD_FEE, stable_ledger: config.icusd_ledger, stable_decimals: 8, pool_enum: state::Pool::IcpswapIcusd, dex_label: "ICPSwap-icUSD", uses_vp: false, venue: state::Venue::Icpswap, fee_pips: 0 },
                sell_side: CrossPoolSide { pool: config.icpswap_pool, icp_is_token0: config.icpswap_icp_is_token0, stable_token_name: "ckUSDC", stable_fee: CKUSDC_FEE, stable_ledger: config.ckusdc_ledger, stable_decimals: 6, pool_enum: state::Pool::IcpswapCkusdc, dex_label: "ICPSwap", uses_vp: false, venue: state::Venue::Icpswap, fee_pips: 0 },
            };
            match compute_optimal_cross_pool_trade(&config, &cross).await {
                Ok(dr) => {
                    if dr.direction.is_none() { state::log_activity("arb_skip", &format!("[B] No direction: {}", dr.message)); return; }
                    state::log_activity("arb_start", &format!("[B] Force-execute {:?} spread {} bps est profit {}", dr.direction.as_ref().unwrap(), dr.spread_bps, dr.expected_profit_usd));
                    match dr.direction.as_ref().unwrap() {
                        Direction::RumiToIcpswap => execute_cross_pool_forward(&cross, &dr).await,
                        Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross, &dr).await,
                    }
                }
                Err(e) => log_error(&format!("[B] Computation failed: {}", e)),
            }
        }
        "C" => {
            let (ckusdt_resolved, has_ckusdt_pool) = state::read_state(|s| (s.ckusdt_token_ordering_resolved, s.config.icpswap_ckusdt_pool != Principal::anonymous()));
            if !has_ckusdt_pool { state::log_activity("arb_skip", "[C] No ckUSDT pool configured"); return; }
            if !ckusdt_resolved {
                let (pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_ckusdt_pool, s.config.icp_ledger));
                match prices::fetch_icpswap_token_ordering(pool, icp_ledger).await {
                    Ok(icp_is_token0) => state::mutate_state(|s| { s.config.icpswap_ckusdt_icp_is_token0 = icp_is_token0; s.ckusdt_token_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[C] ckUSDT ordering failed: {}", e)); return; }
                }
            }
            let config = state::read_state(|s| s.config.clone());
            let target = IcpswapTarget {
                pool: config.icpswap_ckusdt_pool, icp_is_token0: config.icpswap_ckusdt_icp_is_token0,
                label: "ICPSwap-ckUSDT", strategy_tag: "C", stable_token_name: "ckUSDT",
                stable_fee: CKUSDT_FEE, stable_ledger: config.ckusdt_ledger,
                pool_enum: state::Pool::IcpswapCkusdt, stable_decimals: 6, uses_vp: false,
                venue: state::Venue::Icpswap,
                fee_pips: 0,
            };
            match compute_optimal_trade(&config, &target).await {
                Ok(dr) => {
                    if dr.direction.is_none() { state::log_activity("arb_skip", &format!("[C] No direction: {}", dr.message)); return; }
                    state::log_activity("arb_start", &format!("[C] Force-execute {:?} spread {} bps est profit {}", dr.direction.as_ref().unwrap(), dr.spread_bps, dr.expected_profit_usd));
                    match dr.direction.as_ref().unwrap() {
                        Direction::RumiToIcpswap => execute_rumi_to_icpswap(&config, &target, &dr).await,
                        Direction::IcpswapToRumi => execute_icpswap_to_rumi(&config, &target, &dr).await,
                    }
                }
                Err(e) => log_error(&format!("[C] Computation failed: {}", e)),
            }
        }
        "D" => {
            let (icusd_resolved, has_icusd_pool) = state::read_state(|s| (s.icusd_token_ordering_resolved, s.config.icpswap_icusd_pool != Principal::anonymous()));
            if !has_icusd_pool { state::log_activity("arb_skip", "[D] No icUSD pool configured"); return; }
            if !icusd_resolved {
                let (pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_icusd_pool, s.config.icp_ledger));
                match prices::fetch_icpswap_token_ordering(pool, icp_ledger).await {
                    Ok(icp_is_token0) => state::mutate_state(|s| { s.config.icpswap_icusd_icp_is_token0 = icp_is_token0; s.icusd_token_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[D] icUSD ordering failed: {}", e)); return; }
                }
            }
            let config = state::read_state(|s| s.config.clone());
            let target = IcpswapTarget {
                pool: config.icpswap_icusd_pool, icp_is_token0: config.icpswap_icusd_icp_is_token0,
                label: "ICPSwap-icUSD", strategy_tag: "D", stable_token_name: "icUSD",
                stable_fee: ICUSD_FEE, stable_ledger: config.icusd_ledger,
                pool_enum: state::Pool::IcpswapIcusd, stable_decimals: 8, uses_vp: false,
                venue: state::Venue::Icpswap,
                fee_pips: 0,
            };
            match compute_optimal_trade(&config, &target).await {
                Ok(dr) => {
                    if dr.direction.is_none() { state::log_activity("arb_skip", &format!("[D] No direction: {}", dr.message)); return; }
                    state::log_activity("arb_start", &format!("[D] Force-execute {:?} spread {} bps est profit {}", dr.direction.as_ref().unwrap(), dr.spread_bps, dr.expected_profit_usd));
                    match dr.direction.as_ref().unwrap() {
                        Direction::RumiToIcpswap => execute_rumi_to_icpswap(&config, &target, &dr).await,
                        Direction::IcpswapToRumi => execute_icpswap_to_rumi(&config, &target, &dr).await,
                    }
                }
                Err(e) => log_error(&format!("[D] Computation failed: {}", e)),
            }
        }
        "F" => {
            let (icusd_resolved, has_icusd_pool) = state::read_state(|s| (s.icusd_token_ordering_resolved, s.config.icpswap_icusd_pool != Principal::anonymous()));
            if !has_icusd_pool { state::log_activity("arb_skip", "[F] No icUSD pool configured"); return; }
            if !icusd_resolved {
                let (pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_icusd_pool, s.config.icp_ledger));
                match prices::fetch_icpswap_token_ordering(pool, icp_ledger).await {
                    Ok(icp_is_token0) => state::mutate_state(|s| { s.config.icpswap_icusd_icp_is_token0 = icp_is_token0; s.icusd_token_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[F] icUSD ordering failed: {}", e)); return; }
                }
            }
            let (ckusdt_resolved, has_ckusdt_pool) = state::read_state(|s| (s.ckusdt_token_ordering_resolved, s.config.icpswap_ckusdt_pool != Principal::anonymous()));
            if !has_ckusdt_pool { state::log_activity("arb_skip", "[F] No ckUSDT pool configured"); return; }
            if !ckusdt_resolved {
                let (pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_ckusdt_pool, s.config.icp_ledger));
                match prices::fetch_icpswap_token_ordering(pool, icp_ledger).await {
                    Ok(icp_is_token0) => state::mutate_state(|s| { s.config.icpswap_ckusdt_icp_is_token0 = icp_is_token0; s.ckusdt_token_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[F] ckUSDT ordering failed: {}", e)); return; }
                }
            }
            let config = state::read_state(|s| s.config.clone());
            let cross = CrossPoolTarget {
                strategy_tag: "F",
                buy_side: CrossPoolSide { pool: config.icpswap_icusd_pool, icp_is_token0: config.icpswap_icusd_icp_is_token0, stable_token_name: "icUSD", stable_fee: ICUSD_FEE, stable_ledger: config.icusd_ledger, stable_decimals: 8, pool_enum: state::Pool::IcpswapIcusd, dex_label: "ICPSwap-icUSD", uses_vp: false, venue: state::Venue::Icpswap, fee_pips: 0 },
                sell_side: CrossPoolSide { pool: config.icpswap_ckusdt_pool, icp_is_token0: config.icpswap_ckusdt_icp_is_token0, stable_token_name: "ckUSDT", stable_fee: CKUSDT_FEE, stable_ledger: config.ckusdt_ledger, stable_decimals: 6, pool_enum: state::Pool::IcpswapCkusdt, dex_label: "ICPSwap-ckUSDT", uses_vp: false, venue: state::Venue::Icpswap, fee_pips: 0 },
            };
            match compute_optimal_cross_pool_trade(&config, &cross).await {
                Ok(dr) => {
                    if dr.direction.is_none() { state::log_activity("arb_skip", &format!("[F] No direction: {}", dr.message)); return; }
                    state::log_activity("arb_start", &format!("[F] Force-execute {:?} spread {} bps est profit {}", dr.direction.as_ref().unwrap(), dr.spread_bps, dr.expected_profit_usd));
                    match dr.direction.as_ref().unwrap() {
                        Direction::RumiToIcpswap => execute_cross_pool_forward(&cross, &dr).await,
                        Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross, &dr).await,
                    }
                }
                Err(e) => log_error(&format!("[F] Computation failed: {}", e)),
            }
        }
        "K" => {
            let resolved = state::read_state(|s| s.token_ordering_resolved);
            if !resolved {
                let (pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_pool, s.config.icp_ledger));
                match prices::fetch_icpswap_token_ordering(pool, icp_ledger).await {
                    Ok(icp_is_token0) => state::mutate_state(|s| { s.config.icpswap_icp_is_token0 = icp_is_token0; s.token_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[K] Token ordering failed: {}", e)); return; }
                }
            }
            let config = state::read_state(|s| s.config.clone());
            if config.partydex_ckusdc_pool == Principal::anonymous() {
                state::log_activity("arb_skip", "[K] No PartyDEX ckUSDC pool configured"); return;
            }
            let cross = CrossPoolTarget {
                strategy_tag: "K",
                buy_side: CrossPoolSide { pool: config.partydex_ckusdc_pool, icp_is_token0: true, stable_token_name: "ckUSDC", stable_fee: CKUSDC_FEE, stable_ledger: config.ckusdc_ledger, stable_decimals: 6, pool_enum: state::Pool::PartyDexIcpCkusdc, dex_label: "PartyDEX-ckUSDC", uses_vp: false, venue: state::Venue::PartyDex, fee_pips: config.partydex_ckusdc_fee_pips },
                sell_side: CrossPoolSide { pool: config.icpswap_pool, icp_is_token0: config.icpswap_icp_is_token0, stable_token_name: "ckUSDC", stable_fee: CKUSDC_FEE, stable_ledger: config.ckusdc_ledger, stable_decimals: 6, pool_enum: state::Pool::IcpswapCkusdc, dex_label: "ICPSwap", uses_vp: false, venue: state::Venue::Icpswap, fee_pips: 0 },
            };
            match compute_optimal_cross_pool_trade(&config, &cross).await {
                Ok(dr) => {
                    if dr.direction.is_none() { state::log_activity("arb_skip", &format!("[K] No direction: {}", dr.message)); return; }
                    state::log_activity("arb_start", &format!("[K] Force-execute {:?} spread {} bps est profit {}", dr.direction.as_ref().unwrap(), dr.spread_bps, dr.expected_profit_usd));
                    match dr.direction.as_ref().unwrap() {
                        Direction::RumiToIcpswap => execute_cross_pool_forward(&cross, &dr).await,
                        Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross, &dr).await,
                    }
                }
                Err(e) => log_error(&format!("[K] Computation failed: {}", e)),
            }
        }
        "L" => {
            let (ckusdt_resolved, has_ckusdt_pool) = state::read_state(|s| (s.ckusdt_token_ordering_resolved, s.config.icpswap_ckusdt_pool != Principal::anonymous()));
            if !has_ckusdt_pool { state::log_activity("arb_skip", "[L] No ckUSDT pool configured"); return; }
            if !ckusdt_resolved {
                let (pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_ckusdt_pool, s.config.icp_ledger));
                match prices::fetch_icpswap_token_ordering(pool, icp_ledger).await {
                    Ok(icp_is_token0) => state::mutate_state(|s| { s.config.icpswap_ckusdt_icp_is_token0 = icp_is_token0; s.ckusdt_token_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[L] ckUSDT ordering failed: {}", e)); return; }
                }
            }
            let config = state::read_state(|s| s.config.clone());
            if config.partydex_ckusdc_pool == Principal::anonymous() {
                state::log_activity("arb_skip", "[L] No PartyDEX ckUSDC pool configured"); return;
            }
            let cross = CrossPoolTarget {
                strategy_tag: "L",
                buy_side: CrossPoolSide { pool: config.partydex_ckusdc_pool, icp_is_token0: true, stable_token_name: "ckUSDC", stable_fee: CKUSDC_FEE, stable_ledger: config.ckusdc_ledger, stable_decimals: 6, pool_enum: state::Pool::PartyDexIcpCkusdc, dex_label: "PartyDEX-ckUSDC", uses_vp: false, venue: state::Venue::PartyDex, fee_pips: config.partydex_ckusdc_fee_pips },
                sell_side: CrossPoolSide { pool: config.icpswap_ckusdt_pool, icp_is_token0: config.icpswap_ckusdt_icp_is_token0, stable_token_name: "ckUSDT", stable_fee: CKUSDT_FEE, stable_ledger: config.ckusdt_ledger, stable_decimals: 6, pool_enum: state::Pool::IcpswapCkusdt, dex_label: "ICPSwap-ckUSDT", uses_vp: false, venue: state::Venue::Icpswap, fee_pips: 0 },
            };
            match compute_optimal_cross_pool_trade(&config, &cross).await {
                Ok(dr) => {
                    if dr.direction.is_none() { state::log_activity("arb_skip", &format!("[L] No direction: {}", dr.message)); return; }
                    state::log_activity("arb_start", &format!("[L] Force-execute {:?} spread {} bps est profit {}", dr.direction.as_ref().unwrap(), dr.spread_bps, dr.expected_profit_usd));
                    match dr.direction.as_ref().unwrap() {
                        Direction::RumiToIcpswap => execute_cross_pool_forward(&cross, &dr).await,
                        Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross, &dr).await,
                    }
                }
                Err(e) => log_error(&format!("[L] Computation failed: {}", e)),
            }
        }
        "M" => {
            let (icusd_resolved, has_icusd_pool) = state::read_state(|s| (s.icusd_token_ordering_resolved, s.config.icpswap_icusd_pool != Principal::anonymous()));
            if !has_icusd_pool { state::log_activity("arb_skip", "[M] No icUSD pool configured"); return; }
            if !icusd_resolved {
                let (pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_icusd_pool, s.config.icp_ledger));
                match prices::fetch_icpswap_token_ordering(pool, icp_ledger).await {
                    Ok(icp_is_token0) => state::mutate_state(|s| { s.config.icpswap_icusd_icp_is_token0 = icp_is_token0; s.icusd_token_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[M] icUSD ordering failed: {}", e)); return; }
                }
            }
            let config = state::read_state(|s| s.config.clone());
            if config.partydex_ckusdc_pool == Principal::anonymous() {
                state::log_activity("arb_skip", "[M] No PartyDEX ckUSDC pool configured"); return;
            }
            let cross = CrossPoolTarget {
                strategy_tag: "M",
                buy_side: CrossPoolSide { pool: config.partydex_ckusdc_pool, icp_is_token0: true, stable_token_name: "ckUSDC", stable_fee: CKUSDC_FEE, stable_ledger: config.ckusdc_ledger, stable_decimals: 6, pool_enum: state::Pool::PartyDexIcpCkusdc, dex_label: "PartyDEX-ckUSDC", uses_vp: false, venue: state::Venue::PartyDex, fee_pips: config.partydex_ckusdc_fee_pips },
                sell_side: CrossPoolSide { pool: config.icpswap_icusd_pool, icp_is_token0: config.icpswap_icusd_icp_is_token0, stable_token_name: "icUSD", stable_fee: ICUSD_FEE, stable_ledger: config.icusd_ledger, stable_decimals: 8, pool_enum: state::Pool::IcpswapIcusd, dex_label: "ICPSwap-icUSD", uses_vp: false, venue: state::Venue::Icpswap, fee_pips: 0 },
            };
            match compute_optimal_cross_pool_trade(&config, &cross).await {
                Ok(dr) => {
                    if dr.direction.is_none() { state::log_activity("arb_skip", &format!("[M] No direction: {}", dr.message)); return; }
                    state::log_activity("arb_start", &format!("[M] Force-execute {:?} spread {} bps est profit {}", dr.direction.as_ref().unwrap(), dr.spread_bps, dr.expected_profit_usd));
                    match dr.direction.as_ref().unwrap() {
                        Direction::RumiToIcpswap => execute_cross_pool_forward(&cross, &dr).await,
                        Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross, &dr).await,
                    }
                }
                Err(e) => log_error(&format!("[M] Computation failed: {}", e)),
            }
        }
        "N" => {
            let resolved = state::read_state(|s| s.token_ordering_resolved);
            if !resolved {
                let (pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_pool, s.config.icp_ledger));
                match prices::fetch_icpswap_token_ordering(pool, icp_ledger).await {
                    Ok(icp_is_token0) => state::mutate_state(|s| { s.config.icpswap_icp_is_token0 = icp_is_token0; s.token_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[N] Token ordering failed: {}", e)); return; }
                }
            }
            let config = state::read_state(|s| s.config.clone());
            if config.partydex_ckusdt_pool == Principal::anonymous() {
                state::log_activity("arb_skip", "[N] No PartyDEX ckUSDT pool configured"); return;
            }
            let cross = CrossPoolTarget {
                strategy_tag: "N",
                buy_side: CrossPoolSide { pool: config.partydex_ckusdt_pool, icp_is_token0: true, stable_token_name: "ckUSDT", stable_fee: CKUSDT_FEE, stable_ledger: config.ckusdt_ledger, stable_decimals: 6, pool_enum: state::Pool::PartyDexIcpCkusdt, dex_label: "PartyDEX-ckUSDT", uses_vp: false, venue: state::Venue::PartyDex, fee_pips: config.partydex_ckusdt_fee_pips },
                sell_side: CrossPoolSide { pool: config.icpswap_pool, icp_is_token0: config.icpswap_icp_is_token0, stable_token_name: "ckUSDC", stable_fee: CKUSDC_FEE, stable_ledger: config.ckusdc_ledger, stable_decimals: 6, pool_enum: state::Pool::IcpswapCkusdc, dex_label: "ICPSwap", uses_vp: false, venue: state::Venue::Icpswap, fee_pips: 0 },
            };
            match compute_optimal_cross_pool_trade(&config, &cross).await {
                Ok(dr) => {
                    if dr.direction.is_none() { state::log_activity("arb_skip", &format!("[N] No direction: {}", dr.message)); return; }
                    state::log_activity("arb_start", &format!("[N] Force-execute {:?} spread {} bps est profit {}", dr.direction.as_ref().unwrap(), dr.spread_bps, dr.expected_profit_usd));
                    match dr.direction.as_ref().unwrap() {
                        Direction::RumiToIcpswap => execute_cross_pool_forward(&cross, &dr).await,
                        Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross, &dr).await,
                    }
                }
                Err(e) => log_error(&format!("[N] Computation failed: {}", e)),
            }
        }
        "O" => {
            let (ckusdt_resolved, has_ckusdt_pool) = state::read_state(|s| (s.ckusdt_token_ordering_resolved, s.config.icpswap_ckusdt_pool != Principal::anonymous()));
            if !has_ckusdt_pool { state::log_activity("arb_skip", "[O] No ckUSDT pool configured"); return; }
            if !ckusdt_resolved {
                let (pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_ckusdt_pool, s.config.icp_ledger));
                match prices::fetch_icpswap_token_ordering(pool, icp_ledger).await {
                    Ok(icp_is_token0) => state::mutate_state(|s| { s.config.icpswap_ckusdt_icp_is_token0 = icp_is_token0; s.ckusdt_token_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[O] ckUSDT ordering failed: {}", e)); return; }
                }
            }
            let config = state::read_state(|s| s.config.clone());
            if config.partydex_ckusdt_pool == Principal::anonymous() {
                state::log_activity("arb_skip", "[O] No PartyDEX ckUSDT pool configured"); return;
            }
            let cross = CrossPoolTarget {
                strategy_tag: "O",
                buy_side: CrossPoolSide { pool: config.partydex_ckusdt_pool, icp_is_token0: true, stable_token_name: "ckUSDT", stable_fee: CKUSDT_FEE, stable_ledger: config.ckusdt_ledger, stable_decimals: 6, pool_enum: state::Pool::PartyDexIcpCkusdt, dex_label: "PartyDEX-ckUSDT", uses_vp: false, venue: state::Venue::PartyDex, fee_pips: config.partydex_ckusdt_fee_pips },
                sell_side: CrossPoolSide { pool: config.icpswap_ckusdt_pool, icp_is_token0: config.icpswap_ckusdt_icp_is_token0, stable_token_name: "ckUSDT", stable_fee: CKUSDT_FEE, stable_ledger: config.ckusdt_ledger, stable_decimals: 6, pool_enum: state::Pool::IcpswapCkusdt, dex_label: "ICPSwap-ckUSDT", uses_vp: false, venue: state::Venue::Icpswap, fee_pips: 0 },
            };
            match compute_optimal_cross_pool_trade(&config, &cross).await {
                Ok(dr) => {
                    if dr.direction.is_none() { state::log_activity("arb_skip", &format!("[O] No direction: {}", dr.message)); return; }
                    state::log_activity("arb_start", &format!("[O] Force-execute {:?} spread {} bps est profit {}", dr.direction.as_ref().unwrap(), dr.spread_bps, dr.expected_profit_usd));
                    match dr.direction.as_ref().unwrap() {
                        Direction::RumiToIcpswap => execute_cross_pool_forward(&cross, &dr).await,
                        Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross, &dr).await,
                    }
                }
                Err(e) => log_error(&format!("[O] Computation failed: {}", e)),
            }
        }
        "P" => {
            let (icusd_resolved, has_icusd_pool) = state::read_state(|s| (s.icusd_token_ordering_resolved, s.config.icpswap_icusd_pool != Principal::anonymous()));
            if !has_icusd_pool { state::log_activity("arb_skip", "[P] No icUSD pool configured"); return; }
            if !icusd_resolved {
                let (pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_icusd_pool, s.config.icp_ledger));
                match prices::fetch_icpswap_token_ordering(pool, icp_ledger).await {
                    Ok(icp_is_token0) => state::mutate_state(|s| { s.config.icpswap_icusd_icp_is_token0 = icp_is_token0; s.icusd_token_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[P] icUSD ordering failed: {}", e)); return; }
                }
            }
            let config = state::read_state(|s| s.config.clone());
            if config.partydex_ckusdt_pool == Principal::anonymous() {
                state::log_activity("arb_skip", "[P] No PartyDEX ckUSDT pool configured"); return;
            }
            let cross = CrossPoolTarget {
                strategy_tag: "P",
                buy_side: CrossPoolSide { pool: config.partydex_ckusdt_pool, icp_is_token0: true, stable_token_name: "ckUSDT", stable_fee: CKUSDT_FEE, stable_ledger: config.ckusdt_ledger, stable_decimals: 6, pool_enum: state::Pool::PartyDexIcpCkusdt, dex_label: "PartyDEX-ckUSDT", uses_vp: false, venue: state::Venue::PartyDex, fee_pips: config.partydex_ckusdt_fee_pips },
                sell_side: CrossPoolSide { pool: config.icpswap_icusd_pool, icp_is_token0: config.icpswap_icusd_icp_is_token0, stable_token_name: "icUSD", stable_fee: ICUSD_FEE, stable_ledger: config.icusd_ledger, stable_decimals: 8, pool_enum: state::Pool::IcpswapIcusd, dex_label: "ICPSwap-icUSD", uses_vp: false, venue: state::Venue::Icpswap, fee_pips: 0 },
            };
            match compute_optimal_cross_pool_trade(&config, &cross).await {
                Ok(dr) => {
                    if dr.direction.is_none() { state::log_activity("arb_skip", &format!("[P] No direction: {}", dr.message)); return; }
                    state::log_activity("arb_start", &format!("[P] Force-execute {:?} spread {} bps est profit {}", dr.direction.as_ref().unwrap(), dr.spread_bps, dr.expected_profit_usd));
                    match dr.direction.as_ref().unwrap() {
                        Direction::RumiToIcpswap => execute_cross_pool_forward(&cross, &dr).await,
                        Direction::IcpswapToRumi => execute_cross_pool_reverse(&cross, &dr).await,
                    }
                }
                Err(e) => log_error(&format!("[P] Computation failed: {}", e)),
            }
        }
        "Q" => {
            let config = state::read_state(|s| s.config.clone());
            if config.partydex_ckusdc_pool == Principal::anonymous() {
                state::log_activity("arb_skip", "[Q] No PartyDEX ckUSDC pool configured"); return;
            }
            let target = IcpswapTarget {
                pool: config.partydex_ckusdc_pool, icp_is_token0: true,
                label: "PartyDEX-ckUSDC", strategy_tag: "Q", stable_token_name: "ckUSDC",
                stable_fee: CKUSDC_FEE, stable_ledger: config.ckusdc_ledger,
                pool_enum: state::Pool::PartyDexIcpCkusdc, stable_decimals: 6, uses_vp: false,
                venue: state::Venue::PartyDex,
                fee_pips: config.partydex_ckusdc_fee_pips,
            };
            match compute_optimal_trade(&config, &target).await {
                Ok(dr) => {
                    if dr.direction.is_none() { state::log_activity("arb_skip", &format!("[Q] No direction: {}", dr.message)); return; }
                    state::log_activity("arb_start", &format!("[Q] Force-execute {:?} spread {} bps est profit {}", dr.direction.as_ref().unwrap(), dr.spread_bps, dr.expected_profit_usd));
                    match dr.direction.as_ref().unwrap() {
                        Direction::RumiToIcpswap => execute_rumi_to_icpswap(&config, &target, &dr).await,
                        Direction::IcpswapToRumi => execute_icpswap_to_rumi(&config, &target, &dr).await,
                    }
                }
                Err(e) => log_error(&format!("[Q] Computation failed: {}", e)),
            }
        }
        "R" => {
            let config = state::read_state(|s| s.config.clone());
            if config.partydex_ckusdt_pool == Principal::anonymous() {
                state::log_activity("arb_skip", "[R] No PartyDEX ckUSDT pool configured"); return;
            }
            let target = IcpswapTarget {
                pool: config.partydex_ckusdt_pool, icp_is_token0: true,
                label: "PartyDEX-ckUSDT", strategy_tag: "R", stable_token_name: "ckUSDT",
                stable_fee: CKUSDT_FEE, stable_ledger: config.ckusdt_ledger,
                pool_enum: state::Pool::PartyDexIcpCkusdt, stable_decimals: 6, uses_vp: false,
                venue: state::Venue::PartyDex,
                fee_pips: config.partydex_ckusdt_fee_pips,
            };
            match compute_optimal_trade(&config, &target).await {
                Ok(dr) => {
                    if dr.direction.is_none() { state::log_activity("arb_skip", &format!("[R] No direction: {}", dr.message)); return; }
                    state::log_activity("arb_start", &format!("[R] Force-execute {:?} spread {} bps est profit {}", dr.direction.as_ref().unwrap(), dr.spread_bps, dr.expected_profit_usd));
                    match dr.direction.as_ref().unwrap() {
                        Direction::RumiToIcpswap => execute_rumi_to_icpswap(&config, &target, &dr).await,
                        Direction::IcpswapToRumi => execute_icpswap_to_rumi(&config, &target, &dr).await,
                    }
                }
                Err(e) => log_error(&format!("[R] Computation failed: {}", e)),
            }
        }
        "S" => {
            let config = state::read_state(|s| s.config.clone());
            if config.icpswap_icusd_bob_pool == Principal::anonymous()
                || config.icpswap_bob_icp_pool == Principal::anonymous()
            {
                state::log_activity("arb_skip", "[S] BOB pools not configured"); return;
            }
            // Resolve both BOB pool orderings on demand (per-letter pattern).
            let (bob_icp_resolved, icusd_bob_resolved) = state::read_state(|s| {
                (s.bob_icp_ordering_resolved, s.icusd_bob_ordering_resolved)
            });
            if !bob_icp_resolved {
                match prices::fetch_icpswap_token_ordering(config.icpswap_bob_icp_pool, config.icp_ledger).await {
                    Ok(icp_is_token0) => state::mutate_state(|s| { s.config.icpswap_bob_icp_icp_is_token0 = icp_is_token0; s.bob_icp_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[S] BOB/ICP pool ordering resolution failed: {}", e)); return; }
                }
            }
            if !icusd_bob_resolved {
                match prices::fetch_icpswap_token_ordering(config.icpswap_icusd_bob_pool, config.icusd_ledger).await {
                    Ok(icusd_is_token0) => state::mutate_state(|s| { s.config.icpswap_icusd_bob_icusd_is_token0 = icusd_is_token0; s.icusd_bob_ordering_resolved = true; }),
                    Err(e) => { log_error(&format!("[S] icUSD/BOB pool ordering resolution failed: {}", e)); return; }
                }
            }
            let config = state::read_state(|s| s.config.clone()); // re-read post-resolution
            let target = BobTarget {
                icusd_bob_pool: config.icpswap_icusd_bob_pool,
                icusd_is_token0: config.icpswap_icusd_bob_icusd_is_token0,
                bob_icp_pool: config.icpswap_bob_icp_pool,
                bob_icp_icp_is_token0: config.icpswap_bob_icp_icp_is_token0,
                bob_ledger: config.bob_ledger,
                bob_fee: config.bob_ledger_fee,
                icusd_ledger: config.icusd_ledger,
                icusd_fee: ICUSD_FEE,
                icp_ledger: config.icp_ledger,
            };
            match find_optimal_bob(&config, &target).await {
                Ok(dr) => {
                    if dr.direction.is_none() {
                        state::log_activity("arb_skip", "[S] No direction (spread below threshold or quotes failed)");
                        return;
                    }
                    // Admin override, matching A–R force-execute semantics:
                    // runs regardless of bob_execution_enabled — logged so a
                    // dry-run-mode execution is unambiguous in the activity log.
                    state::log_activity("arb_start", &format!(
                        "[S] Force-execute {:?} spread {} bps est profit {} (admin override; bob_execution_enabled={})",
                        dr.direction.unwrap(), dr.spread_bps, dr.expected_profit_usd, config.bob_execution_enabled
                    ));
                    execute_bob(&config, &target, &dr).await;
                }
                Err(e) => log_error(&format!("[S] Computation failed: {}", e)),
            }
        }
        _ => state::log_activity("arb_skip", &format!("Unknown strategy tag: {}", strategy_tag)),
    }
}

// ─── Dry Run: Compute Optimal Trade ───

/// Identifies an ICPSwap stable/ICP pool that a Rumi-vs-ICPSwap strategy will arb against.
#[derive(Clone, Copy)]
pub struct IcpswapTarget {
    pub pool: Principal,
    pub icp_is_token0: bool,
    pub label: &'static str,    // "ICPSwap" (ckUSDC), "ICPSwap-ckUSDT", "ICPSwap-icUSD"
    pub strategy_tag: &'static str, // "A", "C", or "D" — used in log messages
    pub stable_token_name: &'static str, // "ckUSDC", "ckUSDT", or "icUSD"
    pub stable_fee: u64,        // native units (10_000 for ck*, 100_000 for icUSD)
    pub stable_ledger: Principal,
    pub pool_enum: state::Pool,
    pub stable_decimals: u8,    // 6 for ck*, 8 for icUSD/3USD
    /// If true, the stable token is 3USD and amounts must be VP-adjusted for USD conversion.
    pub uses_vp: bool,
    /// Which DEX venue this leg trades against. Always `Icpswap` in PR2a —
    /// no target sets `PartyDex` yet (that's PR2b).
    pub venue: state::Venue,
    /// Fee tier (pips) for PartyDEX pool_swaps. Ignored when venue is Icpswap.
    pub fee_pips: u32,
}

/// Identifies one side of a cross-pool (ICPSwap-vs-ICPSwap) arbitrage.
#[derive(Clone, Copy)]
pub struct CrossPoolSide {
    pub pool: Principal,
    pub icp_is_token0: bool,
    pub stable_token_name: &'static str,
    pub stable_fee: u64,
    pub stable_ledger: Principal,
    pub stable_decimals: u8,
    pub pool_enum: state::Pool,
    pub dex_label: &'static str,
    /// If true, the stable token is 3USD and amounts must be VP-adjusted for USD conversion.
    pub uses_vp: bool,
    /// Which DEX venue this side trades against. Always `Icpswap` in PR2a.
    pub venue: state::Venue,
    /// Fee tier (pips) for PartyDEX pool_swaps. Ignored when venue is Icpswap.
    pub fee_pips: u32,
}

/// Defines a cross-pool strategy: arb between two ICPSwap stable/ICP pools.
/// `buy_side` is the pool where ICP is cheaper when spread > 0.
#[derive(Clone, Copy)]
pub struct CrossPoolTarget {
    pub strategy_tag: &'static str,
    pub buy_side: CrossPoolSide,
    pub sell_side: CrossPoolSide,
}

/// Convert a stable token amount in native units to 6-dec USD (assumes ≈ $1 peg).
fn stable_to_usd_6dec(amount: u64, decimals: u8) -> i64 {
    if decimals > 6 {
        let div = 10u64.pow((decimals - 6) as u32);
        (amount / div) as i64
    } else if decimals < 6 {
        let mul = 10u64.pow((6 - decimals) as u32);
        amount.saturating_mul(mul) as i64
    } else {
        amount as i64
    }
}

/// Convert a 6-dec USD amount to a stable token's native units.
fn usd_6dec_to_stable(amount_usd: u64, decimals: u8) -> u64 {
    if decimals > 6 {
        let mul = 10u64.pow((decimals - 6) as u32);
        amount_usd.saturating_mul(mul)
    } else if decimals < 6 {
        let div = 10u64.pow((6 - decimals) as u32);
        amount_usd / div
    } else {
        amount_usd
    }
}

/// VP-aware version of stable_to_usd_6dec. When vp > 0, multiplies the result
/// by virtual_price / 1e18 to account for 3USD being worth ~$1.057 not $1.
fn stable_to_usd_6dec_vp(amount: u64, decimals: u8, vp: u64) -> i64 {
    let base = stable_to_usd_6dec(amount, decimals);
    if vp > 0 {
        (base as u128 * vp as u128 / VP_PRECISION) as i64
    } else {
        base
    }
}

/// VP-aware version of usd_6dec_to_stable. When vp > 0, divides by VP first.
fn usd_6dec_to_stable_vp(amount_usd: u64, decimals: u8, vp: u64) -> u64 {
    let adjusted = if vp > 0 {
        (amount_usd as u128 * VP_PRECISION / vp as u128) as u64
    } else {
        amount_usd
    };
    usd_6dec_to_stable(adjusted, decimals)
}

pub async fn compute_optimal_trade(
    config: &state::BotConfig,
    target: &IcpswapTarget,
) -> Result<DryRunResult, String> {
    let mut result = DryRunResult::default();

    // Fetch prices (use cycle-cached VP if available). The venue-side price is
    // routed through venue_price_icp — for target.venue == Icpswap (every
    // target in PR2a) this is exactly the same fetch_icpswap_price call that
    // fetch_all_prices_with_vp made internally, so behavior is unchanged.
    let cached_vp = fetch_virtual_price_cached(config.rumi_3pool).await.ok();
    let rumi_fut = prices::fetch_rumi_price(config.rumi_amm, RUMI_POOL_ID, config.icp_ledger);
    let venue_fut = venue_price_icp(target.pool, target.icp_is_token0, target.venue, target.fee_pips);
    let (rumi_result, venue_result, vp) = match cached_vp {
        Some(vp) => {
            let (r, v) = futures::future::join(rumi_fut, venue_fut).await;
            (r, v, vp)
        }
        None => {
            let (r, v, vv) = futures::future::join3(rumi_fut, venue_fut, prices::fetch_virtual_price(config.rumi_3pool)).await;
            (r, v, vv?)
        }
    };
    let prices = PriceData {
        rumi_icp_price_3usd_native: rumi_result?,
        virtual_price: vp,
        icpswap_icp_price_ckusdc_native: venue_result?,
        icpswap_stable_decimals: target.stable_decimals,
    };

    result.rumi_price_usd = prices.rumi_price_usd_6dec();
    result.virtual_price = prices.virtual_price;
    // If the ICPSwap stable is 3USD, adjust its price by VP for true USD value
    let icpswap_raw_usd = prices.icpswap_price_usd_6dec();
    result.icpswap_price_usd = if target.uses_vp && prices.virtual_price > 0 {
        (icpswap_raw_usd as u128 * prices.virtual_price as u128 / VP_PRECISION) as u64
    } else {
        icpswap_raw_usd
    };
    // Compute spread from adjusted prices
    let (r, i) = (result.rumi_price_usd as i64, result.icpswap_price_usd as i64);
    result.spread_bps = if r == 0 || i == 0 { 0 } else {
        ((i - r) * 10_000 / r.min(i)) as i32
    };

    // Fetch balances (cycle-cached; used by snapshot even when spread is below minimum)
    let (bal_3usd, bal_stable) = futures::future::join(
        fetch_balance_cached(config.three_usd_ledger),
        fetch_balance_cached(target.stable_ledger),
    ).await;
    result.balance_3usd = bal_3usd.unwrap_or(0);
    result.balance_ckusdc = bal_stable.unwrap_or(0);

    let abs_spread = result.spread_bps.unsigned_abs();
    if abs_spread < config.min_spread_bps {
        result.message = format!("[{}] Spread {} bps < minimum {} bps", target.strategy_tag, abs_spread, config.min_spread_bps);
        return Ok(result);
    }

    if result.spread_bps > 0 {
        // ICP more expensive on ICPSwap → buy on Rumi (3USD→ICP), sell on ICPSwap (ICP→stable)
        result.direction = Some(Direction::RumiToIcpswap);
        result.optimal_input_token = Some(Token::ThreeUSD);
        result.expected_output_token = Some(Token::CkUSDC);

        // Min balance: per-venue floor (decision #3), converted to 3USD native
        // units — the same venue-aware floor the IcpswapToRumi sibling applies.
        // 3USD is 8-dec, so usd_6dec_to_stable(.., 8) reproduces the pre-PR2a
        // 1_000_000 (=$0.01) floor for ICPSwap and raises it to ~$0.10 for
        // PartyDEX (matters for Q/R when the profitable direction sells ICP
        // into PartyDEX). Mirrors the sibling's unit conversion exactly.
        let min_3usd = usd_6dec_to_stable(min_trade_floor_usd(target.venue), 8);
        if result.balance_3usd < min_3usd {
            result.message = format!("[{}] Insufficient 3USD balance", target.strategy_tag);
            return Ok(result);
        }

        // Cap by max_trade_size_usd (converted to 3USD)
        let max_3usd = if prices.virtual_price > 0 {
            (config.max_trade_size_usd as u128 * VP_PRECISION * 100 / prices.virtual_price as u128) as u64
        } else { result.balance_3usd };
        let max_input = result.balance_3usd.min(max_3usd);

        find_optimal_rumi_to_icpswap(config, target, max_input, &prices, &mut result).await;
    } else {
        // ICP more expensive on Rumi → buy on ICPSwap (stable→ICP), sell on Rumi (ICP→3USD)
        result.direction = Some(Direction::IcpswapToRumi);
        result.optimal_input_token = Some(Token::CkUSDC);
        result.expected_output_token = Some(Token::ThreeUSD);

        // Reserve fee for the ICRC-2 approve that ICPSwap's depositFromAndSwap triggers
        let usable_stable = result.balance_ckusdc.saturating_sub(target.stable_fee);
        // Min balance: per-venue floor (decision #3), scaled to native units.
        // ICPSwap keeps the pre-PR2a $0.01 floor; PartyDEX needs ~$0.10.
        let min_native = usd_6dec_to_stable(min_trade_floor_usd(target.venue), target.stable_decimals);
        if usable_stable < min_native {
            result.message = format!("[{}] Insufficient {} balance", target.strategy_tag, target.stable_token_name);
            return Ok(result);
        }

        // Cap by max_trade_size_usd, converted to native stable units (VP-aware for 3USD)
        let vp_for_cap = if target.uses_vp { prices.virtual_price } else { 0 };
        let max_native = usd_6dec_to_stable_vp(config.max_trade_size_usd, target.stable_decimals, vp_for_cap);
        let max_input = usable_stable.min(max_native);

        find_optimal_icpswap_to_rumi(config, target, max_input, &prices, &mut result).await;
    }

    Ok(result)
}

/// Find optimal trade size for Rumi→ICPSwap direction.
/// Evaluates NUM_CANDIDATES amounts and picks the profit-maximizing one.
async fn find_optimal_rumi_to_icpswap(
    config: &state::BotConfig,
    target: &IcpswapTarget,
    max_input: u64,
    prices: &PriceData,
    result: &mut DryRunResult,
) {
    // Generate candidate amounts (evenly spaced fractions of max)
    let candidates: Vec<u64> = (1..=NUM_CANDIDATES)
        .map(|i| max_input * i / NUM_CANDIDATES)
        .filter(|&a| a > 0)
        .collect();

    // Round 1: Quote Rumi (3USD→ICP) for all candidates in parallel
    let rumi_futs: Vec<_> = candidates.iter().map(|&amount| {
        prices::fetch_rumi_quote_for_amount(
            config.rumi_amm, RUMI_POOL_ID, config.three_usd_ledger, amount,
        )
    }).collect();
    let rumi_results = futures::future::join_all(rumi_futs).await;

    // Collect successful (input, icp_out) pairs
    let mut stage1: Vec<(u64, u64)> = Vec::new();
    for (i, res) in rumi_results.into_iter().enumerate() {
        match res {
            Ok(icp_out) if icp_out > 0 => stage1.push((candidates[i], icp_out)),
            _ => {} // skip failed quotes
        }
    }

    if stage1.is_empty() {
        result.message = "All Rumi quotes failed".to_string();
        return;
    }

    // Round 2: Quote the venue (ICP→ckUSDC) for all ICP amounts in parallel
    // Subtract ICP transfer fees: output fee from leg 1 + input fee for leg 2
    let icpswap_futs: Vec<_> = stage1.iter().map(|&(_, icp_amount)| {
        let usable = icp_amount.saturating_sub(ICP_FEE * 2);
        venue_quote_icp_to_stable(target.pool, target.icp_is_token0, target.venue, target.fee_pips, usable)
    }).collect();
    let icpswap_results = futures::future::join_all(icpswap_futs).await;

    // Compute profit for each candidate
    let mut best_idx: Option<usize> = None;
    let mut best_profit: i64 = 0;

    let output_vp = if target.uses_vp { prices.virtual_price } else { 0 };
    for (i, icpswap_res) in icpswap_results.into_iter().enumerate() {
        let (input_3usd, icp_amount) = stage1[i];
        let ckusdc_out = match icpswap_res {
            Ok(out) => out,
            Err(_) => continue,
        };

        let input_usd = (input_3usd as u128 * prices.virtual_price as u128 / VP_PRECISION / 100) as i64;
        let output_usd = stable_to_usd_6dec_vp(ckusdc_out, target.stable_decimals, output_vp);
        let fee_usd = stable_to_usd_6dec_vp(target.stable_fee, target.stable_decimals, output_vp)
            + partydex_extra_fee_usd(target.venue, target.stable_fee, target.stable_decimals, output_vp, icp_amount, ckusdc_out);
        let profit = output_usd - input_usd - fee_usd;

        result.candidates_evaluated.push(CandidateResult {
            input_amount: input_3usd,
            icp_amount,
            output_amount: ckusdc_out,
            profit_usd: profit,
        });

        if profit > best_profit {
            best_profit = profit;
            best_idx = Some(result.candidates_evaluated.len() - 1);
        }
    }

    match best_idx {
        Some(idx) => {
            let best = &result.candidates_evaluated[idx];
            result.should_trade = best.profit_usd > 0;
            result.optimal_input_amount = best.input_amount;
            result.expected_icp_amount = best.icp_amount;
            result.expected_output_amount = best.output_amount;
            result.expected_profit_usd = best.profit_usd;
            result.message = format!(
                "[{}] Optimal: {} 3USD → {} ICP → {} {} = {} profit",
                target.strategy_tag, best.input_amount, best.icp_amount, best.output_amount, target.stable_token_name, best.profit_usd
            );
        }
        None => {
            result.message = format!("[{}] No profitable trade found", target.strategy_tag);
        }
    }
}

/// Find optimal trade size for ICPSwap→Rumi direction.
async fn find_optimal_icpswap_to_rumi(
    config: &state::BotConfig,
    target: &IcpswapTarget,
    max_input: u64,
    prices: &PriceData,
    result: &mut DryRunResult,
) {
    let candidates: Vec<u64> = (1..=NUM_CANDIDATES)
        .map(|i| max_input * i / NUM_CANDIDATES)
        .filter(|&a| a > 0)
        .collect();

    // Round 1: Quote the venue (stable→ICP) for all candidates in parallel
    let icpswap_futs: Vec<_> = candidates.iter().map(|&amount| {
        venue_quote_stable_to_icp(target.pool, target.icp_is_token0, target.venue, target.fee_pips, amount)
    }).collect();
    let icpswap_results = futures::future::join_all(icpswap_futs).await;

    let mut stage1: Vec<(u64, u64)> = Vec::new();
    for (i, res) in icpswap_results.into_iter().enumerate() {
        match res {
            Ok(icp_out) if icp_out > 0 => stage1.push((candidates[i], icp_out)),
            _ => {}
        }
    }

    if stage1.is_empty() {
        result.message = format!("[{}] All ICPSwap quotes failed", target.strategy_tag);
        return;
    }

    // Round 2: Quote Rumi (ICP→3USD) for all ICP amounts in parallel
    // Subtract ICP transfer fees: output fee from leg 1 + input fee for leg 2
    let rumi_futs: Vec<_> = stage1.iter().map(|&(_, icp_amount)| {
        let usable = icp_amount.saturating_sub(ICP_FEE * 2);
        prices::fetch_rumi_quote_for_amount(
            config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, usable,
        )
    }).collect();
    let rumi_results = futures::future::join_all(rumi_futs).await;

    let input_vp = if target.uses_vp { prices.virtual_price } else { 0 };
    let mut best_idx: Option<usize> = None;
    let mut best_profit: i64 = 0;

    for (i, rumi_res) in rumi_results.into_iter().enumerate() {
        let (input_ckusdc, icp_amount) = stage1[i];
        let three_usd_out = match rumi_res {
            Ok(out) => out,
            Err(_) => continue,
        };

        let input_usd = stable_to_usd_6dec_vp(input_ckusdc, target.stable_decimals, input_vp);
        let output_usd = (three_usd_out as u128 * prices.virtual_price as u128 / VP_PRECISION / 100) as i64;
        let fee_usd = stable_to_usd_6dec_vp(target.stable_fee, target.stable_decimals, input_vp)
            + partydex_extra_fee_usd(target.venue, target.stable_fee, target.stable_decimals, input_vp, icp_amount, input_ckusdc);
        let profit = output_usd - input_usd - fee_usd;

        result.candidates_evaluated.push(CandidateResult {
            input_amount: input_ckusdc,
            icp_amount,
            output_amount: three_usd_out,
            profit_usd: profit,
        });

        if profit > best_profit {
            best_profit = profit;
            best_idx = Some(result.candidates_evaluated.len() - 1);
        }
    }

    match best_idx {
        Some(idx) => {
            let best = &result.candidates_evaluated[idx];
            result.should_trade = best.profit_usd > 0;
            result.optimal_input_amount = best.input_amount;
            result.expected_icp_amount = best.icp_amount;
            result.expected_output_amount = best.output_amount;
            result.expected_profit_usd = best.profit_usd;
            result.message = format!(
                "[{}] Optimal: {} {} → {} ICP → {} 3USD = {} profit",
                target.strategy_tag, best.input_amount, target.stable_token_name, best.icp_amount, best.output_amount, best.profit_usd
            );
        }
        None => {
            result.message = format!("[{}] No profitable trade found", target.strategy_tag);
        }
    }
}

// ─── Cross-Pool Strategies (B, F, …): ICPSwap pool vs ICPSwap pool ───

pub async fn compute_optimal_cross_pool_trade(
    config: &state::BotConfig,
    target: &CrossPoolTarget,
) -> Result<DryRunResult, String> {
    let mut result = DryRunResult::default();

    // Fetch VP if either side is 3USD (uses_vp) — cycle-cached
    let needs_vp = target.buy_side.uses_vp || target.sell_side.uses_vp;
    let vp = if needs_vp {
        fetch_virtual_price_cached(config.rumi_3pool).await.unwrap_or(1_000_000_000_000_000_000)
    } else { 0 };
    result.virtual_price = vp;

    // Fetch prices from both pools/venues in parallel
    let (buy_res, sell_res) = futures::future::join(
        venue_price_icp(target.buy_side.pool, target.buy_side.icp_is_token0, target.buy_side.venue, target.buy_side.fee_pips),
        venue_price_icp(target.sell_side.pool, target.sell_side.icp_is_token0, target.sell_side.venue, target.sell_side.fee_pips),
    ).await;
    let buy_price_native = buy_res?;
    let sell_price_native = sell_res?;

    let buy_vp = if target.buy_side.uses_vp { vp } else { 0 };
    let sell_vp = if target.sell_side.uses_vp { vp } else { 0 };
    let buy_usd = stable_to_usd_6dec_vp(buy_price_native, target.buy_side.stable_decimals, buy_vp) as u64;
    let sell_usd = stable_to_usd_6dec_vp(sell_price_native, target.sell_side.stable_decimals, sell_vp) as u64;

    result.rumi_price_usd = buy_usd;     // reusing field for buy-side price
    result.icpswap_price_usd = sell_usd;  // reusing field for sell-side price

    let (b, s) = (buy_usd as i64, sell_usd as i64);
    result.spread_bps = if b == 0 || s == 0 { 0 } else {
        ((s - b) * 10_000 / b.min(s)) as i32
    };

    // Fetch balances (cycle-cached — needed for snapshot)
    let (bal_buy, bal_sell) = futures::future::join(
        fetch_balance_cached(target.buy_side.stable_ledger),
        fetch_balance_cached(target.sell_side.stable_ledger),
    ).await;
    result.balance_3usd = bal_buy.unwrap_or(0);    // reusing for buy-side balance
    result.balance_ckusdc = bal_sell.unwrap_or(0);  // reusing for sell-side balance

    let abs_spread = result.spread_bps.unsigned_abs();
    if abs_spread < config.min_spread_bps {
        result.message = format!("[{}] Spread {} bps < minimum {} bps", target.strategy_tag, abs_spread, config.min_spread_bps);
        return Ok(result);
    }

    if result.spread_bps > 0 {
        // ICP cheaper on buy_side → buy there, sell on sell_side
        result.direction = Some(Direction::RumiToIcpswap);
        result.optimal_input_token = Some(Token::ThreeUSD);
        result.expected_output_token = Some(Token::CkUSDC);

        let usable = result.balance_3usd.saturating_sub(target.buy_side.stable_fee);
        let min_native = usd_6dec_to_stable_vp(min_trade_floor_usd(target.buy_side.venue), target.buy_side.stable_decimals, buy_vp);
        if usable < min_native {
            result.message = format!("[{}] Insufficient {} balance", target.strategy_tag, target.buy_side.stable_token_name);
            return Ok(result);
        }

        let max_native = usd_6dec_to_stable_vp(config.max_trade_size_usd, target.buy_side.stable_decimals, buy_vp);
        let max_input = usable.min(max_native);

        find_optimal_cross_pool_forward(target, max_input, buy_vp, sell_vp, &mut result).await;
    } else {
        // ICP cheaper on sell_side → buy there, sell on buy_side
        result.direction = Some(Direction::IcpswapToRumi);
        result.optimal_input_token = Some(Token::CkUSDC);
        result.expected_output_token = Some(Token::ThreeUSD);

        let usable = result.balance_ckusdc.saturating_sub(target.sell_side.stable_fee);
        let min_native = usd_6dec_to_stable_vp(min_trade_floor_usd(target.sell_side.venue), target.sell_side.stable_decimals, sell_vp);
        if usable < min_native {
            result.message = format!("[{}] Insufficient {} balance", target.strategy_tag, target.sell_side.stable_token_name);
            return Ok(result);
        }

        let max_native = usd_6dec_to_stable_vp(config.max_trade_size_usd, target.sell_side.stable_decimals, sell_vp);
        let max_input = usable.min(max_native);

        find_optimal_cross_pool_reverse(target, max_input, buy_vp, sell_vp, &mut result).await;
    }

    Ok(result)
}

/// Cross-pool forward: buy ICP on buy_side, sell on sell_side
async fn find_optimal_cross_pool_forward(
    target: &CrossPoolTarget,
    max_input: u64,
    buy_vp: u64,
    sell_vp: u64,
    result: &mut DryRunResult,
) {
    let candidates: Vec<u64> = (1..=NUM_CANDIDATES)
        .map(|i| max_input * i / NUM_CANDIDATES)
        .filter(|&a| a > 0)
        .collect();

    // Round 1: Quote buy_side pool (stable→ICP)
    let futs1: Vec<_> = candidates.iter().map(|&amount| {
        venue_quote_stable_to_icp(target.buy_side.pool, target.buy_side.icp_is_token0, target.buy_side.venue, target.buy_side.fee_pips, amount)
    }).collect();
    let results1 = futures::future::join_all(futs1).await;

    let mut stage1: Vec<(u64, u64)> = Vec::new();
    for (i, res) in results1.into_iter().enumerate() {
        match res {
            Ok(icp_out) if icp_out > 0 => stage1.push((candidates[i], icp_out)),
            _ => {}
        }
    }

    if stage1.is_empty() {
        result.message = format!("[{}] All {}→ICP quotes failed", target.strategy_tag, target.buy_side.stable_token_name);
        return;
    }

    // Round 2: Quote sell_side pool (ICP→stable)
    let futs2: Vec<_> = stage1.iter().map(|&(_, icp_amount)| {
        let usable = icp_amount.saturating_sub(ICP_FEE * 2);
        venue_quote_icp_to_stable(target.sell_side.pool, target.sell_side.icp_is_token0, target.sell_side.venue, target.sell_side.fee_pips, usable)
    }).collect();
    let results2 = futures::future::join_all(futs2).await;

    let mut best_idx: Option<usize> = None;
    let mut best_profit: i64 = 0;

    for (i, res) in results2.into_iter().enumerate() {
        let (input_amount, icp_amount) = stage1[i];
        let output_amount = match res {
            Ok(out) => out,
            Err(_) => continue,
        };

        let input_usd = stable_to_usd_6dec_vp(input_amount, target.buy_side.stable_decimals, buy_vp);
        let output_usd = stable_to_usd_6dec_vp(output_amount, target.sell_side.stable_decimals, sell_vp);
        let fees = stable_to_usd_6dec_vp(target.buy_side.stable_fee, target.buy_side.stable_decimals, buy_vp)
                 + stable_to_usd_6dec_vp(target.sell_side.stable_fee, target.sell_side.stable_decimals, sell_vp)
                 + partydex_extra_fee_usd(target.buy_side.venue, target.buy_side.stable_fee, target.buy_side.stable_decimals, buy_vp, icp_amount, input_amount)
                 + partydex_extra_fee_usd(target.sell_side.venue, target.sell_side.stable_fee, target.sell_side.stable_decimals, sell_vp, icp_amount, output_amount);
        let profit = output_usd - input_usd - fees;

        result.candidates_evaluated.push(CandidateResult {
            input_amount,
            icp_amount,
            output_amount,
            profit_usd: profit,
        });

        if profit > best_profit {
            best_profit = profit;
            best_idx = Some(result.candidates_evaluated.len() - 1);
        }
    }

    match best_idx {
        Some(idx) => {
            let best = &result.candidates_evaluated[idx];
            result.should_trade = best.profit_usd > 0;
            result.optimal_input_amount = best.input_amount;
            result.expected_icp_amount = best.icp_amount;
            result.expected_output_amount = best.output_amount;
            result.expected_profit_usd = best.profit_usd;
            result.message = format!(
                "[{}] Optimal: {} {} → {} ICP → {} {} = {} profit",
                target.strategy_tag, best.input_amount, target.buy_side.stable_token_name,
                best.icp_amount, best.output_amount, target.sell_side.stable_token_name, best.profit_usd
            );
        }
        None => {
            result.message = format!("[{}] No profitable trade found", target.strategy_tag);
        }
    }
}

/// Cross-pool reverse: buy ICP on sell_side, sell on buy_side
async fn find_optimal_cross_pool_reverse(
    target: &CrossPoolTarget,
    max_input: u64,
    buy_vp: u64,
    sell_vp: u64,
    result: &mut DryRunResult,
) {
    let candidates: Vec<u64> = (1..=NUM_CANDIDATES)
        .map(|i| max_input * i / NUM_CANDIDATES)
        .filter(|&a| a > 0)
        .collect();

    // Round 1: Quote sell_side pool (stable→ICP)
    let futs1: Vec<_> = candidates.iter().map(|&amount| {
        venue_quote_stable_to_icp(target.sell_side.pool, target.sell_side.icp_is_token0, target.sell_side.venue, target.sell_side.fee_pips, amount)
    }).collect();
    let results1 = futures::future::join_all(futs1).await;

    let mut stage1: Vec<(u64, u64)> = Vec::new();
    for (i, res) in results1.into_iter().enumerate() {
        match res {
            Ok(icp_out) if icp_out > 0 => stage1.push((candidates[i], icp_out)),
            _ => {}
        }
    }

    if stage1.is_empty() {
        result.message = format!("[{}] All {}→ICP quotes failed", target.strategy_tag, target.sell_side.stable_token_name);
        return;
    }

    // Round 2: Quote buy_side pool (ICP→stable)
    let futs2: Vec<_> = stage1.iter().map(|&(_, icp_amount)| {
        let usable = icp_amount.saturating_sub(ICP_FEE * 2);
        venue_quote_icp_to_stable(target.buy_side.pool, target.buy_side.icp_is_token0, target.buy_side.venue, target.buy_side.fee_pips, usable)
    }).collect();
    let results2 = futures::future::join_all(futs2).await;

    let mut best_idx: Option<usize> = None;
    let mut best_profit: i64 = 0;

    for (i, res) in results2.into_iter().enumerate() {
        let (input_amount, icp_amount) = stage1[i];
        let output_amount = match res {
            Ok(out) => out,
            Err(_) => continue,
        };

        let input_usd = stable_to_usd_6dec_vp(input_amount, target.sell_side.stable_decimals, sell_vp);
        let output_usd = stable_to_usd_6dec_vp(output_amount, target.buy_side.stable_decimals, buy_vp);
        let fees = stable_to_usd_6dec_vp(target.sell_side.stable_fee, target.sell_side.stable_decimals, sell_vp)
                 + stable_to_usd_6dec_vp(target.buy_side.stable_fee, target.buy_side.stable_decimals, buy_vp)
                 + partydex_extra_fee_usd(target.sell_side.venue, target.sell_side.stable_fee, target.sell_side.stable_decimals, sell_vp, icp_amount, input_amount)
                 + partydex_extra_fee_usd(target.buy_side.venue, target.buy_side.stable_fee, target.buy_side.stable_decimals, buy_vp, icp_amount, output_amount);
        let profit = output_usd - input_usd - fees;

        result.candidates_evaluated.push(CandidateResult {
            input_amount,
            icp_amount,
            output_amount,
            profit_usd: profit,
        });

        if profit > best_profit {
            best_profit = profit;
            best_idx = Some(result.candidates_evaluated.len() - 1);
        }
    }

    match best_idx {
        Some(idx) => {
            let best = &result.candidates_evaluated[idx];
            result.should_trade = best.profit_usd > 0;
            result.optimal_input_amount = best.input_amount;
            result.expected_icp_amount = best.icp_amount;
            result.expected_output_amount = best.output_amount;
            result.expected_profit_usd = best.profit_usd;
            result.message = format!(
                "[{}] Optimal: {} {} → {} ICP → {} {} = {} profit",
                target.strategy_tag, best.input_amount, target.sell_side.stable_token_name,
                best.icp_amount, best.output_amount, target.buy_side.stable_token_name, best.profit_usd
            );
        }
        None => {
            result.message = format!("[{}] No profitable trade found", target.strategy_tag);
        }
    }
}

// ─── Strategy S (icUSD/BOB triangular) evaluator ───
//
// BOB is the transit asset (the role ICP plays in strategies A–R); endpoints
// are icUSD and banded ICP inventory. Reference: fair icUSD-per-BOB =
// (ICP per BOB from the BOB/ICP pool) × (USD per ICP from the best stable/ICP
// quote), with icUSD treated as $1 (matching stable_to_usd_6dec). Unused
// until the cycle wiring task; allow(dead_code) keeps cargo check clean.

/// Strategy S venue bundle, built inline in `run_arb_cycle` like the other
/// targets. allow(dead_code): `bob_ledger`/`icp_ledger` are part of the
/// plan-specified shape but current call sites read those off BotConfig.
#[allow(dead_code)]
pub struct BobTarget {
    pub icusd_bob_pool: Principal,
    pub icusd_is_token0: bool,
    pub bob_icp_pool: Principal,
    pub bob_icp_icp_is_token0: bool,
    pub bob_ledger: Principal,
    pub bob_fee: u64,
    pub icusd_ledger: Principal,
    pub icusd_fee: u64,
    pub icp_ledger: Principal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BobDirection {
    /// icUSD → BOB (our pool) → ICP (BOB/ICP). BOB cheap in our pool.
    Forward,
    /// ICP → BOB (BOB/ICP) → icUSD (our pool). BOB rich in our pool.
    Reverse,
}

pub struct BobDryRun {
    pub should_trade: bool,
    pub direction: Option<BobDirection>,
    /// Native input units: icUSD (8 dec) for Forward, ICP e8s for Reverse.
    pub input_amount: u64,
    /// Expected transit BOB from leg 1 (gross quote output, 8 dec).
    pub bob_amount: u64,
    /// Expected leg-2 output: ICP e8s (Forward) or icUSD 8 dec (Reverse).
    pub output_amount: u64,
    pub expected_profit_usd: i64,
    pub spread_bps: u32,
    pub usd_per_icp_6dec: u64,
    pub pool_price_icusd_per_bob_8dec: u64,
    pub ref_price_icusd_per_bob_8dec: u64,
}

impl Default for BobDryRun {
    fn default() -> Self {
        Self {
            should_trade: false,
            direction: None,
            input_amount: 0,
            bob_amount: 0,
            output_amount: 0,
            expected_profit_usd: 0,
            spread_bps: 0,
            usd_per_icp_6dec: 0,
            pool_price_icusd_per_bob_8dec: 0,
            ref_price_icusd_per_bob_8dec: 0,
        }
    }
}

/// USD mark (6-dec) for a BOB amount at the icUSD-per-BOB reference price
/// (8-dec icUSD per 1 BOB; icUSD ≈ $1, so /100 lifts 8-dec icUSD to 6-dec USD).
pub(crate) fn mark_bob_usd(bob_e8s: u64, ref_price_icusd_per_bob_8dec: u64) -> i64 {
    (bob_e8s as u128 * ref_price_icusd_per_bob_8dec as u128 / 100_000_000 / 100) as i64
}

async fn find_optimal_bob(config: &state::BotConfig, target: &BobTarget) -> Result<BobDryRun, String> {
    let mut result = BobDryRun::default();

    // 1. Reference USD/ICP once, at a 1-ICP probe size. Median across the
    //    candidate pools, not the max — see median_stable_usd_per_icp for
    //    the manipulation rationale.
    let usd_per_icp = match median_stable_usd_per_icp(config, 100_000_000).await {
        Some(rate) if rate > 0 => rate,
        _ => {
            state::log_activity("dry_run", "[S] No stable/ICP reference quote available");
            return Ok(result);
        }
    };
    result.usd_per_icp_6dec = usd_per_icp;

    // 2. Pool price and reference price at a 1-BOB probe (both quotes sell BOB).
    const BOB_PROBE: u64 = 100_000_000; // 1 BOB (8 dec)
    let (pool_res, ref_res) = futures::future::join(
        prices::fetch_icpswap_quote_for_amount(target.icusd_bob_pool, BOB_PROBE, !target.icusd_is_token0),
        prices::fetch_icpswap_quote_for_amount(target.bob_icp_pool, BOB_PROBE, !target.bob_icp_icp_is_token0),
    ).await;
    let pool_icusd_per_bob = pool_res?; // icUSD (8 dec) out per 1 BOB in
    let ref_icp_per_bob = ref_res?;     // ICP (e8s) out per 1 BOB in
    // Reference icUSD-per-BOB (8 dec): (icp_e8s × usd_6dec / 1e8) is 6-dec
    // USD per BOB; ×100 lifts to 8-dec icUSD (icUSD ≈ $1). Combined: /1e6.
    let ref_icusd_per_bob = (ref_icp_per_bob as u128 * usd_per_icp as u128 / 1_000_000) as u64;
    result.pool_price_icusd_per_bob_8dec = pool_icusd_per_bob;
    result.ref_price_icusd_per_bob_8dec = ref_icusd_per_bob;
    if pool_icusd_per_bob == 0 || ref_icusd_per_bob == 0 {
        state::log_activity("dry_run", "[S] Zero probe quote (pool or reference)");
        return Ok(result);
    }

    // 3. Spread (pool vs reference deviation) and direction.
    let (p, r) = (pool_icusd_per_bob as i64, ref_icusd_per_bob as i64);
    result.spread_bps = ((p - r) * 10_000 / p.min(r)).unsigned_abs() as u32;
    if (result.spread_bps as u64) < config.bob_min_spread_bps {
        state::log_activity("dry_run", &format!(
            "[S] Spread {} bps < minimum {} bps (pool {} vs ref {} icUSD/BOB 8dec)",
            result.spread_bps, config.bob_min_spread_bps, pool_icusd_per_bob, ref_icusd_per_bob
        ));
        return Ok(result);
    }
    // Pool pays less icUSD per BOB than fair → BOB cheap in our pool → Forward.
    let direction = if p < r { BobDirection::Forward } else { BobDirection::Reverse };
    result.direction = Some(direction);

    // 4. Candidate ladder: bob_max_trade_size_usd × 1/4..4/4, converted to
    //    input units. Forward is additionally capped by usable icUSD balance
    //    (Reverse is not — the execution top-up leg covers ICP shortfalls).
    let max_input: u64 = match direction {
        BobDirection::Forward => {
            let bal_icusd = fetch_balance_cached(target.icusd_ledger).await.unwrap_or(0);
            let usable = bal_icusd.saturating_sub(target.icusd_fee);
            let min_native = usd_6dec_to_stable(min_trade_floor_usd(state::Venue::Icpswap), 8);
            if usable < min_native {
                state::log_activity("dry_run", "[S] Insufficient icUSD balance");
                return Ok(result);
            }
            usable.min(usd_6dec_to_stable(config.bob_max_trade_size_usd, 8))
        }
        BobDirection::Reverse => {
            // USD → ICP e8s at the reference rate.
            (config.bob_max_trade_size_usd as u128 * 100_000_000 / usd_per_icp as u128) as u64
        }
    };
    let candidates: Vec<u64> = (1..=NUM_CANDIDATES)
        .map(|i| max_input * i / NUM_CANDIDATES)
        .filter(|&a| a > 0)
        .collect();
    if candidates.is_empty() {
        state::log_activity("dry_run", "[S] No candidate sizes > 0");
        return Ok(result);
    }

    // Round 1: input → BOB on the entry pool.
    let futs1: Vec<_> = candidates.iter().map(|&amount| match direction {
        BobDirection::Forward =>
            prices::fetch_icpswap_quote_for_amount(target.icusd_bob_pool, amount, target.icusd_is_token0),
        BobDirection::Reverse =>
            prices::fetch_icpswap_quote_for_amount(target.bob_icp_pool, amount, target.bob_icp_icp_is_token0),
    }).collect();
    let results1 = futures::future::join_all(futs1).await;

    let mut stage1: Vec<(u64, u64)> = Vec::new();
    for (i, res) in results1.into_iter().enumerate() {
        match res {
            Ok(bob_out) if bob_out > 0 => stage1.push((candidates[i], bob_out)),
            _ => {}
        }
    }
    if stage1.is_empty() {
        state::log_activity("dry_run", &format!(
            "[S] All {}→BOB quotes failed",
            if direction == BobDirection::Forward { "icUSD" } else { "ICP" }
        ));
        return Ok(result);
    }

    // Round 2: transit BOB → output on the exit pool. The bot actually
    // receives (bob_out - bob_fee) after the pool withdraw and spends another
    // bob_fee on the next-hop deposit — same *2 convention as ICP transit.
    let futs2: Vec<_> = stage1.iter().map(|&(_, bob_out)| {
        let usable_bob = bob_out.saturating_sub(target.bob_fee * 2);
        match direction {
            BobDirection::Forward =>
                prices::fetch_icpswap_quote_for_amount(target.bob_icp_pool, usable_bob, !target.bob_icp_icp_is_token0),
            BobDirection::Reverse =>
                prices::fetch_icpswap_quote_for_amount(target.icusd_bob_pool, usable_bob, !target.icusd_is_token0),
        }
    }).collect();
    let results2 = futures::future::join_all(futs2).await;

    // Ledger fees marked to USD: one icUSD fee (entry or exit stable) plus the
    // two transit BOB fees at the reference mark. Slightly conservative — the
    // BOB fees are also reflected in the reduced round-2 quote input above.
    let fees_usd = stable_to_usd_6dec(target.icusd_fee, 8)
        + mark_bob_usd(target.bob_fee * 2, ref_icusd_per_bob);

    let mut best: Option<(u64, u64, u64, i64)> = None; // (input, bob, output, profit)
    for (i, res) in results2.into_iter().enumerate() {
        let (input_amount, bob_out) = stage1[i];
        let output_amount = match res {
            Ok(out) if out > 0 => out,
            _ => continue,
        };
        let profit = match direction {
            BobDirection::Forward => {
                // Terminal ICP joins inventory net of transfer + next-hop
                // approval spend (same convention as the A–R leg-2 usable math).
                let icp_out_net = output_amount.saturating_sub(ICP_FEE * 2);
                mark_icp_usd(icp_out_net, usd_per_icp)
                    - stable_to_usd_6dec(input_amount, 8)
                    - fees_usd
            }
            BobDirection::Reverse => {
                stable_to_usd_6dec(output_amount, 8)
                    - mark_icp_usd(input_amount, usd_per_icp)
                    - fees_usd
            }
        };
        if best.map_or(profit > 0, |(_, _, _, bp)| profit > bp) {
            best = Some((input_amount, bob_out, output_amount, profit));
        }
    }

    match best {
        Some((input_amount, bob_amount, output_amount, profit)) => {
            result.input_amount = input_amount;
            result.bob_amount = bob_amount;
            result.output_amount = output_amount;
            result.expected_profit_usd = profit;
            result.should_trade = profit > 0
                && (config.min_profit_usd <= 0 || profit >= config.min_profit_usd);
            let (in_tok, out_tok) = match direction {
                BobDirection::Forward => ("icUSD", "ICP"),
                BobDirection::Reverse => ("ICP", "icUSD"),
            };
            state::log_activity("dry_run", &format!(
                "[S] Optimal: {} {} → {} BOB → {} {} = {} profit (spread {} bps, pool {} vs ref {} icUSD/BOB 8dec)",
                input_amount, in_tok, bob_amount, output_amount, out_tok, profit,
                result.spread_bps, pool_icusd_per_bob, ref_icusd_per_bob
            ));
        }
        None => {
            state::log_activity("dry_run", &format!(
                "[S] No profitable trade found (spread {} bps, pool {} vs ref {} icUSD/BOB 8dec)",
                result.spread_bps, pool_icusd_per_bob, ref_icusd_per_bob
            ));
        }
    }

    Ok(result)
}

// ─── Execute Trades ───

async fn execute_rumi_to_icpswap(config: &state::BotConfig, target: &IcpswapTarget, dry_run: &DryRunResult) {
    let slippage = slippage_bps_clamped(config);
    let trade_amount_3usd = dry_run.optimal_input_amount;
    let min_icp_out = dry_run.expected_icp_amount * (10_000 - slippage) / 10_000;

    // 3USD cost in 6-dec USD
    let cost_usd_6dec = (trade_amount_3usd as u128 * dry_run.virtual_price as u128 / VP_PRECISION / 100) as i64;

    state::log_activity("swap", &format!(
        "[{}] Leg 1: Rumi swap {} 3USD → ICP (min: {})", target.strategy_tag, trade_amount_3usd, min_icp_out
    ));

    let icp_out = match swaps::rumi_swap(
        config.rumi_amm, RUMI_POOL_ID, config.three_usd_ledger, trade_amount_3usd, min_icp_out,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("Leg 1 OK: {} 3USD → {} ICP", trade_amount_3usd, amount));
            // Record Leg 1: sold 3USD (stablecoin), bought ICP (transit)
            state::append_trade_leg(state::TradeLeg {
                timestamp: ic_cdk::api::time(),
                leg_type: state::LegType::Leg1,
                dex: "Rumi".to_string(),
                sold_token: "3USD".to_string(),
                sold_amount: trade_amount_3usd,
                bought_token: "ICP".to_string(),
                bought_amount: amount,
                sold_usd_value: cost_usd_6dec,
                bought_usd_value: 0, // ICP is transit, no USD value
                fees_usd: 0,         // no stablecoin fee on this leg
            });
            state::mutate_state(|s| {
                s.pending_exit = Some(state::PendingExit {
                    entry_pool: state::Pool::RumiThreeUsd,
                    intended_exit_pool: target.pool_enum,
                    timestamp: ic_cdk::api::time(),
                    icp_amount: amount,
                });
            });
            amount
        }
        Err(e) => {
            let msg = format!("[{}] Rumi swap 3USD→ICP failed: {}", target.strategy_tag, e);
            state::log_activity("swap", &format!("Leg 1 FAILED: {}", msg));
            log_error(&msg);
            return;
        }
    };

    // Rumi reports gross output, but the bot receives (icp_out - ICP_FEE) after the
    // output transfer fee, and ICPSwap's depositFromAndSwap costs another ICP_FEE.
    let usable_icp = icp_out.saturating_sub(ICP_FEE * 2);
    let min_stable_out = dry_run.expected_output_amount * (10_000 - slippage) / 10_000;

    state::log_activity("swap", &format!(
        "[{}] Leg 2: ICPSwap swap {} ICP → {} (min: {}, raw from Rumi: {})", target.strategy_tag, usable_icp, target.stable_token_name, min_stable_out, icp_out
    ));

    let stable_out = match venue_swap_icp_to_stable(
        target.pool, target.icp_is_token0, target.venue, target.fee_pips,
        usable_icp, min_stable_out, config.icp_ledger, target.stable_ledger, target.stable_fee,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("[{}] Leg 2 OK: {} ICP → {} {}", target.strategy_tag, icp_out, amount, target.stable_token_name));
            state::append_trade_leg(state::TradeLeg {
                timestamp: ic_cdk::api::time(),
                leg_type: state::LegType::Leg2,
                dex: target.label.to_string(),
                sold_token: "ICP".to_string(),
                sold_amount: usable_icp,
                bought_token: target.stable_token_name.to_string(),
                bought_amount: amount,
                sold_usd_value: 0,            // ICP is transit
                bought_usd_value: stable_to_usd_6dec_vp(amount, target.stable_decimals, if target.uses_vp { dry_run.virtual_price } else { 0 }),
                fees_usd: stable_to_usd_6dec_vp(target.stable_fee, target.stable_decimals, if target.uses_vp { dry_run.virtual_price } else { 0 }),
            });
            state::mutate_state(|s| { s.pending_exit = None; });
            amount
        }
        Err(e) => {
            let msg = format!("[{}] ICPSwap swap ICP→{} failed (holding {} ICP): {}", target.strategy_tag, target.stable_token_name, icp_out, e);
            state::log_activity("swap", &format!("[{}] Leg 2 FAILED: {}", target.strategy_tag, msg));
            log_error(&msg);
            return; // Leg 1 is already recorded; drain will pick up the ICP
        }
    };

    let target_vp = if target.uses_vp { dry_run.virtual_price } else { 0 };
    // Mirror find_optimal_rumi_to_icpswap's fee model: the PartyDEX leg here is
    // the ICP→stable sell (target.venue), so the extra deposit/withdraw ledger
    // fees only bite when target.venue == PartyDex (0 for Icpswap).
    let net_profit = stable_to_usd_6dec_vp(stable_out, target.stable_decimals, target_vp)
        - cost_usd_6dec
        - stable_to_usd_6dec_vp(target.stable_fee, target.stable_decimals, target_vp)
        - partydex_extra_fee_usd(target.venue, target.stable_fee, target.stable_decimals, target_vp, icp_out, stable_out);
    state::log_activity("trade", &format!(
        "[{}] COMPLETE RumiToIcpswap: {} 3USD → {} ICP → {} {} | profit: {} (6dec USD)",
        target.strategy_tag, trade_amount_3usd, icp_out, stable_out, target.stable_token_name, net_profit
    ));
}

async fn execute_icpswap_to_rumi(config: &state::BotConfig, target: &IcpswapTarget, dry_run: &DryRunResult) {
    let slippage = slippage_bps_clamped(config);
    let trade_amount_stable = dry_run.optimal_input_amount;
    let min_icp_out = dry_run.expected_icp_amount * (10_000 - slippage) / 10_000;

    state::log_activity("swap", &format!(
        "[{}] Leg 1: ICPSwap swap {} {} → ICP (min: {})", target.strategy_tag, trade_amount_stable, target.stable_token_name, min_icp_out
    ));

    let icp_out = match venue_swap_stable_to_icp(
        target.pool, target.icp_is_token0, target.venue, target.fee_pips,
        trade_amount_stable, min_icp_out, target.stable_ledger, target.stable_fee, config.icp_ledger,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("[{}] Leg 1 OK: {} {} → {} ICP", target.strategy_tag, trade_amount_stable, target.stable_token_name, amount));
            let target_vp = if target.uses_vp { dry_run.virtual_price } else { 0 };
            state::append_trade_leg(state::TradeLeg {
                timestamp: ic_cdk::api::time(),
                leg_type: state::LegType::Leg1,
                dex: target.label.to_string(),
                sold_token: target.stable_token_name.to_string(),
                sold_amount: trade_amount_stable,
                bought_token: "ICP".to_string(),
                bought_amount: amount,
                sold_usd_value: stable_to_usd_6dec_vp(trade_amount_stable, target.stable_decimals, target_vp),
                bought_usd_value: 0, // ICP is transit
                fees_usd: stable_to_usd_6dec_vp(target.stable_fee, target.stable_decimals, target_vp),
            });
            state::mutate_state(|s| {
                s.pending_exit = Some(state::PendingExit {
                    entry_pool: target.pool_enum,
                    intended_exit_pool: state::Pool::RumiThreeUsd,
                    timestamp: ic_cdk::api::time(),
                    icp_amount: amount,
                });
            });
            amount
        }
        Err(e) => {
            let msg = format!("[{}] ICPSwap swap {}→ICP failed: {}", target.strategy_tag, target.stable_token_name, e);
            state::log_activity("swap", &format!("[{}] Leg 1 FAILED: {}", target.strategy_tag, msg));
            log_error(&msg);
            return;
        }
    };

    // ICPSwap reports gross output, but the bot receives (icp_out - ICP_FEE) after the
    // output transfer fee, and Rumi's transfer_from costs another ICP_FEE.
    let usable_icp = icp_out.saturating_sub(ICP_FEE * 2);
    let min_3usd_out = dry_run.expected_output_amount * (10_000 - slippage) / 10_000;

    state::log_activity("swap", &format!(
        "[{}] Leg 2: Rumi swap {} ICP → 3USD (min: {}, raw from ICPSwap: {})", target.strategy_tag, usable_icp, min_3usd_out, icp_out
    ));

    // Fetch VP for 3USD USD valuation
    let vp = prices::fetch_virtual_price(config.rumi_3pool).await.unwrap_or(1_000_000_000_000_000_000);

    let three_usd_out = match swaps::rumi_swap(
        config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, usable_icp, min_3usd_out,
    ).await {
        Ok(amount) => {
            let out_usd_6dec = (amount as u128 * vp as u128 / VP_PRECISION / 100) as i64;
            state::log_activity("swap", &format!("[{}] Leg 2 OK: {} ICP → {} 3USD", target.strategy_tag, icp_out, amount));
            state::append_trade_leg(state::TradeLeg {
                timestamp: ic_cdk::api::time(),
                leg_type: state::LegType::Leg2,
                dex: "Rumi".to_string(),
                sold_token: "ICP".to_string(),
                sold_amount: usable_icp,
                bought_token: "3USD".to_string(),
                bought_amount: amount,
                sold_usd_value: 0,       // ICP is transit
                bought_usd_value: out_usd_6dec,
                fees_usd: 0,             // no stablecoin fee on this leg
            });
            state::mutate_state(|s| { s.pending_exit = None; });
            amount
        }
        Err(e) => {
            let msg = format!("[{}] Rumi swap ICP→3USD failed (holding {} ICP): {}", target.strategy_tag, icp_out, e);
            state::log_activity("swap", &format!("[{}] Leg 2 FAILED: {}", target.strategy_tag, msg));
            log_error(&msg);
            return; // Leg 1 is already recorded; drain will pick up the ICP
        }
    };

    let target_vp = if target.uses_vp { dry_run.virtual_price } else { 0 };
    let input_usd_6dec = stable_to_usd_6dec_vp(trade_amount_stable, target.stable_decimals, target_vp);
    let output_usd_6dec = (three_usd_out as u128 * vp as u128 / VP_PRECISION / 100) as i64;
    // Mirror find_optimal_icpswap_to_rumi's fee model: the PartyDEX leg here is
    // the stable→ICP buy (target.venue), extra ledger fees only for PartyDex.
    let net_profit = output_usd_6dec - input_usd_6dec
        - stable_to_usd_6dec_vp(target.stable_fee, target.stable_decimals, target_vp)
        - partydex_extra_fee_usd(target.venue, target.stable_fee, target.stable_decimals, target_vp, icp_out, trade_amount_stable);

    state::log_activity("trade", &format!(
        "[{}] COMPLETE IcpswapToRumi: {} {} → {} ICP → {} 3USD | profit: {} (6dec USD)",
        target.strategy_tag, trade_amount_stable, target.stable_token_name, icp_out, three_usd_out, net_profit
    ));
}

// ─── Cross-Pool Execute Trades ───

/// Cross-pool forward: buy_side stable → ICP → sell_side stable
async fn execute_cross_pool_forward(target: &CrossPoolTarget, dry_run: &DryRunResult) {
    let (slippage, icp_ledger) = state::read_state(|s| (slippage_bps_clamped(&s.config), s.config.icp_ledger));
    let trade_amount = dry_run.optimal_input_amount;
    let min_icp_out = dry_run.expected_icp_amount * (10_000 - slippage) / 10_000;
    let buy_vp = if target.buy_side.uses_vp { dry_run.virtual_price } else { 0 };
    let sell_vp = if target.sell_side.uses_vp { dry_run.virtual_price } else { 0 };
    let cost_usd_6dec = stable_to_usd_6dec_vp(trade_amount, target.buy_side.stable_decimals, buy_vp);

    state::log_activity("swap", &format!(
        "[{}] Leg 1: {} swap {} {} → ICP (min: {})",
        target.strategy_tag, target.buy_side.dex_label, trade_amount, target.buy_side.stable_token_name, min_icp_out
    ));

    let icp_out = match venue_swap_stable_to_icp(
        target.buy_side.pool, target.buy_side.icp_is_token0, target.buy_side.venue, target.buy_side.fee_pips,
        trade_amount, min_icp_out, target.buy_side.stable_ledger, target.buy_side.stable_fee, icp_ledger,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("[{}] Leg 1 OK: {} {} → {} ICP",
                target.strategy_tag, trade_amount, target.buy_side.stable_token_name, amount));
            state::append_trade_leg(state::TradeLeg {
                timestamp: ic_cdk::api::time(),
                leg_type: state::LegType::Leg1,
                dex: target.buy_side.dex_label.to_string(),
                sold_token: target.buy_side.stable_token_name.to_string(),
                sold_amount: trade_amount,
                bought_token: "ICP".to_string(),
                bought_amount: amount,
                sold_usd_value: cost_usd_6dec,
                bought_usd_value: 0,
                fees_usd: stable_to_usd_6dec_vp(target.buy_side.stable_fee, target.buy_side.stable_decimals, buy_vp),
            });
            state::mutate_state(|s| {
                s.pending_exit = Some(state::PendingExit {
                    entry_pool: target.buy_side.pool_enum,
                    intended_exit_pool: target.sell_side.pool_enum,
                    timestamp: ic_cdk::api::time(),
                    icp_amount: amount,
                });
            });
            amount
        }
        Err(e) => {
            let msg = format!("[{}] {} {}→ICP failed: {}",
                target.strategy_tag, target.buy_side.dex_label, target.buy_side.stable_token_name, e);
            state::log_activity("swap", &format!("[{}] Leg 1 FAILED: {}", target.strategy_tag, msg));
            log_error(&msg);
            return;
        }
    };

    let usable_icp = icp_out.saturating_sub(ICP_FEE * 2);
    let min_out = dry_run.expected_output_amount * (10_000 - slippage) / 10_000;

    state::log_activity("swap", &format!(
        "[{}] Leg 2: {} swap {} ICP → {} (min: {})",
        target.strategy_tag, target.sell_side.dex_label, usable_icp, target.sell_side.stable_token_name, min_out
    ));

    let output = match venue_swap_icp_to_stable(
        target.sell_side.pool, target.sell_side.icp_is_token0, target.sell_side.venue, target.sell_side.fee_pips,
        usable_icp, min_out, icp_ledger, target.sell_side.stable_ledger, target.sell_side.stable_fee,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("[{}] Leg 2 OK: {} ICP → {} {}",
                target.strategy_tag, icp_out, amount, target.sell_side.stable_token_name));
            state::append_trade_leg(state::TradeLeg {
                timestamp: ic_cdk::api::time(),
                leg_type: state::LegType::Leg2,
                dex: target.sell_side.dex_label.to_string(),
                sold_token: "ICP".to_string(),
                sold_amount: usable_icp,
                bought_token: target.sell_side.stable_token_name.to_string(),
                bought_amount: amount,
                sold_usd_value: 0,
                bought_usd_value: stable_to_usd_6dec_vp(amount, target.sell_side.stable_decimals, sell_vp),
                fees_usd: stable_to_usd_6dec_vp(target.sell_side.stable_fee, target.sell_side.stable_decimals, sell_vp),
            });
            state::mutate_state(|s| { s.pending_exit = None; });
            amount
        }
        Err(e) => {
            let msg = format!("[{}] {} ICP→{} failed (holding {} ICP): {}",
                target.strategy_tag, target.sell_side.dex_label, target.sell_side.stable_token_name, icp_out, e);
            state::log_activity("swap", &format!("[{}] Leg 2 FAILED: {}", target.strategy_tag, msg));
            log_error(&msg);
            return;
        }
    };

    // Mirror find_optimal_cross_pool_forward's fee model: extra PartyDEX
    // deposit/withdraw ledger fees per leg, keyed on each side's venue (0 for
    // the Icpswap side). buy_side is the stable→ICP leg (input = trade_amount),
    // sell_side is the ICP→stable leg (output = output).
    let net_profit = stable_to_usd_6dec_vp(output, target.sell_side.stable_decimals, sell_vp)
        - cost_usd_6dec
        - stable_to_usd_6dec_vp(target.sell_side.stable_fee, target.sell_side.stable_decimals, sell_vp)
        - stable_to_usd_6dec_vp(target.buy_side.stable_fee, target.buy_side.stable_decimals, buy_vp)
        - partydex_extra_fee_usd(target.buy_side.venue, target.buy_side.stable_fee, target.buy_side.stable_decimals, buy_vp, icp_out, trade_amount)
        - partydex_extra_fee_usd(target.sell_side.venue, target.sell_side.stable_fee, target.sell_side.stable_decimals, sell_vp, icp_out, output);
    state::log_activity("trade", &format!(
        "[{}] COMPLETE {}→{}: {} {} → {} ICP → {} {} | profit: {} (6dec USD)",
        target.strategy_tag, target.buy_side.stable_token_name, target.sell_side.stable_token_name,
        trade_amount, target.buy_side.stable_token_name, icp_out,
        output, target.sell_side.stable_token_name, net_profit
    ));
}

/// Cross-pool reverse: sell_side stable → ICP → buy_side stable
async fn execute_cross_pool_reverse(target: &CrossPoolTarget, dry_run: &DryRunResult) {
    let (slippage, icp_ledger) = state::read_state(|s| (slippage_bps_clamped(&s.config), s.config.icp_ledger));
    let trade_amount = dry_run.optimal_input_amount;
    let min_icp_out = dry_run.expected_icp_amount * (10_000 - slippage) / 10_000;
    let buy_vp = if target.buy_side.uses_vp { dry_run.virtual_price } else { 0 };
    let sell_vp = if target.sell_side.uses_vp { dry_run.virtual_price } else { 0 };

    state::log_activity("swap", &format!(
        "[{}] Leg 1: {} swap {} {} → ICP (min: {})",
        target.strategy_tag, target.sell_side.dex_label, trade_amount, target.sell_side.stable_token_name, min_icp_out
    ));

    let icp_out = match venue_swap_stable_to_icp(
        target.sell_side.pool, target.sell_side.icp_is_token0, target.sell_side.venue, target.sell_side.fee_pips,
        trade_amount, min_icp_out, target.sell_side.stable_ledger, target.sell_side.stable_fee, icp_ledger,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("[{}] Leg 1 OK: {} {} → {} ICP",
                target.strategy_tag, trade_amount, target.sell_side.stable_token_name, amount));
            state::append_trade_leg(state::TradeLeg {
                timestamp: ic_cdk::api::time(),
                leg_type: state::LegType::Leg1,
                dex: target.sell_side.dex_label.to_string(),
                sold_token: target.sell_side.stable_token_name.to_string(),
                sold_amount: trade_amount,
                bought_token: "ICP".to_string(),
                bought_amount: amount,
                sold_usd_value: stable_to_usd_6dec_vp(trade_amount, target.sell_side.stable_decimals, sell_vp),
                bought_usd_value: 0,
                fees_usd: stable_to_usd_6dec_vp(target.sell_side.stable_fee, target.sell_side.stable_decimals, sell_vp),
            });
            state::mutate_state(|s| {
                s.pending_exit = Some(state::PendingExit {
                    entry_pool: target.sell_side.pool_enum,
                    intended_exit_pool: target.buy_side.pool_enum,
                    timestamp: ic_cdk::api::time(),
                    icp_amount: amount,
                });
            });
            amount
        }
        Err(e) => {
            let msg = format!("[{}] {} {}→ICP failed: {}",
                target.strategy_tag, target.sell_side.dex_label, target.sell_side.stable_token_name, e);
            state::log_activity("swap", &format!("[{}] Leg 1 FAILED: {}", target.strategy_tag, msg));
            log_error(&msg);
            return;
        }
    };

    let usable_icp = icp_out.saturating_sub(ICP_FEE * 2);
    let min_out = dry_run.expected_output_amount * (10_000 - slippage) / 10_000;

    state::log_activity("swap", &format!(
        "[{}] Leg 2: {} swap {} ICP → {} (min: {})",
        target.strategy_tag, target.buy_side.dex_label, usable_icp, target.buy_side.stable_token_name, min_out
    ));

    let output = match venue_swap_icp_to_stable(
        target.buy_side.pool, target.buy_side.icp_is_token0, target.buy_side.venue, target.buy_side.fee_pips,
        usable_icp, min_out, icp_ledger, target.buy_side.stable_ledger, target.buy_side.stable_fee,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("[{}] Leg 2 OK: {} ICP → {} {}",
                target.strategy_tag, icp_out, amount, target.buy_side.stable_token_name));
            state::append_trade_leg(state::TradeLeg {
                timestamp: ic_cdk::api::time(),
                leg_type: state::LegType::Leg2,
                dex: target.buy_side.dex_label.to_string(),
                sold_token: "ICP".to_string(),
                sold_amount: usable_icp,
                bought_token: target.buy_side.stable_token_name.to_string(),
                bought_amount: amount,
                sold_usd_value: 0,
                bought_usd_value: stable_to_usd_6dec_vp(amount, target.buy_side.stable_decimals, buy_vp),
                fees_usd: stable_to_usd_6dec_vp(target.buy_side.stable_fee, target.buy_side.stable_decimals, buy_vp),
            });
            state::mutate_state(|s| { s.pending_exit = None; });
            amount
        }
        Err(e) => {
            let msg = format!("[{}] {} ICP→{} failed (holding {} ICP): {}",
                target.strategy_tag, target.buy_side.dex_label, target.buy_side.stable_token_name, icp_out, e);
            state::log_activity("swap", &format!("[{}] Leg 2 FAILED: {}", target.strategy_tag, msg));
            log_error(&msg);
            return;
        }
    };

    let input_usd = stable_to_usd_6dec_vp(trade_amount, target.sell_side.stable_decimals, sell_vp);
    let output_usd = stable_to_usd_6dec_vp(output, target.buy_side.stable_decimals, buy_vp);
    // Mirror find_optimal_cross_pool_reverse's fee model: sell_side is the
    // stable→ICP leg (input = trade_amount), buy_side is the ICP→stable leg
    // (output = output); extra ledger fees only for the PartyDex side.
    let net_profit = output_usd - input_usd
        - stable_to_usd_6dec_vp(target.sell_side.stable_fee, target.sell_side.stable_decimals, sell_vp)
        - stable_to_usd_6dec_vp(target.buy_side.stable_fee, target.buy_side.stable_decimals, buy_vp)
        - partydex_extra_fee_usd(target.sell_side.venue, target.sell_side.stable_fee, target.sell_side.stable_decimals, sell_vp, icp_out, trade_amount)
        - partydex_extra_fee_usd(target.buy_side.venue, target.buy_side.stable_fee, target.buy_side.stable_decimals, buy_vp, icp_out, output);

    state::log_activity("trade", &format!(
        "[{}] COMPLETE {}→{}: {} {} → {} ICP → {} {} | profit: {} (6dec USD)",
        target.strategy_tag, target.sell_side.stable_token_name, target.buy_side.stable_token_name,
        trade_amount, target.sell_side.stable_token_name, icp_out,
        output, target.buy_side.stable_token_name, net_profit
    ));
}

// ─── Strategy S Execution ───

/// Executes a Strategy S top-up: buy ICP on the winning stable pool from a
/// `best_stable_icp_per_usd` quote. Returns (icp_received, dex_label,
/// stable_token_name, sold_usd_6dec, fee_usd_6dec).
async fn execute_topup_swap(
    config: &state::BotConfig,
    quote: &TopUpQuote,
    min_icp_out: u64,
) -> Result<(u64, &'static str, &'static str, i64, i64), String> {
    match quote.pool {
        state::Pool::RumiThreeUsd => {
            let vp = fetch_virtual_price_cached(config.rumi_3pool).await
                .unwrap_or(1_000_000_000_000_000_000);
            swaps::rumi_swap(config.rumi_amm, RUMI_POOL_ID, config.three_usd_ledger, quote.stable_in_amount, min_icp_out).await
                .map(|icp| (icp, "Rumi", "3USD", stable_to_usd_6dec_vp(quote.stable_in_amount, 8, vp), 0i64))
                .map_err(|e| e.to_string())
        }
        state::Pool::IcpswapCkusdc => {
            swaps::icpswap_swap(config.icpswap_pool, quote.stable_in_amount, !config.icpswap_icp_is_token0, min_icp_out, CKUSDC_FEE, ICP_FEE).await
                .map(|icp| (icp, "ICPSwap", "ckUSDC", stable_to_usd_6dec(quote.stable_in_amount, 6), stable_to_usd_6dec(CKUSDC_FEE, 6)))
                .map_err(|e| e.to_string())
        }
        state::Pool::IcpswapIcusd => {
            swaps::icpswap_swap(config.icpswap_icusd_pool, quote.stable_in_amount, !config.icpswap_icusd_icp_is_token0, min_icp_out, ICUSD_FEE, ICP_FEE).await
                .map(|icp| (icp, "ICPSwap-icUSD", "icUSD", stable_to_usd_6dec(quote.stable_in_amount, 8), stable_to_usd_6dec(ICUSD_FEE, 8)))
                .map_err(|e| e.to_string())
        }
        state::Pool::IcpswapCkusdt => {
            swaps::icpswap_swap(config.icpswap_ckusdt_pool, quote.stable_in_amount, !config.icpswap_ckusdt_icp_is_token0, min_icp_out, CKUSDT_FEE, ICP_FEE).await
                .map(|icp| (icp, "ICPSwap-ckUSDT", "ckUSDT", stable_to_usd_6dec(quote.stable_in_amount, 6), stable_to_usd_6dec(CKUSDT_FEE, 6)))
                .map_err(|e| e.to_string())
        }
        other => Err(format!("{:?} is not a top-up candidate pool", other)),
    }
}

/// Strategy S execution. Unlike A–R (which book ICP legs at zero USD), every
/// ICP and BOB leg here is marked to the reference USD quote fetched at trade
/// time (spec §3) so each S trade's net_profit_usd is complete on completion.
async fn execute_bob(config: &state::BotConfig, target: &BobTarget, dry_run: &BobDryRun) {
    let slippage = slippage_bps_clamped(config);
    let usd_per_icp = dry_run.usd_per_icp_6dec;
    let ref_price = dry_run.ref_price_icusd_per_bob_8dec;
    let direction = match dry_run.direction {
        Some(d) => d,
        None => return,
    };
    // Total ledger-fee model, matching find_optimal_bob: one icUSD fee plus
    // the two transit BOB fees at the reference mark.
    let fees_total = stable_to_usd_6dec(target.icusd_fee, 8)
        + mark_bob_usd(target.bob_fee * 2, ref_price);

    match direction {
        BobDirection::Forward => {
            // Leg 1: icUSD → BOB on the icUSD/BOB pool.
            let trade_amount = dry_run.input_amount;
            let min_bob_out = dry_run.bob_amount * (10_000 - slippage) / 10_000;
            state::log_activity("swap", &format!(
                "[S] Leg 1: ICPSwap-icUSD-BOB swap {} icUSD → BOB (min: {})", trade_amount, min_bob_out
            ));
            let bob_out = match swaps::icpswap_swap(
                target.icusd_bob_pool, trade_amount, target.icusd_is_token0,
                min_bob_out, target.icusd_fee, target.bob_fee,
            ).await {
                Ok(amount) => {
                    state::log_activity("swap", &format!("[S] Leg 1 OK: {} icUSD → {} BOB", trade_amount, amount));
                    state::append_trade_leg(state::TradeLeg {
                        timestamp: ic_cdk::api::time(),
                        leg_type: state::LegType::Leg1,
                        dex: "ICPSwap-icUSD-BOB".to_string(),
                        sold_token: "icUSD".to_string(),
                        sold_amount: trade_amount,
                        bought_token: "BOB".to_string(),
                        bought_amount: amount,
                        sold_usd_value: stable_to_usd_6dec(trade_amount, 8),
                        bought_usd_value: mark_bob_usd(amount, ref_price),
                        fees_usd: stable_to_usd_6dec(target.icusd_fee, 8),
                    });
                    state::mutate_state(|s| {
                        s.pending_bob_exit = Some(state::PendingBobExit {
                            entry_pool: state::BobPool::IcusdBob,
                            bob_amount: amount,
                        });
                    });
                    amount
                }
                Err(e) => {
                    let msg = format!("[S] ICPSwap-icUSD-BOB icUSD→BOB failed: {}", e);
                    state::log_activity("swap", &format!("[S] Leg 1 FAILED: {}", msg));
                    log_error(&msg);
                    return;
                }
            };

            // Leg 2: BOB → ICP on the BOB/ICP pool.
            let usable_bob = bob_out.saturating_sub(target.bob_fee * 2);
            let min_icp_out = dry_run.output_amount * (10_000 - slippage) / 10_000;
            state::log_activity("swap", &format!(
                "[S] Leg 2: ICPSwap-BOB-ICP swap {} BOB → ICP (min: {}, raw from leg 1: {})",
                usable_bob, min_icp_out, bob_out
            ));
            let icp_out = match swaps::icpswap_swap(
                target.bob_icp_pool, usable_bob, !target.bob_icp_icp_is_token0,
                min_icp_out, target.bob_fee, ICP_FEE,
            ).await {
                Ok(amount) => {
                    state::log_activity("swap", &format!("[S] Leg 2 OK: {} BOB → {} ICP", usable_bob, amount));
                    state::append_trade_leg(state::TradeLeg {
                        timestamp: ic_cdk::api::time(),
                        leg_type: state::LegType::Leg2,
                        dex: "ICPSwap-BOB-ICP".to_string(),
                        sold_token: "BOB".to_string(),
                        sold_amount: usable_bob,
                        bought_token: "ICP".to_string(),
                        bought_amount: amount,
                        sold_usd_value: mark_bob_usd(usable_bob, ref_price),
                        bought_usd_value: mark_icp_usd(amount, usd_per_icp),
                        fees_usd: mark_bob_usd(target.bob_fee * 2, ref_price),
                    });
                    state::mutate_state(|s| { s.pending_bob_exit = None; });
                    amount
                }
                Err(e) => {
                    let msg = format!("[S] ICPSwap-BOB-ICP BOB→ICP failed (holding {} BOB): {}", bob_out, e);
                    state::log_activity("swap", &format!("[S] Leg 2 FAILED: {}", msg));
                    log_error(&msg);
                    return; // drain_residual_bob recovers next cycle
                }
            };

            let net_profit = mark_icp_usd(icp_out.saturating_sub(ICP_FEE * 2), usd_per_icp)
                - stable_to_usd_6dec(trade_amount, 8)
                - fees_total;
            state::log_activity("trade", &format!(
                "[S] COMPLETE Forward: {} icUSD → {} BOB → {} ICP | profit: {} (6dec USD)",
                trade_amount, bob_out, icp_out, net_profit
            ));
        }
        BobDirection::Reverse => {
            let icp_needed = dry_run.input_amount;

            // Band check: never let the trade push inventory below the floor.
            // If it would, prepend a TopUp leg buying the shortfall from
            // whichever stable pool gives the most ICP per USD.
            let icp_balance = match fetch_balance(config.icp_ledger).await {
                Ok(b) => b,
                Err(e) => {
                    log_error(&format!("[S] ICP balance read failed before reverse trade: {}", e));
                    return;
                }
            };
            if icp_balance.saturating_sub(icp_needed) < config.icp_inventory_floor_e8s {
                let shortfall_e8s = icp_needed
                    .saturating_add(config.icp_inventory_floor_e8s)
                    .saturating_sub(icp_balance);
                // Inflate by the slippage tolerance: the swap's min_out is
                // quote × (1 - slippage), so an exactly-to-the-floor quote
                // could fill up to slippage_bps short of the floor. Sizing
                // the quote up by the same factor keeps the worst-case fill
                // on/above the floor. (slippage is clamped ≤ 10_000; the
                // .max(1) guards the degenerate 100%-slippage divisor.)
                let shortfall_e8s =
                    (shortfall_e8s as u128 * 10_000 / (10_000 - slippage).max(1) as u128) as u64;
                let shortfall_usd = mark_icp_usd(shortfall_e8s, usd_per_icp).max(0) as u64;
                state::log_activity("swap", &format!(
                    "[S] TopUp: balance {} - needed {} would breach floor {}; buying ~{} e8s (~{} 6dec USD)",
                    icp_balance, icp_needed, config.icp_inventory_floor_e8s, shortfall_e8s, shortfall_usd
                ));
                // Deliberately best_stable_icp_per_usd (max output), NOT the
                // median: this is a real execution leg, so best-execution is
                // correct — the fill is bounded by its own fresh quote's
                // slippage floor, unlike the reference/marks where an
                // outlier max would skew accounting.
                let quote = match best_stable_icp_per_usd(config, shortfall_usd).await {
                    Some(q) if q.icp_out_e8s > 0 => q,
                    _ => {
                        log_error("[S] TopUp quote unavailable; aborting reverse trade");
                        return;
                    }
                };
                let min_icp = quote.icp_out_e8s * (10_000 - slippage) / 10_000;
                match execute_topup_swap(config, &quote, min_icp).await {
                    Ok((icp_received, dex, token_name, sold_usd, fee_usd)) => {
                        state::log_activity("swap", &format!(
                            "[S] TopUp OK: {} {} → {} ICP via {}", quote.stable_in_amount, token_name, icp_received, dex
                        ));
                        state::append_trade_leg(state::TradeLeg {
                            timestamp: ic_cdk::api::time(),
                            leg_type: state::LegType::TopUp,
                            dex: dex.to_string(),
                            sold_token: token_name.to_string(),
                            sold_amount: quote.stable_in_amount,
                            bought_token: "ICP".to_string(),
                            bought_amount: icp_received,
                            sold_usd_value: sold_usd,
                            bought_usd_value: mark_icp_usd(icp_received, usd_per_icp),
                            fees_usd: fee_usd,
                        });
                    }
                    Err(e) => {
                        log_error(&format!("[S] TopUp swap failed; aborting reverse trade: {}", e));
                        return;
                    }
                }
            }

            // Leg 1: ICP → BOB on the BOB/ICP pool.
            let min_bob_out = dry_run.bob_amount * (10_000 - slippage) / 10_000;
            state::log_activity("swap", &format!(
                "[S] Leg 1: ICPSwap-BOB-ICP swap {} ICP → BOB (min: {})", icp_needed, min_bob_out
            ));
            let bob_out = match swaps::icpswap_swap(
                target.bob_icp_pool, icp_needed, target.bob_icp_icp_is_token0,
                min_bob_out, ICP_FEE, target.bob_fee,
            ).await {
                Ok(amount) => {
                    state::log_activity("swap", &format!("[S] Leg 1 OK: {} ICP → {} BOB", icp_needed, amount));
                    state::append_trade_leg(state::TradeLeg {
                        timestamp: ic_cdk::api::time(),
                        leg_type: state::LegType::Leg1,
                        dex: "ICPSwap-BOB-ICP".to_string(),
                        sold_token: "ICP".to_string(),
                        sold_amount: icp_needed,
                        bought_token: "BOB".to_string(),
                        bought_amount: amount,
                        sold_usd_value: mark_icp_usd(icp_needed, usd_per_icp),
                        bought_usd_value: mark_bob_usd(amount, ref_price),
                        fees_usd: 0,
                    });
                    state::mutate_state(|s| {
                        s.pending_bob_exit = Some(state::PendingBobExit {
                            entry_pool: state::BobPool::BobIcp,
                            bob_amount: amount,
                        });
                    });
                    amount
                }
                Err(e) => {
                    let msg = format!("[S] ICPSwap-BOB-ICP ICP→BOB failed: {}", e);
                    state::log_activity("swap", &format!("[S] Leg 1 FAILED: {}", msg));
                    log_error(&msg);
                    return;
                }
            };

            // Leg 2: BOB → icUSD on the icUSD/BOB pool.
            let usable_bob = bob_out.saturating_sub(target.bob_fee * 2);
            let min_icusd_out = dry_run.output_amount * (10_000 - slippage) / 10_000;
            state::log_activity("swap", &format!(
                "[S] Leg 2: ICPSwap-icUSD-BOB swap {} BOB → icUSD (min: {}, raw from leg 1: {})",
                usable_bob, min_icusd_out, bob_out
            ));
            let icusd_out = match swaps::icpswap_swap(
                target.icusd_bob_pool, usable_bob, !target.icusd_is_token0,
                min_icusd_out, target.bob_fee, target.icusd_fee,
            ).await {
                Ok(amount) => {
                    state::log_activity("swap", &format!("[S] Leg 2 OK: {} BOB → {} icUSD", usable_bob, amount));
                    state::append_trade_leg(state::TradeLeg {
                        timestamp: ic_cdk::api::time(),
                        leg_type: state::LegType::Leg2,
                        dex: "ICPSwap-icUSD-BOB".to_string(),
                        sold_token: "BOB".to_string(),
                        sold_amount: usable_bob,
                        bought_token: "icUSD".to_string(),
                        bought_amount: amount,
                        sold_usd_value: mark_bob_usd(usable_bob, ref_price),
                        bought_usd_value: stable_to_usd_6dec(amount, 8),
                        fees_usd: stable_to_usd_6dec(target.icusd_fee, 8)
                            + mark_bob_usd(target.bob_fee * 2, ref_price),
                    });
                    state::mutate_state(|s| { s.pending_bob_exit = None; });
                    amount
                }
                Err(e) => {
                    let msg = format!("[S] ICPSwap-icUSD-BOB BOB→icUSD failed (holding {} BOB): {}", bob_out, e);
                    state::log_activity("swap", &format!("[S] Leg 2 FAILED: {}", msg));
                    log_error(&msg);
                    return; // drain_residual_bob recovers next cycle
                }
            };

            let net_profit = stable_to_usd_6dec(icusd_out, 8)
                - mark_icp_usd(icp_needed, usd_per_icp)
                - fees_total;
            state::log_activity("trade", &format!(
                "[S] COMPLETE Reverse: {} ICP → {} BOB → {} icUSD | profit: {} (6dec USD)",
                icp_needed, bob_out, icusd_out, net_profit
            ));
        }
    }
}

// ─── Helpers ───

async fn drain_residual_icp(config: &state::BotConfig) -> Result<(), String> {
    // Skip drain if volume bot is mid-trade — its tokens are temporarily
    // in the default account and must not be touched.
    if volume::is_volume_cycle_in_progress() {
        return Ok(());
    }
    let slippage = slippage_bps_clamped(config);

    let icp_balance = fetch_balance(config.icp_ledger).await?;

    // Steady state: only skim inventory above the band ceiling. During a
    // pending_exit recovery (stranded leg-2 ICP), drain down to the floor —
    // the leg1_cap below still limits the drain to what that trade put here.
    // Also exclude any ICP stranded by the volume bot — those belong to it.
    let has_pending = state::read_state(|s| s.pending_exit.is_some());
    let band_reserve = if has_pending {
        config.icp_inventory_floor_e8s
    } else {
        config.icp_inventory_ceiling_e8s
    };
    let volume_stranded = state::read_state(|s| s.volume_stranded_icp);
    let reserved = band_reserve.saturating_add(volume_stranded);
    let drainable = icp_balance.saturating_sub(reserved);
    if drainable <= ICP_FEE * 2 {
        // Normally just "nothing to skim" — but with a pending_exit it means
        // stranded leg-2 ICP can't be recovered (e.g. inventory floor set
        // above the live balance). Surface that instead of stalling silently.
        if has_pending {
            state::log_activity("drain", &format!(
                "pending_exit set but only {} ICP above reserve {} (balance {}); recovery deferred",
                drainable, reserved, icp_balance
            ));
        }
        return Ok(());
    }

    // Cap drain to the Leg1 ICP amount — only drain what the arb bot put there.
    // If pending_exit has an icp_amount, use it as the ceiling.
    let leg1_cap = state::read_state(|s| {
        s.pending_exit.as_ref().and_then(|pe| if pe.icp_amount > 0 { Some(pe.icp_amount) } else { None })
    });
    let drain_amount = match leg1_cap {
        Some(cap) => drainable.min(cap).saturating_sub(ICP_FEE),
        None => drainable - ICP_FEE,
    };
    if drain_amount <= ICP_FEE {
        return Ok(());
    }

    state::log_activity("drain", &format!("Draining {} residual ICP (balance: {})", drain_amount, icp_balance));

    // Determine the entry pool to AVOID. Primary source: pending_exit recorded
    // at Leg 1 success. Fallback safety net: the most recent Leg1 trade_leg
    // (works even after a canister restart that cleared pending_exit).
    let pending_exit: Option<state::PendingExit> =
        state::read_state(|s| s.pending_exit.clone());
    // Look only at the SINGLE most recent Leg1 and use its pool if the dex maps.
    // The closure returns Some for ANY Leg1, so the scan stops at the newest one
    // instead of skipping past unmapped legs into stale history: a strategy S
    // Leg1 ("ICPSwap-icUSD-BOB" / "ICPSwap-BOB-ICP") doesn't map to an ICP pool,
    // so it yields Some(None) here — `.flatten()` turns that into "no fallback
    // exclusion" rather than reaching back to an already-resolved A–R Leg1.
    let fallback_entry_pool: Option<state::Pool> = state::find_map_last_trade_leg(|l| {
        match l.leg_type {
            state::LegType::Leg1 => Some(dex_string_to_pool(&l.dex)),
            _ => None,
        }
    }).flatten();
    let entry_pool: Option<state::Pool> = pending_exit.as_ref().map(|pe| pe.entry_pool).or(fallback_entry_pool);
    let intended_exit: Option<state::Pool> = pending_exit.as_ref().map(|pe| pe.intended_exit_pool);

    // Quote all four pools in parallel where available.
    let rumi_quote = prices::fetch_rumi_price(config.rumi_amm, RUMI_POOL_ID, config.icp_ledger);
    let icpswap_ck_quote = prices::fetch_icpswap_price(config.icpswap_pool, config.icpswap_icp_is_token0);
    let vp_fut = prices::fetch_virtual_price(config.rumi_3pool);
    let has_icusd_pool = config.icpswap_icusd_pool != Principal::anonymous();
    let icusd_resolved = state::read_state(|s| s.icusd_token_ordering_resolved);
    let has_ckusdt_pool = config.icpswap_ckusdt_pool != Principal::anonymous();
    let ckusdt_resolved = state::read_state(|s| s.ckusdt_token_ordering_resolved);

    let (rumi_res, icpswap_ck_res, vp_res) =
        futures::future::join3(rumi_quote, icpswap_ck_quote, vp_fut).await;

    let icpswap_icusd_res: Result<u64, String> = if has_icusd_pool && icusd_resolved {
        prices::fetch_icpswap_price(config.icpswap_icusd_pool, config.icpswap_icusd_icp_is_token0).await
    } else {
        Err("icUSD pool unavailable".to_string())
    };

    let icpswap_ckusdt_res: Result<u64, String> = if has_ckusdt_pool && ckusdt_resolved {
        prices::fetch_icpswap_price(config.icpswap_ckusdt_pool, config.icpswap_ckusdt_icp_is_token0).await
    } else {
        Err("ckUSDT pool unavailable".to_string())
    };

    let has_3usd_icpswap_pool = config.icpswap_3usd_pool != Principal::anonymous();
    let three_usd_icpswap_resolved = state::read_state(|s| s.icpswap_3usd_token_ordering_resolved);
    let icpswap_3usd_res: Result<u64, String> = if has_3usd_icpswap_pool && three_usd_icpswap_resolved {
        prices::fetch_icpswap_price(config.icpswap_3usd_pool, config.icpswap_3usd_icp_is_token0).await
    } else {
        Err("3USD ICPSwap pool unavailable".to_string())
    };

    let vp_val = vp_res.as_ref().copied().unwrap_or(1_000_000_000_000_000_000);

    // Build candidate list: (Pool, usd_out_6dec, min_out_raw)
    #[derive(Clone, Copy)]
    struct Candidate {
        pool: state::Pool,
        usd_out: u64,
        min_out: u64,
    }
    let mut candidates: Vec<Candidate> = Vec::new();

    if let Ok(quote_per_icp) = rumi_res {
        // quote_per_icp is 3USD (8 dec) per ICP. Scale to drain_amount.
        let out_3usd = (quote_per_icp as u128 * drain_amount as u128 / 100_000_000) as u64;
        let usd_out = (out_3usd as u128 * vp_val as u128 / VP_PRECISION / 100) as u64;
        let min_out = (out_3usd as u128 * (10_000 - slippage) as u128 / 10_000) as u64;
        candidates.push(Candidate { pool: state::Pool::RumiThreeUsd, usd_out, min_out });
    }
    if let Ok(quote_per_icp) = icpswap_ck_res {
        // quote is ckUSDC (6 dec) per ICP; ckUSDC ≈ USD.
        let out_ck = (quote_per_icp as u128 * drain_amount as u128 / 100_000_000) as u64;
        let min_out = (out_ck as u128 * (10_000 - slippage) as u128 / 10_000) as u64;
        candidates.push(Candidate { pool: state::Pool::IcpswapCkusdc, usd_out: out_ck, min_out });
    }
    if let Ok(quote_per_icp) = icpswap_icusd_res {
        // icUSD is 8 dec ≈ $1. Scale then divide by 100 for 6-dec USD.
        let out_icusd = (quote_per_icp as u128 * drain_amount as u128 / 100_000_000) as u64;
        let usd_out = (out_icusd / 100) as u64;
        let min_out = (out_icusd as u128 * (10_000 - slippage) as u128 / 10_000) as u64;
        candidates.push(Candidate { pool: state::Pool::IcpswapIcusd, usd_out, min_out });
    }
    if let Ok(quote_per_icp) = icpswap_ckusdt_res {
        // ckUSDT is 6 dec ≈ $1.
        let out_ck = (quote_per_icp as u128 * drain_amount as u128 / 100_000_000) as u64;
        let min_out = (out_ck as u128 * (10_000 - slippage) as u128 / 10_000) as u64;
        candidates.push(Candidate { pool: state::Pool::IcpswapCkusdt, usd_out: out_ck, min_out });
    }
    if let Ok(quote_per_icp) = icpswap_3usd_res {
        // 3USD is 8 dec, worth VP/1e18 USD each. Scale to drain_amount then VP-adjust for USD.
        let out_3usd = (quote_per_icp as u128 * drain_amount as u128 / 100_000_000) as u64;
        let usd_out = (out_3usd as u128 * vp_val as u128 / VP_PRECISION / 100) as u64;
        let min_out = (out_3usd as u128 * (10_000 - slippage) as u128 / 10_000) as u64;
        candidates.push(Candidate { pool: state::Pool::IcpswapThreeUsd, usd_out, min_out });
    }

    if candidates.is_empty() {
        state::mutate_state(|s| s.pending_exit = None);
        return Err("No pool quotes available during ICP drain".to_string());
    }

    // Filter out the entry pool — we never want to sell back into it.
    let filtered: Vec<Candidate> = candidates.iter()
        .filter(|c| entry_pool.map_or(true, |ep| ep != c.pool))
        .copied()
        .collect();

    if filtered.is_empty() {
        state::log_activity("drain", &format!(
            "Holding {} ICP: entry pool {:?} is the only option; refusing to drain back into it.",
            drain_amount, entry_pool
        ));
        // Do NOT clear pending_exit — next cycle may open more options.
        return Ok(());
    }

    // Build an ordered try list: intended exit first (if present and not filtered),
    // then the rest sorted by USD output desc.
    let mut order: Vec<Candidate> = Vec::new();
    if let Some(ip) = intended_exit {
        if let Some(c) = filtered.iter().find(|c| c.pool == ip) {
            order.push(*c);
        }
    }
    let mut rest: Vec<Candidate> = filtered.iter()
        .filter(|c| !order.iter().any(|o| o.pool == c.pool))
        .copied()
        .collect();
    rest.sort_by(|a, b| b.usd_out.cmp(&a.usd_out));
    order.extend(rest);

    // Helper to record a drain leg
    fn record_drain_leg(dex: &str, icp_sold: u64, token_out: &str, amount_out: u64, usd_value_out: i64, fees: i64) {
        state::log_activity("drain", &format!("Drained {} ICP → {} {} via {}", icp_sold, amount_out, token_out, dex));
        state::append_trade_leg(state::TradeLeg {
            timestamp: ic_cdk::api::time(),
            leg_type: state::LegType::Drain,
            dex: dex.to_string(),
            sold_token: "ICP".to_string(),
            sold_amount: icp_sold,
            bought_token: token_out.to_string(),
            bought_amount: amount_out,
            sold_usd_value: 0,  // ICP is transit
            bought_usd_value: usd_value_out,
            fees_usd: fees,
        });
    }

    // Try each candidate in order until one succeeds.
    let mut any_success = false;
    let mut remaining_amount = drain_amount;
    for (i, cand) in order.iter().enumerate() {
        // Refresh balance if this is a retry after failure. Respect the full
        // reserve (band + volume-stranded ICP) and never exceed the original
        // leg1-capped drain amount.
        if i > 0 {
            let bal = fetch_balance(config.icp_ledger).await.unwrap_or(0);
            let d = bal.saturating_sub(reserved);
            if d <= ICP_FEE * 2 { break; }
            remaining_amount = (d - ICP_FEE).min(remaining_amount);
        }
        // Use 0 slippage on fallback attempts — we already failed once, just get out.
        let min_out = if i == 0 { cand.min_out } else { 0 };
        let result = match cand.pool {
            state::Pool::RumiThreeUsd => {
                swaps::rumi_swap(config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, remaining_amount, min_out).await
                    .map(|out| {
                        let usd = (out as u128 * vp_val as u128 / VP_PRECISION / 100) as i64;
                        ("Rumi", "3USD", out, usd, 0i64)
                    })
            }
            state::Pool::IcpswapCkusdc => {
                swaps::icpswap_swap(config.icpswap_pool, remaining_amount, config.icpswap_icp_is_token0, min_out, ICP_FEE, CKUSDC_FEE).await
                    .map(|out| ("ICPSwap", "ckUSDC", out, out as i64, CKUSDC_FEE as i64))
            }
            state::Pool::IcpswapIcusd => {
                swaps::icpswap_swap(config.icpswap_icusd_pool, remaining_amount, config.icpswap_icusd_icp_is_token0, min_out, ICP_FEE, ICUSD_FEE).await
                    .map(|out| {
                        let usd = (out / 100) as i64;
                        ("ICPSwap-icUSD", "icUSD", out, usd, (ICUSD_FEE / 100) as i64)
                    })
            }
            state::Pool::IcpswapCkusdt => {
                swaps::icpswap_swap(config.icpswap_ckusdt_pool, remaining_amount, config.icpswap_ckusdt_icp_is_token0, min_out, ICP_FEE, CKUSDT_FEE).await
                    .map(|out| ("ICPSwap-ckUSDT", "ckUSDT", out, out as i64, CKUSDT_FEE as i64))
            }
            state::Pool::IcpswapThreeUsd => {
                // 3USD has 0 transfer fee
                swaps::icpswap_swap(config.icpswap_3usd_pool, remaining_amount, config.icpswap_3usd_icp_is_token0, min_out, ICP_FEE, 0).await
                    .map(|out| {
                        let usd = (out as u128 * vp_val as u128 / VP_PRECISION / 100) as i64;
                        ("ICPSwap-3USD", "3USD", out, usd, 0i64)
                    })
            }
            // PartyDEX pools are never added as drain candidates (decision #5 — PartyDEX
            // legs always settle ICP back to the main balance, so ICPSwap/Rumi drain
            // already covers recovery). Unreachable in practice; kept for exhaustiveness.
            state::Pool::PartyDexIcpCkusdc | state::Pool::PartyDexIcpCkusdt => {
                Err(swaps::SwapError::SwapFailed("PartyDEX pools are not drain candidates".to_string()))
            }
        };
        match result {
            Ok((dex, tok, out, usd, fees)) => {
                record_drain_leg(dex, remaining_amount, tok, out, usd, fees);
                any_success = true;
                break;
            }
            Err(e) => {
                state::log_activity("drain", &format!("Drain via {:?} failed: {}", cand.pool, e));
            }
        }
    }

    // Clear pending_exit ONLY on success — mirror of drain_residual_bob. On a
    // total drain failure the leg-2 ICP is still stranded here, so the marker
    // must survive to keep the entry-pool exclusion intact for next cycle's
    // retry; clearing it regardless would let a retry sell that ICP back into
    // the very pool it was bought from.
    if !any_success && !order.is_empty() {
        return Err("All drain attempts failed".to_string());
    }
    state::mutate_state(|s| s.pending_exit = None);
    Ok(())
}

/// Strategy S stranded-BOB recovery. The bot never intentionally holds BOB
/// between cycles, so any balance above dust is residue from a failed leg 2
/// (or a manual send). Sells it via whichever BOB pool it did NOT enter
/// through — mirror of `drain_residual_icp`'s entry-pool exclusion — with
/// reference USD marks on the leg record (spec §3). No-op while both BOB
/// pools are unconfigured/unresolved or the balance is dust.
async fn drain_residual_bob(config: &state::BotConfig) -> Result<(), String> {
    let (bob_icp_resolved, icusd_bob_resolved) = state::read_state(|s| {
        (s.bob_icp_ordering_resolved, s.icusd_bob_ordering_resolved)
    });
    let has_bob_icp = config.icpswap_bob_icp_pool != Principal::anonymous() && bob_icp_resolved;
    let has_icusd_bob = config.icpswap_icusd_bob_pool != Principal::anonymous() && icusd_bob_resolved;
    if !has_bob_icp && !has_icusd_bob {
        return Ok(());
    }

    let bob_balance = fetch_balance(config.bob_ledger).await?;
    if bob_balance <= config.bob_ledger_fee * 10 {
        // Dust — not worth a swap. If a pending exit is still marked, the BOB
        // it tracked is gone (nothing left to recover): clear it so it can't
        // hold the drain-relevance gate open forever.
        if state::read_state(|s| s.pending_bob_exit.is_some()) {
            state::mutate_state(|s| s.pending_bob_exit = None);
        }
        return Ok(());
    }

    let slippage = slippage_bps_clamped(config);
    // Leave one transfer fee of headroom for the pool deposit.
    let drain_amount = bob_balance.saturating_sub(config.bob_ledger_fee);

    state::log_activity("drain", &format!(
        "Draining {} residual BOB (balance: {})", drain_amount, bob_balance
    ));

    let entry_pool: Option<state::BobPool> =
        state::read_state(|s| s.pending_bob_exit.as_ref().map(|pe| pe.entry_pool));

    // USD/ICP reference for marking the BOB→ICP candidate (0 if unavailable —
    // the icUSD candidate still carries a real mark in that case). Median,
    // not max — same manipulation rationale as find_optimal_bob's reference.
    let usd_per_icp = median_stable_usd_per_icp(config, 100_000_000).await.unwrap_or(0);

    // Quote both pools selling drain_amount BOB; usd_out is the marked value.
    struct BobCandidate {
        pool: state::BobPool,
        usd_out: i64,
        min_out: u64,
    }
    let mut candidates: Vec<BobCandidate> = Vec::new();
    if has_bob_icp {
        match prices::fetch_icpswap_quote_for_amount(
            config.icpswap_bob_icp_pool, drain_amount, !config.icpswap_bob_icp_icp_is_token0,
        ).await {
            Ok(icp_out) if icp_out > 0 => candidates.push(BobCandidate {
                pool: state::BobPool::BobIcp,
                usd_out: mark_icp_usd(icp_out, usd_per_icp),
                min_out: (icp_out as u128 * (10_000 - slippage) as u128 / 10_000) as u64,
            }),
            _ => {}
        }
    }
    if has_icusd_bob {
        match prices::fetch_icpswap_quote_for_amount(
            config.icpswap_icusd_bob_pool, drain_amount, !config.icpswap_icusd_bob_icusd_is_token0,
        ).await {
            Ok(icusd_out) if icusd_out > 0 => candidates.push(BobCandidate {
                pool: state::BobPool::IcusdBob,
                usd_out: stable_to_usd_6dec(icusd_out, 8),
                min_out: (icusd_out as u128 * (10_000 - slippage) as u128 / 10_000) as u64,
            }),
            _ => {}
        }
    }

    if candidates.is_empty() {
        // Keep pending_bob_exit — quotes may come back next cycle, and the
        // entry-pool exclusion must survive for the retry (see below).
        return Err("No pool quotes available during BOB drain".to_string());
    }

    // Never sell back into the entry pool.
    let mut order: Vec<BobCandidate> = candidates.into_iter()
        .filter(|c| entry_pool.map_or(true, |ep| ep != c.pool))
        .collect();
    if order.is_empty() {
        state::log_activity("drain", &format!(
            "Holding {} BOB: entry pool {:?} is the only option; refusing to drain back into it.",
            drain_amount, entry_pool
        ));
        // Do NOT clear pending_bob_exit — next cycle may open more options.
        return Ok(());
    }
    order.sort_by(|a, b| b.usd_out.cmp(&a.usd_out));

    let mut any_success = false;
    for (i, cand) in order.iter().enumerate() {
        // Use 0 slippage on fallback attempts — we already failed once, just get out.
        let min_out = if i == 0 { cand.min_out } else { 0 };
        // Per-unit fee mark from this candidate's own quote.
        let fee_usd = (config.bob_ledger_fee as u128 * cand.usd_out.max(0) as u128
            / drain_amount as u128) as i64;
        let result = match cand.pool {
            state::BobPool::BobIcp => {
                swaps::icpswap_swap(
                    config.icpswap_bob_icp_pool, drain_amount, !config.icpswap_bob_icp_icp_is_token0,
                    min_out, config.bob_ledger_fee, ICP_FEE,
                ).await.map(|out| ("ICPSwap-BOB-ICP", "ICP", out, mark_icp_usd(out, usd_per_icp)))
            }
            state::BobPool::IcusdBob => {
                swaps::icpswap_swap(
                    config.icpswap_icusd_bob_pool, drain_amount, !config.icpswap_icusd_bob_icusd_is_token0,
                    min_out, config.bob_ledger_fee, ICUSD_FEE,
                ).await.map(|out| ("ICPSwap-icUSD-BOB", "icUSD", out, stable_to_usd_6dec(out, 8)))
            }
        };
        match result {
            Ok((dex, token_out, amount_out, usd_value_out)) => {
                state::log_activity("drain", &format!(
                    "Drained {} BOB → {} {} via {}", drain_amount, amount_out, token_out, dex
                ));
                state::append_trade_leg(state::TradeLeg {
                    timestamp: ic_cdk::api::time(),
                    leg_type: state::LegType::Drain,
                    dex: dex.to_string(),
                    sold_token: "BOB".to_string(),
                    sold_amount: drain_amount,
                    // BOB marked at the realized exit value (reference marks,
                    // spec §3) — unlike ICP drains, which book sold at 0.
                    sold_usd_value: usd_value_out,
                    bought_token: token_out.to_string(),
                    bought_amount: amount_out,
                    bought_usd_value: usd_value_out,
                    fees_usd: fee_usd,
                });
                any_success = true;
                break;
            }
            Err(e) => {
                state::log_activity("drain", &format!("BOB drain via {:?} failed: {}", cand.pool, e));
            }
        }
    }

    // Clear pending_bob_exit ONLY on success. Deliberate divergence from
    // drain_residual_icp's clear-regardless pattern: the BOB is still sitting
    // here after a failed drain, so the marker must survive to keep the
    // entry-pool exclusion intact for next cycle's retry (and to keep the
    // drain-relevance gate open while execution is disabled). A truly stale
    // marker with no BOB behind it is cleared by the dust branch above.
    if !any_success {
        return Err("All BOB drain attempts failed".to_string());
    }
    state::mutate_state(|s| s.pending_bob_exit = None);
    Ok(())
}

fn dex_string_to_pool(dex: &str) -> Option<state::Pool> {
    match dex {
        "Rumi" => Some(state::Pool::RumiThreeUsd),
        "ICPSwap" => Some(state::Pool::IcpswapCkusdc),
        "ICPSwap-icUSD" => Some(state::Pool::IcpswapIcusd),
        "ICPSwap-ckUSDT" => Some(state::Pool::IcpswapCkusdt),
        "ICPSwap-3USD" => Some(state::Pool::IcpswapThreeUsd),
        _ => None,
    }
}

pub async fn fetch_balance(ledger: Principal) -> Result<u64, String> {
    let account = icrc_ledger_types::icrc1::account::Account {
        owner: ic_cdk::api::id(),
        subaccount: None,
    };
    let result: Result<(Nat,), _> = ic_cdk::call(ledger, "icrc1_balance_of", (account,)).await;
    match result {
        Ok((balance,)) => Ok(nat_to_u64(&balance)),
        Err((code, msg)) => Err(format!("Balance query failed ({:?}): {}", code, msg)),
    }
}

fn log_error(msg: &str) {
    state::log_error(msg.to_string());
}
