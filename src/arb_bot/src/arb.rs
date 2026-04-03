use candid::{CandidType, Nat, Principal};
use std::cell::Cell;

use crate::prices::{self, PriceData, nat_to_u64};
use crate::state::{self, Direction, ErrorRecord, Token, TradeRecord};
use crate::swaps;

const ICP_FEE: u64 = 10_000;        // 0.0001 ICP
const CKUSDC_FEE: u64 = 10_000;      // 0.01 ckUSDC
// Note: 3USD has no transfer fee (0)

// Pool ID is deterministic: sorted principals joined by "_"
const RUMI_POOL_ID: &str = "fohh4-yyaaa-aaaap-qtkpa-cai_ryjl3-tyaaa-aaaaa-aaaba-cai";

/// Slippage tolerance in basis points (50 bps = 0.5%)
const SLIPPAGE_BPS: u64 = 50;

/// Number of candidate trade sizes to evaluate
const NUM_CANDIDATES: u64 = 6;

/// VP precision (1e18)
const VP_PRECISION: u128 = 1_000_000_000_000_000_000;

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

    // Resolve ICPSwap token ordering on first cycle
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

    let config = state::read_state(|s| s.config.clone());
    if config.paused { return; }

    if let Err(e) = drain_residual_icp(&config).await {
        log_error(&format!("Drain residual ICP failed: {}", e));
    }

    let dry_run = match compute_optimal_trade(&config).await {
        Ok(dr) => dr,
        Err(e) => { log_error(&format!("Trade computation failed: {}", e)); return; }
    };

    if !dry_run.should_trade {
        state::log_activity("arb_skip", &dry_run.message);
        return;
    }

    state::log_activity("arb_start", &format!(
        "Starting {:?} trade: {} {:?} → est {} ICP → est {} {:?} (spread: {} bps, est profit: {})",
        dry_run.direction.as_ref().unwrap(),
        dry_run.optimal_input_amount,
        dry_run.optimal_input_token.as_ref().unwrap(),
        dry_run.expected_icp_amount,
        dry_run.expected_output_amount,
        dry_run.expected_output_token.as_ref().unwrap(),
        dry_run.spread_bps,
        dry_run.expected_profit_usd,
    ));

    // Execute the optimal trade
    match dry_run.direction.as_ref().unwrap() {
        Direction::RumiToIcpswap => {
            execute_rumi_to_icpswap(&config, &dry_run).await;
        }
        Direction::IcpswapToRumi => {
            execute_icpswap_to_rumi(&config, &dry_run).await;
        }
    }
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

    let abs_spread = result.spread_bps.unsigned_abs();
    if abs_spread < config.min_spread_bps {
        result.message = format!("Spread {} bps < minimum {} bps", abs_spread, config.min_spread_bps);
        return Ok(result);
    }

    // Fetch balances
    let (bal_3usd, bal_ckusdc) = futures::future::join(
        fetch_balance(config.three_usd_ledger),
        fetch_balance(config.ckusdc_ledger),
    ).await;
    result.balance_3usd = bal_3usd.unwrap_or(0);
    result.balance_ckusdc = bal_ckusdc.unwrap_or(0);

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
    let icpswap_futs: Vec<_> = stage1.iter().map(|&(_, icp_amount)| {
        prices::fetch_icpswap_quote_for_amount(
            config.icpswap_pool, icp_amount, config.icpswap_icp_is_token0,
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
    let rumi_futs: Vec<_> = stage1.iter().map(|&(_, icp_amount)| {
        prices::fetch_rumi_quote_for_amount(
            config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, icp_amount,
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

// ─── Execute Trades ───

async fn execute_rumi_to_icpswap(config: &state::BotConfig, dry_run: &DryRunResult) {
    let trade_amount_3usd = dry_run.optimal_input_amount;
    let min_icp_out = dry_run.expected_icp_amount * (10_000 - SLIPPAGE_BPS) / 10_000;

    state::log_activity("swap", &format!(
        "Leg 1: Rumi swap {} 3USD → ICP (min: {})", trade_amount_3usd, min_icp_out
    ));

    let icp_out = match swaps::rumi_swap(
        config.rumi_amm, RUMI_POOL_ID, config.three_usd_ledger, trade_amount_3usd, min_icp_out,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("Leg 1 OK: {} 3USD → {} ICP", trade_amount_3usd, amount));
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
            amount
        }
        Err(e) => {
            let msg = format!("ICPSwap swap ICP→ckUSDC failed (holding {} ICP): {}", icp_out, e);
            state::log_activity("swap", &format!("Leg 2 FAILED: {}", msg));
            log_error(&msg);
            return;
        }
    };

    let input_usd_6dec = (trade_amount_3usd as u128 * dry_run.virtual_price as u128 / VP_PRECISION / 100) as i64;
    let output_usd_6dec = ckusdc_out as i64;
    let ledger_fees_usd = CKUSDC_FEE as i64;
    let net_profit = output_usd_6dec - input_usd_6dec - ledger_fees_usd;

    state::log_activity("trade", &format!(
        "COMPLETE RumiToIcpswap: {} 3USD → {} ICP → {} ckUSDC | profit: {} (6dec USD)",
        trade_amount_3usd, icp_out, ckusdc_out, net_profit
    ));

    state::mutate_state(|s| {
        s.trades.push(TradeRecord {
            timestamp: ic_cdk::api::time(),
            direction: Direction::RumiToIcpswap,
            icp_amount: icp_out,
            input_amount: trade_amount_3usd,
            input_token: Token::ThreeUSD,
            output_amount: ckusdc_out,
            output_token: Token::CkUSDC,
            virtual_price: dry_run.virtual_price,
            ledger_fees_usd,
            net_profit_usd: net_profit,
            spread_bps: dry_run.spread_bps.unsigned_abs(),
        });
    });
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

    let three_usd_out = match swaps::rumi_swap(
        config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, usable_icp, min_3usd_out,
    ).await {
        Ok(amount) => {
            state::log_activity("swap", &format!("Leg 2 OK: {} ICP → {} 3USD", icp_out, amount));
            amount
        }
        Err(e) => {
            let msg = format!("Rumi swap ICP→3USD failed (holding {} ICP): {}", icp_out, e);
            state::log_activity("swap", &format!("Leg 2 FAILED: {}", msg));
            log_error(&msg);
            return;
        }
    };

    let input_usd_6dec = trade_amount_ckusdc as i64;
    let output_usd_6dec = (three_usd_out as u128 * dry_run.virtual_price as u128 / VP_PRECISION / 100) as i64;
    let ledger_fees_usd = CKUSDC_FEE as i64;
    let net_profit = output_usd_6dec - input_usd_6dec - ledger_fees_usd;

    state::log_activity("trade", &format!(
        "COMPLETE IcpswapToRumi: {} ckUSDC → {} ICP → {} 3USD | profit: {} (6dec USD)",
        trade_amount_ckusdc, icp_out, three_usd_out, net_profit
    ));

    state::mutate_state(|s| {
        s.trades.push(TradeRecord {
            timestamp: ic_cdk::api::time(),
            direction: Direction::IcpswapToRumi,
            icp_amount: icp_out,
            input_amount: trade_amount_ckusdc,
            input_token: Token::CkUSDC,
            output_amount: three_usd_out,
            output_token: Token::ThreeUSD,
            virtual_price: dry_run.virtual_price,
            ledger_fees_usd,
            net_profit_usd: net_profit,
            spread_bps: dry_run.spread_bps.unsigned_abs(),
        });
    });
}

// ─── Helpers ───

async fn drain_residual_icp(config: &state::BotConfig) -> Result<(), String> {
    let icp_balance = fetch_balance(config.icp_ledger).await?;

    if icp_balance <= ICP_FEE * 2 {
        return Ok(());
    }

    // Reserve fee for the icrc2_transfer_from the DEX will trigger
    let drain_amount = icp_balance - ICP_FEE;

    state::log_activity("drain", &format!("Draining {} residual ICP (balance: {})", drain_amount, icp_balance));

    let rumi_quote = prices::fetch_rumi_price(config.rumi_amm, RUMI_POOL_ID, config.icp_ledger);
    let icpswap_quote = prices::fetch_icpswap_price(config.icpswap_pool, config.icpswap_icp_is_token0);
    let vp = prices::fetch_virtual_price(config.rumi_3pool);

    let (rumi_res, icpswap_res, vp_res) =
        futures::future::join3(rumi_quote, icpswap_quote, vp).await;

    let rumi_usd = rumi_res.ok().and_then(|r| {
        vp_res.as_ref().ok().map(|vp| (r as u128 * *vp as u128 / VP_PRECISION / 100) as u64)
    });
    let icpswap_usd = icpswap_res.ok();

    // Try best-rate DEX first, fall back to the other on failure
    let try_rumi = |amt: u64| swaps::rumi_swap(config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, amt, 0);
    let try_icpswap = |amt: u64| swaps::icpswap_swap(config.icpswap_pool, amt, config.icpswap_icp_is_token0, 0, ICP_FEE, CKUSDC_FEE);

    match (rumi_usd, icpswap_usd) {
        (Some(r), Some(i)) if r >= i => {
            match try_rumi(drain_amount).await {
                Ok(out) => { state::log_activity("drain", &format!("Drained {} ICP → {} 3USD via Rumi", drain_amount, out)); }
                Err(e) => {
                    state::log_activity("drain", &format!("Rumi drain failed ({}), falling back to ICPSwap", e));
                    // Rumi may have taken the ICP — re-check balance
                    let remaining = fetch_balance(config.icp_ledger).await.unwrap_or(0);
                    if remaining > ICP_FEE * 2 {
                        let fallback_amount = remaining - ICP_FEE;
                        match try_icpswap(fallback_amount).await {
                            Ok(out) => state::log_activity("drain", &format!("Drained {} ICP → {} ckUSDC via ICPSwap (fallback)", fallback_amount, out)),
                            Err(e2) => state::log_activity("drain", &format!("ICPSwap fallback also failed: {}", e2)),
                        }
                    }
                }
            }
        }
        (_, Some(_)) => {
            match try_icpswap(drain_amount).await {
                Ok(out) => state::log_activity("drain", &format!("Drained {} ICP → {} ckUSDC via ICPSwap", drain_amount, out)),
                Err(e) => state::log_activity("drain", &format!("Drain via ICPSwap failed: {}", e)),
            }
        }
        (Some(_), None) => {
            match try_rumi(drain_amount).await {
                Ok(out) => state::log_activity("drain", &format!("Drained {} ICP → {} 3USD via Rumi", drain_amount, out)),
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
