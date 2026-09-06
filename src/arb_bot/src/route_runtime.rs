//! Durable, source-bound route execution. Authorization defaults off.
//! Every submitted resume is a read-only reconciliation; updates are never replayed.
use crate::{route_arb::*, state};
use candid::{CandidType, Deserialize, Principal};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{cell::Cell, future::Future, pin::Pin};

#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct RuntimeEdge {
    pub edge_id: String,
    pub pool_id: String,
    pub pool_principal: Principal,
    pub venue: VenueKind,
    pub from: Asset,
    pub to: Asset,
}
impl From<&DirectedEdge> for RuntimeEdge {
    fn from(e: &DirectedEdge) -> Self {
        Self {
            edge_id: e.edge_id.clone(),
            pool_id: e.pool_id.into(),
            pool_principal: e.pool_principal,
            venue: e.venue,
            from: e.from,
            to: e.to,
        }
    }
}
#[derive(Deserialize, Serialize, Clone, Debug)]
struct RuntimeLegTrace {
    leg_index: u8,
    edge: RuntimeEdge,
    quoted_input_native: u64,
    quoted_output_native: Option<u64>,
    minimum_output_native: u64,
    input_fee_native: u64,
    output_fee_native: u64,
    submitted_at_ns: Option<u64>,
    settled_at_ns: Option<u64>,
    reconciled_at_ns: Option<u64>,
    settlement: Option<RuntimeSettlement>,
    status: RouteExecutionLegStatusV1,
    incident: Option<String>,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct RuntimeRequest {
    pub intent_id: [u8; 32],
    pub execution_id: String,
    pub leg_index: u8,
    pub owner: Principal,
    pub edge: RuntimeEdge,
    /// Amount accepted by venue, excluding the input ledger fee.
    pub input_native: u64,
    /// Net credit to the bot account, after output ledger fee.
    pub min_output_native: u64,
    pub input_fee_native: u64,
    pub output_fee_native: u64,
    pub prepared_at_ns: u64,
}
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct RuntimeSettlement {
    pub input_debit_native: u64,
    pub effective_input_native: u64,
    pub output_credit_native: u64,
    pub refund_credit_native: u64,
    pub evidence: Vec<ReconciliationEvidenceV1>,
}
#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum RuntimeSubmissionOutcome {
    Accepted,
    RejectedBeforeDebit(String),
    Unknown(String),
}
#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum AdapterIntent {
    Rumi(crate::route_rumi::Intent),
    IcpSwap(crate::route_icpswap::Intent),
}
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct RuntimeExecution {
    pub record: ExecutionRecordV1,
    pub generation: u64,
    pub original: RouteCandidateReportV1,
    pub minimum_final_native: u64,
    pub current_wallet_native: u64,
    pub request: Option<RuntimeRequest>,
    pub intent: Option<AdapterIntent>,
    pub settlements: Vec<RuntimeSettlement>,
    pub submitted_intents: Vec<(RuntimeRequest, AdapterIntent)>,
    pub realized_profit: Option<i128>,
    #[serde(default)]
    leg_traces: Vec<RuntimeLegTrace>,
}
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
struct DurableRuntime {
    #[serde(default)]
    authorized: bool,
    #[serde(default)]
    sequence: u64,
    current: Option<RuntimeExecution>,
    last_terminal: Option<RuntimeExecution>,
    last_error: Option<String>,
    last_tick_ns: u64,
    #[serde(default)]
    last_served_icp: bool,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct RuntimeStatus {
    pub compiled_support: bool,
    pub live_authorized: bool,
    pub enabled: bool,
    pub dry_run: bool,
    pub last_error: Option<String>,
    pub last_tick_ns: u64,
    /// Stable-par USD6 or ICP e8s, based on attributable completed movements.
    pub last_realized_profit: Option<i128>,
    pub last_profit_class: Option<CandidateClass>,
}
fn load() -> Result<DurableRuntime, String> {
    let bytes = state::runtime_bytes();
    if bytes.is_empty() {
        return Ok(DurableRuntime::default());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("durable runtime decode failed: {e}"))
}
fn save(s: &DurableRuntime) -> Result<(), String> {
    state::set_runtime_bytes(serde_json::to_vec(s).map_err(|e| e.to_string())?)
}
fn detail_for(ex: &RuntimeExecution) -> RouteExecutionDetailV1 {
    let detail_available = ex.leg_traces.len() == ex.original.legs.len() && !ex.leg_traces.is_empty();
    let legs = if detail_available {
        ex.leg_traces
            .iter()
            .map(|trace| {
                let settlement = trace.settlement.as_ref();
                let request = ex
                    .request
                    .as_ref()
                    .filter(|request| request.leg_index == trace.leg_index)
                    .or_else(|| {
                        ex.submitted_intents
                            .iter()
                            .find(|(request, _)| request.leg_index == trace.leg_index)
                            .map(|(request, _)| request)
                    });
                let reconciled_at_ns = trace.reconciled_at_ns.or_else(|| {
                    settlement.and_then(|settlement| {
                        settlement.evidence.iter().map(|e| e.observed_at_ns).max()
                    })
                });
                RouteExecutionLegV1 {
                    leg_index: trace.leg_index,
                    status: trace.status.clone(),
                    edge_id: trace.edge.edge_id.clone(),
                    pool_id: trace.edge.pool_id.clone(),
                    pool_principal: trace.edge.pool_principal,
                    venue: trace.edge.venue,
                    from: trace.edge.from,
                    to: trace.edge.to,
                    quoted_input_native: trace.quoted_input_native,
                    quoted_output_native: trace.quoted_output_native,
                    minimum_output_native: trace.minimum_output_native,
                    input_fee_native: trace.input_fee_native,
                    output_fee_native: trace.output_fee_native,
                    actual_input_debit_native: settlement.map(|s| s.input_debit_native),
                    actual_effective_input_native: settlement.map(|s| s.effective_input_native),
                    actual_output_credit_native: settlement.map(|s| s.output_credit_native),
                    refund_credit_native: settlement.map(|s| s.refund_credit_native),
                    prepared_at_ns: request.map(|request| request.prepared_at_ns),
                    submitted_at_ns: trace.submitted_at_ns,
                    settled_at_ns: trace.settled_at_ns,
                    reconciled_at_ns,
                    evidence: settlement.map(|s| s.evidence.clone()).unwrap_or_default(),
                    incident: trace.incident.clone(),
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    RouteExecutionDetailV1 {
        record: ex.record.clone(),
        asset_path: ex.original.asset_path.clone(),
        legs,
        detail_available,
    }
}
pub fn route_execution_detail(ex: &RuntimeExecution) -> RouteExecutionDetailV1 {
    detail_for(ex)
}
fn persist(ex: &RuntimeExecution) -> Result<(), String> {
    let mut s = load()?;
    let bytes = serde_json::to_vec(ex).map_err(|e| e.to_string())?;
    let config = config().0;
    if bytes.len() > config.max_execution_record_bytes as usize {
        return Err("typed execution exceeds configured durable record capacity".into());
    }
    s.current = Some(ex.clone());
    save(&s)?;
    state::put_current_route_execution(ex.record.clone())?;
    if !ex.leg_traces.is_empty() {
        state::put_route_execution_detail(detail_for(ex))?;
    }
    if ex.record.phase.is_terminal() {
        Ok(())
    } else {
        reserve_active(ex, true)
    }
}
fn reserve_active(ex: &RuntimeExecution, active: bool) -> Result<(), String> {
    let index = usize::from(ex.record.current_leg_index)
        + usize::from(ex.record.phase == ExecutionPhaseV1::LegSettled);
    let asset = *ex
        .original
        .asset_path
        .get(index)
        .ok_or("active reservation route index invalid")?;
    for a in Asset::ALL {
        state::put_ownership_reservation(OwnershipReservationV1 {
            reservation_id: format!("active:{}:{:?}", ex.record.execution_id, a),
            asset: a,
            amount_native: if a == asset {
                ex.current_wallet_native
            } else {
                0
            },
            whole_asset_freeze: false,
            kind: ReservationKindV1::ActiveRoute,
            owner: MutationOwnerV1::RouteExecution,
            operation_id: ex.record.execution_id.clone(),
            reconciled_at_ns: ex.record.updated_at_ns,
            active: active && a == asset,
        })?;
    }
    Ok(())
}
fn quote_reservations() -> ReservationTotals {
    let mut totals = crate::current_route_reservation_totals();
    if let Ok(Some(ex)) = load().map(|s| s.current) {
        for offset in [0, 100, 200] {
            if let Ok(rows) = state::get_ownership_reservations_page(offset, 100) {
                for r in rows.into_iter().filter(|r| {
                    r.active
                        && r.operation_id == ex.record.execution_id
                        && r.kind == ReservationKindV1::ActiveRoute
                }) {
                    // Only this route's own proved inventory is spendable by its tail.
                    if let Some(total) = totals.active.get(r.asset) {
                        totals
                            .active
                            .set(r.asset, total.checked_sub(u128::from(r.amount_native)));
                    }
                }
            }
        }
    }
    totals
}
fn config() -> (RouteArbConfigV1, u64) {
    state::read_state(|s| (s.route_arb.clone(), s.route_arb_config_generation))
}
pub fn status() -> Result<RuntimeStatus, String> {
    let s = load()?;
    let c = config().0;
    Ok(RuntimeStatus {
        compiled_support: true,
        live_authorized: s.authorized,
        enabled: c.enabled,
        dry_run: c.dry_run,
        last_error: s.last_error,
        last_tick_ns: s.last_tick_ns,
        last_profit_class: s.last_terminal.as_ref().map(|e| e.original.candidate_class),
        last_realized_profit: s.last_terminal.and_then(|e| e.realized_profit),
    })
}
/// Public endpoint must enforce admin before calling. This source ships false.
pub(crate) fn set_authorized(authorized: bool) -> Result<RuntimeStatus, String> {
    let mut s = load()?;
    s.authorized = authorized;
    save(&s)?;
    status()
}
fn authorized() -> Result<(), String> {
    let c = config().0;
    if !load()?.authorized || !c.enabled || c.dry_run {
        return Err("runtime execution is disabled, dry-run, or not operator-authorized".into());
    }
    Ok(())
}
fn guard_generation(ex: &RuntimeExecution) -> Result<(), String> {
    if config().1 != ex.generation {
        return Err("route policy generation changed".into());
    }
    let lock = state::get_mutation_lock().ok_or("route mutation lock missing")?;
    if lock.operation_id != ex.record.execution_id || lock.owner != MutationOwnerV1::RouteExecution
    {
        return Err("route mutation lock ownership changed".into());
    }
    Ok(())
}
thread_local! { static BUSY: Cell<bool> = const { Cell::new(false) }; }
struct BusyGuard;
impl BusyGuard {
    fn enter() -> Result<Self, String> {
        BUSY.with(|busy| {
            if busy.replace(true) {
                Err("runtime callback already in flight".into())
            } else {
                Ok(Self)
            }
        })
    }
}
impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.with(|busy| busy.set(false));
    }
}

/// Injection boundary used by deterministic tests. Persistence and orchestration
/// remain identical to production; doubles replace only outbound venue/ledger I/O.
pub trait RuntimeIo {
    fn now(&self) -> u64;
    fn owner(&self) -> Principal;
    fn quote<'a>(
        &'a self,
        config: &'a RouteArbConfigV1,
        item: &'a RouteWorkItem,
    ) -> Pin<Box<dyn Future<Output = Result<RouteCandidateReportV1, String>> + 'a>>;
    fn prepare<'a>(
        &'a self,
        request: &'a RuntimeRequest,
    ) -> Pin<Box<dyn Future<Output = Result<AdapterIntent, String>> + 'a>>;
    fn submit<'a>(
        &'a self,
        intent: &'a AdapterIntent,
    ) -> Pin<Box<dyn Future<Output = RuntimeSubmissionOutcome> + 'a>>;
    fn reconcile<'a>(
        &'a self,
        intent: &'a AdapterIntent,
    ) -> Pin<Box<dyn Future<Output = Result<Option<RuntimeSettlement>, String>> + 'a>>;
}
struct LiveIo;
impl RuntimeIo for LiveIo {
    fn now(&self) -> u64 {
        ic_cdk::api::time()
    }
    fn owner(&self) -> Principal {
        ic_cdk::id()
    }
    fn quote<'a>(
        &'a self,
        c: &'a RouteArbConfigV1,
        item: &'a RouteWorkItem,
    ) -> Pin<Box<dyn Future<Output = Result<RouteCandidateReportV1, String>> + 'a>> {
        Box::pin(async move {
            let rows =
                quote_observation_items(c, std::slice::from_ref(item), &quote_reservations()).await;
            rows.into_iter()
                .next()
                .map(|r| r.0)
                .ok_or_else(|| "quote unavailable".into())
        })
    }
    fn prepare<'a>(
        &'a self,
        r: &'a RuntimeRequest,
    ) -> Pin<Box<dyn Future<Output = Result<AdapterIntent, String>> + 'a>> {
        Box::pin(async move {
            match r.edge.venue {
                VenueKind::Rumi3Pool => {
                    crate::route_rumi::prepare(r).await.map(AdapterIntent::Rumi)
                }
                VenueKind::IcpSwap => crate::route_icpswap::prepare(r)
                    .await
                    .map(AdapterIntent::IcpSwap),
            }
        })
    }
    fn submit<'a>(
        &'a self,
        i: &'a AdapterIntent,
    ) -> Pin<Box<dyn Future<Output = RuntimeSubmissionOutcome> + 'a>> {
        Box::pin(async move {
            match i {
                AdapterIntent::Rumi(i) => crate::route_rumi::submit_once_outcome(i).await,
                AdapterIntent::IcpSwap(i) => match crate::route_icpswap::submit_once(i).await {
                    Ok(()) => RuntimeSubmissionOutcome::Accepted,
                    Err(e) => RuntimeSubmissionOutcome::Unknown(e),
                },
            }
        })
    }
    fn reconcile<'a>(
        &'a self,
        i: &'a AdapterIntent,
    ) -> Pin<Box<dyn Future<Output = Result<Option<RuntimeSettlement>, String>> + 'a>> {
        Box::pin(async move {
            match i {
                AdapterIntent::Rumi(i) => crate::route_rumi::reconcile(i).await,
                AdapterIntent::IcpSwap(i) => crate::route_icpswap::reconcile(i).await,
            }
        })
    }
}
fn native(v: u128) -> Result<u64, String> {
    u64::try_from(v).map_err(|_| "native amount overflow".into())
}
fn final_floor(c: &RouteArbConfigV1, original: &RouteCandidateReportV1) -> Result<u64, String> {
    let p = original.principal_native;
    let (basis, absolute, bps) = if original.start_asset == Asset::Icp {
        (
            p,
            u128::from(c.min_icp_profit_e8s),
            u128::from(c.min_icp_profit_bps),
        )
    } else {
        (
            u128::try_from(par_usd_6dec_checked(p, original.start_asset.decimals())?)
                .map_err(|_| "invalid basis")?,
            u128::from(c.min_stable_profit_usd_6dec),
            u128::from(c.min_stable_profit_bps),
        )
    };
    let relative = basis
        .checked_mul(bps)
        .ok_or("profit floor overflow")?
        .checked_add(9_999)
        .ok_or("profit floor overflow")?
        / 10_000;
    let end = basis
        .checked_add(absolute.max(relative))
        .ok_or("final floor overflow")?;
    if original.end_asset == Asset::Icp {
        native(end)
    } else {
        native(
            end.checked_mul(10u128.pow(u32::from(original.end_asset.decimals() - 6)))
                .ok_or("native floor overflow")?,
        )
    }
}
fn validate_quote(
    q: &RouteCandidateReportV1,
    now: u64,
    c: &RouteArbConfigV1,
    whole: bool,
) -> Result<(), String> {
    checked_quote_age(now, q.quote_timestamp_ns, c.quote_max_age_ns)?;
    if q.legs.is_empty() || !q.full_fill || q.allowance_status != "sufficient" {
        return Err(q
            .rejection_reason
            .clone()
            .unwrap_or_else(|| "quote lacks full-fill/allowance proof".into()));
    }
    if !q.eligible {
        let tail_profit_only = !whole
            && q.rejection_reason.as_deref().is_some_and(|r| {
                r == "candidate endpoints do not share an admitted profit domain"
                    || r.starts_with("below stable")
                    || r.starts_with("below ICP")
            });
        if !tail_profit_only {
            return Err(q
                .rejection_reason
                .clone()
                .unwrap_or_else(|| "quote ineligible".into()));
        }
    }
    Ok(())
}
fn route_item(ex: &RuntimeExecution, tail: bool) -> Result<RouteWorkItem, String> {
    let universe = build_work_universe(&config().0)?;
    let mut item = universe
        .items
        .into_iter()
        .find(|i| {
            i.route.route_id == ex.original.route_id
                && i.size_ladder_index == ex.original.size_ladder_index
        })
        .ok_or("persisted route no longer in admitted universe")?;
    if tail {
        let index = usize::from(ex.record.current_leg_index);
        item.route.edges = item.route.edges[index..].to_vec();
        item.route.asset_path = item.route.asset_path[index..].to_vec();
        item.principal_native = ex.current_wallet_native;
    }
    Ok(item)
}
fn fingerprint<T: Serialize>(value: &T) -> Result<[u8; 32], String> {
    Ok(Sha256::digest(serde_json::to_vec(value).map_err(|e| e.to_string())?).into())
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn trace_mut(ex: &mut RuntimeExecution, leg_index: u8) -> Result<&mut RuntimeLegTrace, String> {
    ex.leg_traces
        .iter_mut()
        .find(|trace| trace.leg_index == leg_index)
        .ok_or_else(|| format!("missing route execution trace for leg {leg_index}"))
}
fn initial_leg_traces(
    candidate: &RouteCandidateReportV1,
    edges: &[DirectedEdge],
) -> Result<Vec<RuntimeLegTrace>, String> {
    if candidate.legs.len() != edges.len() || candidate.asset_path.len() != edges.len() + 1 {
        return Err("route quote legs do not match resolved route edges".into());
    }
    candidate
        .legs
        .iter()
        .zip(edges)
        .enumerate()
        .map(|(index, (leg, edge))| {
            if leg.edge_id != edge.edge_id
                || leg.from != edge.from
                || leg.to != edge.to
            {
                return Err(format!("route quote edge does not match selected route at leg {index}"));
            }
            Ok(RuntimeLegTrace {
                leg_index: index as u8,
                edge: edge.into(),
                quoted_input_native: native(leg.venue_input)?,
                quoted_output_native: Some(native(leg.gross_output)?),
                minimum_output_native: native(leg.wallet_after)?,
                input_fee_native: native(leg.entry_ledger_fee)?,
                output_fee_native: native(leg.output_ledger_fee)?,
                submitted_at_ns: None,
                settled_at_ns: None,
                reconciled_at_ns: None,
                settlement: None,
                status: RouteExecutionLegStatusV1::Quoted,
                incident: None,
            })
        })
        .collect()
}
async fn prepare_next<I: RuntimeIo>(
    io: &I,
    ex: &mut RuntimeExecution,
    q: RouteCandidateReportV1,
) -> Result<(), String> {
    guard_generation(ex)?;
    authorized()?;
    let item = route_item(ex, true)?;
    let leg = q.legs.first().ok_or("missing quoted leg")?;
    let edge = item.route.edges.first().ok_or("missing route edge")?;
    if q.principal_native != u128::from(ex.current_wallet_native)
        || leg.wallet_before != u128::from(ex.current_wallet_native)
        || q.legs.iter().map(|l| l.edge_id.as_str()).ne(item
            .route
            .edges
            .iter()
            .map(|e| e.edge_id.as_str()))
        || q.legs.len() != item.route.edges.len()
    {
        return Err("quote changed exact route identity".into());
    }
    if native(q.legs.last().unwrap().wallet_after)? < ex.minimum_final_native {
        return Err("remaining route below original principal profit floor".into());
    }
    let id = fingerprint(&(
        ex.record.execution_id.clone(),
        ex.record.current_leg_index,
        ex.generation,
    ))?;
    let req = RuntimeRequest {
        intent_id: id,
        execution_id: ex.record.execution_id.clone(),
        leg_index: ex.record.current_leg_index,
        owner: io.owner(),
        edge: edge.into(),
        input_native: native(leg.venue_input)?,
        min_output_native: native(leg.wallet_after)?,
        input_fee_native: native(leg.entry_ledger_fee)?,
        output_fee_native: native(leg.output_ledger_fee)?,
        prepared_at_ns: io.now(),
    };
    // A conservative exact quoted-output floor is stronger than the backwards
    // route-wide floor; never lower the original final principal/profit target.
    let intent = io.prepare(&req).await?;
    guard_generation(ex)?;
    authorized()?;
    checked_quote_age(io.now(), q.quote_timestamp_ns, config().0.quote_max_age_ns)?;
    ex.record.phase = ExecutionPhaseV1::LegPrepared;
    ex.record.quote_timestamp_ns = q.quote_timestamp_ns;
    ex.record.planned_input_native = req.input_native;
    ex.record.required_min_output_native = req.min_output_native;
    ex.record.adapter_request_fingerprint = Some(hex(&fingerprint(&intent)?));
    ex.record.submission_started_at_ns = None;
    ex.record.updated_at_ns = io.now();
    ex.request = Some(req.clone());
    ex.intent = Some(intent);
    let leg_index = ex.record.current_leg_index;
    let trace = trace_mut(ex, leg_index)?;
    trace.quoted_input_native = req.input_native;
    trace.quoted_output_native = Some(native(leg.gross_output)?);
    trace.minimum_output_native = req.min_output_native;
    trace.input_fee_native = req.input_fee_native;
    trace.output_fee_native = req.output_fee_native;
    trace.status = RouteExecutionLegStatusV1::Prepared;
    trace.incident = None;
    persist(ex)
}
pub async fn prepare(route_id: &str) -> Result<ExecutionRecordV1, String> {
    prepare_with(&LiveIo, route_id).await
}
pub async fn prepare_with<I: RuntimeIo>(
    io: &I,
    route_id: &str,
) -> Result<ExecutionRecordV1, String> {
    let _busy = BusyGuard::enter()?;
    authorized()?;
    let mut s = load()?;
    if s.current.is_some() {
        return Err("one route is already active".into());
    }
    let (c, generation) = config();
    state::admit_route_capacity(&c)?;
    let selected = state::read_state(|s| {
        s.route_observation
            .as_ref()
            .filter(|o| o.scan_complete)
            .and_then(|o| {
                [
                    o.best_stable_candidate.as_ref(),
                    o.best_icp_candidate.as_ref(),
                ]
                .into_iter()
                .flatten()
                .find(|q| q.route_id == route_id)
                .cloned()
            })
    })
    .ok_or("route is not a completed observation selection")?;
    // Observation is only a selection hint; the mandatory locked quote below
    // establishes freshness even when a complete scan took longer than TTL.
    if !selected.eligible || !selected.full_fill {
        return Err("selection is not eligible".into());
    }
    let admitted = enumerate_routes(c.max_route_legs)?;
    for offset in [0, 100, 200] {
        for held in state::get_held_positions_page(offset, 100)? {
            let same_cycle = selected.canonical_cycle_id.as_ref().is_some_and(|id| {
                admitted
                    .iter()
                    .find(|r| r.route_id == held.originating_route_id)
                    .and_then(|r| r.canonical_cycle_id.as_ref())
                    == Some(id)
            });
            if held.originating_route_id == selected.route_id || same_cycle {
                return Err("route or canonical cycle has reserved held inventory".into());
            }
        }
    }
    let minimum_final_native = final_floor(&c, &selected)?;
    let current_wallet_native = native(selected.principal_native)?;
    let resolved_edges = resolve_route_edges(&selected.venue_edges, &selected.asset_path)?;
    let leg_traces = initial_leg_traces(&selected, &resolved_edges)?;
    s.sequence = s
        .sequence
        .checked_add(1)
        .ok_or("execution sequence exhausted")?;
    let execution_id = format!("route-execution-{}-{}", generation, s.sequence);
    let mut ex = RuntimeExecution {
        record: prepare_execution(&selected, &execution_id, io.now())?,
        generation,
        minimum_final_native,
        current_wallet_native,
        original: selected,
        request: None,
        intent: None,
        settlements: vec![],
        submitted_intents: vec![],
        realized_profit: None,
        leg_traces,
    };
    state::acquire_mutation_lock(&execution_id, MutationOwnerV1::RouteExecution, io.now())?;
    // Persist planned route and sequence before the first read-only await too.
    s.current = Some(ex.clone());
    save(&s)?;
    state::put_current_route_execution(ex.record.clone())?;
    // Establish the detail row before the first quote await. A crash or
    // upgrade at that boundary must leave an explicit, quote-only read model.
    state::put_route_execution_detail(detail_for(&ex))?;
    reserve_active(&ex, true)?;
    let result = async {
        let item = route_item(&ex, false)?;
        let q = io.quote(&c, &item).await?;
        guard_generation(&ex)?;
        validate_quote(&q, io.now(), &c, true)?;
        prepare_next(io, &mut ex, q).await
    }
    .await;
    if let Err(e) = result {
        ex.record.incident = Some(e.chars().take(512).collect());
        ex.record.phase = ExecutionPhaseV1::Aborted;
        finish(&mut ex, io.now())?;
        return Err(e);
    }
    Ok(ex.record)
}
fn held_basis(ex: &RuntimeExecution) -> Result<HeldBasisV1, String> {
    if ex.original.start_asset == Asset::Icp {
        Ok(HeldBasisV1::IcpNative {
            principal_icp_e8s: native(ex.original.principal_native)?,
        })
    } else {
        Ok(HeldBasisV1::StablePar {
            start_asset: ex.original.start_asset,
            principal_native: native(ex.original.principal_native)?,
            principal_usd_6dec: u64::try_from(par_usd_6dec_checked(
                ex.original.principal_native,
                ex.original.start_asset.decimals(),
            )?)
            .map_err(|_| "basis overflow")?,
        })
    }
}
fn hold(
    ex: &mut RuntimeExecution,
    mut lots: Vec<HeldLotV1>,
    reason: String,
    now: u64,
) -> Result<(), String> {
    if lots.is_empty() {
        return Err("cannot hold without reconciled inventory lots".into());
    }
    if lots.len() == 1 {
        if let Some((r, _)) = ex.submitted_intents.last() {
            if lots[0].asset == r.edge.to {
                lots[0].attributable_fees_native = r.output_fee_native;
            }
        }
    }
    state::put_held_position(HeldPositionV1 {
        position_id: format!("held:{}", ex.record.execution_id),
        originating_execution_id: ex.record.execution_id.clone(),
        originating_route_id: ex.record.route_id.clone(),
        basis: held_basis(ex)?,
        lots,
        reason: reason.chars().take(512).collect(),
        first_held_timestamp_ns: now,
        last_reconciled_timestamp_ns: now,
    })?;
    ex.record.phase = ExecutionPhaseV1::HeldInventory;
    ex.record.incident = Some(reason.chars().take(512).collect());
    finish(ex, now)
}
fn lot(asset: Asset, amount: u64) -> HeldLotV1 {
    HeldLotV1 {
        asset,
        native_amount: amount,
        attributable_fees_native: 0,
        reserved_native: amount,
    }
}
fn finish(ex: &mut RuntimeExecution, now: u64) -> Result<(), String> {
    ex.record.updated_at_ns = now;
    ex.record.realized_profit = ex.realized_profit;
    if let Some(trace) = ex
        .leg_traces
        .iter_mut()
        .find(|trace| trace.leg_index == ex.record.current_leg_index)
    {
        match ex.record.phase {
            ExecutionPhaseV1::Aborted => trace.status = RouteExecutionLegStatusV1::Aborted,
            ExecutionPhaseV1::HeldInventory => {
                trace.status = RouteExecutionLegStatusV1::HeldInventory
            }
            _ => {}
        }
        trace.incident = ex.record.incident.clone();
    }
    persist(ex)?;
    state::complete_current_route_execution(ex.record.clone())?;
    reserve_active(ex, false)?;
    if state::get_mutation_lock().is_some() {
        state::release_reconciled_route_lock(&ex.record.execution_id)?;
    }
    let mut s = load()?;
    s.current = None;
    s.last_served_icp = ex.original.candidate_class == CandidateClass::IcpReturning;
    s.last_terminal = Some(ex.clone());
    save(&s)?;
    state::mutate_state(|s| s.route_observation = None);
    Ok(())
}
pub fn has_current() -> Result<bool, String> {
    Ok(load()?.current.is_some())
}
fn current(id: &str) -> Result<RuntimeExecution, String> {
    let ex = load()?.current.ok_or("no active runtime execution")?;
    if ex.record.execution_id != id {
        return Err("execution id mismatch".into());
    }
    Ok(ex)
}
pub async fn advance(id: &str) -> Result<ExecutionRecordV1, String> {
    advance_with(&LiveIo, id).await
}
pub async fn advance_with<I: RuntimeIo>(io: &I, id: &str) -> Result<ExecutionRecordV1, String> {
    let _busy = BusyGuard::enter()?;
    let mut ex = current(id)?;
    match ex.record.phase {
        ExecutionPhaseV1::LegSubmitted
        | ExecutionPhaseV1::AwaitingSettlement
        | ExecutionPhaseV1::ReconciliationRequired => return reconcile_inner(io, ex).await,
        ExecutionPhaseV1::LegSettled => {
            let next = usize::from(ex.record.current_leg_index) + 1;
            if next == ex.original.legs.len() {
                ex.record.phase = ExecutionPhaseV1::Completed;
                ex.realized_profit = Some(if ex.original.start_asset == Asset::Icp {
                    i128::from(ex.current_wallet_native)
                        - i128::try_from(ex.original.principal_native)
                            .map_err(|_| "basis overflow")?
                } else {
                    par_usd_6dec_checked(
                        u128::from(ex.current_wallet_native),
                        ex.original.end_asset.decimals(),
                    )? - par_usd_6dec_checked(
                        ex.original.principal_native,
                        ex.original.start_asset.decimals(),
                    )?
                });
                finish(&mut ex, io.now())?;
                return Ok(ex.record);
            }
            let asset = ex.original.asset_path[next];
            if let Err(e) = guard_generation(&ex).and_then(|_| authorized()) {
                let amount = ex.current_wallet_native;
                hold(&mut ex, vec![lot(asset, amount)], e, io.now())?;
                return Ok(ex.record);
            }
            ex.record.current_leg_index += 1;
            ex.record.phase = ExecutionPhaseV1::RemainingRouteRequoted;
            persist(&ex)?;
        }
        ExecutionPhaseV1::LegPrepared
        | ExecutionPhaseV1::RemainingRouteRequoted
        | ExecutionPhaseV1::Planned => {}
        phase if phase.is_terminal() => {
            // Recovery after a returned storage error during terminal finalization.
            // HeldInventory was persisted only after all held reservations.
            finish(&mut ex, io.now())?;
            return Ok(ex.record);
        }
        _ => return Err("unsupported route phase".into()),
    }
    if ex.record.phase != ExecutionPhaseV1::LegPrepared {
        let result = async {
            guard_generation(&ex)?;
            authorized()?;
            let c = config().0;
            let item = route_item(&ex, true)?;
            let q = io.quote(&c, &item).await?;
            guard_generation(&ex)?;
            validate_quote(&q, io.now(), &c, false)?;
            prepare_next(io, &mut ex, q).await
        }
        .await;
        if let Err(e) = result {
            if ex.settlements.is_empty() {
                ex.record.phase = ExecutionPhaseV1::Aborted;
                ex.record.incident = Some(e);
                finish(&mut ex, io.now())?;
            } else {
                let asset = ex.original.asset_path[usize::from(ex.record.current_leg_index)];
                let amount = ex.current_wallet_native;
                hold(&mut ex, vec![lot(asset, amount)], e, io.now())?;
            }
        }
        return Ok(ex.record);
    }
    if let Err(e) = guard_generation(&ex)
        .and_then(|_| authorized())
        .and_then(|_| {
            checked_quote_age(
                io.now(),
                ex.record.quote_timestamp_ns,
                config().0.quote_max_age_ns,
            )
            .map(|_| ())
        })
    {
        if ex.settlements.is_empty() {
            ex.record.phase = ExecutionPhaseV1::Aborted;
            ex.record.incident = Some(e);
            finish(&mut ex, io.now())?;
        } else {
            let asset = ex.original.asset_path[usize::from(ex.record.current_leg_index)];
            let amount = ex.current_wallet_native;
            hold(&mut ex, vec![lot(asset, amount)], e, io.now())?;
        }
        return Ok(ex.record);
    }
    state::admit_route_capacity(&config().0)?;
    // This write is the economic idempotency boundary. A trap/upgrade after it
    // is conservative: all subsequent dispatches reconcile this exact intent.
    let leg_index = ex.record.current_leg_index;
    ex.submitted_intents.push((
        ex.request.clone().ok_or("prepared request missing")?,
        ex.intent.clone().ok_or("prepared intent missing")?,
    ));
    let submitted_at_ns = io.now();
    let trace = trace_mut(&mut ex, leg_index)?;
    trace.status = RouteExecutionLegStatusV1::Submitted;
    trace.submitted_at_ns = Some(submitted_at_ns);
    trace.incident = None;
    ex.record.phase = ExecutionPhaseV1::LegSubmitted;
    ex.record.submission_started_at_ns = Some(submitted_at_ns);
    ex.record.updated_at_ns = io.now();
    persist(&ex)?;
    state::mutate_state(|s| s.route_observation = None);
    let response = io
        .submit(ex.intent.as_ref().ok_or("prepared intent missing")?)
        .await;
    ex.record.phase = ExecutionPhaseV1::AwaitingSettlement;
    match response {
        RuntimeSubmissionOutcome::Accepted => {
            ex.record.incident = None;
            let trace = trace_mut(&mut ex, leg_index)?;
            trace.status = RouteExecutionLegStatusV1::AwaitingSettlement;
        }
        RuntimeSubmissionOutcome::Unknown(e) => {
            let incident = e.chars().take(512).collect::<String>();
            ex.record.incident = Some(incident.clone());
            let trace = trace_mut(&mut ex, leg_index)?;
            trace.status = RouteExecutionLegStatusV1::ReconciliationRequired;
            trace.incident = Some(incident.clone());
        }
        RuntimeSubmissionOutcome::RejectedBeforeDebit(e) => {
            let incident = e.chars().take(512).collect::<String>();
            ex.record.incident = Some(incident.clone());
            let trace = trace_mut(&mut ex, leg_index)?;
            trace.status = RouteExecutionLegStatusV1::RejectedBeforeDebit;
            trace.incident = Some(incident.clone());
            ex.record.updated_at_ns = io.now();
            persist(&ex)?;
            if ex.settlements.is_empty() {
                ex.record.phase = ExecutionPhaseV1::Aborted;
                ex.realized_profit = Some(0);
                let trace = trace_mut(&mut ex, leg_index)?;
                trace.status = RouteExecutionLegStatusV1::Aborted;
                finish(&mut ex, io.now())?;
            } else {
                let amount = ex.current_wallet_native;
                let asset = ex.request.as_ref().ok_or("request missing")?.edge.from;
                let trace = trace_mut(&mut ex, leg_index)?;
                trace.status = RouteExecutionLegStatusV1::HeldInventory;
                hold(&mut ex, vec![lot(asset, amount)], incident, io.now())?;
            }
            return Ok(ex.record);
        }
    }
    ex.record.updated_at_ns = io.now();
    persist(&ex)?;
    Ok(ex.record)
}
pub async fn reconcile(id: &str) -> Result<ExecutionRecordV1, String> {
    reconcile_with(&LiveIo, id).await
}
pub async fn reconcile_with<I: RuntimeIo>(io: &I, id: &str) -> Result<ExecutionRecordV1, String> {
    let _busy = BusyGuard::enter()?;
    reconcile_inner(io, current(id)?).await
}
async fn reconcile_inner<I: RuntimeIo>(
    io: &I,
    mut ex: RuntimeExecution,
) -> Result<ExecutionRecordV1, String> {
    if !matches!(
        ex.record.phase,
        ExecutionPhaseV1::LegSubmitted
            | ExecutionPhaseV1::AwaitingSettlement
            | ExecutionPhaseV1::ReconciliationRequired
    ) {
        return Ok(ex.record);
    }
    let limit = config().0.reconciliation_queries_per_cycle;
    if limit < 3 {
        return Err("adapter reconciliation needs a bounded budget of at least three reads".into());
    }
    let result = io
        .reconcile(ex.intent.as_ref().ok_or("submitted intent missing")?)
        .await;
    ex.record.reconciliation_query_count = 3;
    ex.record.updated_at_ns = io.now();
    let settlement = match result {
        Ok(Some(s)) => s,
        other => {
            let incident: String = match other {
                Err(e) => e.chars().take(512).collect(),
                _ => "source-bound receipt is not yet complete".into(),
            };
            let leg_index = ex.record.current_leg_index;
            ex.record.incident = Some(incident.clone());
            let trace = trace_mut(&mut ex, leg_index)?;
            trace.status = RouteExecutionLegStatusV1::AwaitingSettlement;
            trace.incident = Some(incident);
            if io
                .now()
                .saturating_sub(ex.record.submission_started_at_ns.unwrap_or(io.now()))
                >= config().0.settlement_timeout_ns
            {
                ex.record.phase = ExecutionPhaseV1::ReconciliationRequired;
                let trace = trace_mut(&mut ex, leg_index)?;
                trace.status = RouteExecutionLegStatusV1::ReconciliationRequired;
                state::mark_mutation_lock_reconciliation_required(&ex.record.execution_id)?;
            } else {
                ex.record.phase = ExecutionPhaseV1::AwaitingSettlement;
            }
            persist(&ex)?;
            return Ok(ex.record);
        }
    };
    let r = ex.request.as_ref().ok_or("request missing")?.clone();
    if settlement.effective_input_native > r.input_native
        || settlement.input_debit_native
            > r.input_native
                .checked_add(r.input_fee_native)
                .ok_or("debit overflow")?
        || settlement.refund_credit_native > r.input_native
    {
        return Err("adapter settlement violates native request limits".into());
    }
    if settlement.evidence.is_empty() {
        return Err("adapter settlement lacks source receipt evidence".into());
    }
    if ex.record.evidence.len() + settlement.evidence.len()
        > usize::from(config().0.max_reconciliation_evidence_items)
    {
        return Err("settlement evidence capacity exhausted; lock retained".into());
    }
    ex.record.evidence.extend(settlement.evidence.clone());
    ex.settlements.push(settlement.clone());
    ex.record.incident = None;
    let leg_index = ex.record.current_leg_index;
    let settled_at_ns = settlement.evidence.iter().map(|e| e.observed_at_ns).max();
    {
        let trace = trace_mut(&mut ex, leg_index)?;
        trace.settlement = Some(settlement.clone());
        trace.settled_at_ns = settled_at_ns;
        trace.reconciled_at_ns = settled_at_ns;
        trace.incident = None;
    }
    if settlement.effective_input_native != r.input_native
        || settlement.refund_credit_native > 0
        || settlement.output_credit_native < r.min_output_native
    {
        let unspent = ex
            .current_wallet_native
            .checked_sub(settlement.input_debit_native)
            .ok_or("source debit exceeds route wallet")?;
        let returned = unspent
            .checked_add(settlement.refund_credit_native)
            .ok_or("refund overflow")?;
        if settlement.output_credit_native == 0
            && r.edge.from == ex.original.start_asset
            && ex.settlements.len() == 1
        {
            ex.record.phase = ExecutionPhaseV1::Aborted;
            let trace = trace_mut(&mut ex, leg_index)?;
            trace.status = RouteExecutionLegStatusV1::Aborted;
            ex.realized_profit = Some(if r.edge.from == Asset::Icp {
                i128::from(returned)
                    - i128::try_from(ex.original.principal_native).map_err(|_| "basis overflow")?
            } else {
                par_usd_6dec_checked(u128::from(returned), r.edge.from.decimals())?
                    - par_usd_6dec_checked(ex.original.principal_native, r.edge.from.decimals())?
            });
            finish(&mut ex, io.now())?;
        } else {
            let trace = trace_mut(&mut ex, leg_index)?;
            trace.status = if settlement.refund_credit_native > 0 {
                RouteExecutionLegStatusV1::Refunded
            } else {
                RouteExecutionLegStatusV1::HeldInventory
            };
            trace.incident = Some("fully reconciled partial fill or insufficient output".into());
            let mut lots = vec![];
            if returned > 0 {
                let mut input_lot = lot(r.edge.from, returned);
                input_lot.attributable_fees_native = settlement
                    .input_debit_native
                    .checked_sub(settlement.effective_input_native)
                    .and_then(|n| n.checked_sub(settlement.refund_credit_native))
                    .ok_or("refund accounting exceeds source debit")?;
                lots.push(input_lot);
            }
            if settlement.output_credit_native > 0 {
                let mut output_lot = lot(r.edge.to, settlement.output_credit_native);
                output_lot.attributable_fees_native = r.output_fee_native;
                lots.push(output_lot);
            }
            hold(
                &mut ex,
                lots,
                "fully reconciled partial fill or insufficient output".into(),
                io.now(),
            )?;
        }
    } else {
        if settlement.input_debit_native
            != r.input_native
                .checked_add(r.input_fee_native)
                .ok_or("debit overflow")?
        {
            return Err("full settlement does not account for exact input fee".into());
        }
        ex.current_wallet_native = settlement.output_credit_native;
        ex.record.phase = ExecutionPhaseV1::LegSettled;
        let trace = trace_mut(&mut ex, leg_index)?;
        trace.status = RouteExecutionLegStatusV1::Settled;
        persist(&ex)?;
    }
    Ok(ex.record)
}
/// One scheduler dispatch: reconcile/advance existing execution before selection.
pub async fn service_tick() -> Result<(), String> {
    let mut s = load()?;
    s.last_tick_ns = ic_cdk::api::time();
    save(&s)?;
    let result = if let Some(ex) = s.current {
        advance(&ex.record.execution_id).await.map(|_| ())
    } else if authorized().is_ok() {
        let prefer_stable = s.last_served_icp;
        let selected = state::read_state(|s| {
            s.route_observation
                .as_ref()
                .filter(|o| o.scan_complete)
                .and_then(|o| {
                    if prefer_stable {
                        o.best_stable_candidate
                            .as_ref()
                            .or(o.best_icp_candidate.as_ref())
                    } else {
                        o.best_icp_candidate
                            .as_ref()
                            .or(o.best_stable_candidate.as_ref())
                    }
                })
                .map(|q| q.route_id.clone())
        });
        match selected {
            Some(id) => {
                let result = prepare(&id).await.map(|_| ());
                if result.is_err() {
                    state::mutate_state(|s| s.route_observation = None);
                }
                result
            }
            None => Ok(()),
        }
    } else {
        Ok(())
    };
    let mut s = load()?;
    s.last_error = result.as_ref().err().map(|e| e.chars().take(512).collect());
    save(&s)?;
    result
}

/// Scheduler observability also covers read-only observation failures.
pub fn note_scheduler_result(result: &Result<(), String>) -> Result<(), String> {
    let mut s = load()?;
    s.last_tick_ns = ic_cdk::api::time();
    s.last_error = result.as_ref().err().map(|e| e.chars().take(512).collect());
    save(&s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use std::cell::RefCell;
    #[derive(Default)]
    struct Double {
        now: Cell<u64>,
        submissions: Cell<u32>,
        quotes: Cell<u32>,
        pending: Cell<bool>,
        reject: Cell<bool>,
        partial: Cell<bool>,
        refunded: Cell<bool>,
        fail_quote: Cell<bool>,
        change_generation: Cell<bool>,
        accepted: Cell<bool>,
        calls: RefCell<Vec<RuntimeRequest>>,
    }
    fn quoted(item: &RouteWorkItem, now: u64) -> RouteCandidateReportV1 {
        let mut wallet = u128::from(item.principal_native);
        let mut legs = vec![];
        for edge in &item.route.edges {
            // Convert nominal units then add a deterministic 10% net quote gain.
            let output = wallet * 10u128.pow(u32::from(edge.to.decimals()))
                / 10u128.pow(u32::from(edge.from.decimals()))
                * 11
                / 10;
            legs.push(QuoteLegReportV1 {
                edge_id: edge.edge_id.clone(),
                from: edge.from,
                to: edge.to,
                wallet_before: wallet,
                entry_ledger_fee: 10,
                venue_input: wallet - 10,
                gross_output: output + 10,
                output_ledger_fee: 10,
                wallet_after: output,
                full_fill: true,
            });
            wallet = output;
        }
        RouteCandidateReportV1 {
            route_id: item.route.route_id.clone(),
            canonical_cycle_id: item.route.canonical_cycle_id.clone(),
            candidate_class: item.route.candidate_class,
            asset_path: item.route.asset_path.clone(),
            venue_edges: item.route.edges.iter().map(|e| e.edge_id.clone()).collect(),
            start_asset: item.route.start_asset(),
            end_asset: item.route.end_asset(),
            principal_native: u128::from(item.principal_native),
            net_profit_native: 100_000,
            net_profit_bps: 1000,
            size_ladder_index: item.size_ladder_index,
            par_assumption: item.route.start_asset() != item.route.end_asset(),
            full_fill: true,
            allowance_status: "sufficient".into(),
            inventory_effect: "within configured bands".into(),
            quote_timestamp_ns: now,
            legs,
            eligible: true,
            rejection_reason: None,
        }
    }
    impl RuntimeIo for Double {
        fn now(&self) -> u64 {
            self.now.get()
        }
        fn owner(&self) -> Principal {
            Principal::from_slice(&[1])
        }
        fn quote<'a>(
            &'a self,
            _: &'a RouteArbConfigV1,
            item: &'a RouteWorkItem,
        ) -> Pin<Box<dyn Future<Output = Result<RouteCandidateReportV1, String>> + 'a>> {
            Box::pin(async move {
                assert!(
                    state::get_mutation_lock().is_some(),
                    "whole/tail quote must hold account lock"
                );
                let execution = state::get_current_route_execution().expect("planned record");
                let detail = state::get_route_execution_detail(&execution.execution_id)
                    .unwrap()
                    .expect("detail must exist before first quote await");
                assert!(detail.detail_available);
                self.quotes.set(self.quotes.get() + 1);
                if self.change_generation.get() {
                    state::mutate_state(|s| s.route_arb_config_generation += 1);
                }
                if self.fail_quote.get() {
                    return Err("quote unavailable".into());
                }
                Ok(quoted(item, self.now()))
            })
        }
        fn prepare<'a>(
            &'a self,
            r: &'a RuntimeRequest,
        ) -> Pin<Box<dyn Future<Output = Result<AdapterIntent, String>> + 'a>> {
            Box::pin(async move {
                // Test double persists the exact request using the production typed intent.
                self.calls.borrow_mut().push(r.clone());
                Ok(AdapterIntent::Rumi(crate::route_rumi::Intent {
                    pool: r.edge.pool_principal,
                    owner: r.owner,
                    edge_id: r.edge.edge_id.clone(),
                    request: crate::route_rumi::SwapRequestV1 {
                        intent_id: r.intent_id.to_vec(),
                        i: r.edge.from.index() as u8,
                        j: r.edge.to.index() as u8,
                        dx: u128::from(r.input_native),
                        min_dy: u128::from(r.min_output_native),
                    },
                    input_ledger: asset_pins()[r.edge.from.index()].ledger,
                    output_ledger: asset_pins()[r.edge.to.index()].ledger,
                    input_fee: r.input_fee_native,
                    output_fee: r.output_fee_native,
                }))
            })
        }
        fn submit<'a>(
            &'a self,
            _: &'a AdapterIntent,
        ) -> Pin<Box<dyn Future<Output = RuntimeSubmissionOutcome> + 'a>> {
            Box::pin(async move {
                let ex = load().unwrap().current.unwrap();
                assert_eq!(ex.record.phase, ExecutionPhaseV1::LegSubmitted);
                assert!(ex.intent.is_some() && ex.request.is_some());
                assert!(state::get_mutation_lock().is_some());
                // Reopen the actual stable cell at the outbound boundary, as after upgrade.
                state::reopen_runtime_cell_for_test();
                assert_eq!(
                    load().unwrap().current.unwrap().record.phase,
                    ExecutionPhaseV1::LegSubmitted
                );
                self.submissions.set(self.submissions.get() + 1);
                if self.reject.get() {
                    RuntimeSubmissionOutcome::RejectedBeforeDebit(
                        "capacity refused before debit".into(),
                    )
                } else if self.accepted.get() {
                    RuntimeSubmissionOutcome::Accepted
                } else {
                    RuntimeSubmissionOutcome::Unknown("lost response".into())
                }
            })
        }
        fn reconcile<'a>(
            &'a self,
            _: &'a AdapterIntent,
        ) -> Pin<Box<dyn Future<Output = Result<Option<RuntimeSettlement>, String>> + 'a>> {
            Box::pin(async move {
                if self.pending.get() {
                    return Ok(None);
                }
                let ex = load().unwrap().current.unwrap();
                let r = ex.request.unwrap();
                let (effective, output, refund) = if self.refunded.get() {
                    (0, 0, r.input_native - r.input_fee_native)
                } else if self.partial.get() {
                    (
                        r.input_native / 2,
                        r.min_output_native / 2,
                        r.input_native - r.input_native / 2 - r.input_fee_native,
                    )
                } else {
                    (r.input_native, r.min_output_native, 0)
                };
                Ok(Some(RuntimeSettlement {
                    input_debit_native: r.input_native + r.input_fee_native,
                    effective_input_native: effective,
                    output_credit_native: output,
                    refund_credit_native: refund,
                    evidence: vec![ReconciliationEvidenceV1 {
                        evidence_kind: "typed test venue receipt".into(),
                        source_reference: hex(&r.intent_id),
                        amount_native: output,
                        observed_at_ns: self.now(),
                    }],
                }))
            })
        }
    }
    fn setup(legs: usize) -> (Double, String) {
        state::init_state(state::BotState::default());
        state::release_mutation_lock_for_test();
        save(&DurableRuntime::default()).unwrap();
        state::mutate_state(|s| {
            s.route_arb.enabled = true;
            s.route_arb.dry_run = false;
        });
        set_authorized(true).unwrap();
        let c = config().0;
        let item = build_work_universe(&c)
            .unwrap()
            .items
            .into_iter()
            .find(|i| {
                i.route.edges.len() == legs
                    && i.route.start_asset() == Asset::CkUsdc
                    && i.route.end_asset().is_stable()
            })
            .unwrap();
        let q = quoted(&item, 1);
        let mut observation =
            ObservationAccumulatorV1::new("fixture-selection".into(), 1, 1, 1, 1, true);
        observation.scan_complete = true;
        observation.best_stable_candidate = Some(q);
        state::mutate_state(|s| s.route_observation = Some(observation));
        let io = Double::default();
        io.now.set(100_000_000_000); // old hint deliberately outside TTL
        (io, item.route.route_id)
    }
    #[test]
    fn real_flow_stable_restart_lost_response_no_replay_and_completion() {
        let (io, route) = setup(1);
        let ex = block_on(prepare_with(&io, &route)).unwrap();
        assert_eq!(io.quotes.get(), 1);
        assert_eq!(io.submissions.get(), 0);
        assert_eq!(ex.phase, ExecutionPhaseV1::LegPrepared);
        block_on(advance_with(&io, &ex.execution_id)).unwrap();
        io.pending.set(true);
        io.now.set(io.now.get() + config().0.settlement_timeout_ns);
        let pending = block_on(advance_with(&io, &ex.execution_id)).unwrap();
        assert_eq!(pending.phase, ExecutionPhaseV1::ReconciliationRequired);
        assert!(state::get_mutation_lock().unwrap().reconciliation_required);
        state::reopen_runtime_cell_for_test();
        set_authorized(false).unwrap(); // reconciliation continues when administratively disabled
        io.pending.set(false);
        assert_eq!(
            block_on(advance_with(&io, &ex.execution_id)).unwrap().phase,
            ExecutionPhaseV1::LegSettled
        );
        assert_eq!(
            block_on(advance_with(&io, &ex.execution_id)).unwrap().phase,
            ExecutionPhaseV1::Completed
        );
        assert_eq!(io.submissions.get(), 1);
        assert!(state::get_mutation_lock().is_none());
        assert!(status().unwrap().last_realized_profit.unwrap() > 0);
        assert!(block_on(advance_with(&io, &ex.execution_id)).is_err());
    }
    #[test]
    fn accepted_submission_persists_awaiting_settlement_leg_status() {
        let (io, route) = setup(1);
        let ex = block_on(prepare_with(&io, &route)).unwrap();
        io.accepted.set(true);
        assert_eq!(
            block_on(advance_with(&io, &ex.execution_id)).unwrap().phase,
            ExecutionPhaseV1::AwaitingSettlement
        );
        let detail = state::get_route_execution_detail(&ex.execution_id)
            .unwrap()
            .expect("execution detail");
        assert_eq!(detail.legs[0].status, RouteExecutionLegStatusV1::AwaitingSettlement);
        assert!(detail.legs[0].submitted_at_ns.is_some());
        assert_eq!(
            block_on(reconcile_with(&io, &ex.execution_id)).unwrap().phase,
            ExecutionPhaseV1::LegSettled
        );
        let settled = state::get_route_execution_detail(&ex.execution_id)
            .unwrap()
            .expect("settled execution detail");
        assert_eq!(settled.legs[0].status, RouteExecutionLegStatusV1::Settled);
        assert!(settled.legs[0].actual_output_credit_native.is_some());
    }
    #[test]
    fn pre_detail_runtime_json_decodes_without_inferred_leg_facts() {
        let (io, route) = setup(1);
        let record = block_on(prepare_with(&io, &route)).unwrap();
        let mut old_runtime: serde_json::Value =
            serde_json::from_slice(&state::runtime_bytes()).unwrap();
        old_runtime["current"]
            .as_object_mut()
            .unwrap()
            .remove("leg_traces");
        state::set_runtime_bytes(serde_json::to_vec(&old_runtime).unwrap()).unwrap();
        state::reopen_runtime_cell_for_test();
        let recovered = load().unwrap().current.unwrap();
        assert_eq!(recovered.record.execution_id, record.execution_id);
        let detail = route_execution_detail(&recovered);
        assert!(!detail.detail_available);
        assert!(detail.legs.is_empty());
    }
    #[test]
    fn changed_generation_during_whole_quote_aborts_and_releases_unused_lock() {
        let (io, route) = setup(1);
        io.change_generation.set(true);
        assert!(block_on(prepare_with(&io, &route))
            .unwrap_err()
            .contains("generation"));
        assert_eq!(io.submissions.get(), 0);
        assert!(state::get_mutation_lock().is_none());
        assert!(!has_current().unwrap());
    }
    #[test]
    fn stale_prepared_quote_does_not_submit() {
        let (io, route) = setup(1);
        let ex = block_on(prepare_with(&io, &route)).unwrap();
        io.now.set(io.now.get() + config().0.quote_max_age_ns + 1);
        assert_eq!(
            block_on(advance_with(&io, &ex.execution_id)).unwrap().phase,
            ExecutionPhaseV1::Aborted
        );
        assert_eq!(io.submissions.get(), 0);
        assert!(state::get_mutation_lock().is_none());
    }
    #[test]
    fn definitive_predebit_rejection_aborts_without_receipt_wait() {
        let (io, route) = setup(1);
        io.reject.set(true);
        let ex = block_on(prepare_with(&io, &route)).unwrap();
        assert_eq!(
            block_on(advance_with(&io, &ex.execution_id)).unwrap().phase,
            ExecutionPhaseV1::Aborted
        );
        assert_eq!(io.submissions.get(), 1);
        assert!(state::get_mutation_lock().is_none());
        assert_eq!(status().unwrap().last_realized_profit, Some(0));
    }
    #[test]
    fn partial_fill_reserves_both_lots_before_terminal_release() {
        let (io, route) = setup(1);
        let ex = block_on(prepare_with(&io, &route)).unwrap();
        block_on(advance_with(&io, &ex.execution_id)).unwrap();
        io.partial.set(true);
        assert_eq!(
            block_on(reconcile_with(&io, &ex.execution_id))
                .unwrap()
                .phase,
            ExecutionPhaseV1::HeldInventory
        );
        assert_eq!(io.submissions.get(), 1);
        let held = state::get_held_positions_page(0, 100).unwrap();
        let position = held
            .iter()
            .find(|p| p.originating_execution_id == ex.execution_id)
            .unwrap();
        assert_eq!(position.lots.len(), 2);
        for l in &position.lots {
            assert_eq!(
                state::reservation_totals_for_asset(l.asset).held,
                l.native_amount
            );
        }
        assert!(state::get_mutation_lock().is_none());
        assert_eq!(status().unwrap().last_realized_profit, None);
    }
    #[test]
    fn settled_tail_requotes_original_basis_and_holds_on_quote_failure() {
        let (io, route) = setup(2);
        let ex = block_on(prepare_with(&io, &route)).unwrap();
        block_on(advance_with(&io, &ex.execution_id)).unwrap();
        block_on(reconcile_with(&io, &ex.execution_id)).unwrap();
        let minimum = load().unwrap().current.unwrap().minimum_final_native;
        io.fail_quote.set(true);
        assert_eq!(
            block_on(advance_with(&io, &ex.execution_id)).unwrap().phase,
            ExecutionPhaseV1::HeldInventory
        );
        let last = load().unwrap().last_terminal.unwrap();
        assert_eq!(last.minimum_final_native, minimum);
        assert_eq!(io.quotes.get(), 2);
        assert_eq!(io.submissions.get(), 1);
        assert!(state::get_mutation_lock().is_none());
    }
    #[test]
    fn complete_multileg_flow_requotes_and_preserves_original_basis() {
        let (io, route) = setup(2);
        let ex = block_on(prepare_with(&io, &route)).unwrap();
        let original_min = load().unwrap().current.unwrap().minimum_final_native;
        block_on(advance_with(&io, &ex.execution_id)).unwrap();
        block_on(reconcile_with(&io, &ex.execution_id)).unwrap();
        let second = block_on(advance_with(&io, &ex.execution_id)).unwrap();
        assert_eq!(second.phase, ExecutionPhaseV1::LegPrepared);
        assert_eq!(io.quotes.get(), 2);
        assert_eq!(
            load().unwrap().current.unwrap().minimum_final_native,
            original_min
        );
        block_on(advance_with(&io, &ex.execution_id)).unwrap();
        block_on(reconcile_with(&io, &ex.execution_id)).unwrap();
        assert_eq!(
            block_on(advance_with(&io, &ex.execution_id)).unwrap().phase,
            ExecutionPhaseV1::Completed
        );
        assert_eq!(io.submissions.get(), 2);
        assert_ne!(
            io.calls.borrow()[0].intent_id,
            io.calls.borrow()[1].intent_id
        );
        assert!(state::get_mutation_lock().is_none());
        for asset in Asset::ALL {
            assert_eq!(state::reservation_totals_for_asset(asset).active_route, 0);
        }
    }
    #[test]
    fn authenticated_full_refund_aborts_with_actual_fee_loss() {
        let (io, route) = setup(1);
        let ex = block_on(prepare_with(&io, &route)).unwrap();
        block_on(advance_with(&io, &ex.execution_id)).unwrap();
        io.refunded.set(true);
        assert_eq!(
            block_on(reconcile_with(&io, &ex.execution_id))
                .unwrap()
                .phase,
            ExecutionPhaseV1::Aborted
        );
        assert_eq!(status().unwrap().last_realized_profit, Some(-20));
        assert!(state::get_mutation_lock().is_none());
        assert_eq!(io.submissions.get(), 1);
    }
    #[test]
    fn changed_generation_after_settlement_holds_before_second_submission() {
        let (io, route) = setup(2);
        let ex = block_on(prepare_with(&io, &route)).unwrap();
        block_on(advance_with(&io, &ex.execution_id)).unwrap();
        block_on(reconcile_with(&io, &ex.execution_id)).unwrap();
        state::mutate_state(|s| s.route_arb_config_generation += 1);
        assert_eq!(
            block_on(advance_with(&io, &ex.execution_id)).unwrap().phase,
            ExecutionPhaseV1::HeldInventory
        );
        assert_eq!(io.submissions.get(), 1);
        assert_eq!(io.quotes.get(), 1);
        assert!(state::get_mutation_lock().is_none());
    }
    #[test]
    fn terminal_checkpoint_retry_does_not_duplicate_archive_or_submit() {
        let (io, route) = setup(1);
        let ex = block_on(prepare_with(&io, &route)).unwrap();
        block_on(advance_with(&io, &ex.execution_id)).unwrap();
        block_on(reconcile_with(&io, &ex.execution_id)).unwrap();
        block_on(advance_with(&io, &ex.execution_id)).unwrap();
        let before = state::get_terminal_route_executions_page(0, 100)
            .unwrap()
            .len();
        let mut durable = load().unwrap();
        durable.current = durable.last_terminal.clone();
        save(&durable).unwrap();
        assert_eq!(
            block_on(advance_with(&io, &ex.execution_id)).unwrap().phase,
            ExecutionPhaseV1::Completed
        );
        assert_eq!(
            state::get_terminal_route_executions_page(0, 100)
                .unwrap()
                .len(),
            before
        );
        assert_eq!(io.submissions.get(), 1);
        assert!(!has_current().unwrap());
    }
    #[test]
    fn default_upgrade_authorization_and_capacity_admission_are_inert() {
        state::init_state(state::BotState::default());
        save(&DurableRuntime::default()).unwrap();
        state::reopen_runtime_cell_for_test();
        assert!(!status().unwrap().live_authorized);
        assert!(status().unwrap().dry_run);
        let (io, route) = setup(1);
        state::mutate_state(|s| s.route_arb.max_open_held_positions = 0);
        assert!(block_on(prepare_with(&io, &route)).is_err());
        assert_eq!(io.submissions.get(), 0);
        assert!(state::get_mutation_lock().is_none());
    }
}
