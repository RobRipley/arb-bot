use candid::{CandidType, Nat, Principal};
use std::cell::Cell;

use crate::prices::{self, PriceData, nat_to_u64};
use crate::state::{self, Direction, Token};
use crate::swaps;

const ICP_FEE: u64 = 10_000;        // 0.0001 ICP
const CKUSDC_FEE: u64 = 10_000;      // 0.01 ckUSDC
const CKUSDT_FEE: u64 = 10_000;      // 0.01 ckUSDT (6 decimals)
const ICUSD_FEE: u64 = 100_000;      // 0.001 icUSD (8 decimals)
// Note: 3USD has no transfer fee (0)

// Pool ID is deterministic: sorted principals joined by "_"
const RUMI_POOL_ID: &str = "fohh4-yyaaa-aaaap-qtkpa-cai_ryjl3-tyaaa-aaaaa-aaaba-cai";

/// Slippage tolerance in basis points (50 bps = 0.5%)
const SLIPPAGE_BPS: u64 = 50;

/// Number of candidate trade sizes to evaluate
const NUM_CANDIDATES: u64 = 6;

/// VP precision (1e18)
const VP_PRECISION: u128 = 1_000_000_000_000_000_000;

/// ICP reserve: keep at least 1 ICP in the bot for approval fees etc.
const ICP_RESERVE: u64 = 100_000_000; // 1 ICP (8 decimals)

thread_local! {
    static CYCLE_IN_PROGRESS: Cell<bool> = Cell::new(false);
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
        fn drop(&mut self) { CYCLE_IN_PROGRESS.with(|c| c.set(false)); }
    }
    let _guard = Guard;

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

    let config = state::read_state(|s| s.config.clone());
    if config.paused { return; }

    if let Err(e) = drain_residual_icp(&config).await {
        log_error(&format!("Drain residual ICP failed: {}", e));
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
    };

    // Evaluate Strategy A (Rumi vs ICPSwap ckUSDC/ICP)
    let dry_run_a = match compute_optimal_trade(&config, &target_a).await {
        Ok(dr) => Some(dr),
        Err(e) => { log_error(&format!("Strategy A computation failed: {}", e)); None }
    };

    // Evaluate Strategy B (ICPSwap icUSD/ICP vs ICPSwap ckUSDC/ICP)
    let icusd_resolved = state::read_state(|s| s.icusd_token_ordering_resolved);
    let dry_run_b = if has_icusd_pool && icusd_resolved {
        match compute_optimal_trade_b(&config).await {
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
    };
    let dry_run_c = if has_ckusdt_pool && ckusdt_resolved {
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
    };
    let dry_run_d = if has_icusd_pool && icusd_resolved {
        match compute_optimal_trade(&config, &target_d).await {
            Ok(dr) => Some(dr),
            Err(e) => { log_error(&format!("Strategy D computation failed: {}", e)); None }
        }
    } else {
        None
    };

    // Fetch extra balances for snapshot (ICP, icUSD, ckUSDT) — dry runs already have 3USD/ckUSDC/ckUSDT.
    // Still need ICP and icUSD here; ckUSDT balance falls back to dry_run_c if available.
    let ckusdt_fallback_ledger = if config.ckusdt_ledger != Principal::anonymous() {
        config.ckusdt_ledger
    } else {
        candid::Principal::from_text("cngnf-vqaaa-aaaar-qag4q-cai").unwrap()
    };
    let (bal_icp, bal_icusd, bal_ckusdt) = futures::future::join3(
        fetch_balance(config.icp_ledger),
        async {
            if has_icusd_pool { fetch_balance(config.icusd_ledger).await } else { Ok(0) }
        },
        fetch_balance(ckusdt_fallback_ledger),
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
        icpswap_icp_price_ckusdc: dry_run_a.as_ref().map(|d| d.icpswap_price_usd).unwrap_or(0),
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
        icpswap_icp_price_ckusdt: dry_run_c.as_ref().map(|d| d.icpswap_price_usd).unwrap_or(0),
        spread_c_bps: dry_run_c.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        spread_d_bps: dry_run_d.as_ref().map(|d| d.spread_bps).unwrap_or(0),
        traded: false,
        strategy_used: String::new(),
    };

    // Pick the best strategy across A, B, C, D
    let profit_a = dry_run_a.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_b = dry_run_b.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_c = dry_run_c.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_d = dry_run_d.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);

    if profit_a <= 0 && profit_b <= 0 && profit_c <= 0 && profit_d <= 0 {
        // Nothing profitable
        let mut parts: Vec<String> = Vec::new();
        if let Some(a) = &dry_run_a { parts.push(format!("A: {}", a.message)); }
        if let Some(b) = &dry_run_b { parts.push(format!("B: {}", b.message)); }
        if let Some(c) = &dry_run_c { parts.push(format!("C: {}", c.message)); }
        if let Some(d) = &dry_run_d { parts.push(format!("D: {}", d.message)); }
        let msg = if parts.is_empty() { "All strategies failed".to_string() } else { parts.join(" | ") };
        state::log_activity("arb_skip", &msg);
        state::append_snapshot(snapshot);
        return;
    }

    // Check minimum profit threshold against the best of the four
    let best_profit = profit_a.max(profit_b).max(profit_c).max(profit_d);
    if config.min_profit_usd > 0 && best_profit < config.min_profit_usd {
        let msg = format!("Best profit ${:.2} < min ${:.2}",
            best_profit as f64 / 1e6, config.min_profit_usd as f64 / 1e6);
        state::log_activity("arb_skip", &msg);
        state::append_snapshot(snapshot);
        return;
    }

    // Dispatch to the strategy with the highest profit
    let winner = if profit_a >= profit_b && profit_a >= profit_c && profit_a >= profit_d {
        "A"
    } else if profit_d >= profit_b && profit_d >= profit_c {
        "D"
    } else if profit_c >= profit_b {
        "C"
    } else {
        "B"
    };
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
        _ => {
            let dry_run = dry_run_b.unwrap();
            log_start("B", &dry_run);
            match dry_run.direction.as_ref().unwrap() {
                Direction::RumiToIcpswap => execute_icusd_to_ckusdc(&config, &dry_run).await,
                Direction::IcpswapToRumi => execute_ckusdc_to_icusd(&config, &dry_run).await,
            }
        }
    }

    // Record snapshot after trade execution
    state::append_snapshot(snapshot);
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
    pub stable_decimals: u8,    // 6 for ck*, 8 for icUSD
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

pub async fn compute_optimal_trade(
    config: &state::BotConfig,
    target: &IcpswapTarget,
) -> Result<DryRunResult, String> {
    let mut result = DryRunResult::default();

    // Fetch prices
    let prices = prices::fetch_all_prices(
        config.rumi_amm, RUMI_POOL_ID, config.icp_ledger,
        config.rumi_3pool, target.pool, target.icp_is_token0,
        target.stable_decimals,
    ).await?;

    result.rumi_price_usd = prices.rumi_price_usd_6dec();
    result.icpswap_price_usd = prices.icpswap_price_usd_6dec();
    result.virtual_price = prices.virtual_price;
    result.spread_bps = prices.spread_bps();

    // Fetch balances (always, even when spread is below minimum — needed for snapshot)
    let (bal_3usd, bal_stable) = futures::future::join(
        fetch_balance(config.three_usd_ledger),
        fetch_balance(target.stable_ledger),
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

        if result.balance_3usd < 1_000_000 {
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
        // Min balance: $0.01 worth, scaled to native units
        let min_native = usd_6dec_to_stable(10_000, target.stable_decimals);
        if usable_stable < min_native {
            result.message = format!("[{}] Insufficient {} balance", target.strategy_tag, target.stable_token_name);
            return Ok(result);
        }

        // Cap by max_trade_size_usd, converted to native stable units
        let max_native = usd_6dec_to_stable(config.max_trade_size_usd, target.stable_decimals);
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

    // Round 2: Quote ICPSwap (ICP→ckUSDC) for all ICP amounts in parallel
    // Subtract ICP transfer fees: output fee from leg 1 + input fee for leg 2
    let icpswap_futs: Vec<_> = stage1.iter().map(|&(_, icp_amount)| {
        let usable = icp_amount.saturating_sub(ICP_FEE * 2);
        prices::fetch_icpswap_quote_for_amount(
            target.pool, usable, target.icp_is_token0,
        )
    }).collect();
    let icpswap_results = futures::future::join_all(icpswap_futs).await;

    // Compute profit for each candidate
    let mut best_idx: Option<usize> = None;
    let mut best_profit: i64 = 0;

    for (i, icpswap_res) in icpswap_results.into_iter().enumerate() {
        let (input_3usd, icp_amount) = stage1[i];
        let ckusdc_out = match icpswap_res {
            Ok(out) => out,
            Err(_) => continue,
        };

        let input_usd = (input_3usd as u128 * prices.virtual_price as u128 / VP_PRECISION / 100) as i64;
        let output_usd = stable_to_usd_6dec(ckusdc_out, target.stable_decimals);
        let fee_usd = stable_to_usd_6dec(target.stable_fee, target.stable_decimals);
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

    // Round 1: Quote ICPSwap (stable→ICP) for all candidates in parallel
    let icpswap_futs: Vec<_> = candidates.iter().map(|&amount| {
        prices::fetch_icpswap_quote_for_amount(
            target.pool, amount, !target.icp_is_token0,
        )
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

    let mut best_idx: Option<usize> = None;
    let mut best_profit: i64 = 0;

    for (i, rumi_res) in rumi_results.into_iter().enumerate() {
        let (input_ckusdc, icp_amount) = stage1[i];
        let three_usd_out = match rumi_res {
            Ok(out) => out,
            Err(_) => continue,
        };

        let input_usd = stable_to_usd_6dec(input_ckusdc, target.stable_decimals);
        let output_usd = (three_usd_out as u128 * prices.virtual_price as u128 / VP_PRECISION / 100) as i64;
        let fee_usd = stable_to_usd_6dec(target.stable_fee, target.stable_decimals);
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

// ─── Strategy B: ICPSwap icUSD/ICP vs ICPSwap ckUSDC/ICP ───

pub async fn compute_optimal_trade_b(config: &state::BotConfig) -> Result<DryRunResult, String> {
    let mut result = DryRunResult::default();

    let prices = prices::fetch_strategy_b_prices(
        config.icpswap_icusd_pool, config.icpswap_icusd_icp_is_token0,
        config.icpswap_pool, config.icpswap_icp_is_token0,
    ).await?;

    result.rumi_price_usd = prices.icusd_price_usd_6dec();  // reusing field for "buy side" price
    result.icpswap_price_usd = prices.ckusdc_price_usd_6dec();
    result.virtual_price = 0; // not applicable for Strategy B
    result.spread_bps = prices.spread_bps();

    // Fetch balances (always, even when spread is below minimum — needed for snapshot)
    let (bal_icusd, bal_ckusdc) = futures::future::join(
        fetch_balance(config.icusd_ledger),
        fetch_balance(config.ckusdc_ledger),
    ).await;
    result.balance_3usd = bal_icusd.unwrap_or(0); // reusing field for icUSD balance
    result.balance_ckusdc = bal_ckusdc.unwrap_or(0);

    let abs_spread = result.spread_bps.unsigned_abs();
    if abs_spread < config.min_spread_bps {
        result.message = format!("[B] Spread {} bps < minimum {} bps", abs_spread, config.min_spread_bps);
        return Ok(result);
    }

    if result.spread_bps > 0 {
        // ICP more expensive on ckUSDC pool → buy on icUSD pool (icUSD→ICP), sell on ckUSDC pool (ICP→ckUSDC)
        result.direction = Some(Direction::RumiToIcpswap); // reusing: "buy side" → "sell side"
        result.optimal_input_token = Some(Token::ThreeUSD); // represents icUSD here
        result.expected_output_token = Some(Token::CkUSDC);

        // Reserve fee for approve
        let usable_icusd = result.balance_3usd.saturating_sub(ICUSD_FEE);
        if usable_icusd < 1_000_000 {
            result.message = "[B] Insufficient icUSD balance".to_string();
            return Ok(result);
        }

        // Cap by max_trade_size_usd (icUSD ≈ $1, 8 dec → 6 dec is /100)
        let max_icusd = config.max_trade_size_usd * 100; // 6-dec USD → 8-dec icUSD
        let max_input = usable_icusd.min(max_icusd);

        find_optimal_icusd_to_ckusdc(config, max_input, &mut result).await;
    } else {
        // ICP more expensive on icUSD pool → buy on ckUSDC pool (ckUSDC→ICP), sell on icUSD pool (ICP→icUSD)
        result.direction = Some(Direction::IcpswapToRumi); // reusing: "ref side" → "buy side"
        result.optimal_input_token = Some(Token::CkUSDC);
        result.expected_output_token = Some(Token::ThreeUSD); // represents icUSD here

        let usable_ckusdc = result.balance_ckusdc.saturating_sub(CKUSDC_FEE);
        if usable_ckusdc < 10_000 {
            result.message = "[B] Insufficient ckUSDC balance".to_string();
            return Ok(result);
        }

        let max_input = usable_ckusdc.min(config.max_trade_size_usd);

        find_optimal_ckusdc_to_icusd(config, max_input, &mut result).await;
    }

    Ok(result)
}

/// Strategy B: Buy ICP on icUSD pool, sell on ckUSDC pool
async fn find_optimal_icusd_to_ckusdc(
    config: &state::BotConfig,
    max_input: u64,
    result: &mut DryRunResult,
) {
    let candidates: Vec<u64> = (1..=NUM_CANDIDATES)
        .map(|i| max_input * i / NUM_CANDIDATES)
        .filter(|&a| a > 0)
        .collect();

    // Round 1: Quote icUSD/ICP pool (icUSD→ICP)
    let futs1: Vec<_> = candidates.iter().map(|&amount| {
        prices::fetch_icpswap_quote_for_amount(
            config.icpswap_icusd_pool, amount, !config.icpswap_icusd_icp_is_token0,
        )
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
        result.message = "[B] All icUSD→ICP quotes failed".to_string();
        return;
    }

    // Round 2: Quote ckUSDC/ICP pool (ICP→ckUSDC)
    // Subtract ICP transfer fees between legs
    let futs2: Vec<_> = stage1.iter().map(|&(_, icp_amount)| {
        let usable = icp_amount.saturating_sub(ICP_FEE * 2);
        prices::fetch_icpswap_quote_for_amount(
            config.icpswap_pool, usable, config.icpswap_icp_is_token0,
        )
    }).collect();
    let results2 = futures::future::join_all(futs2).await;

    let mut best_idx: Option<usize> = None;
    let mut best_profit: i64 = 0;

    for (i, res) in results2.into_iter().enumerate() {
        let (input_icusd, icp_amount) = stage1[i];
        let ckusdc_out = match res {
            Ok(out) => out,
            Err(_) => continue,
        };

        // icUSD is 8 dec ≈ $1, so /100 → 6-dec USD
        let input_usd = (input_icusd / 100) as i64;
        let output_usd = ckusdc_out as i64;
        // Fees: ICUSD_FEE for approve on buy side, CKUSDC_FEE on sell side output
        let fees = (ICUSD_FEE / 100) as i64 + CKUSDC_FEE as i64;
        let profit = output_usd - input_usd - fees;

        result.candidates_evaluated.push(CandidateResult {
            input_amount: input_icusd,
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
                "[B] Optimal: {} icUSD → {} ICP → {} ckUSDC = {} profit",
                best.input_amount, best.icp_amount, best.output_amount, best.profit_usd
            );
        }
        None => {
            result.message = "[B] No profitable trade found".to_string();
        }
    }
}

/// Strategy B: Buy ICP on ckUSDC pool, sell on icUSD pool
async fn find_optimal_ckusdc_to_icusd(
    config: &state::BotConfig,
    max_input: u64,
    result: &mut DryRunResult,
) {
    let candidates: Vec<u64> = (1..=NUM_CANDIDATES)
        .map(|i| max_input * i / NUM_CANDIDATES)
        .filter(|&a| a > 0)
        .collect();

    // Round 1: Quote ckUSDC/ICP pool (ckUSDC→ICP)
    let futs1: Vec<_> = candidates.iter().map(|&amount| {
        prices::fetch_icpswap_quote_for_amount(
            config.icpswap_pool, amount, !config.icpswap_icp_is_token0,
        )
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
        result.message = "[B] All ckUSDC→ICP quotes failed".to_string();
        return;
    }

    // Round 2: Quote icUSD/ICP pool (ICP→icUSD)
    // Subtract ICP transfer fees between legs
    let futs2: Vec<_> = stage1.iter().map(|&(_, icp_amount)| {
        let usable = icp_amount.saturating_sub(ICP_FEE * 2);
        prices::fetch_icpswap_quote_for_amount(
            config.icpswap_icusd_pool, usable, config.icpswap_icusd_icp_is_token0,
        )
    }).collect();
    let results2 = futures::future::join_all(futs2).await;

    let mut best_idx: Option<usize> = None;
    let mut best_profit: i64 = 0;

    for (i, res) in results2.into_iter().enumerate() {
        let (input_ckusdc, icp_amount) = stage1[i];
        let icusd_out = match res {
            Ok(out) => out,
            Err(_) => continue,
        };

        let input_usd = input_ckusdc as i64;
        let output_usd = (icusd_out / 100) as i64; // 8-dec → 6-dec
        let fees = CKUSDC_FEE as i64 + (ICUSD_FEE / 100) as i64;
        let profit = output_usd - input_usd - fees;

        result.candidates_evaluated.push(CandidateResult {
            input_amount: input_ckusdc,
            icp_amount,
            output_amount: icusd_out,
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
                "[B] Optimal: {} ckUSDC → {} ICP → {} icUSD = {} profit",
                best.input_amount, best.icp_amount, best.output_amount, best.profit_usd
            );
        }
        None => {
            result.message = "[B] No profitable trade found".to_string();
        }
    }
}

// ─── Execute Trades ───

async fn execute_rumi_to_icpswap(config: &state::BotConfig, target: &IcpswapTarget, dry_run: &DryRunResult) {
    let trade_amount_3usd = dry_run.optimal_input_amount;
    let min_icp_out = dry_run.expected_icp_amount * (10_000 - SLIPPAGE_BPS) / 10_000;

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
    let min_stable_out = dry_run.expected_output_amount * (10_000 - SLIPPAGE_BPS) / 10_000;

    state::log_activity("swap", &format!(
        "[{}] Leg 2: ICPSwap swap {} ICP → {} (min: {}, raw from Rumi: {})", target.strategy_tag, usable_icp, target.stable_token_name, min_stable_out, icp_out
    ));

    let stable_out = match swaps::icpswap_swap(
        target.pool, usable_icp, target.icp_is_token0, min_stable_out, ICP_FEE, target.stable_fee,
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
                bought_usd_value: stable_to_usd_6dec(amount, target.stable_decimals),
                fees_usd: stable_to_usd_6dec(target.stable_fee, target.stable_decimals),
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

    let net_profit = stable_to_usd_6dec(stable_out, target.stable_decimals)
        - cost_usd_6dec
        - stable_to_usd_6dec(target.stable_fee, target.stable_decimals);
    state::log_activity("trade", &format!(
        "[{}] COMPLETE RumiToIcpswap: {} 3USD → {} ICP → {} {} | profit: {} (6dec USD)",
        target.strategy_tag, trade_amount_3usd, icp_out, stable_out, target.stable_token_name, net_profit
    ));
}

async fn execute_icpswap_to_rumi(config: &state::BotConfig, target: &IcpswapTarget, dry_run: &DryRunResult) {
    let trade_amount_stable = dry_run.optimal_input_amount;
    let min_icp_out = dry_run.expected_icp_amount * (10_000 - SLIPPAGE_BPS) / 10_000;

    state::log_activity("swap", &format!(
        "[{}] Leg 1: ICPSwap swap {} {} → ICP (min: {})", target.strategy_tag, trade_amount_stable, target.stable_token_name, min_icp_out
    ));

    let icp_out = match swaps::icpswap_swap(
        target.pool, trade_amount_stable, !target.icp_is_token0, min_icp_out, target.stable_fee, ICP_FEE,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("[{}] Leg 1 OK: {} {} → {} ICP", target.strategy_tag, trade_amount_stable, target.stable_token_name, amount));
            state::append_trade_leg(state::TradeLeg {
                timestamp: ic_cdk::api::time(),
                leg_type: state::LegType::Leg1,
                dex: target.label.to_string(),
                sold_token: target.stable_token_name.to_string(),
                sold_amount: trade_amount_stable,
                bought_token: "ICP".to_string(),
                bought_amount: amount,
                sold_usd_value: stable_to_usd_6dec(trade_amount_stable, target.stable_decimals),
                bought_usd_value: 0, // ICP is transit
                fees_usd: stable_to_usd_6dec(target.stable_fee, target.stable_decimals),
            });
            state::mutate_state(|s| {
                s.pending_exit = Some(state::PendingExit {
                    entry_pool: target.pool_enum,
                    intended_exit_pool: state::Pool::RumiThreeUsd,
                    timestamp: ic_cdk::api::time(),
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
    let min_3usd_out = dry_run.expected_output_amount * (10_000 - SLIPPAGE_BPS) / 10_000;

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

    let input_usd_6dec = stable_to_usd_6dec(trade_amount_stable, target.stable_decimals);
    let output_usd_6dec = (three_usd_out as u128 * vp as u128 / VP_PRECISION / 100) as i64;
    let net_profit = output_usd_6dec - input_usd_6dec - stable_to_usd_6dec(target.stable_fee, target.stable_decimals);

    state::log_activity("trade", &format!(
        "[{}] COMPLETE IcpswapToRumi: {} {} → {} ICP → {} 3USD | profit: {} (6dec USD)",
        target.strategy_tag, trade_amount_stable, target.stable_token_name, icp_out, three_usd_out, net_profit
    ));
}

// ─── Strategy B Execute Trades ───

/// Strategy B: icUSD → ICP (icUSD pool) → ckUSDC (ckUSDC pool)
async fn execute_icusd_to_ckusdc(config: &state::BotConfig, dry_run: &DryRunResult) {
    let trade_amount_icusd = dry_run.optimal_input_amount;
    let min_icp_out = dry_run.expected_icp_amount * (10_000 - SLIPPAGE_BPS) / 10_000;

    // icUSD cost in 6-dec USD (icUSD is 8 dec, ≈ $1)
    let cost_usd_6dec = (trade_amount_icusd / 100) as i64;

    state::log_activity("swap", &format!(
        "[B] Leg 1: ICPSwap icUSD pool swap {} icUSD → ICP (min: {})", trade_amount_icusd, min_icp_out
    ));

    let icp_out = match swaps::icpswap_swap(
        config.icpswap_icusd_pool, trade_amount_icusd,
        !config.icpswap_icusd_icp_is_token0, // icUSD→ICP: selling non-ICP token
        min_icp_out, ICUSD_FEE, ICP_FEE,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("[B] Leg 1 OK: {} icUSD → {} ICP", trade_amount_icusd, amount));
            state::append_trade_leg(state::TradeLeg {
                timestamp: ic_cdk::api::time(),
                leg_type: state::LegType::Leg1,
                dex: "ICPSwap-icUSD".to_string(),
                sold_token: "icUSD".to_string(),
                sold_amount: trade_amount_icusd,
                bought_token: "ICP".to_string(),
                bought_amount: amount,
                sold_usd_value: cost_usd_6dec,
                bought_usd_value: 0,
                fees_usd: (ICUSD_FEE / 100) as i64,
            });
            state::mutate_state(|s| {
                s.pending_exit = Some(state::PendingExit {
                    entry_pool: state::Pool::IcpswapIcusd,
                    intended_exit_pool: state::Pool::IcpswapCkusdc,
                    timestamp: ic_cdk::api::time(),
                });
            });
            amount
        }
        Err(e) => {
            let msg = format!("[B] ICPSwap icUSD→ICP failed: {}", e);
            state::log_activity("swap", &format!("[B] Leg 1 FAILED: {}", msg));
            log_error(&msg);
            return;
        }
    };

    // ICPSwap reports gross output; bot receives (icp_out - ICP_FEE), depositFromAndSwap costs another ICP_FEE
    let usable_icp = icp_out.saturating_sub(ICP_FEE * 2);
    let min_ckusdc_out = dry_run.expected_output_amount * (10_000 - SLIPPAGE_BPS) / 10_000;

    state::log_activity("swap", &format!(
        "[B] Leg 2: ICPSwap ckUSDC pool swap {} ICP → ckUSDC (min: {})", usable_icp, min_ckusdc_out
    ));

    let ckusdc_out = match swaps::icpswap_swap(
        config.icpswap_pool, usable_icp,
        config.icpswap_icp_is_token0, // ICP→ckUSDC
        min_ckusdc_out, ICP_FEE, CKUSDC_FEE,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("[B] Leg 2 OK: {} ICP → {} ckUSDC", icp_out, amount));
            state::append_trade_leg(state::TradeLeg {
                timestamp: ic_cdk::api::time(),
                leg_type: state::LegType::Leg2,
                dex: "ICPSwap".to_string(),
                sold_token: "ICP".to_string(),
                sold_amount: usable_icp,
                bought_token: "ckUSDC".to_string(),
                bought_amount: amount,
                sold_usd_value: 0,
                bought_usd_value: amount as i64,
                fees_usd: CKUSDC_FEE as i64,
            });
            state::mutate_state(|s| { s.pending_exit = None; });
            amount
        }
        Err(e) => {
            let msg = format!("[B] ICPSwap ICP→ckUSDC failed (holding {} ICP): {}", icp_out, e);
            state::log_activity("swap", &format!("[B] Leg 2 FAILED: {}", msg));
            log_error(&msg);
            return;
        }
    };

    let net_profit = ckusdc_out as i64 - cost_usd_6dec - CKUSDC_FEE as i64 - (ICUSD_FEE / 100) as i64;
    state::log_activity("trade", &format!(
        "[B] COMPLETE icUSD→ckUSDC: {} icUSD → {} ICP → {} ckUSDC | profit: {} (6dec USD)",
        trade_amount_icusd, icp_out, ckusdc_out, net_profit
    ));
}

/// Strategy B: ckUSDC → ICP (ckUSDC pool) → icUSD (icUSD pool)
async fn execute_ckusdc_to_icusd(config: &state::BotConfig, dry_run: &DryRunResult) {
    let trade_amount_ckusdc = dry_run.optimal_input_amount;
    let min_icp_out = dry_run.expected_icp_amount * (10_000 - SLIPPAGE_BPS) / 10_000;

    state::log_activity("swap", &format!(
        "[B] Leg 1: ICPSwap ckUSDC pool swap {} ckUSDC → ICP (min: {})", trade_amount_ckusdc, min_icp_out
    ));

    let icp_out = match swaps::icpswap_swap(
        config.icpswap_pool, trade_amount_ckusdc,
        !config.icpswap_icp_is_token0, // ckUSDC→ICP
        min_icp_out, CKUSDC_FEE, ICP_FEE,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("[B] Leg 1 OK: {} ckUSDC → {} ICP", trade_amount_ckusdc, amount));
            state::append_trade_leg(state::TradeLeg {
                timestamp: ic_cdk::api::time(),
                leg_type: state::LegType::Leg1,
                dex: "ICPSwap".to_string(),
                sold_token: "ckUSDC".to_string(),
                sold_amount: trade_amount_ckusdc,
                bought_token: "ICP".to_string(),
                bought_amount: amount,
                sold_usd_value: trade_amount_ckusdc as i64,
                bought_usd_value: 0,
                fees_usd: CKUSDC_FEE as i64,
            });
            state::mutate_state(|s| {
                s.pending_exit = Some(state::PendingExit {
                    entry_pool: state::Pool::IcpswapCkusdc,
                    intended_exit_pool: state::Pool::IcpswapIcusd,
                    timestamp: ic_cdk::api::time(),
                });
            });
            amount
        }
        Err(e) => {
            let msg = format!("[B] ICPSwap ckUSDC→ICP failed: {}", e);
            state::log_activity("swap", &format!("[B] Leg 1 FAILED: {}", msg));
            log_error(&msg);
            return;
        }
    };

    let usable_icp = icp_out.saturating_sub(ICP_FEE * 2);
    let min_icusd_out = dry_run.expected_output_amount * (10_000 - SLIPPAGE_BPS) / 10_000;

    state::log_activity("swap", &format!(
        "[B] Leg 2: ICPSwap icUSD pool swap {} ICP → icUSD (min: {})", usable_icp, min_icusd_out
    ));

    let icusd_out = match swaps::icpswap_swap(
        config.icpswap_icusd_pool, usable_icp,
        config.icpswap_icusd_icp_is_token0, // ICP→icUSD
        min_icusd_out, ICP_FEE, ICUSD_FEE,
    ).await {
        Ok(amount) => {
            let out_usd_6dec = (amount / 100) as i64;
            state::log_activity("swap", &format!("[B] Leg 2 OK: {} ICP → {} icUSD", icp_out, amount));
            state::append_trade_leg(state::TradeLeg {
                timestamp: ic_cdk::api::time(),
                leg_type: state::LegType::Leg2,
                dex: "ICPSwap-icUSD".to_string(),
                sold_token: "ICP".to_string(),
                sold_amount: usable_icp,
                bought_token: "icUSD".to_string(),
                bought_amount: amount,
                sold_usd_value: 0,
                bought_usd_value: out_usd_6dec,
                fees_usd: (ICUSD_FEE / 100) as i64,
            });
            state::mutate_state(|s| { s.pending_exit = None; });
            amount
        }
        Err(e) => {
            let msg = format!("[B] ICPSwap ICP→icUSD failed (holding {} ICP): {}", icp_out, e);
            state::log_activity("swap", &format!("[B] Leg 2 FAILED: {}", msg));
            log_error(&msg);
            return;
        }
    };

    let input_usd_6dec = trade_amount_ckusdc as i64;
    let output_usd_6dec = (icusd_out / 100) as i64;
    let net_profit = output_usd_6dec - input_usd_6dec - CKUSDC_FEE as i64 - (ICUSD_FEE / 100) as i64;

    state::log_activity("trade", &format!(
        "[B] COMPLETE ckUSDC→icUSD: {} ckUSDC → {} ICP → {} icUSD | profit: {} (6dec USD)",
        trade_amount_ckusdc, icp_out, icusd_out, net_profit
    ));
}

// ─── Helpers ───

async fn drain_residual_icp(config: &state::BotConfig) -> Result<(), String> {
    let icp_balance = fetch_balance(config.icp_ledger).await?;

    // Keep ICP_RESERVE in the bot for approval fees etc.
    let drainable = icp_balance.saturating_sub(ICP_RESERVE);
    if drainable <= ICP_FEE * 2 {
        return Ok(());
    }

    // Reserve fee for the icrc2_transfer_from the DEX will trigger
    let drain_amount = drainable - ICP_FEE;

    state::log_activity("drain", &format!("Draining {} residual ICP (balance: {})", drain_amount, icp_balance));

    // Determine the entry pool to AVOID. Primary source: pending_exit recorded
    // at Leg 1 success. Fallback safety net: the most recent Leg1 trade_leg
    // (works even after a canister restart that cleared pending_exit).
    let pending_exit: Option<state::PendingExit> =
        state::read_state(|s| s.pending_exit.clone());
    // Scan backward through trade_legs for the most recent Leg1 with a
    // recognized dex string. Skip any Leg1 whose dex doesn't map (e.g.
    // historical backfilled entries from deprecated strategies).
    let fallback_entry_pool: Option<state::Pool> = state::find_map_last_trade_leg(|l| {
        match l.leg_type {
            state::LegType::Leg1 => dex_string_to_pool(&l.dex),
            _ => None,
        }
    });
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
        let min_out = (out_3usd as u128 * (10_000 - SLIPPAGE_BPS) as u128 / 10_000) as u64;
        candidates.push(Candidate { pool: state::Pool::RumiThreeUsd, usd_out, min_out });
    }
    if let Ok(quote_per_icp) = icpswap_ck_res {
        // quote is ckUSDC (6 dec) per ICP; ckUSDC ≈ USD.
        let out_ck = (quote_per_icp as u128 * drain_amount as u128 / 100_000_000) as u64;
        let min_out = (out_ck as u128 * (10_000 - SLIPPAGE_BPS) as u128 / 10_000) as u64;
        candidates.push(Candidate { pool: state::Pool::IcpswapCkusdc, usd_out: out_ck, min_out });
    }
    if let Ok(quote_per_icp) = icpswap_icusd_res {
        // icUSD is 8 dec ≈ $1. Scale then divide by 100 for 6-dec USD.
        let out_icusd = (quote_per_icp as u128 * drain_amount as u128 / 100_000_000) as u64;
        let usd_out = (out_icusd / 100) as u64;
        let min_out = (out_icusd as u128 * (10_000 - SLIPPAGE_BPS) as u128 / 10_000) as u64;
        candidates.push(Candidate { pool: state::Pool::IcpswapIcusd, usd_out, min_out });
    }
    if let Ok(quote_per_icp) = icpswap_ckusdt_res {
        // ckUSDT is 6 dec ≈ $1.
        let out_ck = (quote_per_icp as u128 * drain_amount as u128 / 100_000_000) as u64;
        let min_out = (out_ck as u128 * (10_000 - SLIPPAGE_BPS) as u128 / 10_000) as u64;
        candidates.push(Candidate { pool: state::Pool::IcpswapCkusdt, usd_out: out_ck, min_out });
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
        // Refresh balance if this is a retry after failure.
        if i > 0 {
            let bal = fetch_balance(config.icp_ledger).await.unwrap_or(0);
            let d = bal.saturating_sub(ICP_RESERVE);
            if d <= ICP_FEE * 2 { break; }
            remaining_amount = d - ICP_FEE;
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

    // Clear pending_exit regardless: either we drained (success) or all non-entry
    // pools failed (stale state cleared so next cycle can reassess).
    state::mutate_state(|s| s.pending_exit = None);

    if !any_success && !order.is_empty() {
        return Err("All drain attempts failed".to_string());
    }
    Ok(())
}

fn dex_string_to_pool(dex: &str) -> Option<state::Pool> {
    match dex {
        "Rumi" => Some(state::Pool::RumiThreeUsd),
        "ICPSwap" => Some(state::Pool::IcpswapCkusdc),
        "ICPSwap-icUSD" => Some(state::Pool::IcpswapIcusd),
        "ICPSwap-ckUSDT" => Some(state::Pool::IcpswapCkusdt),
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
