use candid::{CandidType, Deserialize, Nat, Principal};
use ic_cdk_macros::{init, post_upgrade, pre_upgrade, query, update};
use ic_cdk_timers::TimerId;
use std::cell::RefCell;

pub mod state; // pub so integration tests can verify serde upgrade defaults
mod prices;
mod swaps;
mod partydex;
mod arb;
mod volume;

use state::{BotConfig, TradeRecord, TradeLeg, ErrorRecord, ActivityRecord, CycleSnapshot};

thread_local! {
    static ARB_TIMER_ID: RefCell<Option<TimerId>> = const { RefCell::new(None) };
    static VOLUME_TIMER_ID: RefCell<Option<TimerId>> = const { RefCell::new(None) };
}

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
    setup_volume_timer();
}

#[pre_upgrade]
fn pre_upgrade() {
    state::save_to_stable_memory();
}

#[post_upgrade]
fn post_upgrade() {
    state::load_from_stable_memory();
    setup_timer();
    setup_volume_timer();
}

fn setup_timer() {
    ARB_TIMER_ID.with(|id| {
        if let Some(prev) = id.borrow_mut().take() {
            ic_cdk_timers::clear_timer(prev);
        }
    });
    let interval = state::read_state(|s| s.config.arb_interval_secs).max(1);
    let new_id = ic_cdk_timers::set_timer_interval(
        std::time::Duration::from_secs(interval),
        || ic_cdk::spawn(arb::run_arb_cycle()),
    );
    ARB_TIMER_ID.with(|id| *id.borrow_mut() = Some(new_id));
}

fn setup_volume_timer() {
    VOLUME_TIMER_ID.with(|id| {
        if let Some(prev) = id.borrow_mut().take() {
            ic_cdk_timers::clear_timer(prev);
        }
    });
    let interval = state::read_state(|s| s.volume.interval_secs).max(1);
    let new_id = ic_cdk_timers::set_timer_interval(
        std::time::Duration::from_secs(interval),
        || ic_cdk::spawn(async { let _ = volume::run_volume_cycle().await; }),
    );
    VOLUME_TIMER_ID.with(|id| *id.borrow_mut() = Some(new_id));
}

fn require_admin() {
    let caller = ic_cdk::api::caller();
    if caller == Principal::anonymous() {
        ic_cdk::trap("Unauthorized: anonymous caller not allowed");
    }
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
    state::get_trades_page(offset, limit)
}

#[derive(CandidType)]
pub struct TradeSummary {
    pub total_legs: u64,
    pub total_usd_in: i64,           // 6-dec: total stablecoins spent
    pub total_usd_out: i64,          // 6-dec: total stablecoins received
    pub total_fees_usd: i64,         // 6-dec: total ledger fees
    pub net_pnl_usd: i64,           // out - in - fees
    pub leg1_count: u64,
    pub leg2_count: u64,
    pub drain_count: u64,
    pub rumi_count: u64,
    pub icpswap_count: u64,
    pub unpaired_drain_usd: i64,     // 6-dec: bought_usd from drains with no matching Leg1
    pub unpaired_drain_sold_usd: i64, // 6-dec: sold_usd (ICP cost) from those same drains
}

#[query]
fn get_trade_legs(offset: u64, limit: u64) -> Vec<TradeLeg> {
    state::get_trade_legs_page(offset, limit)
}

#[query]
fn get_summary() -> TradeSummary {
    let mut summary = TradeSummary {
        total_legs: state::trade_legs_len(),
        total_usd_in: 0,
        total_usd_out: 0,
        total_fees_usd: 0,
        net_pnl_usd: 0,
        leg1_count: 0,
        leg2_count: 0,
        drain_count: 0,
        rumi_count: 0,
        icpswap_count: 0,
        unpaired_drain_usd: 0,
        unpaired_drain_sold_usd: 0,
    };
    let mut has_pending_leg1 = false;
    state::fold_trade_legs((), |_, leg| {
        summary.total_usd_in += leg.sold_usd_value;
        summary.total_usd_out += leg.bought_usd_value;
        summary.total_fees_usd += leg.fees_usd;
        match leg.leg_type {
            state::LegType::Leg1 => {
                summary.leg1_count += 1;
                has_pending_leg1 = true;
            }
            state::LegType::Leg2 => {
                summary.leg2_count += 1;
                has_pending_leg1 = false;
            }
            state::LegType::Drain => {
                summary.drain_count += 1;
                if !has_pending_leg1 {
                    // This drain has no matching Leg1 — it's recovering
                    // pre-existing ICP, not arb profit
                    summary.unpaired_drain_usd += leg.bought_usd_value;
                    summary.unpaired_drain_sold_usd += leg.sold_usd_value;
                }
            }
            state::LegType::TopUp => {
                // Strategy S ICP inventory top-up ahead of a reverse trade.
                // Flows into the shared usd_in/usd_out/fees sums above; no
                // dedicated counter (TradeSummary is candid-frozen).
            }
        }
        if leg.dex == "Rumi" { summary.rumi_count += 1; }
        else { summary.icpswap_count += 1; }
    });
    summary.net_pnl_usd = summary.total_usd_out - summary.total_usd_in - summary.total_fees_usd;
    summary
}

#[query]
fn get_errors(offset: u64, limit: u64) -> Vec<ErrorRecord> {
    state::get_errors_page(offset, limit)
}

#[query]
fn get_activity_log(offset: u64, limit: u64) -> Vec<ActivityRecord> {
    state::get_activity_page(offset, limit)
}

#[query]
fn get_snapshots(offset: u64, limit: u64) -> Vec<CycleSnapshot> {
    state::get_snapshots_page(offset, limit)
}

// ─── Price Query ───

#[derive(CandidType)]
pub struct PriceInfo {
    pub rumi_icp_price_3usd: u64,      // 3USD per 1 ICP (8 decimals)
    pub rumi_icp_price_usd_6dec: u64,   // USD per 1 ICP (6 decimals)
    pub icpswap_icp_price_ckusdc: u64,  // ckUSDC per 1 ICP (6 decimals)
    pub virtual_price: u64,             // 3pool virtual price (8 decimals)
    pub spread_bps: i32,                // positive = Rumi cheaper
    // Strategy B
    pub icpswap_icusd_icp_price: u64,   // icUSD per 1 ICP (8 decimals), 0 if not configured
    pub strategy_b_spread_bps: i32,     // positive = icUSD pool cheaper
}

#[update]
async fn get_prices() -> PriceInfo {
    require_admin();
    let config = state::read_state(|s| s.config.clone());
    let pool_id = "fohh4-yyaaa-aaaap-qtkpa-cai_ryjl3-tyaaa-aaaaa-aaaba-cai";
    let strategy_a_fut = prices::fetch_all_prices(
        config.rumi_amm, pool_id, config.icp_ledger,
        config.rumi_3pool, config.icpswap_pool, config.icpswap_icp_is_token0,
        6, // ckUSDC = 6 decimals
    );

    let has_icusd_pool = config.icpswap_icusd_pool != Principal::anonymous();
    let icusd_resolved = state::read_state(|s| s.icusd_token_ordering_resolved);

    let icusd_price_fut = async {
        if has_icusd_pool && icusd_resolved {
            prices::fetch_icpswap_price(config.icpswap_icusd_pool, config.icpswap_icusd_icp_is_token0).await.ok()
        } else {
            None
        }
    };

    let (a_result, icusd_price) = futures::future::join(strategy_a_fut, icusd_price_fut).await;

    match a_result {
        Ok(p) => {
            // Strategy B spread: icUSD/ICP vs ckUSDC/ICP (both in 6-dec USD)
            let icusd_usd = icusd_price.map(|v| v / 100).unwrap_or(0);  // 8 dec → 6 dec
            let ckusdc_usd = p.icpswap_price_usd_6dec();
            let b_spread = if icusd_usd > 0 && ckusdc_usd > 0 {
                let (i, c) = (icusd_usd as i64, ckusdc_usd as i64);
                ((c - i) * 10_000 / i.min(c)) as i32
            } else { 0 };
            PriceInfo {
                rumi_icp_price_3usd: p.rumi_icp_price_3usd_native,
                rumi_icp_price_usd_6dec: p.rumi_price_usd_6dec(),
                icpswap_icp_price_ckusdc: p.icpswap_icp_price_ckusdc_native,
                virtual_price: p.virtual_price,
                spread_bps: p.spread_bps(),
                icpswap_icusd_icp_price: icusd_price.unwrap_or(0),
                strategy_b_spread_bps: b_spread,
            }
        },
        Err(e) => ic_cdk::trap(&format!("Price fetch failed: {}", e)),
    }
}

// ─── Admin Methods ───

#[update]
fn set_config(config: BotConfig) {
    require_admin();
    // Reject a wholesale config write that would break the ICP inventory band
    // invariant (floor >= 1 ICP, ceiling > floor) — the drain depends on it.
    // A stale cached dashboard omitting the band fields decodes to the valid
    // serde defaults, so this only trips on genuinely bad values.
    if config.icp_inventory_floor_e8s < 100_000_000
        || config.icp_inventory_ceiling_e8s <= config.icp_inventory_floor_e8s
    {
        ic_cdk::trap("set_config rejected: invalid ICP inventory band (need floor >= 1 ICP and ceiling > floor)");
    }
    state::mutate_state(|s| {
        let original_owner = s.config.owner;
        s.config = config;
        // Preserve owner — only the canister controller can change ownership
        s.config.owner = original_owner;
    });
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

/// Kill switch for the Rumi AMM (3USD/ICP) venue — pauses Strategies A/C/D/Q/R
/// (every strategy that trades against `rumi_amm`) without touching the
/// other ICPSwap/PartyDEX cross-pool strategies or the global `paused` flag.
#[update]
fn set_rumi_amm_paused(paused: bool) -> Result<(), String> {
    require_admin();
    state::mutate_state(|s| { s.config.rumi_amm_paused = paused; });
    state::log_activity("admin", &format!(
        "rumi_amm_paused set to {} by {}", paused, ic_cdk::api::caller()
    ));
    Ok(())
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

    let mut approvals: Vec<(&str, Principal, Principal)> = vec![
        ("3USD→RumiAMM", config.three_usd_ledger, config.rumi_amm),
        ("ICP→RumiAMM", config.icp_ledger, config.rumi_amm),
        ("ICP→ICPSwap", config.icp_ledger, config.icpswap_pool),
        ("ckUSDC→ICPSwap", config.ckusdc_ledger, config.icpswap_pool),
        ("icUSD→3pool", icusd, config.rumi_3pool),
        ("ckUSDT→3pool", ckusdt, config.rumi_3pool),
        ("ckUSDC→3pool", config.ckusdc_ledger, config.rumi_3pool),
    ];

    // Strategy B approvals (if icUSD pool is configured)
    if config.icpswap_icusd_pool != Principal::anonymous() {
        approvals.push(("icUSD→ICPSwap-icUSD", config.icusd_ledger, config.icpswap_icusd_pool));
        approvals.push(("ICP→ICPSwap-icUSD", config.icp_ledger, config.icpswap_icusd_pool));
    }

    // Strategy C approvals (if ckUSDT pool is configured)
    if config.icpswap_ckusdt_pool != Principal::anonymous() {
        approvals.push(("ckUSDT→ICPSwap-ckUSDT", config.ckusdt_ledger, config.icpswap_ckusdt_pool));
        approvals.push(("ICP→ICPSwap-ckUSDT", config.icp_ledger, config.icpswap_ckusdt_pool));
    }

    // Strategy G/H/I/J approvals (if 3USD ICPSwap pool is configured)
    if config.icpswap_3usd_pool != Principal::anonymous() {
        approvals.push(("3USD→ICPSwap-3USD", config.three_usd_ledger, config.icpswap_3usd_pool));
        approvals.push(("ICP→ICPSwap-3USD", config.icp_ledger, config.icpswap_3usd_pool));
    }

    // PartyDEX approvals (Strategies K/L/M/Q in PR2b, if the ckUSDC pool is configured)
    if config.partydex_ckusdc_pool != Principal::anonymous() {
        approvals.push(("ICP→PartyDEX-ckUSDC", config.icp_ledger, config.partydex_ckusdc_pool));
        approvals.push(("ckUSDC→PartyDEX-ckUSDC", config.ckusdc_ledger, config.partydex_ckusdc_pool));
    }

    // PartyDEX approvals (Strategies N/O/P/R in PR2b, if the ckUSDT pool is configured)
    if config.partydex_ckusdt_pool != Principal::anonymous() {
        approvals.push(("ICP→PartyDEX-ckUSDT", config.icp_ledger, config.partydex_ckusdt_pool));
        approvals.push(("ckUSDT→PartyDEX-ckUSDT", config.ckusdt_ledger, config.partydex_ckusdt_pool));
    }

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

    // Volume bot subaccount approvals
    let volume_approvals = vec![
        ("Vol: icUSD→ICPSwap-icUSD", config.icusd_ledger, config.icpswap_icusd_pool),
        ("Vol: ICP→ICPSwap-icUSD", config.icp_ledger, config.icpswap_icusd_pool),
        ("Vol: 3USD→ICPSwap-3USD", config.three_usd_ledger, config.icpswap_3usd_pool),
        ("Vol: ICP→ICPSwap-3USD", config.icp_ledger, config.icpswap_3usd_pool),
        ("Vol: ICP→RumiAMM", config.icp_ledger, config.rumi_amm),
        ("Vol: 3USD→RumiAMM", config.three_usd_ledger, config.rumi_amm),
        ("Vol: icUSD→3pool", config.icusd_ledger, config.rumi_3pool),
    ];
    for (label, token, spender) in volume_approvals {
        match swaps::approve_infinite_subaccount(token, spender, swaps::VOLUME_SUBACCOUNT).await {
            Ok(_) => ok.push(format!("{}: OK", label)),
            Err(e) => errors.push(format!("{}: FAILED {:?}", label, e)),
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

/// Manual recovery lever for funds stranded INSIDE a PartyDEX pool's internal
/// balance (e.g. a trade settled but the post-trade withdraw failed). Sweeps
/// the full available base and quote balances back to the bot's main account.
/// Returns `(base_withdrawn, quote_withdrawn)` in native units. The normal
/// sweep-entire-available-balance on the next successful trade auto-recovers
/// otherwise; this is the escape hatch for the "pool never trades again" case.
#[update]
async fn recover_partydex_balance(pool: Principal) -> Result<(u64, u64), String> {
    require_admin();
    let result = partydex::withdraw_all(pool).await;
    match &result {
        Ok((base_out, quote_out)) => state::log_activity("recover", &format!(
            "recover_partydex_balance({}) swept base={} quote={} by {}",
            pool, base_out, quote_out, ic_cdk::api::caller()
        )),
        Err(e) => state::log_activity("recover", &format!(
            "recover_partydex_balance({}) failed: {} (by {})", pool, e, ic_cdk::api::caller()
        )),
    }
    result
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
    require_admin();
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
    require_admin();
    if coin_index > 2 { ic_cdk::trap("Invalid coin index (0-2)"); }

    let rumi_3pool = state::read_state(|s| s.config.rumi_3pool);

    match swaps::pool_calc_redeem(rumi_3pool, lp_amount, coin_index).await {
        Ok(amount_out) => PoolQuote { estimated_output: amount_out },
        Err(e) => ic_cdk::trap(&format!("Quote failed: {}", e)),
    }
}

/// Swap one stablecoin for another directly via the 3pool.
/// coin_in/coin_out: 0=icUSD, 1=ckUSDT, 2=ckUSDC
#[update]
async fn pool_exchange(coin_in: u8, coin_out: u8, amount_in: u64, min_out: u64) {
    require_admin();
    if coin_in > 2 || coin_out > 2 || coin_in == coin_out {
        ic_cdk::trap("Invalid coin indices (0-2, must differ)");
    }

    let rumi_3pool = state::read_state(|s| s.config.rumi_3pool);
    let name_in = match coin_in { 0 => "icUSD", 1 => "ckUSDT", _ => "ckUSDC" };
    let name_out = match coin_out { 0 => "icUSD", 1 => "ckUSDT", _ => "ckUSDC" };
    match swaps::pool_swap(rumi_3pool, coin_in, coin_out, amount_in, min_out).await {
        Ok(amount_out) => {
            state::log_activity("pool_exchange", &format!(
                "{} {} → {} {} (min: {}) by {}",
                amount_in, name_in, amount_out, name_out, min_out, ic_cdk::api::caller()
            ));
        }
        Err(e) => {
            state::log_activity("pool_exchange", &format!(
                "FAILED: {} {} → {} (min: {}) — {} by {}",
                amount_in, name_in, name_out, min_out, e, ic_cdk::api::caller()
            ));
            ic_cdk::trap(&format!("Pool exchange failed: {}", e));
        }
    }
}

/// Quote a direct stablecoin-to-stablecoin swap via the 3pool.
#[update]
async fn pool_quote_exchange(coin_in: u8, coin_out: u8, amount_in: u64) -> PoolQuote {
    require_admin();
    if coin_in > 2 || coin_out > 2 || coin_in == coin_out {
        ic_cdk::trap("Invalid coin indices (0-2, must differ)");
    }

    let rumi_3pool = state::read_state(|s| s.config.rumi_3pool);
    match swaps::pool_calc_swap(rumi_3pool, coin_in, coin_out, amount_in).await {
        Ok(amount_out) => PoolQuote { estimated_output: amount_out },
        Err(e) => ic_cdk::trap(&format!("Quote failed: {}", e)),
    }
}

// ─── Rumi AMM Manual Swap ───

const RUMI_POOL_ID: &str = "fohh4-yyaaa-aaaap-qtkpa-cai_ryjl3-tyaaa-aaaaa-aaaba-cai";

/// Quote a Rumi AMM swap (ICP ↔ 3USD). token_in is the ledger of the token being sold.
#[update]
async fn rumi_quote(token_in: Principal, amount: u64) -> PoolQuote {
    require_admin();
    let rumi_amm = state::read_state(|s| s.config.rumi_amm);
    match prices::fetch_rumi_quote_for_amount(rumi_amm, RUMI_POOL_ID, token_in, amount).await {
        Ok(out) => PoolQuote { estimated_output: out },
        Err(e) => ic_cdk::trap(&format!("Rumi quote failed: {}", e)),
    }
}

/// Execute a Rumi AMM swap (ICP ↔ 3USD). token_in is the ledger of the token being sold.
#[update]
async fn rumi_manual_swap(token_in: Principal, amount: u64, min_out: u64) {
    require_admin();
    let rumi_amm = state::read_state(|s| s.config.rumi_amm);
    let caller = ic_cdk::api::caller();

    match swaps::rumi_swap(rumi_amm, RUMI_POOL_ID, token_in, amount, min_out).await {
        Ok(out) => {
            state::log_activity("swap", &format!(
                "Rumi AMM manual swap: {} in (token_in={}) → {} out (min: {}) by {}",
                amount, token_in, out, min_out, caller
            ));
        }
        Err(e) => {
            state::log_activity("swap", &format!(
                "Rumi AMM manual swap FAILED: {} in (token_in={}) — {} by {}",
                amount, token_in, e, caller
            ));
            ic_cdk::trap(&format!("Rumi swap failed: {}", e));
        }
    }
}

// ─── Volume Subaccount Manual Swaps ───

const ICP_FEE: u64 = 10_000;
const THREE_USD_FEE: u64 = 0;
const VOL_ICUSD_FEE: u64 = 100_000;

/// Swap ICP ↔ icUSD using the volume subaccount.
/// Handles the full multi-hop route: ICP ↔ 3USD (Rumi AMM) ↔ icUSD (3pool).
/// direction: "icp_to_icusd" or "icusd_to_icp"
#[update]
async fn volume_swap(icp_to_icusd: bool, amount: u64, min_out: u64) {
    require_admin();
    let (rumi_amm, icp_ledger, three_usd_ledger, icusd_ledger, rumi_3pool) = state::read_state(|s| {
        (s.config.rumi_amm, s.config.icp_ledger, s.config.three_usd_ledger,
         Principal::from_text("t6bor-paaaa-aaaap-qrd5q-cai").unwrap(),
         s.config.rumi_3pool)
    });
    let caller = ic_cdk::api::caller();

    if icp_to_icusd {
        // ICP → 3USD (Rumi) → icUSD (3pool redeem)

        // Step 1: Transfer ICP from volume subaccount to main
        if let Err(e) = swaps::transfer_from_subaccount(icp_ledger, amount, swaps::VOLUME_SUBACCOUNT).await {
            ic_cdk::trap(&format!("Volume swap: ICP transfer from subaccount failed: {:?}", e));
        }

        // Step 2: Swap ICP → 3USD on Rumi
        let swap_input = amount.saturating_sub(ICP_FEE);
        let three_usd_out = match swaps::rumi_swap(rumi_amm, RUMI_POOL_ID, icp_ledger, swap_input, 0).await {
            Ok(out) => out,
            Err(e) => {
                let recovery = swap_input.saturating_sub(ICP_FEE);
                if recovery > 0 { let _ = swaps::transfer_to_subaccount(icp_ledger, recovery, swaps::VOLUME_SUBACCOUNT).await; }
                ic_cdk::trap(&format!("Volume swap: Rumi ICP→3USD failed: {:?}", e));
            }
        };

        // Step 3: Redeem 3USD → icUSD via 3pool (coin_index 0 = icUSD)
        let icusd_out = match swaps::pool_remove_one_coin(rumi_3pool, three_usd_out, 0, min_out).await {
            Ok(out) => out,
            Err(e) => {
                // 3USD stays in default account (no subaccount support)
                ic_cdk::trap(&format!("Volume swap: 3pool redeem failed: {}", e));
            }
        };

        // Step 4: Transfer icUSD back to volume subaccount
        if icusd_out > VOL_ICUSD_FEE {
            if let Err(e) = swaps::transfer_to_subaccount(icusd_ledger, icusd_out - VOL_ICUSD_FEE, swaps::VOLUME_SUBACCOUNT).await {
                state::log_activity("volume_swap", &format!("WARNING: icUSD transfer back failed: {:?}", e));
            }
        }
        state::log_activity("volume_swap", &format!(
            "Volume swap: {} ICP → {} icUSD by {}", amount, icusd_out, caller
        ));
    } else {
        // icUSD → 3USD (3pool deposit) → ICP (Rumi)

        // Step 1: Transfer icUSD from volume subaccount to main
        if let Err(e) = swaps::transfer_from_subaccount(icusd_ledger, amount, swaps::VOLUME_SUBACCOUNT).await {
            ic_cdk::trap(&format!("Volume swap: icUSD transfer from subaccount failed: {:?}", e));
        }

        // Step 2: Deposit icUSD → 3USD via 3pool (coin_index 0 = icUSD)
        let deposit_amount = amount.saturating_sub(VOL_ICUSD_FEE);
        let mut amounts = vec![Nat::from(0u64), Nat::from(0u64), Nat::from(0u64)];
        amounts[0] = Nat::from(deposit_amount);
        let three_usd_out = match swaps::pool_add_liquidity(rumi_3pool, amounts, 0).await {
            Ok(lp) => lp,
            Err(e) => {
                let recovery = deposit_amount.saturating_sub(VOL_ICUSD_FEE);
                if recovery > 0 { let _ = swaps::transfer_to_subaccount(icusd_ledger, recovery, swaps::VOLUME_SUBACCOUNT).await; }
                ic_cdk::trap(&format!("Volume swap: 3pool deposit failed: {}", e));
            }
        };

        // Step 3: Swap 3USD → ICP on Rumi
        let icp_out = match swaps::rumi_swap(rumi_amm, RUMI_POOL_ID, three_usd_ledger, three_usd_out, min_out).await {
            Ok(out) => out,
            Err(e) => {
                // 3USD stays in default account (no subaccount support)
                ic_cdk::trap(&format!("Volume swap: Rumi 3USD→ICP failed: {:?}", e));
            }
        };

        // Step 4: Transfer ICP back to volume subaccount
        if icp_out > ICP_FEE {
            if let Err(e) = swaps::transfer_to_subaccount(icp_ledger, icp_out - ICP_FEE, swaps::VOLUME_SUBACCOUNT).await {
                state::log_activity("volume_swap", &format!("WARNING: ICP transfer back failed: {:?}", e));
            }
        }
        state::log_activity("volume_swap", &format!(
            "Volume swap: {} icUSD → {} ICP by {}", amount, icp_out, caller
        ));
    }
}

/// One-time backfill: append historical trade legs to the log.
/// NOTE: Post stable-memory migration, this now APPENDS (previously prepended).
/// Chronological ordering of historical entries is not preserved.
#[update]
fn backfill_trade_legs(legs: Vec<TradeLeg>) {
    require_admin();
    let count = state::append_trade_legs_batch(legs);
    state::log_activity("admin", &format!("Backfilled {} historical trade legs", count));
}

#[update]
async fn manual_arb_cycle() {
    require_admin();
    state::log_activity("admin", &format!("Manual arb cycle triggered by {}", ic_cdk::api::caller()));
    arb::run_arb_cycle().await;
}

#[update]
async fn execute_strategy_a() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy A by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("A").await;
}

#[update]
async fn execute_strategy_b() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy B by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("B").await;
}

#[update]
async fn execute_strategy_c() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy C by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("C").await;
}

#[update]
async fn execute_strategy_d() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy D by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("D").await;
}

#[update]
async fn execute_strategy_f() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy F by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("F").await;
}

#[update]
async fn execute_strategy_k() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy K by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("K").await;
}

#[update]
async fn execute_strategy_l() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy L by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("L").await;
}

#[update]
async fn execute_strategy_m() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy M by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("M").await;
}

#[update]
async fn execute_strategy_n() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy N by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("N").await;
}

#[update]
async fn execute_strategy_o() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy O by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("O").await;
}

#[update]
async fn execute_strategy_p() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy P by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("P").await;
}

#[update]
async fn execute_strategy_q() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy Q by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("Q").await;
}

#[update]
async fn execute_strategy_r() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy R by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("R").await;
}

#[update]
async fn dry_run_arb_cycle() -> arb::DryRunResult {
    require_admin();

    // Ensure token ordering is resolved first (Strategy A)
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

    // Resolve Strategy B token ordering if needed
    let (icusd_resolved, has_icusd_pool) = state::read_state(|s| {
        (s.icusd_token_ordering_resolved, s.config.icpswap_icusd_pool != Principal::anonymous())
    });
    if has_icusd_pool && !icusd_resolved {
        let (icusd_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_icusd_pool, s.config.icp_ledger));
        if let Ok(icp_is_token0) = prices::fetch_icpswap_token_ordering(icusd_pool, icp_ledger).await {
            state::mutate_state(|s| {
                s.config.icpswap_icusd_icp_is_token0 = icp_is_token0;
                s.icusd_token_ordering_resolved = true;
            });
        }
    }

    let config = state::read_state(|s| s.config.clone());
    let target_a = arb::IcpswapTarget {
        pool: config.icpswap_pool,
        icp_is_token0: config.icpswap_icp_is_token0,
        label: "ICPSwap",
        strategy_tag: "A",
        stable_token_name: "ckUSDC",
        stable_fee: 10_000,
        stable_ledger: config.ckusdc_ledger,
        pool_enum: state::Pool::IcpswapCkusdc,
        stable_decimals: 6,
        uses_vp: false,
        venue: state::Venue::Icpswap,
        fee_pips: 0,
    };
    match arb::compute_optimal_trade(&config, &target_a).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("Computation failed: {}", e);
            result
        }
    }
}

#[update]
async fn dry_run_strategy_c() -> arb::DryRunResult {
    require_admin();

    let (ckusdt_resolved, has_ckusdt_pool) = state::read_state(|s| {
        (s.ckusdt_token_ordering_resolved, s.config.icpswap_ckusdt_pool != Principal::anonymous())
    });
    if !has_ckusdt_pool {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy C not configured (no ckUSDT pool)".to_string();
        return result;
    }
    if !ckusdt_resolved {
        let (ckusdt_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_ckusdt_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(ckusdt_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_ckusdt_icp_is_token0 = icp_is_token0;
                    s.ckusdt_token_ordering_resolved = true;
                });
            }
            Err(e) => {
                let mut result = arb::DryRunResult::default();
                result.message = format!("Failed to resolve ckUSDT pool token ordering: {}", e);
                return result;
            }
        }
    }
    let config = state::read_state(|s| s.config.clone());
    let target_c = arb::IcpswapTarget {
        pool: config.icpswap_ckusdt_pool,
        icp_is_token0: config.icpswap_ckusdt_icp_is_token0,
        label: "ICPSwap-ckUSDT",
        strategy_tag: "C",
        stable_token_name: "ckUSDT",
        stable_fee: 10_000,
        stable_ledger: config.ckusdt_ledger,
        pool_enum: state::Pool::IcpswapCkusdt,
        stable_decimals: 6,
        uses_vp: false,
        venue: state::Venue::Icpswap,
        fee_pips: 0,
    };
    match arb::compute_optimal_trade(&config, &target_c).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("[C] Computation failed: {}", e);
            result
        }
    }
}

#[update]
async fn dry_run_strategy_d() -> arb::DryRunResult {
    require_admin();

    let (icusd_resolved, has_icusd_pool) = state::read_state(|s| {
        (s.icusd_token_ordering_resolved, s.config.icpswap_icusd_pool != Principal::anonymous())
    });
    if !has_icusd_pool {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy D not configured (no icUSD pool)".to_string();
        return result;
    }
    if !icusd_resolved {
        let (icusd_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_icusd_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(icusd_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_icusd_icp_is_token0 = icp_is_token0;
                    s.icusd_token_ordering_resolved = true;
                });
            }
            Err(e) => {
                let mut result = arb::DryRunResult::default();
                result.message = format!("Failed to resolve icUSD pool token ordering: {}", e);
                return result;
            }
        }
    }
    let config = state::read_state(|s| s.config.clone());
    let target_d = arb::IcpswapTarget {
        pool: config.icpswap_icusd_pool,
        icp_is_token0: config.icpswap_icusd_icp_is_token0,
        label: "ICPSwap-icUSD",
        strategy_tag: "D",
        stable_token_name: "icUSD",
        stable_fee: 100_000,
        stable_ledger: config.icusd_ledger,
        pool_enum: state::Pool::IcpswapIcusd,
        stable_decimals: 8,
        uses_vp: false,
        venue: state::Venue::Icpswap,
        fee_pips: 0,
    };
    match arb::compute_optimal_trade(&config, &target_d).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("[D] Computation failed: {}", e);
            result
        }
    }
}

#[update]
async fn dry_run_strategy_b() -> arb::DryRunResult {
    require_admin();

    // Resolve both pool orderings
    let resolved = state::read_state(|s| s.token_ordering_resolved);
    if !resolved {
        let (icpswap_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_pool, s.config.icp_ledger));
        if let Ok(icp_is_token0) = prices::fetch_icpswap_token_ordering(icpswap_pool, icp_ledger).await {
            state::mutate_state(|s| {
                s.config.icpswap_icp_is_token0 = icp_is_token0;
                s.token_ordering_resolved = true;
            });
        }
    }
    let (icusd_resolved, has_icusd_pool) = state::read_state(|s| {
        (s.icusd_token_ordering_resolved, s.config.icpswap_icusd_pool != Principal::anonymous())
    });
    if !has_icusd_pool {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy B not configured (no icUSD pool)".to_string();
        return result;
    }
    if !icusd_resolved {
        let (icusd_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_icusd_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(icusd_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_icusd_icp_is_token0 = icp_is_token0;
                    s.icusd_token_ordering_resolved = true;
                });
            }
            Err(e) => {
                let mut result = arb::DryRunResult::default();
                result.message = format!("Failed to resolve icUSD pool token ordering: {}", e);
                return result;
            }
        }
    }

    let config = state::read_state(|s| s.config.clone());
    let target = build_cross_b(&config);
    match arb::compute_optimal_cross_pool_trade(&config, &target).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("[B] Computation failed: {}", e);
            result
        }
    }
}

#[update]
async fn dry_run_strategy_f() -> arb::DryRunResult {
    require_admin();

    // Resolve ckUSDC/ICP pool ordering (needed as reference)
    let resolved = state::read_state(|s| s.token_ordering_resolved);
    if !resolved {
        let (icpswap_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_pool, s.config.icp_ledger));
        if let Ok(icp_is_token0) = prices::fetch_icpswap_token_ordering(icpswap_pool, icp_ledger).await {
            state::mutate_state(|s| {
                s.config.icpswap_icp_is_token0 = icp_is_token0;
                s.token_ordering_resolved = true;
            });
        }
    }
    // Resolve icUSD pool ordering
    let (icusd_resolved, has_icusd_pool) = state::read_state(|s| {
        (s.icusd_token_ordering_resolved, s.config.icpswap_icusd_pool != Principal::anonymous())
    });
    if !has_icusd_pool {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy F not configured (no icUSD pool)".to_string();
        return result;
    }
    if !icusd_resolved {
        let (icusd_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_icusd_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(icusd_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_icusd_icp_is_token0 = icp_is_token0;
                    s.icusd_token_ordering_resolved = true;
                });
            }
            Err(e) => {
                let mut result = arb::DryRunResult::default();
                result.message = format!("Failed to resolve icUSD pool token ordering: {}", e);
                return result;
            }
        }
    }
    // Resolve ckUSDT pool ordering
    let (ckusdt_resolved, has_ckusdt_pool) = state::read_state(|s| {
        (s.ckusdt_token_ordering_resolved, s.config.icpswap_ckusdt_pool != Principal::anonymous())
    });
    if !has_ckusdt_pool {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy F not configured (no ckUSDT pool)".to_string();
        return result;
    }
    if !ckusdt_resolved {
        let (ckusdt_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_ckusdt_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(ckusdt_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_ckusdt_icp_is_token0 = icp_is_token0;
                    s.ckusdt_token_ordering_resolved = true;
                });
            }
            Err(e) => {
                let mut result = arb::DryRunResult::default();
                result.message = format!("Failed to resolve ckUSDT pool token ordering: {}", e);
                return result;
            }
        }
    }

    let config = state::read_state(|s| s.config.clone());
    let target = build_cross_f(&config);
    match arb::compute_optimal_cross_pool_trade(&config, &target).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("[F] Computation failed: {}", e);
            result
        }
    }
}

#[update]
async fn dry_run_strategy_k() -> arb::DryRunResult {
    require_admin();

    // K's ICPSwap side reuses the main ckUSDC/ICP pool ordering.
    let resolved = state::read_state(|s| s.token_ordering_resolved);
    if !resolved {
        let (icpswap_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_pool, s.config.icp_ledger));
        if let Ok(icp_is_token0) = prices::fetch_icpswap_token_ordering(icpswap_pool, icp_ledger).await {
            state::mutate_state(|s| {
                s.config.icpswap_icp_is_token0 = icp_is_token0;
                s.token_ordering_resolved = true;
            });
        }
    }
    let config = state::read_state(|s| s.config.clone());
    if config.partydex_ckusdc_pool == Principal::anonymous() {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy K not configured (no PartyDEX ckUSDC pool)".to_string();
        return result;
    }
    let target = build_cross_k(&config);
    match arb::compute_optimal_cross_pool_trade(&config, &target).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("[K] Computation failed: {}", e);
            result
        }
    }
}

#[update]
async fn dry_run_strategy_l() -> arb::DryRunResult {
    require_admin();

    let (ckusdt_resolved, has_ckusdt_pool) = state::read_state(|s| {
        (s.ckusdt_token_ordering_resolved, s.config.icpswap_ckusdt_pool != Principal::anonymous())
    });
    if !has_ckusdt_pool {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy L not configured (no ckUSDT pool)".to_string();
        return result;
    }
    if !ckusdt_resolved {
        let (ckusdt_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_ckusdt_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(ckusdt_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_ckusdt_icp_is_token0 = icp_is_token0;
                    s.ckusdt_token_ordering_resolved = true;
                });
            }
            Err(e) => {
                let mut result = arb::DryRunResult::default();
                result.message = format!("Failed to resolve ckUSDT pool token ordering: {}", e);
                return result;
            }
        }
    }
    let config = state::read_state(|s| s.config.clone());
    if config.partydex_ckusdc_pool == Principal::anonymous() {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy L not configured (no PartyDEX ckUSDC pool)".to_string();
        return result;
    }
    let target = build_cross_l(&config);
    match arb::compute_optimal_cross_pool_trade(&config, &target).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("[L] Computation failed: {}", e);
            result
        }
    }
}

#[update]
async fn dry_run_strategy_m() -> arb::DryRunResult {
    require_admin();

    let (icusd_resolved, has_icusd_pool) = state::read_state(|s| {
        (s.icusd_token_ordering_resolved, s.config.icpswap_icusd_pool != Principal::anonymous())
    });
    if !has_icusd_pool {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy M not configured (no icUSD pool)".to_string();
        return result;
    }
    if !icusd_resolved {
        let (icusd_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_icusd_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(icusd_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_icusd_icp_is_token0 = icp_is_token0;
                    s.icusd_token_ordering_resolved = true;
                });
            }
            Err(e) => {
                let mut result = arb::DryRunResult::default();
                result.message = format!("Failed to resolve icUSD pool token ordering: {}", e);
                return result;
            }
        }
    }
    let config = state::read_state(|s| s.config.clone());
    if config.partydex_ckusdc_pool == Principal::anonymous() {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy M not configured (no PartyDEX ckUSDC pool)".to_string();
        return result;
    }
    let target = build_cross_m(&config);
    match arb::compute_optimal_cross_pool_trade(&config, &target).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("[M] Computation failed: {}", e);
            result
        }
    }
}

#[update]
async fn dry_run_strategy_n() -> arb::DryRunResult {
    require_admin();

    // N's ICPSwap side reuses the main ckUSDC/ICP pool ordering.
    let resolved = state::read_state(|s| s.token_ordering_resolved);
    if !resolved {
        let (icpswap_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_pool, s.config.icp_ledger));
        if let Ok(icp_is_token0) = prices::fetch_icpswap_token_ordering(icpswap_pool, icp_ledger).await {
            state::mutate_state(|s| {
                s.config.icpswap_icp_is_token0 = icp_is_token0;
                s.token_ordering_resolved = true;
            });
        }
    }
    let config = state::read_state(|s| s.config.clone());
    if config.partydex_ckusdt_pool == Principal::anonymous() {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy N not configured (no PartyDEX ckUSDT pool)".to_string();
        return result;
    }
    let target = build_cross_n(&config);
    match arb::compute_optimal_cross_pool_trade(&config, &target).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("[N] Computation failed: {}", e);
            result
        }
    }
}

#[update]
async fn dry_run_strategy_o() -> arb::DryRunResult {
    require_admin();

    let (ckusdt_resolved, has_ckusdt_pool) = state::read_state(|s| {
        (s.ckusdt_token_ordering_resolved, s.config.icpswap_ckusdt_pool != Principal::anonymous())
    });
    if !has_ckusdt_pool {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy O not configured (no ckUSDT pool)".to_string();
        return result;
    }
    if !ckusdt_resolved {
        let (ckusdt_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_ckusdt_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(ckusdt_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_ckusdt_icp_is_token0 = icp_is_token0;
                    s.ckusdt_token_ordering_resolved = true;
                });
            }
            Err(e) => {
                let mut result = arb::DryRunResult::default();
                result.message = format!("Failed to resolve ckUSDT pool token ordering: {}", e);
                return result;
            }
        }
    }
    let config = state::read_state(|s| s.config.clone());
    if config.partydex_ckusdt_pool == Principal::anonymous() {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy O not configured (no PartyDEX ckUSDT pool)".to_string();
        return result;
    }
    let target = build_cross_o(&config);
    match arb::compute_optimal_cross_pool_trade(&config, &target).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("[O] Computation failed: {}", e);
            result
        }
    }
}

#[update]
async fn dry_run_strategy_p() -> arb::DryRunResult {
    require_admin();

    let (icusd_resolved, has_icusd_pool) = state::read_state(|s| {
        (s.icusd_token_ordering_resolved, s.config.icpswap_icusd_pool != Principal::anonymous())
    });
    if !has_icusd_pool {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy P not configured (no icUSD pool)".to_string();
        return result;
    }
    if !icusd_resolved {
        let (icusd_pool, icp_ledger) = state::read_state(|s| (s.config.icpswap_icusd_pool, s.config.icp_ledger));
        match prices::fetch_icpswap_token_ordering(icusd_pool, icp_ledger).await {
            Ok(icp_is_token0) => {
                state::mutate_state(|s| {
                    s.config.icpswap_icusd_icp_is_token0 = icp_is_token0;
                    s.icusd_token_ordering_resolved = true;
                });
            }
            Err(e) => {
                let mut result = arb::DryRunResult::default();
                result.message = format!("Failed to resolve icUSD pool token ordering: {}", e);
                return result;
            }
        }
    }
    let config = state::read_state(|s| s.config.clone());
    if config.partydex_ckusdt_pool == Principal::anonymous() {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy P not configured (no PartyDEX ckUSDT pool)".to_string();
        return result;
    }
    let target = build_cross_p(&config);
    match arb::compute_optimal_cross_pool_trade(&config, &target).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("[P] Computation failed: {}", e);
            result
        }
    }
}

#[update]
async fn dry_run_strategy_q() -> arb::DryRunResult {
    require_admin();

    let config = state::read_state(|s| s.config.clone());
    if config.partydex_ckusdc_pool == Principal::anonymous() {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy Q not configured (no PartyDEX ckUSDC pool)".to_string();
        return result;
    }
    let target = arb::IcpswapTarget {
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
    match arb::compute_optimal_trade(&config, &target).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("[Q] Computation failed: {}", e);
            result
        }
    }
}

#[update]
async fn dry_run_strategy_r() -> arb::DryRunResult {
    require_admin();

    let config = state::read_state(|s| s.config.clone());
    if config.partydex_ckusdt_pool == Principal::anonymous() {
        let mut result = arb::DryRunResult::default();
        result.message = "Strategy R not configured (no PartyDEX ckUSDT pool)".to_string();
        return result;
    }
    let target = arb::IcpswapTarget {
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
    match arb::compute_optimal_trade(&config, &target).await {
        Ok(dr) => dr,
        Err(e) => {
            let mut result = arb::DryRunResult::default();
            result.message = format!("[R] Computation failed: {}", e);
            result
        }
    }
}

// ─── Cross-pool target builders ───

const ICUSD_FEE: u64 = 100_000;
const CKUSDC_FEE: u64 = 10_000;
const CKUSDT_FEE: u64 = 10_000;

fn build_cross_b(config: &BotConfig) -> arb::CrossPoolTarget {
    arb::CrossPoolTarget {
        strategy_tag: "B",
        buy_side: arb::CrossPoolSide {
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
        },
        sell_side: arb::CrossPoolSide {
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
        },
    }
}

fn build_cross_f(config: &BotConfig) -> arb::CrossPoolTarget {
    arb::CrossPoolTarget {
        strategy_tag: "F",
        buy_side: arb::CrossPoolSide {
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
        },
        sell_side: arb::CrossPoolSide {
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
        },
    }
}

/// PartyDEX ckUSDC pool side, shared by builders K/L/M. icp_is_token0 is
/// irrelevant for PartyDex (ICP is always `base`), set true per plan.
fn partydex_ckusdc_side(config: &BotConfig) -> arb::CrossPoolSide {
    arb::CrossPoolSide {
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
    }
}

/// PartyDEX ckUSDT pool side, shared by builders N/O/P.
fn partydex_ckusdt_side(config: &BotConfig) -> arb::CrossPoolSide {
    arb::CrossPoolSide {
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
    }
}

fn build_cross_k(config: &BotConfig) -> arb::CrossPoolTarget {
    arb::CrossPoolTarget {
        strategy_tag: "K",
        buy_side: partydex_ckusdc_side(config),
        sell_side: arb::CrossPoolSide {
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
        },
    }
}

fn build_cross_l(config: &BotConfig) -> arb::CrossPoolTarget {
    arb::CrossPoolTarget {
        strategy_tag: "L",
        buy_side: partydex_ckusdc_side(config),
        sell_side: arb::CrossPoolSide {
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
        },
    }
}

fn build_cross_m(config: &BotConfig) -> arb::CrossPoolTarget {
    arb::CrossPoolTarget {
        strategy_tag: "M",
        buy_side: partydex_ckusdc_side(config),
        sell_side: arb::CrossPoolSide {
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
        },
    }
}

fn build_cross_n(config: &BotConfig) -> arb::CrossPoolTarget {
    arb::CrossPoolTarget {
        strategy_tag: "N",
        buy_side: partydex_ckusdt_side(config),
        sell_side: arb::CrossPoolSide {
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
        },
    }
}

fn build_cross_o(config: &BotConfig) -> arb::CrossPoolTarget {
    arb::CrossPoolTarget {
        strategy_tag: "O",
        buy_side: partydex_ckusdt_side(config),
        sell_side: arb::CrossPoolSide {
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
        },
    }
}

fn build_cross_p(config: &BotConfig) -> arb::CrossPoolTarget {
    arb::CrossPoolTarget {
        strategy_tag: "P",
        buy_side: partydex_ckusdt_side(config),
        sell_side: arb::CrossPoolSide {
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
        },
    }
}

// ─── Volume Bot Admin ───

#[update]
async fn set_volume_config(pool: state::VolumePool, new_config: state::VolumePoolConfig) -> Result<(), String> {
    require_admin();
    if new_config.trade_size_usd == 0 {
        return Err("trade_size_usd must be >= 1".to_string());
    }
    if new_config.trade_variance_pct > 50 {
        return Err("trade_variance_pct must be <= 50".to_string());
    }
    if new_config.trade_size_usd > new_config.daily_cost_cap_usd && new_config.daily_cost_cap_usd > 0 {
        state::log_activity("volume", &format!(
            "warning: {:?} trade_size_usd ({}) > daily_cost_cap_usd ({})",
            pool, new_config.trade_size_usd, new_config.daily_cost_cap_usd
        ));
    }
    state::mutate_state(|s| {
        match pool {
            state::VolumePool::IcusdIcp => s.volume.icusd_icp = new_config,
            state::VolumePool::ThreeUsdIcp => s.volume.three_usd_icp = new_config,
        }
    });
    Ok(())
}

#[update]
fn set_volume_global(interval_secs: u64, rebalance_drift_pct: u64) {
    require_admin();
    state::mutate_state(|s| {
        s.volume.interval_secs = interval_secs;
        s.volume.rebalance_drift_pct = rebalance_drift_pct;
    });
}

#[update]
fn pause_volume() {
    require_admin();
    state::mutate_state(|s| s.volume.volume_paused = true);
}

#[update]
fn resume_volume() {
    require_admin();
    state::mutate_state(|s| s.volume.volume_paused = false);
}

#[update]
async fn fund_volume_subaccount(token_ledger: Principal, amount: u64) -> Result<(), String> {
    require_admin();
    let three_usd = state::read_state(|s| s.config.three_usd_ledger);
    if token_ledger == three_usd {
        // 3USD ledger ignores subaccounts — funds are already in default account
        return Ok(());
    }
    swaps::transfer_to_subaccount(token_ledger, amount, swaps::VOLUME_SUBACCOUNT)
        .await
        .map(|_| ())
        .map_err(|e| format!("Failed to fund volume subaccount: {:?}", e))
}

#[update]
async fn withdraw_volume_subaccount(token_ledger: Principal, amount: u64) -> Result<(), String> {
    require_admin();
    let three_usd = state::read_state(|s| s.config.three_usd_ledger);
    if token_ledger == three_usd {
        // 3USD ledger ignores subaccounts — funds are already in default account
        return Ok(());
    }
    swaps::transfer_from_subaccount(token_ledger, amount, swaps::VOLUME_SUBACCOUNT)
        .await
        .map(|_| ())
        .map_err(|e| format!("Failed to withdraw from volume subaccount: {:?}", e))
}

#[update]
async fn trigger_volume_cycle() -> String {
    require_admin();
    let outcomes = volume::run_volume_cycle().await;
    if outcomes.is_empty() {
        "cycle ran, no outcomes".to_string()
    } else {
        outcomes.join("; ")
    }
}

#[update]
fn set_arb_interval_secs(interval_secs: u64) -> Result<(), String> {
    require_admin();
    if interval_secs < 30 {
        return Err("interval_secs must be >= 30".to_string());
    }
    if interval_secs > 86_400 {
        return Err("interval_secs must be <= 86400 (1 day)".to_string());
    }
    state::mutate_state(|s| { s.config.arb_interval_secs = interval_secs; });
    setup_timer();
    state::log_activity("admin", &format!("arb_interval_secs set to {}", interval_secs));
    Ok(())
}

/// Sets the ICP inventory band (e8s) the drain uses in place of the old fixed
/// `ICP_RESERVE`. Floor: minimum working balance always left behind. Ceiling:
/// steady-state skim threshold. Single method so the pair can't pass through
/// an invalid intermediate state (e.g. floor temporarily above ceiling).
#[update]
fn set_icp_inventory_band(floor_e8s: u64, ceiling_e8s: u64) -> Result<(), String> {
    require_admin();
    if floor_e8s < 100_000_000 {
        return Err("floor must be >= 1 ICP".into());
    }
    if ceiling_e8s <= floor_e8s {
        return Err("ceiling must be > floor".into());
    }
    state::mutate_state(|s| {
        s.config.icp_inventory_floor_e8s = floor_e8s;
        s.config.icp_inventory_ceiling_e8s = ceiling_e8s;
    });
    state::log_activity("admin", &format!("icp inventory band set to [{}, {}] e8s", floor_e8s, ceiling_e8s));
    Ok(())
}

/// Sets both Strategy S pool principals in one call. Resets the resolved-
/// ordering flag for any pool whose principal actually changes, so a
/// re-pointed pool gets its token ordering re-probed on the next cycle
/// instead of running with stale `icp_is_token0`/`icusd_is_token0` bits.
#[update]
fn set_bob_pools(bob_icp_pool: Principal, icusd_bob_pool: Principal) -> Result<(), String> {
    require_admin();
    state::mutate_state(|s| {
        if s.config.icpswap_bob_icp_pool != bob_icp_pool {
            s.config.icpswap_bob_icp_pool = bob_icp_pool;
            s.bob_icp_ordering_resolved = false;
        }
        if s.config.icpswap_icusd_bob_pool != icusd_bob_pool {
            s.config.icpswap_icusd_bob_pool = icusd_bob_pool;
            s.icusd_bob_ordering_resolved = false;
        }
    });
    state::log_activity("admin", &format!(
        "bob pools set: bob/icp={}, icusd/bob={}", bob_icp_pool, icusd_bob_pool
    ));
    Ok(())
}

/// Sets Strategy S's sizing/gating knobs. Single method so the pair can't
/// pass through an invalid intermediate state.
#[update]
fn set_bob_params(max_trade_size_usd: u64, min_spread_bps: u64) -> Result<(), String> {
    require_admin();
    if max_trade_size_usd == 0 {
        return Err("max_trade_size_usd must be > 0".to_string());
    }
    if min_spread_bps == 0 || min_spread_bps > 10_000 {
        return Err("min_spread_bps must be > 0 and <= 10000 (100%)".to_string());
    }
    state::mutate_state(|s| {
        s.config.bob_max_trade_size_usd = max_trade_size_usd;
        s.config.bob_min_spread_bps = min_spread_bps;
    });
    state::log_activity("admin", &format!(
        "bob params set: max_trade_size_usd={}, min_spread_bps={}", max_trade_size_usd, min_spread_bps
    ));
    Ok(())
}

/// Execution kill switch for Strategy S. Dry-run evaluation + dashboard
/// surfacing run regardless of this flag once both BOB pools are configured;
/// this only gates whether Strategy S can actually execute trades.
#[update]
fn set_bob_execution_enabled(enabled: bool) -> Result<(), String> {
    require_admin();
    state::mutate_state(|s| { s.config.bob_execution_enabled = enabled; });
    state::log_activity("admin", &format!(
        "bob_execution_enabled set to {} by {}", enabled, ic_cdk::api::caller()
    ));
    Ok(())
}

#[update]
fn set_slippage_bps(slippage_bps: u64) -> Result<(), String> {
    require_admin();
    if slippage_bps > 10_000 {
        return Err("slippage_bps must be <= 10000 (100%)".to_string());
    }
    state::mutate_state(|s| { s.config.slippage_bps = slippage_bps; });
    state::log_activity("admin", &format!("slippage_bps set to {}", slippage_bps));
    Ok(())
}

#[update]
async fn get_bot_health() -> state::BotHealthReport {
    require_admin();

    let (bot_config, volume_config, pending_exit, arb_paused, stranded) = state::read_state(|s| (
        s.config.clone(),
        s.volume.clone(),
        s.pending_exit.clone(),
        s.config.paused,
        s.volume_stranded_icp,
    ));

    let arb_in_progress = arb::is_cycle_in_progress();
    let volume_in_progress = volume::is_volume_cycle_in_progress();

    let pools_to_check = [
        state::VolumePool::IcusdIcp,
        state::VolumePool::ThreeUsdIcp,
    ];

    let mut pool_reports: Vec<state::PoolHealth> = Vec::new();

    for pool in pools_to_check {
        let (pool_config, pool_state) = match &pool {
            state::VolumePool::IcusdIcp => (volume_config.icusd_icp.clone(), volume_config.icusd_icp_state.clone()),
            state::VolumePool::ThreeUsdIcp => (volume_config.three_usd_icp.clone(), volume_config.three_usd_icp_state.clone()),
        };

        let (icpswap_pool, icp_is_token0) = match &pool {
            state::VolumePool::IcusdIcp => (bot_config.icpswap_icusd_pool, bot_config.icpswap_icusd_icp_is_token0),
            state::VolumePool::ThreeUsdIcp => (bot_config.icpswap_3usd_pool, bot_config.icpswap_3usd_icp_is_token0),
        };

        let current_price = prices::fetch_icpswap_price(icpswap_pool, icp_is_token0).await.ok();

        let input_token = match &pool_state.next_direction {
            state::VolumeDirection::BuyIcp => match &pool {
                state::VolumePool::IcusdIcp => bot_config.icusd_ledger,
                state::VolumePool::ThreeUsdIcp => bot_config.three_usd_ledger,
            },
            state::VolumeDirection::SellIcp => bot_config.icp_ledger,
        };

        // 3USD ledger ignores subaccounts — check default account
        let input_balance = if input_token == bot_config.three_usd_ledger {
            swaps::icrc1_balance_of_default(input_token).await.ok()
        } else {
            swaps::icrc1_balance_of_subaccount(input_token, swaps::VOLUME_SUBACCOUNT).await.ok()
        };

        let min_required_native: Option<u64> = match (&pool_state.next_direction, &pool, current_price) {
            (state::VolumeDirection::BuyIcp, state::VolumePool::IcusdIcp, _) => Some(pool_config.trade_size_usd * 100),
            (state::VolumeDirection::BuyIcp, state::VolumePool::ThreeUsdIcp, _) => Some(pool_config.trade_size_usd * 100), // 3USD is 8 decimals
            (state::VolumeDirection::SellIcp, _, Some(p)) if p > 0 => {
                let stable_native = match &pool {
                    state::VolumePool::IcusdIcp => pool_config.trade_size_usd * 100,
                    state::VolumePool::ThreeUsdIcp => pool_config.trade_size_usd * 100, // 3USD is 8 decimals
                };
                Some((stable_native as u128 * 100_000_000u128 / p as u128) as u64)
            }
            _ => None,
        };

        let skip_reason: Option<String> = if volume_config.volume_paused {
            Some("volume_paused=true".to_string())
        } else if !pool_config.enabled {
            Some("pool disabled".to_string())
        } else if pool_state.daily_cost_usd >= pool_config.daily_cost_cap_usd as i64 {
            Some(format!("daily cost cap hit: {} >= {}", pool_state.daily_cost_usd, pool_config.daily_cost_cap_usd))
        } else if current_price.is_none() {
            Some("price fetch failed".to_string())
        } else if input_balance.is_none() {
            Some("balance fetch failed".to_string())
        } else if min_required_native.is_none() {
            Some("zero price (cannot compute min_native for SellIcp)".to_string())
        } else if input_balance.unwrap() < min_required_native.unwrap() {
            Some(format!(
                "insufficient balance: {} < {} (need {:?} of {})",
                input_balance.unwrap(), min_required_native.unwrap(), pool_state.next_direction, input_token
            ))
        } else {
            None
        };

        pool_reports.push(state::PoolHealth {
            pool: pool.clone(),
            enabled: pool_config.enabled,
            trade_size_usd: pool_config.trade_size_usd,
            daily_cost_usd: pool_state.daily_cost_usd,
            daily_cost_cap_usd: pool_config.daily_cost_cap_usd,
            last_price: pool_state.last_price,
            current_price,
            next_direction: pool_state.next_direction.clone(),
            input_balance,
            min_required_native,
            skip_reason,
        });
    }

    state::BotHealthReport {
        arb_cycle_in_progress: arb_in_progress,
        volume_cycle_in_progress: volume_in_progress,
        volume_paused: volume_config.volume_paused,
        arb_paused,
        volume_stranded_icp: stranded,
        pending_exit,
        slippage_bps: bot_config.slippage_bps,
        pools: pool_reports,
    }
}

#[update]
async fn trigger_volume_rebalance() {
    require_admin();
    let config = state::read_state(|s| s.config.clone());
    volume::run_rebalance(&config).await;
}

// ─── Volume Bot Queries ───

#[query]
fn get_volume_stats() -> state::VolumeStats {
    state::read_state(|s| state::VolumeStats {
        volume_paused: s.volume.volume_paused,
        interval_secs: s.volume.interval_secs,
        daily_spend_usd: s.volume.daily_spend_usd,
        daily_cost_cap_usd_icusd: s.volume.icusd_icp.daily_cost_cap_usd,
        daily_cost_cap_usd_3usd: s.volume.three_usd_icp.daily_cost_cap_usd,
        icusd_icp: state::VolumePoolStatus {
            config: s.volume.icusd_icp.clone(),
            state: s.volume.icusd_icp_state.clone(),
        },
        three_usd_icp: state::VolumePoolStatus {
            config: s.volume.three_usd_icp.clone(),
            state: s.volume.three_usd_icp_state.clone(),
        },
        total_trade_count: s.volume.icusd_icp_state.trade_count + s.volume.three_usd_icp_state.trade_count,
    })
}

#[query]
fn get_volume_trades(offset: u64, limit: u64) -> Vec<state::VolumeTradeLeg> {
    state::get_volume_trades_page(offset, limit)
}

// ─── Cycles ───

#[query]
fn cycles_balance() -> u128 {
    ic_cdk::api::canister_balance128()
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

// ─── Candid drift guard ───
//
// The canister's Candid interface is hand-maintained across three sources
// (this Rust file, `arb_bot.did`, and the `dashboard.html` IDL block). A
// mismatch produces a silent decode trap on mainnet that nothing catches at
// build time. `candid::export_service!` generates a candid service from the
// actual `#[update]`/`#[query]` signatures above; the integration test in
// `tests/candid.rs` asserts it is structurally equal to the committed
// `arb_bot.did`, catching Rust↔.did drift automatically. The dashboard IDL
// (which export_service! cannot see) is covered by `scripts/check-candid.sh`.
// Run everything with: `scripts/check-candid.sh`.
//
// Not compiled into the wasm canister — it is only referenced by the test.
pub fn generated_candid_interface() -> String {
    candid::export_service!();
    __export_service()
}
