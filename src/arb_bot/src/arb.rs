use candid::{Nat, Principal};

use crate::prices::{self, PriceData, nat_to_u64};
use crate::state::{self, Direction, ErrorRecord, Token, TradeRecord};
use crate::swaps;

const ICP_FEE: u64 = 10_000;
const CKUSDC_FEE: u64 = 10_000;
const THREE_USD_FEE: u64 = 10_000;

const RUMI_POOL_ID: &str = "3usd_icp"; // TBD — confirm with Rumi AMM
const ICPSWAP_ICP_IS_TOKEN0: bool = true; // TBD — verify from pool metadata

pub async fn run_arb_cycle() {
    let config = state::read_state(|s| s.config.clone());

    if config.paused {
        return;
    }

    if let Err(e) = drain_residual_icp(&config).await {
        log_error(&format!("Drain residual ICP failed: {}", e));
    }

    let prices = match prices::fetch_all_prices(
        config.rumi_amm,
        RUMI_POOL_ID,
        config.icp_ledger,
        config.rumi_3pool,
        config.icpswap_pool,
        ICPSWAP_ICP_IS_TOKEN0,
    ).await {
        Ok(p) => p,
        Err(e) => {
            log_error(&format!("Price fetch failed: {}", e));
            return;
        }
    };

    let spread = prices.spread_bps();
    let abs_spread = spread.unsigned_abs();
    if abs_spread < config.min_spread_bps {
        return;
    }

    if spread > 0 {
        execute_rumi_to_icpswap(&config, &prices, abs_spread).await;
    } else {
        execute_icpswap_to_rumi(&config, &prices, abs_spread).await;
    }
}

async fn execute_rumi_to_icpswap(config: &state::BotConfig, prices: &PriceData, spread_bps: u32) {
    let three_usd_balance = match fetch_balance(config.three_usd_ledger).await {
        Ok(b) => b,
        Err(e) => { log_error(&format!("Failed to get 3USD balance: {}", e)); return; }
    };

    if three_usd_balance < 1_000_000 {
        log_error("Insufficient 3USD balance for arb");
        return;
    }

    let max_three_usd = if prices.virtual_price > 0 {
        (config.max_trade_size_usd as u128 * 100_000_000 * 100 / prices.virtual_price as u128) as u64
    } else {
        three_usd_balance
    };
    let trade_amount_3usd = three_usd_balance.min(max_three_usd);

    let icp_out = match swaps::rumi_swap(
        config.rumi_amm, RUMI_POOL_ID, config.three_usd_ledger, trade_amount_3usd, 0,
    ).await {
        Ok(amount) => amount,
        Err(e) => { log_error(&format!("Rumi swap 3USD→ICP failed: {}", e)); return; }
    };

    let ckusdc_out = match swaps::icpswap_swap(
        config.icpswap_pool, icp_out, ICPSWAP_ICP_IS_TOKEN0, 0, ICP_FEE, CKUSDC_FEE,
    ).await {
        Ok(amount) => amount,
        Err(e) => {
            log_error(&format!("ICPSwap swap ICP→ckUSDC failed (holding {} ICP): {}", icp_out, e));
            return;
        }
    };

    let input_usd_6dec = (trade_amount_3usd as u128 * prices.virtual_price as u128 / 100_000_000 / 100) as i64;
    let output_usd_6dec = ckusdc_out as i64;
    let ledger_fees_usd = 10_000i64;
    let net_profit = output_usd_6dec - input_usd_6dec - ledger_fees_usd;

    state::mutate_state(|s| {
        s.trades.push(TradeRecord {
            timestamp: ic_cdk::api::time(),
            direction: Direction::RumiToIcpswap,
            icp_amount: icp_out,
            input_amount: trade_amount_3usd,
            input_token: Token::ThreeUSD,
            output_amount: ckusdc_out,
            output_token: Token::CkUSDC,
            virtual_price: prices.virtual_price,
            ledger_fees_usd,
            net_profit_usd: net_profit,
            spread_bps,
        });
    });
}

async fn execute_icpswap_to_rumi(config: &state::BotConfig, prices: &PriceData, spread_bps: u32) {
    let ckusdc_balance = match fetch_balance(config.ckusdc_ledger).await {
        Ok(b) => b,
        Err(e) => { log_error(&format!("Failed to get ckUSDC balance: {}", e)); return; }
    };

    if ckusdc_balance < 10_000 {
        log_error("Insufficient ckUSDC balance for arb");
        return;
    }

    let trade_amount_ckusdc = ckusdc_balance.min(config.max_trade_size_usd);

    let icp_out = match swaps::icpswap_swap(
        config.icpswap_pool, trade_amount_ckusdc, !ICPSWAP_ICP_IS_TOKEN0, 0, CKUSDC_FEE, ICP_FEE,
    ).await {
        Ok(amount) => amount,
        Err(e) => { log_error(&format!("ICPSwap swap ckUSDC→ICP failed: {}", e)); return; }
    };

    let three_usd_out = match swaps::rumi_swap(
        config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, icp_out, 0,
    ).await {
        Ok(amount) => amount,
        Err(e) => {
            log_error(&format!("Rumi swap ICP→3USD failed (holding {} ICP): {}", icp_out, e));
            return;
        }
    };

    let input_usd_6dec = trade_amount_ckusdc as i64;
    let output_usd_6dec = (three_usd_out as u128 * prices.virtual_price as u128 / 100_000_000 / 100) as i64;
    let ledger_fees_usd = 10_000i64;
    let net_profit = output_usd_6dec - input_usd_6dec - ledger_fees_usd;

    state::mutate_state(|s| {
        s.trades.push(TradeRecord {
            timestamp: ic_cdk::api::time(),
            direction: Direction::IcpswapToRumi,
            icp_amount: icp_out,
            input_amount: trade_amount_ckusdc,
            input_token: Token::CkUSDC,
            output_amount: three_usd_out,
            output_token: Token::ThreeUSD,
            virtual_price: prices.virtual_price,
            ledger_fees_usd,
            net_profit_usd: net_profit,
            spread_bps,
        });
    });
}

async fn drain_residual_icp(config: &state::BotConfig) -> Result<(), String> {
    let icp_balance = fetch_balance(config.icp_ledger).await?;

    if icp_balance <= 100_000 {
        return Ok(());
    }

    let rumi_quote = prices::fetch_rumi_price(config.rumi_amm, RUMI_POOL_ID, config.icp_ledger);
    let icpswap_quote = prices::fetch_icpswap_price(config.icpswap_pool, ICPSWAP_ICP_IS_TOKEN0);
    let vp = prices::fetch_virtual_price(config.rumi_3pool);

    let (rumi_res, icpswap_res, vp_res) =
        futures::future::join3(rumi_quote, icpswap_quote, vp).await;

    let rumi_usd = rumi_res.ok().and_then(|r| {
        vp_res.as_ref().ok().map(|vp| (r as u128 * *vp as u128 / 100_000_000 / 100) as u64)
    });
    let icpswap_usd = icpswap_res.ok();

    match (rumi_usd, icpswap_usd) {
        (Some(r), Some(i)) if r >= i => {
            let _ = swaps::rumi_swap(config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, icp_balance, 0).await;
        }
        (_, Some(_)) => {
            let _ = swaps::icpswap_swap(config.icpswap_pool, icp_balance, ICPSWAP_ICP_IS_TOKEN0, 0, ICP_FEE, CKUSDC_FEE).await;
        }
        (Some(_), None) => {
            let _ = swaps::rumi_swap(config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, icp_balance, 0).await;
        }
        (None, None) => {
            return Err("Both DEX quotes failed during ICP drain".to_string());
        }
    }

    Ok(())
}

async fn fetch_balance(ledger: Principal) -> Result<u64, String> {
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
