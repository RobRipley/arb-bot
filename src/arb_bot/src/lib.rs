use candid::{CandidType, Deserialize, Nat, Principal};
use ic_cdk_macros::{init, post_upgrade, pre_upgrade, query, update};
use ic_cdk_timers::TimerId;
use std::cell::RefCell;

pub mod state; // pub so integration tests can verify serde upgrade defaults
pub mod strategy_t; // pub so integration tests can reach the pure math
pub mod route_arb;
pub mod route_runtime;
pub mod route_scheduler;
pub mod route_rumi;
pub mod route_icpswap;
mod prices;
mod swaps;
mod partydex;
mod arb;
mod volume;

use state::{BotConfig, BotConfigInput, TradeRecord, TradeLeg, ErrorRecord, ActivityRecord, CycleSnapshot};

thread_local! {
    static ARB_TIMER_ID: RefCell<Option<TimerId>> = const { RefCell::new(None) };
    static ROUTE_RUNTIME_TIMER_ID: RefCell<Option<TimerId>> = const { RefCell::new(None) };
    static VOLUME_TIMER_ID: RefCell<Option<TimerId>> = const { RefCell::new(None) };
}

/// `config` is `BotConfigInput`, not `BotConfig` — see that type's doc
/// comment. A caller built against the pre-Strategy-T interface omits the
/// 14 `strategy_t_*` fields entirely; `into_full_config` resolves those to
/// the same inert defaults `BotState::default()` already establishes,
/// since a genuinely fresh install has no prior config to preserve.
#[derive(CandidType, Deserialize)]
pub struct InitArgs {
    pub config: BotConfigInput,
}

#[init]
fn init(args: InitArgs) {
    let inert_defaults = state::BotState::default().config;
    state::init_state(state::BotState {
        config: args.config.into_full_config(&inert_defaults),
        ..Default::default()
    });
    // Can't make inter-canister calls during init, so resolve token ordering
    // on the first timer tick. Start the timer immediately.
    setup_timer();
    setup_volume_timer();
    setup_route_runtime_timer();
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
    setup_route_runtime_timer();
}

/// Retired under Stage-1 of the six-asset route-arbitrage policy: the
/// automatic arb-cycle timer is the "legacy automatic arbitrage timer
/// callback" the spec requires to fail closed. Never registering it is
/// the most directly verifiable way to guarantee it makes zero calls —
/// there is no callback left to invoke. `ARB_TIMER_ID` is left in place
/// (harmless — `clear_timer` on a never-set id is a no-op) rather than
/// removed, to keep this diff minimal; a later stage may remove it
/// entirely once the new engine's own scheduling exists.
fn setup_timer() {
    ARB_TIMER_ID.with(|id| {
        if let Some(prev) = id.borrow_mut().take() {
            ic_cdk_timers::clear_timer(prev);
        }
    });
}

fn setup_route_runtime_timer() {
    ROUTE_RUNTIME_TIMER_ID.with(|slot| {
        if let Some(id) = slot.borrow_mut().take() { ic_cdk_timers::clear_timer(id); }
        let id = ic_cdk_timers::set_timer_interval(std::time::Duration::from_secs(10), || {
            ic_cdk::spawn(async { let _ = route_scheduler::tick().await; });
        });
        *slot.borrow_mut() = Some(id);
    });
}

fn setup_volume_timer() {
    VOLUME_TIMER_ID.with(|id| {
        if let Some(prev) = id.borrow_mut().take() {
            ic_cdk_timers::clear_timer(prev);
        }
    });
    if let Err(reason) = state::read_state(validate_volume_registry) {
        state::log_error(format!("volume timer not armed: {reason}"));
        return;
    }
    let interval = state::read_state(|s| s.volume.interval_secs).max(1);
    let new_id = ic_cdk_timers::set_timer_interval(
        std::time::Duration::from_secs(interval),
        || {
            ic_cdk::spawn(async {
                let _ = run_volume_cycle_if_unfrozen().await;
            })
        },
    );
    VOLUME_TIMER_ID.with(|id| *id.borrow_mut() = Some(new_id));
}

async fn run_volume_cycle_if_unfrozen() -> Vec<String> {
    if let Some(reason) = state::read_state(|s| {
        state::legacy_route_freeze_reason(s, state::LegacyFreezeAsset::Icp)
    }) {
        return vec![reason];
    }
    if let Some(reason) = state::read_state(|s| {
        state::legacy_route_freeze_reason(s, state::LegacyFreezeAsset::Bob)
    }) {
        return vec![reason];
    }
    let config_valid = state::read_state(validate_volume_registry);
    if let Err(reason) = config_valid {
        return vec![reason];
    }
    let operation_id = format!("volume-cycle-{}", ic_cdk::api::time());
    if let Err(reason) = state::acquire_mutation_lock(
        &operation_id, route_arb::MutationOwnerV1::VolumeOperation, ic_cdk::api::time(),
    ) {
        return vec![format!("volume cycle deferred: {reason}")];
    }
    let mut outcomes = volume::run_volume_cycle().await;
    if let Err(error) = sync_volume_owned_reservations() {
        let _ = state::mark_mutation_lock_reconciliation_required(&operation_id);
        outcomes.push(format!(
            "volume cycle requires reconciliation: stranded-fund reservation failed: {error}"
        ));
        return outcomes;
    }
    if let Err(error) = state::release_mutation_lock(&operation_id) {
        outcomes.push(format!("volume cycle could not release account mutation lock: {error}"));
    }
    outcomes
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

fn pinned(text: &str) -> Principal {
    Principal::from_text(text).expect("compile-time principal must be valid")
}

fn validate_volume_registry(state: &state::BotState) -> Result<(), String> {
    let config = &state.config;
    let expected = [
        ("ICP ledger", config.icp_ledger, "ryjl3-tyaaa-aaaaa-aaaba-cai"),
        ("icUSD ledger", config.icusd_ledger, "t6bor-paaaa-aaaap-qrd5q-cai"),
        ("ckUSDT ledger", config.ckusdt_ledger, "cngnf-vqaaa-aaaar-qag4q-cai"),
        ("ckUSDC ledger", config.ckusdc_ledger, "xevnm-gaaaa-aaaar-qafnq-cai"),
        ("3USD/3pool", config.three_usd_ledger, "fohh4-yyaaa-aaaap-qtkpa-cai"),
        ("Rumi AMM", config.rumi_amm, "ijlzs-2yaaa-aaaap-quaaq-cai"),
        ("ICPSwap ICP/icUSD", config.icpswap_icusd_pool, "nqxwe-hiaaa-aaaar-qb5yq-cai"),
        ("ICPSwap ICP/3USD", config.icpswap_3usd_pool, "mu2zw-6iaaa-aaaar-qb56q-cai"),
        ("BOB ledger", config.bob_ledger, "7pail-xaaaa-aaaas-aabmq-cai"),
        ("ICPSwap BOB/ICP", config.icpswap_bob_icp_pool, "ybilh-nqaaa-aaaag-qkhzq-cai"),
        ("ICPSwap ICP/ckUSDC", config.icpswap_pool, "mohjv-bqaaa-aaaag-qjyia-cai"),
        ("ICPSwap ICP/ckUSDT", config.icpswap_ckusdt_pool, "hkstf-6iaaa-aaaag-qkcoq-cai"),
    ];
    for (label, actual, expected) in expected {
        if actual != pinned(expected) {
            return Err(format!("volume operation disabled: {label} does not match immutable pin"));
        }
    }
    if config.icpswap_icusd_bob_pool != Principal::anonymous()
        && config.icpswap_icusd_bob_pool != pinned("gxvvw-aiaaa-aaaar-qcada-cai")
    {
        return Err("volume operation disabled: ICPSwap icUSD/BOB does not match immutable pin".to_string());
    }
    if config.rumi_3pool != pinned("fohh4-yyaaa-aaaap-qtkpa-cai") {
        return Err("volume operation disabled: Rumi 3pool does not match immutable pin".to_string());
    }
    let ordering = [
        ("ICPSwap ICP/ckUSDC", config.icpswap_icp_is_token0, true, state.token_ordering_resolved),
        ("ICPSwap ICP/icUSD", config.icpswap_icusd_icp_is_token0, true, state.icusd_token_ordering_resolved),
        ("ICPSwap ICP/ckUSDT", config.icpswap_ckusdt_icp_is_token0, false, state.ckusdt_token_ordering_resolved),
        ("ICPSwap ICP/3USD", config.icpswap_3usd_icp_is_token0, false, state.icpswap_3usd_token_ordering_resolved),
        ("ICPSwap BOB/ICP", config.icpswap_bob_icp_icp_is_token0, false, state.bob_icp_ordering_resolved),
    ];
    for (label, actual, expected, resolved) in ordering {
        if !resolved || actual != expected {
            return Err(format!(
                "volume operation disabled: {label} token ordering is unresolved or differs from the immutable registry"
            ));
        }
    }
    if config.icpswap_icusd_bob_pool != Principal::anonymous()
        && (!state.icusd_bob_ordering_resolved || config.icpswap_icusd_bob_icusd_is_token0)
    {
        return Err("volume operation disabled: ICPSwap icUSD/BOB token ordering is unresolved or differs from the immutable registry".to_string());
    }
    Ok(())
}

fn is_pinned_volume_ledger(ledger: Principal) -> bool {
    route_arb::asset_for_ledger(ledger).is_some()
        || ledger == pinned("fohh4-yyaaa-aaaap-qtkpa-cai")
        || ledger == pinned("7pail-xaaaa-aaaas-aabmq-cai")
}

fn sync_volume_owned_reservations() -> Result<(), String> {
    let stranded = state::read_state(|s| s.volume_stranded_icp);
    state::put_ownership_reservation(route_arb::OwnershipReservationV1 {
        reservation_id: "volume-stranded-icp".to_string(),
        asset: route_arb::Asset::Icp,
        amount_native: stranded,
        whole_asset_freeze: false,
        kind: route_arb::ReservationKindV1::NonRoute,
        owner: route_arb::MutationOwnerV1::VolumeOperation,
        operation_id: "volume-stranded-recovery".to_string(),
        reconciled_at_ns: ic_cdk::api::time(),
        active: stranded > 0,
    })
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
fn get_route_arb_config_v1() -> route_arb::RouteArbConfigV1 {
    state::read_state(|s| s.route_arb.clone())
}

#[update]
fn set_route_arb_config_v1(config: route_arb::RouteArbConfigV1) -> Result<(), String> {
    require_admin();
    route_arb::validate_route_config(&config)?;
    state::mutate_state(|s| {
        s.route_arb_config_generation = s.route_arb_config_generation.checked_add(1)
            .ok_or_else(|| "route config generation exhausted".to_string())?;
        s.route_arb = config;
        s.route_observation = None;
        Ok::<(), String>(())
    })?;
    Ok(())
}

#[query]
fn get_route_arb_status_v1() -> route_arb::RouteArbStatusV1 {
    let mut status=state::read_state(|s| route_arb::route_status(&s.route_arb));
    match route_runtime::status() {
        Ok(runtime) => {status.execution_compiled_in=runtime.compiled_support;status.live_execution_authorized=runtime.live_authorized;},
        Err(error) => {status.config_valid=false;status.config_incident=Some(error);}
    }
    status
}

#[update]
async fn get_route_wallet_balances_v1() -> Vec<route_arb::WalletAssetBalanceV1> {
    require_admin();
    route_arb::read_wallet_balances(ic_cdk::id()).await
}

#[update]
fn start_route_observation_v1() -> Result<route_arb::ObservationStartV1, String> {
    require_admin();
    start_route_observation_internal()
}

pub(crate) fn start_route_observation_internal() -> Result<route_arb::ObservationStartV1, String> {
    let (config, generation) = state::read_state(|s| (s.route_arb.clone(), s.route_arb_config_generation));
    let universe = route_arb::build_work_universe(&config)?;
    let now = ic_cdk::api::time();
    let observation_id = format!("route-observation-{}-{}", generation, now);
    let mut accumulator = route_arb::ObservationAccumulatorV1::new(
        observation_id.clone(), now, 0, universe.items.len() as u64,
        universe.required_quote_calls,
        universe.required_quote_calls <= u64::from(config.max_quote_calls_per_observation),
    );
    accumulator.route_count = universe.route_count as u32;
    let result = route_arb::ObservationStartV1 {
        observation_id,
        route_count: universe.route_count as u32,
        route_size_count: universe.items.len() as u64,
        required_quote_calls: universe.required_quote_calls,
        quote_call_budget_sufficient: accumulator.quote_call_budget_sufficient,
        next_cursor: 0,
    };
    state::mutate_state(|s| s.route_observation = Some(accumulator));
    Ok(result)
}

#[update]
async fn quote_route_observation_batch_v1(cursor: u64, limit: u16) -> Result<route_arb::ObservationBatchResultV1, String> {
    require_admin();
    quote_route_observation_batch_internal(cursor, limit).await
}

pub(crate) async fn quote_route_observation_batch_internal(cursor: u64, limit: u16) -> Result<route_arb::ObservationBatchResultV1, String> {
    if limit == 0 || limit > route_arb::HARD_MAX_PAGE_SIZE {
        return Err("limit must be between 1 and 100".to_string());
    }
    let (config, generation, active) = state::read_state(|s| {
        (s.route_arb.clone(), s.route_arb_config_generation, s.route_observation.clone())
    });
    let active = active.ok_or_else(|| "no active route observation".to_string())?;
    if active.scan_complete {
        return Err("route observation is already complete".to_string());
    }
    if active.next_cursor != cursor {
        return Err(format!("cursor mismatch: expected {}, got {}", active.next_cursor, cursor));
    }
    if !active.quote_call_budget_sufficient {
        return Err(format!(
            "route observation requires {} quote calls, above configured maximum {}; no quote calls were made",
            active.required_quote_calls, config.max_quote_calls_per_observation
        ));
    }
    let universe = route_arb::build_work_universe(&config)?;
    if active.total_work_items != universe.items.len() as u64 {
        return Err("route universe changed; start a new observation".to_string());
    }
    let start = usize::try_from(cursor).map_err(|_| "cursor exceeds platform range")?;
    let end = start.saturating_add(usize::from(limit)).min(universe.items.len());
    let requested_quote_calls: u64 = universe.items[start..end]
        .iter()
        .map(|item| item.route.edges.len() as u64)
        .sum();
    let remaining_quote_calls = u64::from(config.max_quote_calls_per_observation)
        .checked_sub(active.quote_calls_made)
        .ok_or_else(|| "route observation has already exceeded its configured quote-call budget".to_string())?;
    if requested_quote_calls > remaining_quote_calls {
        return Err(format!(
            "requested batch could require {requested_quote_calls} quote calls with only {remaining_quote_calls} remaining; no quote calls were made"
        ));
    }
    let reservations = current_route_reservation_totals();
    let quoted = route_arb::quote_observation_items(
        &config,
        &universe.items[start..end],
        &reservations,
    ).await;
    let candidates: Vec<_> = quoted.iter().map(|(candidate, _, _)| candidate.clone()).collect();
    let quote_calls = quoted.iter().map(|(_, calls, _)| *calls).sum();
    let full_fill_rejections = quoted.iter().filter(|(_, _, rejected)| *rejected).count() as u64;
    let completed_at = ic_cdk::api::time();
    let observation = state::mutate_state(|s| -> Result<_, String> {
        if s.route_arb_config_generation != generation {
            return Err("route config changed while batch was quoting; results discarded".to_string());
        }
        let current = s.route_observation.as_mut().ok_or_else(|| "route observation disappeared while batch was quoting".to_string())?;
        if current.observation_id != active.observation_id || current.next_cursor != cursor {
            return Err("route observation advanced concurrently; results discarded".to_string());
        }
        route_arb::accumulate_observation_batch(current, cursor, candidates.clone(), quote_calls, full_fill_rejections)?;
        if current.scan_complete {
            current.completed_at_ns = Some(completed_at);
        }
        Ok(current.clone())
    })?;
    if observation.scan_complete {
        state::append_route_observation(observation.clone())?;
    }
    Ok(route_arb::ObservationBatchResultV1 { observation, candidates })
}

fn current_route_reservation_totals() -> route_arb::ReservationTotals {
    let mut totals = route_arb::ReservationTotals::default();
    for asset in route_arb::Asset::ALL {
        let current = state::reservation_totals_for_asset(asset);
        totals.held.set(asset, Some(u128::from(current.held)));
        totals.active.set(asset, Some(u128::from(current.active_route)));
        totals.non_route.set(asset, Some(u128::from(current.non_route)));
        totals.set_whole_asset_frozen(asset, current.whole_asset_frozen);
    }
    totals
}

#[query]
fn get_route_observation_v1() -> Option<route_arb::ObservationAccumulatorV1> {
    state::read_state(|s| s.route_observation.clone())
}

#[query]
fn get_route_observations_v1(offset: u64, limit: u64) -> Result<Vec<route_arb::ObservationAccumulatorV1>, String> {
    state::get_route_observations_page(offset, limit)
}

#[query]
fn get_best_route_candidates_v1() -> route_arb::BestRouteCandidatesV1 {
    state::read_state(|s| match &s.route_observation {
        Some(observation) => route_arb::BestRouteCandidatesV1 {
            observation_id: Some(observation.observation_id.clone()),
            scan_complete: observation.scan_complete,
            stable: observation.best_stable_candidate.clone(),
            icp: observation.best_icp_candidate.clone(),
        },
        None => route_arb::BestRouteCandidatesV1 {
            observation_id: None, scan_complete: false, stable: None, icp: None,
        },
    })
}

#[query]
fn get_route_mutation_lock_v1() -> Option<route_arb::MutationLockV1> {
    state::get_mutation_lock()
}

#[query]
fn get_route_reservations_v1(offset: u64, limit: u64) -> Result<Vec<route_arb::OwnershipReservationV1>, String> {
    state::get_ownership_reservations_page(offset, limit)
}

#[query]
fn get_held_positions_v1(offset: u64, limit: u64) -> Result<Vec<route_arb::HeldPositionV1>, String> {
    state::get_held_positions_page(offset, limit)
}

#[query]
fn get_current_route_execution_v1() -> Option<route_arb::ExecutionRecordV1> {
    state::get_current_route_execution()
}

#[query]
fn get_terminal_route_executions_v1(offset: u64, limit: u64) -> Result<Vec<route_arb::ExecutionRecordV1>, String> {
    state::get_terminal_route_executions_page(offset, limit)
}

#[query]
fn get_route_execution_detail_v1(
    execution_id: String,
) -> Result<route_arb::RouteExecutionDetailV1, String> {
    if let Some(detail) = state::get_route_execution_detail(&execution_id)? {
        return Ok(detail);
    }
    let record = state::find_route_execution_record(&execution_id)?
        .ok_or_else(|| format!("unknown route execution: {execution_id}"))?;
    Ok(route_arb::RouteExecutionDetailV1 {
        record,
        asset_path: vec![],
        legs: vec![],
        detail_available: false,
    })
}

#[query]
fn get_route_runtime_status_v1() -> Result<route_runtime::RuntimeStatus, String> {
    route_runtime::status()
}

#[update]
fn set_route_runtime_authorized_v1(authorized: bool) -> Result<route_runtime::RuntimeStatus, String> {
    require_admin();
    route_runtime::set_authorized(authorized)
}

#[update]
async fn prepare_route_execution_v1(route_id: String) -> Result<route_arb::ExecutionRecordV1, String> {
    require_admin();
    route_runtime::prepare(&route_id).await
}

#[update]
async fn advance_route_execution_v1(execution_id: String) -> Result<route_arb::ExecutionRecordV1, String> {
    require_admin();
    route_runtime::advance(&execution_id).await
}

#[update]
async fn reconcile_route_execution_v1(execution_id: String) -> Result<route_arb::ExecutionRecordV1, String> {
    require_admin();
    route_runtime::reconcile(&execution_id).await
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
    // Track the Leg1→Drain pairing separately per transit denomination. Strategies
    // A–R trade through ICP; strategy S trades through BOB. A single shared flag
    // mispairs when the two interleave chronologically (an S Leg1 followed by an
    // A–R Drain, or vice versa), corrupting unpaired_drain_usd. A leg is BOB-family
    // iff either of its tokens is BOB (only strategy S touches BOB).
    let mut has_pending_icp_leg1 = false;
    let mut has_pending_bob_leg1 = false;
    state::fold_trade_legs((), |_, leg| {
        summary.total_usd_in += leg.sold_usd_value;
        summary.total_usd_out += leg.bought_usd_value;
        summary.total_fees_usd += leg.fees_usd;
        let is_bob = leg.sold_token == "BOB" || leg.bought_token == "BOB";
        match leg.leg_type {
            state::LegType::Leg1 => {
                summary.leg1_count += 1;
                if is_bob { has_pending_bob_leg1 = true; } else { has_pending_icp_leg1 = true; }
            }
            state::LegType::Leg2 => {
                summary.leg2_count += 1;
                if is_bob { has_pending_bob_leg1 = false; } else { has_pending_icp_leg1 = false; }
            }
            state::LegType::Drain => {
                summary.drain_count += 1;
                let has_pending_leg1 = if is_bob { has_pending_bob_leg1 } else { has_pending_icp_leg1 };
                if !has_pending_leg1 {
                    // This drain has no matching Leg1 in its own denomination —
                    // it's recovering pre-existing inventory, not arb profit.
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
    ic_cdk::trap("retired: get_prices is retired under Stage-1 (mixed retired-venue price lookups) — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}

// ─── Admin Methods ───

/// `config` is `BotConfigInput`, not `BotConfig` — see that type's doc
/// comment. A caller built against the pre-Strategy-T interface (old
/// dashboard, old dfx-generated bindings, or any external tooling that
/// hasn't picked up the 14 new fields) sends a payload with none of them
/// set; `into_full_config` preserves whatever this canister's CURRENT
/// config already has for those fields rather than resetting them to the
/// hardcoded default, so a routine old-style config write can never
/// silently wipe Strategy T settings made via `set_strategy_t_*`.
#[update]
fn set_config(_config: BotConfigInput) {
    require_admin();
    ic_cdk::trap("retired: set_config is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
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

/// Admin escape hatch for a stuck arb-cycle lock. Cycles gate on an in-progress
/// flag released by a Drop guard; a wasm trap after the flag commits unwinds
/// without running Drop, wedging every future cycle and manual strategy run
/// until an upgrade. This force-clears the flag (and per-cycle caches) so cycles
/// can resume without redeploying.
#[update]
fn clear_cycle_lock() {
    require_admin();
    let was_locked = arb::force_clear_cycle_lock();
    state::log_activity("admin", &format!(
        "Cycle lock cleared by {} (was_locked={})", ic_cdk::api::caller(), was_locked
    ));
}

/// Kill switch for the Rumi AMM (3USD/ICP) venue — pauses Strategies A/C/D/Q/R
/// (every strategy that trades against `rumi_amm`) without touching the
/// other ICPSwap/PartyDEX cross-pool strategies or the global `paused` flag.
#[update]
fn set_rumi_amm_paused(_paused: bool) -> Result<(), String> {
    require_admin();
    Err("retired: set_rumi_amm_paused is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
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
    "retired: setup_approvals is retired under Stage-1 — active-pool allowances now require a separately named, spender-specific admin action (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md); no approval was granted".to_string()
}

#[update]
async fn withdraw(token_ledger: Principal, to: Principal, amount: u64) {
    require_admin();

    let active_asset = route_arb::asset_for_ledger(token_ledger);
    let is_legacy_recovery_ledger = token_ledger == pinned("fohh4-yyaaa-aaaap-qtkpa-cai")
        || token_ledger == pinned("7pail-xaaaa-aaaas-aabmq-cai");
    if active_asset.is_none() && !is_legacy_recovery_ledger {
        ic_cdk::trap("withdraw rejected: ledger is not in the immutable active/recovery registry");
    }

    let freeze_reason = state::read_state(|s| {
        let asset = if token_ledger == s.config.icp_ledger {
            Some(state::LegacyFreezeAsset::Icp)
        } else if token_ledger == s.config.bob_ledger {
            Some(state::LegacyFreezeAsset::Bob)
        } else {
            None
        };
        asset.and_then(|a| state::legacy_route_freeze_reason(s, a))
    });
    if let Some(reason) = freeze_reason {
        ic_cdk::trap(&reason);
    }

    let operation_id = format!("withdraw-{}", ic_cdk::api::time());
    state::acquire_mutation_lock(
        &operation_id, route_arb::MutationOwnerV1::GenericWithdrawal, ic_cdk::api::time(),
    ).unwrap_or_else(|reason| ic_cdk::trap(&reason));
    if let Some(asset) = active_asset {
        let balance = match swaps::icrc1_balance_of_default(token_ledger).await {
            Ok(balance) => balance,
            Err(error) => {
                let _ = state::release_mutation_lock(&operation_id);
                ic_cdk::trap(&format!("withdraw balance check failed: {error}"));
            }
        };
        let available = state::spendable_native(asset, balance)
            .unwrap_or_else(|error| ic_cdk::trap(&error));
        if amount > available {
            let _ = state::release_mutation_lock(&operation_id);
            ic_cdk::trap("withdraw rejected: amount would consume reserved inventory");
        }
    }

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
            let _ = state::release_mutation_lock(&operation_id);
            state::log_activity("withdraw", &format!(
                "Withdrew {} from ledger {} to {} by {}",
                amount, token_ledger, to, ic_cdk::api::caller()
            ));
        }
        Ok((Err(e),)) => {
            let _ = state::release_mutation_lock(&operation_id);
            let msg = format!("Withdraw failed: {:?} (ledger={}, to={}, amount={})", e, token_ledger, to, amount);
            state::log_activity("withdraw", &msg);
            ic_cdk::trap(&format!("Transfer failed: {:?}", e));
        }
        Err((code, msg)) => {
            let _ = state::mark_mutation_lock_reconciliation_required(&operation_id);
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
    let quote_asset = if pool == pinned("xjiq2-fiaaa-aaaan-q52ra-cai") {
        route_arb::Asset::CkUsdc
    } else if pool == pinned("6b2bo-kyaaa-aaaao-qpira-cai") {
        route_arb::Asset::CkUsdt
    } else {
        return Err("PartyDEX recovery rejected: pool is not one of the two immutable retired recovery pins".to_string());
    };
    if let Some(reason) = state::read_state(|s| state::legacy_route_freeze_reason(s, state::LegacyFreezeAsset::Icp)) {
        return Err(reason);
    }
    let operation_id = format!("partydex-recovery-{}", ic_cdk::api::time());
    state::acquire_mutation_lock(
        &operation_id, route_arb::MutationOwnerV1::RetiredVenueRecovery, ic_cdk::api::time(),
    )?;
    let result = partydex::withdraw_all(pool).await;
    match &result {
        Ok((base_out, quote_out)) => {
            for (asset, amount, suffix) in [
                (route_arb::Asset::Icp, *base_out, "base"),
                (quote_asset, *quote_out, "quote"),
            ] {
                if amount > 0 {
                    if let Err(error) = state::put_ownership_reservation(route_arb::OwnershipReservationV1 {
                        reservation_id: format!("{}-{}", operation_id, suffix),
                        asset, amount_native: amount, whole_asset_freeze: false,
                        kind: route_arb::ReservationKindV1::NonRoute,
                        owner: route_arb::MutationOwnerV1::RetiredVenueRecovery,
                        operation_id: operation_id.clone(), reconciled_at_ns: ic_cdk::api::time(), active: true,
                    }) {
                        let _ = state::mark_mutation_lock_reconciliation_required(&operation_id);
                        return Err(format!("recovery credited funds but reservation failed: {error}"));
                    }
                }
            }
            state::release_mutation_lock(&operation_id)?;
            state::log_activity("recover", &format!(
                "recover_partydex_balance({}) swept base={} quote={} by {}",
                pool, base_out, quote_out, ic_cdk::api::caller()
            ));
        }
        Err(e) => {
            let _ = state::mark_mutation_lock_reconciliation_required(&operation_id);
            state::log_activity("recover", &format!(
                "recover_partydex_balance({}) failed: {} (by {})", pool, e, ic_cdk::api::caller()
            ));
        }
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
async fn pool_deposit(_coin_index: u8, _amount: u64, _min_lp_out: u64) {
    require_admin();
    ic_cdk::trap("retired: pool_deposit is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}

/// Redeem 3USD LP tokens for a single stablecoin.
/// coin_index: 0=icUSD, 1=ckUSDT, 2=ckUSDC
#[update]
async fn pool_redeem(_coin_index: u8, _lp_amount: u64, _min_out: u64) {
    require_admin();
    ic_cdk::trap("retired: pool_redeem is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}

/// Quote how much 3USD LP you'd get from depositing a stablecoin.
#[update]
async fn pool_quote_deposit(_coin_index: u8, _amount: u64) -> PoolQuote {
    require_admin();
    ic_cdk::trap("retired: pool_quote_deposit is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}

/// Quote how much stablecoin you'd get from redeeming 3USD LP.
#[update]
async fn pool_quote_redeem(_coin_index: u8, _lp_amount: u64) -> PoolQuote {
    require_admin();
    ic_cdk::trap("retired: pool_quote_redeem is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}

/// Swap one stablecoin for another directly via the 3pool.
/// coin_in/coin_out: 0=icUSD, 1=ckUSDT, 2=ckUSDC
#[update]
async fn pool_exchange(_coin_in: u8, _coin_out: u8, _amount_in: u64, _min_out: u64) {
    require_admin();
    ic_cdk::trap("retired: pool_exchange is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}

/// Quote a direct stablecoin-to-stablecoin swap via the 3pool.
#[update]
async fn pool_quote_exchange(_coin_in: u8, _coin_out: u8, _amount_in: u64) -> PoolQuote {
    require_admin();
    ic_cdk::trap("retired: pool_quote_exchange is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}

// ─── Rumi AMM Manual Swap ───

const RUMI_POOL_ID: &str = "fohh4-yyaaa-aaaap-qtkpa-cai_ryjl3-tyaaa-aaaaa-aaaba-cai";

/// Quote a Rumi AMM swap (ICP ↔ 3USD). token_in is the ledger of the token being sold.
#[update]
async fn rumi_quote(_token_in: Principal, _amount: u64) -> PoolQuote {
    require_admin();
    ic_cdk::trap("retired: rumi_quote is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}

/// Execute a Rumi AMM swap (ICP ↔ 3USD). token_in is the ledger of the token being sold.
#[update]
async fn rumi_manual_swap(_token_in: Principal, _amount: u64, _min_out: u64) {
    require_admin();
    ic_cdk::trap("retired: rumi_manual_swap is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
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
    state::read_state(validate_volume_registry)
        .unwrap_or_else(|reason| ic_cdk::trap(&reason));
    if let Some(reason) = state::read_state(|s| state::legacy_route_freeze_reason(s, state::LegacyFreezeAsset::Icp)) {
        ic_cdk::trap(&reason);
    }
    let (rumi_amm, icp_ledger, three_usd_ledger, icusd_ledger, rumi_3pool) = state::read_state(|s| {
        (s.config.rumi_amm, s.config.icp_ledger, s.config.three_usd_ledger,
         Principal::from_text("t6bor-paaaa-aaaap-qrd5q-cai").unwrap(),
         s.config.rumi_3pool)
    });
    let caller = ic_cdk::api::caller();
    let operation_id = format!("volume-swap-{}", ic_cdk::api::time());
    state::acquire_mutation_lock(
        &operation_id, route_arb::MutationOwnerV1::VolumeOperation, ic_cdk::api::time(),
    ).unwrap_or_else(|reason| ic_cdk::trap(&reason));
    let mut settlement_ambiguous = false;

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
                settlement_ambiguous = true;
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
                settlement_ambiguous = true;
                state::log_activity("volume_swap", &format!("WARNING: ICP transfer back failed: {:?}", e));
            }
        }
        state::log_activity("volume_swap", &format!(
            "Volume swap: {} icUSD → {} ICP by {}", amount, icp_out, caller
        ));
    }
    if settlement_ambiguous {
        let _ = state::mark_mutation_lock_reconciliation_required(&operation_id);
    } else {
        let _ = state::release_mutation_lock(&operation_id);
    }
}

/// One-time backfill: append historical trade legs to the log.
/// NOTE: Post stable-memory migration, this now APPENDS (previously prepended).
/// Chronological ordering of historical entries is not preserved.
#[update]
fn backfill_trade_legs(_legs: Vec<TradeLeg>) {
    require_admin();
    ic_cdk::trap("retired: backfill_trade_legs is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}

#[update]
async fn manual_arb_cycle() {
    require_admin();
    ic_cdk::trap("retired: manual_arb_cycle is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}

#[update]
async fn execute_strategy_a() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_a is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

#[update]
async fn execute_strategy_b() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_b is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

#[update]
async fn execute_strategy_c() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_c is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

#[update]
async fn execute_strategy_d() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_d is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

#[update]
async fn execute_strategy_f() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_f is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

#[update]
async fn execute_strategy_k() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_k is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

#[update]
async fn execute_strategy_l() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_l is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

#[update]
async fn execute_strategy_m() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_m is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

#[update]
async fn execute_strategy_n() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_n is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

#[update]
async fn execute_strategy_o() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_o is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

#[update]
async fn execute_strategy_p() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_p is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

#[update]
async fn execute_strategy_q() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_q is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

#[update]
async fn execute_strategy_r() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_r is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

/// Admin override for Strategy S — executes regardless of
/// bob_execution_enabled, matching A–R force-execute semantics.
#[update]
async fn execute_strategy_s() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_s is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}

#[update]
async fn dry_run_arb_cycle() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_arb_cycle is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}

#[update]
async fn dry_run_strategy_c() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_strategy_c is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}

#[update]
async fn dry_run_strategy_d() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_strategy_d is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}

#[update]
async fn dry_run_strategy_b() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_strategy_b is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}

#[update]
async fn dry_run_strategy_f() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_strategy_f is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}

#[update]
async fn dry_run_strategy_k() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_strategy_k is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}

#[update]
async fn dry_run_strategy_l() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_strategy_l is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}

#[update]
async fn dry_run_strategy_m() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_strategy_m is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}

#[update]
async fn dry_run_strategy_n() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_strategy_n is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}

#[update]
async fn dry_run_strategy_o() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_strategy_o is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}

#[update]
async fn dry_run_strategy_p() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_strategy_p is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}

#[update]
async fn dry_run_strategy_q() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_strategy_q is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}

#[update]
async fn dry_run_strategy_r() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_strategy_r is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}

#[update]
async fn dry_run_strategy_t() -> strategy_t::StrategyTDryRunResult {
    require_admin();
    // Strategy T's own report type carries no message field; an empty
    // result set is the honest "retired, nothing evaluated" value —
    // never a fabricated candidate.
    strategy_t::StrategyTDryRunResult {
        candidates: Vec::new(),
        best_economic: None,
        best_executable: None,
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
            state::VolumePool::IcusdBob => s.volume.icusd_bob = new_config,
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
    state::read_state(validate_volume_registry)?;
    if !is_pinned_volume_ledger(token_ledger) {
        return Err("volume funding rejected: ledger is not code-pinned".to_string());
    }
    let freeze_reason = state::read_state(|s| {
        let asset = if token_ledger == s.config.icp_ledger {
            Some(state::LegacyFreezeAsset::Icp)
        } else if token_ledger == s.config.bob_ledger {
            Some(state::LegacyFreezeAsset::Bob)
        } else {
            None
        };
        asset.and_then(|a| state::legacy_route_freeze_reason(s, a))
    });
    if let Some(reason) = freeze_reason {
        return Err(reason);
    }
    let three_usd = state::read_state(|s| s.config.three_usd_ledger);
    if token_ledger == three_usd {
        // 3USD ledger ignores subaccounts — funds are already in default account
        return Ok(());
    }
    let operation_id = format!("volume-fund-{}", ic_cdk::api::time());
    state::acquire_mutation_lock(&operation_id, route_arb::MutationOwnerV1::VolumeOperation, ic_cdk::api::time())?;
    match swaps::transfer_to_subaccount(token_ledger, amount, swaps::VOLUME_SUBACCOUNT).await {
        Ok(_) => { state::release_mutation_lock(&operation_id)?; Ok(()) }
        Err(error) => {
            let _ = state::mark_mutation_lock_reconciliation_required(&operation_id);
            Err(format!("Failed to fund volume subaccount: {:?}; reconciliation required", error))
        }
    }
}

#[update]
async fn withdraw_volume_subaccount(token_ledger: Principal, amount: u64) -> Result<(), String> {
    require_admin();
    state::read_state(validate_volume_registry)?;
    if !is_pinned_volume_ledger(token_ledger) {
        return Err("volume withdrawal rejected: ledger is not code-pinned".to_string());
    }
    let freeze_reason = state::read_state(|s| {
        let asset = if token_ledger == s.config.icp_ledger {
            Some(state::LegacyFreezeAsset::Icp)
        } else if token_ledger == s.config.bob_ledger {
            Some(state::LegacyFreezeAsset::Bob)
        } else {
            None
        };
        asset.and_then(|a| state::legacy_route_freeze_reason(s, a))
    });
    if let Some(reason) = freeze_reason {
        return Err(reason);
    }
    let three_usd = state::read_state(|s| s.config.three_usd_ledger);
    if token_ledger == three_usd {
        // 3USD ledger ignores subaccounts — funds are already in default account
        return Ok(());
    }
    let operation_id = format!("volume-withdraw-{}", ic_cdk::api::time());
    state::acquire_mutation_lock(&operation_id, route_arb::MutationOwnerV1::VolumeOperation, ic_cdk::api::time())?;
    match swaps::transfer_from_subaccount(token_ledger, amount, swaps::VOLUME_SUBACCOUNT).await {
        Ok(_) => { state::release_mutation_lock(&operation_id)?; Ok(()) }
        Err(error) => {
            let _ = state::mark_mutation_lock_reconciliation_required(&operation_id);
            Err(format!("Failed to withdraw from volume subaccount: {:?}; reconciliation required", error))
        }
    }
}

#[update]
async fn trigger_volume_cycle() -> String {
    require_admin();
    let outcomes = run_volume_cycle_if_unfrozen().await;
    if outcomes.is_empty() {
        "cycle ran, no outcomes".to_string()
    } else {
        outcomes.join("; ")
    }
}

#[update]
fn set_arb_interval_secs(_interval_secs: u64) -> Result<(), String> {
    require_admin();
    Err("retired: set_arb_interval_secs is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
}

/// Sets the ICP inventory band (e8s) the drain uses in place of the old fixed
/// `ICP_RESERVE`. Floor: minimum working balance always left behind. Ceiling:
/// steady-state skim threshold. Single method so the pair can't pass through
/// an invalid intermediate state (e.g. floor temporarily above ceiling).
#[update]
fn set_icp_inventory_band(_floor_e8s: u64, _ceiling_e8s: u64) -> Result<(), String> {
    require_admin();
    Err("retired: set_icp_inventory_band is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
}

/// Sets the BOB inventory band (e8s, 8 decimals). Mirrors
/// `set_icp_inventory_band` exactly: stored/settable/displayed config only —
/// not wired into any trading or drain logic.
#[update]
fn set_bob_inventory_band(_floor_e8s: u64, _ceiling_e8s: u64) -> Result<(), String> {
    require_admin();
    Err("retired: set_bob_inventory_band is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
}

/// Sets the three Strategy T ICPSwap closing-pool principals.
///
/// IMPORTANT: `strategy_t::ClosingPool::zero_for_one_from` hardcodes each
/// pool's `token0`/`token1` ordering at compile time, asserted against the
/// specific mainnet principals documented on `ClosingPool` (verified live
/// 2026-09-02/03) — it is not runtime-probed. This setter only validates
/// that the three principals are non-anonymous and pairwise distinct; it
/// does NOT verify they are actually the correct, documented pools. Pointing
/// this at a genuinely different pool (or transposing two of the three
/// slots) will silently produce inverted `zeroForOne` swap quotes. Runtime
/// `metadata` probing (matching the `icusd_token_ordering_resolved` /
/// `ckusdt_token_ordering_resolved` pattern used elsewhere in `state.rs`)
/// is out of scope here — re-pointing to different pools requires updating
/// `strategy_t.rs` to match.
#[update]
fn set_strategy_t_pools(_icusd_ckusdc: Principal, _icusd_ckusdt: Principal, _ckusdt_ckusdc: Principal) -> Result<(), String> {
    require_admin();
    Err("retired: set_strategy_t_pools is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
}

/// Master enable switch for Strategy T. Dry-run evaluation (`dry_run_strategy_t`)
/// runs once all three pool principals are non-anonymous, independent of
/// this flag — there is no arb-cycle wiring for Strategy T in this build.
/// This flag and `strategy_t_dry_run` exist for a future live-execution PR;
/// this build has no live-trade path regardless of either value.
#[update]
fn set_strategy_t_enabled(_enabled: bool) {
    require_admin();
    ic_cdk::trap("retired: set_strategy_t_enabled is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}

#[update]
fn set_strategy_t_dry_run(_dry_run: bool) {
    require_admin();
    ic_cdk::trap("retired: set_strategy_t_dry_run is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}

#[update]
fn set_strategy_t_thresholds(_min_profit_usd: i64, _min_profit_bps: u32, _max_trade_size_usd: u64) -> Result<(), String> {
    require_admin();
    Err("retired: set_strategy_t_thresholds is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
}

#[update]
fn set_strategy_t_icusd_band(_floor: u64, _ceiling: u64) -> Result<(), String> {
    require_admin();
    Err("retired: set_strategy_t_icusd_band is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
}

#[update]
fn set_strategy_t_ckusdt_band(_floor: u64, _ceiling: u64) -> Result<(), String> {
    require_admin();
    Err("retired: set_strategy_t_ckusdt_band is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
}

#[update]
fn set_strategy_t_ckusdc_band(_floor: u64, _ceiling: u64) -> Result<(), String> {
    require_admin();
    Err("retired: set_strategy_t_ckusdc_band is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
}

/// Sets both Strategy S pool principals in one call. Resets the resolved-
/// ordering flag for any pool whose principal actually changes, so a
/// re-pointed pool gets its token ordering re-probed on the next cycle
/// instead of running with stale `icp_is_token0`/`icusd_is_token0` bits.
#[update]
fn set_bob_pools(_bob_icp_pool: Principal, _icusd_bob_pool: Principal) -> Result<(), String> {
    require_admin();
    Err("retired: set_bob_pools is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
}

/// Sets Strategy S's sizing/gating knobs. Single method so the pair can't
/// pass through an invalid intermediate state.
#[update]
fn set_bob_params(_max_trade_size_usd: u64, _min_spread_bps: u64) -> Result<(), String> {
    require_admin();
    Err("retired: set_bob_params is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
}

/// Execution kill switch for Strategy S. Dry-run evaluation + dashboard
/// surfacing run regardless of this flag once both BOB pools are configured;
/// this only gates whether Strategy S can actually execute trades.
#[update]
fn set_bob_execution_enabled(_enabled: bool) -> Result<(), String> {
    require_admin();
    Err("retired: set_bob_execution_enabled is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
}

#[update]
fn set_slippage_bps(_slippage_bps: u64) -> Result<(), String> {
    require_admin();
    Err("retired: set_slippage_bps is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
}

#[update]
async fn get_bot_health() -> state::BotHealthReport {
    require_admin();
    ic_cdk::trap("retired: get_bot_health is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}

/// Anonymous-safe stuck-state flags for the logged-out dashboard wedge
/// banner. Booleans only — details require the admin-gated `get_bot_health`.
#[query]
fn get_public_health() -> state::PublicHealth {
    state::read_state(|s| state::PublicHealth {
        has_pending_exit: s.pending_exit.is_some(),
        has_pending_bob_exit: s.pending_bob_exit.is_some(),
        has_stranded_volume_funds: s.volume_stranded_icp > 0 || s.volume_stranded_bob > 0,
        arb_paused: s.config.paused,
        volume_paused: s.volume.volume_paused,
    })
}

#[update]
async fn trigger_volume_rebalance() {
    require_admin();
    state::read_state(validate_volume_registry)
        .unwrap_or_else(|reason| ic_cdk::trap(&reason));
    if let Some(reason) = state::read_state(|s| state::legacy_route_freeze_reason(s, state::LegacyFreezeAsset::Icp)) {
        ic_cdk::trap(&reason);
    }
    if let Some(reason) = state::read_state(|s| state::legacy_route_freeze_reason(s, state::LegacyFreezeAsset::Bob)) {
        ic_cdk::trap(&reason);
    }
    let config = state::read_state(|s| s.config.clone());
    let operation_id = format!("volume-rebalance-{}", ic_cdk::api::time());
    state::acquire_mutation_lock(
        &operation_id, route_arb::MutationOwnerV1::VolumeOperation, ic_cdk::api::time(),
    ).unwrap_or_else(|reason| ic_cdk::trap(&reason));
    volume::run_rebalance(&config).await;
    if let Err(error) = sync_volume_owned_reservations() {
        let _ = state::mark_mutation_lock_reconciliation_required(&operation_id);
        state::log_error(format!(
            "volume rebalance requires reconciliation: stranded-fund reservation failed: {error}"
        ));
        return;
    }
    if let Err(error) = state::release_mutation_lock(&operation_id) {
        state::log_error(format!("volume rebalance could not release account mutation lock: {error}"));
    }
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
        daily_cost_cap_usd_icusd_bob: s.volume.icusd_bob.daily_cost_cap_usd,
        icusd_bob: state::VolumePoolStatus {
            config: s.volume.icusd_bob.clone(),
            state: s.volume.icusd_bob_state.clone(),
        },
        total_trade_count: s.volume.icusd_icp_state.trade_count
            + s.volume.three_usd_icp_state.trade_count
            + s.volume.icusd_bob_state.trade_count,
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
