use candid::{CandidType, Deserialize, Nat, Principal};
use ic_cdk_macros::{init, post_upgrade, pre_upgrade, query, update};

mod state;
mod prices;
mod swaps;
mod arb;

use state::{BotConfig, TradeRecord, ErrorRecord, ActivityRecord};

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
    // Can't make inter-canister calls during init, so resolve token ordering
    // on the first timer tick. Start the timer immediately.
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
        std::time::Duration::from_secs(90),
        || ic_cdk::spawn(arb::run_arb_cycle()),
    );
}

fn require_admin() {
    let caller = ic_cdk::api::caller();
    let authorized = state::read_state(|s| {
        caller == s.config.owner || s.config.admins.contains(&caller)
    });
    if !authorized {
        ic_cdk::trap("Unauthorized: only owner or admins can call this");
    }
}

/// Check if a principal is an authorized admin (used by dashboard to show/hide controls)
#[query]
fn is_admin(principal: Principal) -> bool {
    state::read_state(|s| {
        principal == s.config.owner || s.config.admins.contains(&principal)
    })
}

/// Add an admin principal (owner only)
#[update]
fn add_admin(principal: Principal) {
    let caller = ic_cdk::api::caller();
    let is_owner = state::read_state(|s| caller == s.config.owner);
    if !is_owner {
        ic_cdk::trap("Only owner can add admins");
    }
    state::mutate_state(|s| {
        if !s.config.admins.contains(&principal) {
            s.config.admins.push(principal);
        }
    });
}

/// Remove an admin principal (owner only)
#[update]
fn remove_admin(principal: Principal) {
    let caller = ic_cdk::api::caller();
    let is_owner = state::read_state(|s| caller == s.config.owner);
    if !is_owner {
        ic_cdk::trap("Only owner can remove admins");
    }
    state::mutate_state(|s| {
        s.config.admins.retain(|a| a != &principal);
    });
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

#[query]
fn get_activity_log(offset: u64, limit: u64) -> Vec<ActivityRecord> {
    state::read_state(|s| {
        let len = s.activity_log.len();
        let start = (len as u64).saturating_sub(offset + limit) as usize;
        let end = (len as u64).saturating_sub(offset) as usize;
        s.activity_log[start..end].to_vec()
    })
}

// ─── Price Query ───

#[derive(CandidType)]
pub struct PriceInfo {
    pub rumi_icp_price_3usd: u64,      // 3USD per 1 ICP (8 decimals)
    pub rumi_icp_price_usd_6dec: u64,   // USD per 1 ICP (6 decimals)
    pub icpswap_icp_price_ckusdc: u64,  // ckUSDC per 1 ICP (6 decimals)
    pub virtual_price: u64,             // 3pool virtual price (8 decimals)
    pub spread_bps: i32,                // positive = Rumi cheaper
}

#[update]
async fn get_prices() -> PriceInfo {
    let config = state::read_state(|s| s.config.clone());
    let pool_id = "fohh4-yyaaa-aaaap-qtkpa-cai_ryjl3-tyaaa-aaaaa-aaaba-cai";
    match prices::fetch_all_prices(
        config.rumi_amm, pool_id, config.icp_ledger,
        config.rumi_3pool, config.icpswap_pool, config.icpswap_icp_is_token0,
    ).await {
        Ok(p) => PriceInfo {
            rumi_icp_price_3usd: p.rumi_icp_price_3usd_native,
            rumi_icp_price_usd_6dec: p.rumi_price_usd_6dec(),
            icpswap_icp_price_ckusdc: p.icpswap_icp_price_ckusdc_native,
            virtual_price: p.virtual_price,
            spread_bps: p.spread_bps(),
        },
        Err(e) => ic_cdk::trap(&format!("Price fetch failed: {}", e)),
    }
}

// ─── Admin Methods ───

#[update]
fn set_config(config: BotConfig) {
    require_admin();
    state::mutate_state(|s| s.config = config);
}

#[update]
fn pause() {
    require_admin();
    state::mutate_state(|s| s.config.paused = true);
    state::log_activity("admin", &format!("Bot paused by {}", ic_cdk::api::caller()));
}

#[update]
fn resume() {
    require_admin();
    state::mutate_state(|s| s.config.paused = false);
    state::log_activity("admin", &format!("Bot resumed by {}", ic_cdk::api::caller()));
}

// 3pool underlying token ledgers (icUSD=0, ckUSDT=1, ckUSDC=2)
const ICUSD_LEDGER: &str = "t6bor-paaaa-aaaap-qrd5q-cai";
const CKUSDT_LEDGER: &str = "cngnf-vqaaa-aaaar-qag4q-cai";

fn pool_token_ledger(coin_index: u8) -> Result<Principal, String> {
    let config = state::read_state(|s| s.config.clone());
    match coin_index {
        0 => Principal::from_text(ICUSD_LEDGER).map_err(|e| format!("{}", e)),
        1 => Principal::from_text(CKUSDT_LEDGER).map_err(|e| format!("{}", e)),
        2 => Ok(config.ckusdc_ledger),
        _ => Err("Invalid coin index (must be 0-2)".to_string()),
    }
}

fn pool_token_decimals(coin_index: u8) -> u8 {
    match coin_index { 0 => 8, 1 | 2 => 6, _ => 6 }
}

#[update]
async fn setup_approvals() -> String {
    require_admin();
    let config = state::read_state(|s| s.config.clone());

    let icusd = Principal::from_text(ICUSD_LEDGER).unwrap();
    let ckusdt = Principal::from_text(CKUSDT_LEDGER).unwrap();

    let mut ok = Vec::new();
    let mut errors = Vec::new();

    let approvals: Vec<(&str, Principal, Principal)> = vec![
        ("3USD→RumiAMM", config.three_usd_ledger, config.rumi_amm),
        ("ICP→RumiAMM", config.icp_ledger, config.rumi_amm),
        ("ICP→ICPSwap", config.icp_ledger, config.icpswap_pool),
        ("ckUSDC→ICPSwap", config.ckusdc_ledger, config.icpswap_pool),
        ("icUSD→3pool", icusd, config.rumi_3pool),
        ("ckUSDT→3pool", ckusdt, config.rumi_3pool),
        ("ckUSDC→3pool", config.ckusdc_ledger, config.rumi_3pool),
    ];

    for (label, ledger, spender) in approvals {
        match swaps::approve_infinite(ledger, spender).await {
            Ok(_) => {
                state::log_activity("approval", &format!("{}: approved", label));
                ok.push(label.to_string());
            }
            Err(e) => {
                state::log_activity("approval", &format!("{}: failed — {}", label, e));
                errors.push(format!("{}: {}", label, e));
            }
        }
    }

    let mut msg = format!("{}/{} approvals succeeded", ok.len(), ok.len() + errors.len());
    if !errors.is_empty() {
        msg.push_str(&format!(" (skipped: {})", errors.join("; ")));
    }
    msg
}

#[update]
async fn withdraw(token_ledger: Principal, to: Principal, amount: u64) {
    require_admin();

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
        Ok((Ok(_),)) => {
            state::log_activity("withdraw", &format!(
                "Withdrew {} from ledger {} to {} by {}",
                amount, token_ledger, to, ic_cdk::api::caller()
            ));
        }
        Ok((Err(e),)) => {
            let msg = format!("Withdraw failed: {:?} (ledger={}, to={}, amount={})", e, token_ledger, to, amount);
            state::log_activity("withdraw", &msg);
            ic_cdk::trap(&format!("Transfer failed: {:?}", e));
        }
        Err((code, msg)) => {
            let detail = format!("Withdraw call failed: {:?} {} (ledger={}, to={}, amount={})", code, msg, token_ledger, to, amount);
            state::log_activity("withdraw", &detail);
            ic_cdk::trap(&format!("Transfer call failed: {:?} {}", code, msg));
        }
    }
}

// ─── 3pool Deposit/Redeem ───

#[derive(CandidType)]
pub struct PoolQuote {
    pub estimated_output: u64,
}

/// Deposit a single stablecoin into the 3pool to mint 3USD LP tokens.
/// coin_index: 0=icUSD, 1=ckUSDT, 2=ckUSDC
#[update]
async fn pool_deposit(coin_index: u8, amount: u64, min_lp_out: u64) {
    require_admin();
    if coin_index > 2 { ic_cdk::trap("Invalid coin index (0-2)"); }

    let rumi_3pool = state::read_state(|s| s.config.rumi_3pool);
    let mut amounts = vec![Nat::from(0u64), Nat::from(0u64), Nat::from(0u64)];
    amounts[coin_index as usize] = Nat::from(amount);

    let token_name = match coin_index { 0 => "icUSD", 1 => "ckUSDT", _ => "ckUSDC" };
    match swaps::pool_add_liquidity(rumi_3pool, amounts, min_lp_out).await {
        Ok(lp_minted) => {
            state::log_activity("pool_deposit", &format!(
                "{} {} → {} 3USD LP (min: {}) by {}",
                amount, token_name, lp_minted, min_lp_out, ic_cdk::api::caller()
            ));
        }
        Err(e) => {
            state::log_activity("pool_deposit", &format!(
                "FAILED: {} {} (min_lp: {}) — {} by {}",
                amount, token_name, min_lp_out, e, ic_cdk::api::caller()
            ));
            ic_cdk::trap(&format!("Pool deposit failed: {}", e));
        }
    }
}

/// Redeem 3USD LP tokens for a single stablecoin.
/// coin_index: 0=icUSD, 1=ckUSDT, 2=ckUSDC
#[update]
async fn pool_redeem(coin_index: u8, lp_amount: u64, min_out: u64) {
    require_admin();
    if coin_index > 2 { ic_cdk::trap("Invalid coin index (0-2)"); }

    let rumi_3pool = state::read_state(|s| s.config.rumi_3pool);

    let token_name = match coin_index { 0 => "icUSD", 1 => "ckUSDT", _ => "ckUSDC" };
    match swaps::pool_remove_one_coin(rumi_3pool, lp_amount, coin_index, min_out).await {
        Ok(amount_out) => {
            state::log_activity("pool_redeem", &format!(
                "{} 3USD LP → {} {} (min: {}) by {}",
                lp_amount, amount_out, token_name, min_out, ic_cdk::api::caller()
            ));
        }
        Err(e) => {
            state::log_activity("pool_redeem", &format!(
                "FAILED: {} 3USD LP → {} (min: {}) — {} by {}",
                lp_amount, token_name, min_out, e, ic_cdk::api::caller()
            ));
            ic_cdk::trap(&format!("Pool redeem failed: {}", e));
        }
    }
}

/// Quote how much 3USD LP you'd get from depositing a stablecoin.
#[update]
async fn pool_quote_deposit(coin_index: u8, amount: u64) -> PoolQuote {
    if coin_index > 2 { ic_cdk::trap("Invalid coin index (0-2)"); }

    let rumi_3pool = state::read_state(|s| s.config.rumi_3pool);
    let mut amounts = vec![Nat::from(0u64), Nat::from(0u64), Nat::from(0u64)];
    amounts[coin_index as usize] = Nat::from(amount);

    match swaps::pool_calc_deposit(rumi_3pool, amounts).await {
        Ok(lp_out) => PoolQuote { estimated_output: lp_out },
        Err(e) => ic_cdk::trap(&format!("Quote failed: {}", e)),
    }
}

/// Quote how much stablecoin you'd get from redeeming 3USD LP.
#[update]
async fn pool_quote_redeem(coin_index: u8, lp_amount: u64) -> PoolQuote {
    if coin_index > 2 { ic_cdk::trap("Invalid coin index (0-2)"); }

    let rumi_3pool = state::read_state(|s| s.config.rumi_3pool);

    match swaps::pool_calc_redeem(rumi_3pool, lp_amount, coin_index).await {
        Ok(amount_out) => PoolQuote { estimated_output: amount_out },
        Err(e) => ic_cdk::trap(&format!("Quote failed: {}", e)),
    }
}

#[update]
async fn manual_arb_cycle() {
    require_admin();
    state::log_activity("admin", &format!("Manual arb cycle triggered by {}", ic_cdk::api::caller()));
    arb::run_arb_cycle().await;
}

#[update]
async fn dry_run_arb_cycle() -> arb::DryRunResult {
    require_admin();

    // Ensure token ordering is resolved first
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
                let mut result = arb::DryRunResult::default();
                result.message = format!("Failed to resolve token ordering: {}", e);
                return result;
            }
        }
    }

    let config = state::read_state(|s| s.config.clone());
    match arb::compute_optimal_trade(&config).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("Computation failed: {}", e);
            result
        }
    }
}

// ─── HTTP Dashboard ───

#[derive(CandidType, Deserialize)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(CandidType)]
pub struct HttpResponse {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

const DASHBOARD_HTML: &str = include_str!("dashboard.html");

#[query]
fn http_request(_req: HttpRequest) -> HttpResponse {
    HttpResponse {
        status_code: 200,
        headers: vec![
            ("Content-Type".to_string(), "text/html; charset=utf-8".to_string()),
            ("Cache-Control".to_string(), "no-cache".to_string()),
        ],
        body: DASHBOARD_HTML.as_bytes().to_vec(),
    }
}
