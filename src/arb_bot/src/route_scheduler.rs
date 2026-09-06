//! Bounded automatic observation and executor service. Never submits a swap
//! itself: submitted attempts are owned and resumed only by route_runtime.
use std::cell::Cell;
use crate::{route_arb::ObservationAccumulatorV1, route_runtime::{self, RuntimeStatus}, state};

#[derive(Debug, PartialEq, Eq)]
pub enum TickAction { Idle, ServiceExecution, StartObservation, QuoteBatch(u64), SelectRoute }

pub fn next_action(status: &RuntimeStatus, has_current: bool, observation: Option<&ObservationAccumulatorV1>) -> TickAction {
    // Pausing new trading must not prevent reconciliation of a previous debit.
    if has_current { return TickAction::ServiceExecution; }
    if !status.live_authorized || !status.enabled || status.dry_run { return TickAction::Idle; }
    match observation {
        None => TickAction::StartObservation,
        Some(o) if !o.scan_complete => TickAction::QuoteBatch(o.next_cursor),
        Some(o) if o.best_stable_candidate.is_some() || o.best_icp_candidate.is_some() => TickAction::SelectRoute,
        Some(_) => TickAction::StartObservation,
    }
}
thread_local! { static BUSY: Cell<bool> = const { Cell::new(false) }; }
struct TickGuard;
impl TickGuard {
    fn enter() -> Option<Self> {
        BUSY.with(|b| if b.replace(true) { None } else { Some(Self) })
    }
}
impl Drop for TickGuard { fn drop(&mut self) { BUSY.with(|b|b.set(false)); } }

/// One batch or one executor transition per timer tick, with no overlapping
/// scheduler batches. Manual observation changes are still checked by cursor
/// and policy generation at the durable commit boundary.
pub async fn tick() -> Result<(), String> {
    let Some(_guard) = TickGuard::enter() else { return Ok(()); };
    let status = route_runtime::status()?;
    let current = route_runtime::has_current()?;
    let observation = state::read_state(|s| s.route_observation.clone());
    let result = match next_action(&status, current, observation.as_ref()) {
        TickAction::Idle => Ok(()),
        TickAction::ServiceExecution | TickAction::SelectRoute => route_runtime::service_tick().await,
        TickAction::StartObservation => crate::start_route_observation_internal().map(|_|()),
        TickAction::QuoteBatch(cursor) => crate::quote_route_observation_batch_internal(cursor, 100).await.map(|_|()),
    };
    route_runtime::note_scheduler_result(&result)?;
    result
}
