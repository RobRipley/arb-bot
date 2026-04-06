use candid::{CandidType, Nat, Principal};
use std::cell::Cell;

use crate::prices::{self, PriceData, nat_to_u64};
use crate::state::{self, Direction, ErrorRecord, Token};
use crate::swaps;

const ICP_FEE: u64 = 10_000;        // 0.0001 ICP
const CKUSDC_FEE: u64 = 10_000;      // 0.01 ckUSDC
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

    let config = state::read_state(|s| s.config.clone());
    if config.paused { return; }

    if let Err(e) = drain_residual_icp(&config).await {
        log_error(&format!("Drain residual ICP failed: {}", e));
    }

    // Evaluate Strategy A (Rumi vs ICPSwap ckUSDC/ICP)
    let dry_run_a = match compute_optimal_trade(&config).await {
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

    // Fetch extra balances for snapshot (ICP, icUSD, ckUSDT) — dry runs already have 3USD and ckUSDC
    let ckusdt_ledger = candid::Principal::from_text("cngnf-vqaaa-aaaar-qag4q-cai").unwrap();
    let (bal_icp, bal_icusd, bal_ckusdt) = futures::future::join3(
        fetch_balance(config.icp_ledger),
        async {
            if has_icusd_pool { fetch_balance(config.icusd_ledger).await } else { Ok(0) }
        },
        fetch_balance(ckusdt_ledger),
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
        traded: false,
        strategy_used: String::new(),
    };

    // Pick the best strategy
    let profit_a = dry_run_a.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);
    let profit_b = dry_run_b.as_ref().filter(|d| d.should_trade).map(|d| d.expected_profit_usd).unwrap_or(0);

    if profit_a <= 0 && profit_b <= 0 {
        // Neither strategy is profitable
        let msg = match (&dry_run_a, &dry_run_b) {
            (Some(a), Some(b)) => format!("A: {} | B: {}", a.message, b.message),
            (Some(a), None) => format!("A: {}", a.message),
            (None, Some(b)) => format!("B: {}", b.message),
            (None, None) => "Both strategies failed".to_string(),
        };
        state::log_activity("arb_skip", &msg);
        // Record snapshot even when not trading
        state::mutate_state(|s| s.snapshots.push(snapshot));
        return;
    }

    // Check minimum profit threshold
    let best_profit = profit_a.max(profit_b);
    if config.min_profit_usd > 0 && best_profit < config.min_profit_usd {
        let msg = format!("Best profit ${:.2} < min ${:.2}",
            best_profit as f64 / 1e6, config.min_profit_usd as f64 / 1e6);
        state::log_activity("arb_skip", &msg);
        state::mutate_state(|s| s.snapshots.push(snapshot));
        return;
    }

    if profit_a >= profit_b {
        // Execute Strategy A
        snapshot.traded = true;
        snapshot.strategy_used = "A".to_string();
        let dry_run = dry_run_a.unwrap();
        state::log_activity("arb_start", &format!(
            "[A] Starting {:?} trade: {} {:?} → est {} ICP → est {} {:?} (spread: {} bps, est profit: {})",
            dry_run.direction.as_ref().unwrap(),
            dry_run.optimal_input_amount,
            dry_run.optimal_input_token.as_ref().unwrap(),
            dry_run.expected_icp_amount,
            dry_run.expected_output_amount,
            dry_run.expected_output_token.as_ref().unwrap(),
            dry_run.spread_bps,
            dry_run.expected_profit_usd,
        ));
        match dry_run.direction.as_ref().unwrap() {
            Direction::RumiToIcpswap => execute_rumi_to_icpswap(&config, &dry_run).await,
            Direction::IcpswapToRumi => execute_icpswap_to_rumi(&config, &dry_run).await,
        }
    } else {
        // Execute Strategy B
        snapshot.traded = true;
        snapshot.strategy_used = "B".to_string();
        let dry_run = dry_run_b.unwrap();
        state::log_activity("arb_start", &format!(
            "[B] Starting {:?} trade: {} {:?} → est {} ICP → est {} {:?} (spread: {} bps, est profit: {})",
            dry_run.direction.as_ref().unwrap(),
            dry_run.optimal_input_amount,
            dry_run.optimal_input_token.as_ref().unwrap(),
            dry_run.expected_icp_amount,
            dry_run.expected_output_amount,
            dry_run.expected_output_token.as_ref().unwrap(),
            dry_run.spread_bps,
            dry_run.expected_profit_usd,
        ));
        match dry_run.direction.as_ref().unwrap() {
            Direction::RumiToIcpswap => execute_icusd_to_ckusdc(&config, &dry_run).await,
            Direction::IcpswapToRumi => execute_ckusdc_to_icusd(&config, &dry_run).await,
        }
    }

    // Record snapshot after trade execution
    state::mutate_state(|s| s.snapshots.push(snapshot));
}

// ─── Dry Run: Compute Optimal Trade ───

pub async fn compute_optimal_trade(config: &state::BotConfig) -> Result<DryRunResult, String> {
    let mut result = DryRunResult::default();

    // Fetch prices
    let prices = prices::fetch_all_prices(
        config.rumi_amm, RUMI_POOL_ID, config.icp_ledger,
        config.rumi_3pool, config.icpswap_pool, config.icpswap_icp_is_token0,
    ).await?;

    result.rumi_price_usd = prices.rumi_price_usd_6dec();
    result.icpswap_price_usd = prices.icpswap_icp_price_ckusdc_native;
    result.virtual_price = prices.virtual_price;
    result.spread_bps = prices.spread_bps();

    // Fetch balances (always, even when spread is below minimum — needed for snapshot)
    let (bal_3usd, bal_ckusdc) = futures::future::join(
        fetch_balance(config.three_usd_ledger),
        fetch_balance(config.ckusdc_ledger),
    ).await;
    result.balance_3usd = bal_3usd.unwrap_or(0);
    result.balance_ckusdc = bal_ckusdc.unwrap_or(0);

    let abs_spread = result.spread_bps.unsigned_abs();
    if abs_spread < config.min_spread_bps {
        result.message = format!("Spread {} bps < minimum {} bps", abs_spread, config.min_spread_bps);
        return Ok(result);
    }

    if result.spread_bps > 0 {
        // ICP more expensive on ICPSwap → buy on Rumi (3USD→ICP), sell on ICPSwap (ICP→ckUSDC)
        result.direction = Some(Direction::RumiToIcpswap);
        result.optimal_input_token = Some(Token::ThreeUSD);
        result.expected_output_token = Some(Token::CkUSDC);

        if result.balance_3usd < 1_000_000 {
            result.message = "Insufficient 3USD balance".to_string();
            return Ok(result);
        }

        // Cap by max_trade_size_usd (converted to 3USD)
        let max_3usd = if prices.virtual_price > 0 {
            (config.max_trade_size_usd as u128 * VP_PRECISION * 100 / prices.virtual_price as u128) as u64
        } else { result.balance_3usd };
        let max_input = result.balance_3usd.min(max_3usd);

        find_optimal_rumi_to_icpswap(config, max_input, &prices, &mut result).await;
    } else {
        // ICP more expensive on Rumi → buy on ICPSwap (ckUSDC→ICP), sell on Rumi (ICP→3USD)
        result.direction = Some(Direction::IcpswapToRumi);
        result.optimal_input_token = Some(Token::CkUSDC);
        result.expected_output_token = Some(Token::ThreeUSD);

        // Reserve fee for the ICRC-2 approve that ICPSwap's depositFromAndSwap triggers
        let usable_ckusdc = result.balance_ckusdc.saturating_sub(CKUSDC_FEE);
        if usable_ckusdc < 10_000 {
            result.message = "Insufficient ckUSDC balance".to_string();
            return Ok(result);
        }

        let max_input = usable_ckusdc.min(config.max_trade_size_usd);

        find_optimal_icpswap_to_rumi(config, max_input, &prices, &mut result).await;
    }

    Ok(result)
}

/// Find optimal trade size for Rumi→ICPSwap direction.
/// Evaluates NUM_CANDIDATES amounts and picks the profit-maximizing one.
async fn find_optimal_rumi_to_icpswap(
    config: &state::BotConfig,
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
            config.icpswap_pool, usable, config.icpswap_icp_is_token0,
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
        let output_usd = ckusdc_out as i64;
        let profit = output_usd - input_usd - CKUSDC_FEE as i64;

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
                "Optimal: {} 3USD → {} ICP → {} ckUSDC = {} profit",
                best.input_amount, best.icp_amount, best.output_amount, best.profit_usd
            );
        }
        None => {
            result.message = "No profitable trade found".to_string();
        }
    }
}

/// Find optimal trade size for ICPSwap→Rumi direction.
async fn find_optimal_icpswap_to_rumi(
    config: &state::BotConfig,
    max_input: u64,
    prices: &PriceData,
    result: &mut DryRunResult,
) {
    let candidates: Vec<u64> = (1..=NUM_CANDIDATES)
        .map(|i| max_input * i / NUM_CANDIDATES)
        .filter(|&a| a > 0)
        .collect();

    // Round 1: Quote ICPSwap (ckUSDC→ICP) for all candidates in parallel
    let icpswap_futs: Vec<_> = candidates.iter().map(|&amount| {
        prices::fetch_icpswap_quote_for_amount(
            config.icpswap_pool, amount, !config.icpswap_icp_is_token0,
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
        result.message = "All ICPSwap quotes failed".to_string();
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

        let input_usd = input_ckusdc as i64;
        let output_usd = (three_usd_out as u128 * prices.virtual_price as u128 / VP_PRECISION / 100) as i64;
        let profit = output_usd - input_usd - CKUSDC_FEE as i64;

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
                "Optimal: {} ckUSDC → {} ICP → {} 3USD = {} profit",
                best.input_amount, best.icp_amount, best.output_amount, best.profit_usd
            );
        }
        None => {
            result.message = "No profitable trade found".to_string();
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

async fn execute_rumi_to_icpswap(config: &state::BotConfig, dry_run: &DryRunResult) {
    let trade_amount_3usd = dry_run.optimal_input_amount;
    let min_icp_out = dry_run.expected_icp_amount * (10_000 - SLIPPAGE_BPS) / 10_000;

    // 3USD cost in 6-dec USD
    let cost_usd_6dec = (trade_amount_3usd as u128 * dry_run.virtual_price as u128 / VP_PRECISION / 100) as i64;

    state::log_activity("swap", &format!(
        "Leg 1: Rumi swap {} 3USD → ICP (min: {})", trade_amount_3usd, min_icp_out
    ));

    let icp_out = match swaps::rumi_swap(
        config.rumi_amm, RUMI_POOL_ID, config.three_usd_ledger, trade_amount_3usd, min_icp_out,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("Leg 1 OK: {} 3USD → {} ICP", trade_amount_3usd, amount));
            // Record Leg 1: sold 3USD (stablecoin), bought ICP (transit)
            state::mutate_state(|s| {
                s.trade_legs.push(state::TradeLeg {
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
            });
            amount
        }
        Err(e) => {
            let msg = format!("Rumi swap 3USD→ICP failed: {}", e);
            state::log_activity("swap", &format!("Leg 1 FAILED: {}", msg));
            log_error(&msg);
            return;
        }
    };

    // Rumi reports gross output, but the bot receives (icp_out - ICP_FEE) after the
    // output transfer fee, and ICPSwap's depositFromAndSwap costs another ICP_FEE.
    let usable_icp = icp_out.saturating_sub(ICP_FEE * 2);
    let min_ckusdc_out = dry_run.expected_output_amount * (10_000 - SLIPPAGE_BPS) / 10_000;

    state::log_activity("swap", &format!(
        "Leg 2: ICPSwap swap {} ICP → ckUSDC (min: {}, raw from Rumi: {})", usable_icp, min_ckusdc_out, icp_out
    ));

    let ckusdc_out = match swaps::icpswap_swap(
        config.icpswap_pool, usable_icp, config.icpswap_icp_is_token0, min_ckusdc_out, ICP_FEE, CKUSDC_FEE,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("Leg 2 OK: {} ICP → {} ckUSDC", icp_out, amount));
            // Record Leg 2: sold ICP (transit), bought ckUSDC (stablecoin)
            state::mutate_state(|s| {
                s.trade_legs.push(state::TradeLeg {
                    timestamp: ic_cdk::api::time(),
                    leg_type: state::LegType::Leg2,
                    dex: "ICPSwap".to_string(),
                    sold_token: "ICP".to_string(),
                    sold_amount: usable_icp,
                    bought_token: "ckUSDC".to_string(),
                    bought_amount: amount,
                    sold_usd_value: 0,            // ICP is transit
                    bought_usd_value: amount as i64, // ckUSDC IS USD (6 dec)
                    fees_usd: CKUSDC_FEE as i64,
                });
            });
            amount
        }
        Err(e) => {
            let msg = format!("ICPSwap swap ICP→ckUSDC failed (holding {} ICP): {}", icp_out, e);
            state::log_activity("swap", &format!("Leg 2 FAILED: {}", msg));
            log_error(&msg);
            return; // Leg 1 is already recorded; drain will pick up the ICP
        }
    };

    let net_profit = ckusdc_out as i64 - cost_usd_6dec - CKUSDC_FEE as i64;
    state::log_activity("trade", &format!(
        "COMPLETE RumiToIcpswap: {} 3USD → {} ICP → {} ckUSDC | profit: {} (6dec USD)",
        trade_amount_3usd, icp_out, ckusdc_out, net_profit
    ));
}

async fn execute_icpswap_to_rumi(config: &state::BotConfig, dry_run: &DryRunResult) {
    let trade_amount_ckusdc = dry_run.optimal_input_amount;
    let min_icp_out = dry_run.expected_icp_amount * (10_000 - SLIPPAGE_BPS) / 10_000;

    state::log_activity("swap", &format!(
        "Leg 1: ICPSwap swap {} ckUSDC → ICP (min: {})", trade_amount_ckusdc, min_icp_out
    ));

    let icp_out = match swaps::icpswap_swap(
        config.icpswap_pool, trade_amount_ckusdc, !config.icpswap_icp_is_token0, min_icp_out, CKUSDC_FEE, ICP_FEE,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("Leg 1 OK: {} ckUSDC → {} ICP", trade_amount_ckusdc, amount));
            // Record Leg 1: sold ckUSDC (stablecoin), bought ICP (transit)
            state::mutate_state(|s| {
                s.trade_legs.push(state::TradeLeg {
                    timestamp: ic_cdk::api::time(),
                    leg_type: state::LegType::Leg1,
                    dex: "ICPSwap".to_string(),
                    sold_token: "ckUSDC".to_string(),
                    sold_amount: trade_amount_ckusdc,
                    bought_token: "ICP".to_string(),
                    bought_amount: amount,
                    sold_usd_value: trade_amount_ckusdc as i64, // ckUSDC IS USD
                    bought_usd_value: 0, // ICP is transit
                    fees_usd: CKUSDC_FEE as i64,
                });
            });
            amount
        }
        Err(e) => {
            let msg = format!("ICPSwap swap ckUSDC→ICP failed: {}", e);
            state::log_activity("swap", &format!("Leg 1 FAILED: {}", msg));
            log_error(&msg);
            return;
        }
    };

    // ICPSwap reports gross output, but the bot receives (icp_out - ICP_FEE) after the
    // output transfer fee, and Rumi's transfer_from costs another ICP_FEE.
    let usable_icp = icp_out.saturating_sub(ICP_FEE * 2);
    let min_3usd_out = dry_run.expected_output_amount * (10_000 - SLIPPAGE_BPS) / 10_000;

    state::log_activity("swap", &format!(
        "Leg 2: Rumi swap {} ICP → 3USD (min: {}, raw from ICPSwap: {})", usable_icp, min_3usd_out, icp_out
    ));

    // Fetch VP for 3USD USD valuation
    let vp = prices::fetch_virtual_price(config.rumi_3pool).await.unwrap_or(1_000_000_000_000_000_000);

    let three_usd_out = match swaps::rumi_swap(
        config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, usable_icp, min_3usd_out,
    ).await {
        Ok(amount) => {
            let out_usd_6dec = (amount as u128 * vp as u128 / VP_PRECISION / 100) as i64;
            state::log_activity("swap", &format!("Leg 2 OK: {} ICP → {} 3USD", icp_out, amount));
            // Record Leg 2: sold ICP (transit), bought 3USD (stablecoin)
            state::mutate_state(|s| {
                s.trade_legs.push(state::TradeLeg {
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
            });
            amount
        }
        Err(e) => {
            let msg = format!("Rumi swap ICP→3USD failed (holding {} ICP): {}", icp_out, e);
            state::log_activity("swap", &format!("Leg 2 FAILED: {}", msg));
            log_error(&msg);
            return; // Leg 1 is already recorded; drain will pick up the ICP
        }
    };

    let input_usd_6dec = trade_amount_ckusdc as i64;
    let output_usd_6dec = (three_usd_out as u128 * vp as u128 / VP_PRECISION / 100) as i64;
    let net_profit = output_usd_6dec - input_usd_6dec - CKUSDC_FEE as i64;

    state::log_activity("trade", &format!(
        "COMPLETE IcpswapToRumi: {} ckUSDC → {} ICP → {} 3USD | profit: {} (6dec USD)",
        trade_amount_ckusdc, icp_out, three_usd_out, net_profit
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
            state::mutate_state(|s| {
                s.trade_legs.push(state::TradeLeg {
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
            state::mutate_state(|s| {
                s.trade_legs.push(state::TradeLeg {
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
            });
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
            state::mutate_state(|s| {
                s.trade_legs.push(state::TradeLeg {
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
            state::mutate_state(|s| {
                s.trade_legs.push(state::TradeLeg {
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
            });
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

    let rumi_quote = prices::fetch_rumi_price(config.rumi_amm, RUMI_POOL_ID, config.icp_ledger);
    let icpswap_quote = prices::fetch_icpswap_price(config.icpswap_pool, config.icpswap_icp_is_token0);
    let vp = prices::fetch_virtual_price(config.rumi_3pool);

    let (rumi_res, icpswap_res, vp_res) =
        futures::future::join3(rumi_quote, icpswap_quote, vp).await;

    // Compute slippage-protected min outputs from quotes BEFORE consuming Results.
    // Quotes are for 1 ICP; scale to drain_amount and apply slippage tolerance.
    let rumi_min_out = rumi_res.as_ref().ok().map(|&quote_per_icp| {
        let scaled = quote_per_icp as u128 * drain_amount as u128 / 100_000_000;
        (scaled * (10_000 - SLIPPAGE_BPS) as u128 / 10_000) as u64
    }).unwrap_or(0);
    let icpswap_min_out = icpswap_res.as_ref().ok().map(|&quote_per_icp| {
        let scaled = quote_per_icp as u128 * drain_amount as u128 / 100_000_000;
        (scaled * (10_000 - SLIPPAGE_BPS) as u128 / 10_000) as u64
    }).unwrap_or(0);

    let rumi_usd = rumi_res.ok().and_then(|r| {
        vp_res.as_ref().ok().map(|vp| (r as u128 * *vp as u128 / VP_PRECISION / 100) as u64)
    });
    let icpswap_usd = icpswap_res.ok();

    // Helper to record a drain leg
    fn record_drain_leg(dex: &str, icp_sold: u64, token_out: &str, amount_out: u64, usd_value_out: i64, fees: i64) {
        state::log_activity("drain", &format!("Drained {} ICP → {} {} via {}", icp_sold, amount_out, token_out, dex));
        state::mutate_state(|s| {
            s.trade_legs.push(state::TradeLeg {
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
        });
    }

    // Compute VP for 3USD valuation (use 1.0 as fallback)
    let vp_val = vp_res.as_ref().copied().unwrap_or(1_000_000_000_000_000_000);

    // Try best-rate DEX first, fall back to the other on failure
    let try_rumi = |amt: u64, min_out: u64| swaps::rumi_swap(config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, amt, min_out);
    let try_icpswap = |amt: u64, min_out: u64| swaps::icpswap_swap(config.icpswap_pool, amt, config.icpswap_icp_is_token0, min_out, ICP_FEE, CKUSDC_FEE);

    match (rumi_usd, icpswap_usd) {
        (Some(r), Some(i)) if r >= i => {
            match try_rumi(drain_amount, rumi_min_out).await {
                Ok(out) => {
                    let usd_out = (out as u128 * vp_val as u128 / VP_PRECISION / 100) as i64;
                    record_drain_leg("Rumi", drain_amount, "3USD", out, usd_out, 0);
                }
                Err(e) => {
                    state::log_activity("drain", &format!("Rumi drain failed ({}), falling back to ICPSwap", e));
                    let remaining = fetch_balance(config.icp_ledger).await.unwrap_or(0);
                    let fallback_drainable = remaining.saturating_sub(ICP_RESERVE);
                    if fallback_drainable > ICP_FEE * 2 {
                        let fallback_amount = fallback_drainable - ICP_FEE;
                        // Use 0 slippage for fallback — we already failed once, just get out
                        match try_icpswap(fallback_amount, 0).await {
                            Ok(out) => record_drain_leg("ICPSwap", fallback_amount, "ckUSDC", out, out as i64, CKUSDC_FEE as i64),
                            Err(e2) => state::log_activity("drain", &format!("ICPSwap fallback also failed: {}", e2)),
                        }
                    }
                }
            }
        }
        (_, Some(_)) => {
            match try_icpswap(drain_amount, icpswap_min_out).await {
                Ok(out) => record_drain_leg("ICPSwap", drain_amount, "ckUSDC", out, out as i64, CKUSDC_FEE as i64),
                Err(e) => state::log_activity("drain", &format!("Drain via ICPSwap failed: {}", e)),
            }
        }
        (Some(_), None) => {
            match try_rumi(drain_amount, rumi_min_out).await {
                Ok(out) => {
                    let usd_out = (out as u128 * vp_val as u128 / VP_PRECISION / 100) as i64;
                    record_drain_leg("Rumi", drain_amount, "3USD", out, usd_out, 0);
                }
                Err(e) => state::log_activity("drain", &format!("Drain via Rumi failed: {}", e)),
            }
        }
        (None, None) => {
            return Err("Both DEX quotes failed during ICP drain".to_string());
        }
    }

    Ok(())
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
    state::mutate_state(|s| {
        s.errors.push(ErrorRecord {
            timestamp: ic_cdk::api::time(),
            message: msg.to_string(),
        });
        if s.errors.len() > 1000 {
            s.errors.drain(0..500);
        }
    });
}
