use candid::{CandidType, Deserialize, Nat, Principal};
use ic_cdk_macros::{init, post_upgrade, pre_upgrade, query, update};

mod state;
mod prices;
mod swaps;
mod arb;

use state::{BotConfig, TradeRecord, ErrorRecord};

#[derive(CandidType, Deserialize)]
pub struct InitArgs {
    pub config: BotConfig,
}

#[init]
fn init(args: InitArgs) {
    state::init_state(state::BotState {
        config: args.config,
        ..Default::default()
    });
    setup_timer();
}

#[pre_upgrade]
fn pre_upgrade() {
    state::save_to_stable_memory();
}

#[post_upgrade]
fn post_upgrade() {
    state::load_from_stable_memory();
    setup_timer();
}

fn setup_timer() {
    ic_cdk_timers::set_timer_interval(
        std::time::Duration::from_secs(60),
        || ic_cdk::spawn(arb::run_arb_cycle()),
    );
}

fn require_owner() {
    let caller = ic_cdk::api::caller();
    let owner = state::read_state(|s| s.config.owner);
    if caller != owner {
        ic_cdk::trap("Unauthorized: only owner can call this");
    }
}

// ─── Query Methods ───

#[query]
fn get_config() -> BotConfig {
    state::read_state(|s| s.config.clone())
}

#[query]
fn get_trade_history(offset: u64, limit: u64) -> Vec<TradeRecord> {
    state::read_state(|s| {
        let len = s.trades.len();
        let start = (len as u64).saturating_sub(offset + limit) as usize;
        let end = (len as u64).saturating_sub(offset) as usize;
        s.trades[start..end].to_vec()
    })
}

#[derive(CandidType)]
pub struct TradeSummary {
    pub total_trades: u64,
    pub total_net_profit_usd: i64,
    pub total_ledger_fees_usd: i64,
    pub avg_profit_per_trade_usd: i64,
    pub rumi_to_icpswap_count: u64,
    pub rumi_to_icpswap_profit: i64,
    pub icpswap_to_rumi_count: u64,
    pub icpswap_to_rumi_profit: i64,
}

#[query]
fn get_summary() -> TradeSummary {
    state::read_state(|s| {
        let mut summary = TradeSummary {
            total_trades: s.trades.len() as u64,
            total_net_profit_usd: 0,
            total_ledger_fees_usd: 0,
            avg_profit_per_trade_usd: 0,
            rumi_to_icpswap_count: 0,
            rumi_to_icpswap_profit: 0,
            icpswap_to_rumi_count: 0,
            icpswap_to_rumi_profit: 0,
        };
        for trade in &s.trades {
            summary.total_net_profit_usd += trade.net_profit_usd;
            summary.total_ledger_fees_usd += trade.ledger_fees_usd;
            match trade.direction {
                state::Direction::RumiToIcpswap => {
                    summary.rumi_to_icpswap_count += 1;
                    summary.rumi_to_icpswap_profit += trade.net_profit_usd;
                }
                state::Direction::IcpswapToRumi => {
                    summary.icpswap_to_rumi_count += 1;
                    summary.icpswap_to_rumi_profit += trade.net_profit_usd;
                }
            }
        }
        if summary.total_trades > 0 {
            summary.avg_profit_per_trade_usd = summary.total_net_profit_usd / summary.total_trades as i64;
        }
        summary
    })
}

#[query]
fn get_errors(offset: u64, limit: u64) -> Vec<ErrorRecord> {
    state::read_state(|s| {
        let len = s.errors.len();
        let start = (len as u64).saturating_sub(offset + limit) as usize;
        let end = (len as u64).saturating_sub(offset) as usize;
        s.errors[start..end].to_vec()
    })
}

// ─── Admin Methods ───

#[update]
fn set_config(config: BotConfig) {
    require_owner();
    state::mutate_state(|s| s.config = config);
}

#[update]
fn pause() {
    require_owner();
    state::mutate_state(|s| s.config.paused = true);
}

#[update]
fn resume() {
    require_owner();
    state::mutate_state(|s| s.config.paused = false);
}

#[update]
async fn setup_approvals() {
    require_owner();
    let config = state::read_state(|s| s.config.clone());

    let r1 = swaps::approve_infinite(config.three_usd_ledger, config.rumi_amm).await;
    let r2 = swaps::approve_infinite(config.icp_ledger, config.rumi_amm).await;
    let r3 = swaps::approve_infinite(config.icp_ledger, config.icpswap_pool).await;
    let r4 = swaps::approve_infinite(config.ckusdc_ledger, config.icpswap_pool).await;

    let mut errors = Vec::new();
    if let Err(e) = r1 { errors.push(format!("3USD→RumiAMM: {}", e)); }
    if let Err(e) = r2 { errors.push(format!("ICP→RumiAMM: {}", e)); }
    if let Err(e) = r3 { errors.push(format!("ICP→ICPSwap: {}", e)); }
    if let Err(e) = r4 { errors.push(format!("ckUSDC→ICPSwap: {}", e)); }

    if !errors.is_empty() {
        ic_cdk::trap(&format!("Some approvals failed: {}", errors.join("; ")));
    }
}

#[update]
async fn withdraw(token_ledger: Principal, to: Principal, amount: u64) {
    require_owner();

    let transfer_args = icrc_ledger_types::icrc1::transfer::TransferArg {
        from_subaccount: None,
        to: icrc_ledger_types::icrc1::account::Account { owner: to, subaccount: None },
        amount: Nat::from(amount),
        fee: None,
        memo: None,
        created_at_time: None,
    };

    let result: Result<(Result<Nat, icrc_ledger_types::icrc1::transfer::TransferError>,), _> =
        ic_cdk::call(token_ledger, "icrc1_transfer", (transfer_args,)).await;

    match result {
        Ok((Ok(_),)) => {}
        Ok((Err(e),)) => ic_cdk::trap(&format!("Transfer failed: {:?}", e)),
        Err((code, msg)) => ic_cdk::trap(&format!("Transfer call failed: {:?} {}", code, msg)),
    }
}

#[update]
async fn manual_arb_cycle() {
    require_owner();
    arb::run_arb_cycle().await;
}
