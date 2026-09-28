//! Bounded automatic observation and executor service. Never submits a swap
//! itself: submitted attempts are owned and resumed only by route_runtime.
use std::cell::Cell;
use crate::{route_arb::ObservationAccumulatorV1, route_runtime::{self, RuntimeStatus}, state};

pub const ICUSD_PEG_OBSERVATION_INTERVAL_NS: u64 = 600_000_000_000;

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

/// Profile-aware start gate. Durable execution service has priority even
/// while the profile is inactive; stale generic observations cannot quote or
/// select a route after upgrade unless the fixed profile is explicitly set.
pub fn next_action_for_profile(
    status: &RuntimeStatus,
    has_current: bool,
    observation: Option<&ObservationAccumulatorV1>,
    profile_active: bool,
    last_observation_started_ns: Option<u64>,
    now_ns: u64,
) -> TickAction {
    if has_current {
        return TickAction::ServiceExecution;
    }
    if !profile_active {
        return TickAction::Idle;
    }
    if observation.is_none() {
        if let Some(last_started) = last_observation_started_ns {
            let Some(eligible_at) = last_started.checked_add(ICUSD_PEG_OBSERVATION_INTERVAL_NS) else {
                return TickAction::Idle;
            };
            if now_ns < eligible_at {
                return TickAction::Idle;
            }
        }
    }
    next_action(status, false, observation)
}
thread_local! { static BUSY: Cell<Option<u64>> = const { Cell::new(None) }; }
pub fn in_flight_since_ns() -> Option<u64> { BUSY.with(Cell::get) }
struct TickGuard;
impl TickGuard {
    fn enter() -> Option<Self> {
        Self::enter_at(ic_cdk::api::time())
    }

    fn enter_at(started_ns: u64) -> Option<Self> {
        BUSY.with(|b| {
            if b.get().is_some() {
                None
            } else {
                b.set(Some(started_ns));
                Some(Self)
            }
        })
    }
}
impl Drop for TickGuard { fn drop(&mut self) { BUSY.with(|b|b.set(None)); } }

/// One batch or one executor transition per timer tick, with no overlapping
/// scheduler batches. Manual observation changes are still checked by cursor
/// and policy generation at the durable commit boundary.
pub async fn tick() -> Result<(), String> {
    let Some(_guard) = TickGuard::enter() else {
        return Ok(());
    };
    let status = route_runtime::status()?;
    let current = route_runtime::has_current()?;
    let (observation, profile_active, last_started_ns) = state::read_state(|s| (
        s.route_observation.clone(),
        s.route_arb.icusd_peg_trades_profile_active(),
        s.route_observation_last_started_ns,
    ));
    let result = match next_action_for_profile(
        &status, current, observation.as_ref(), profile_active, last_started_ns, ic_cdk::api::time(),
    ) {
        TickAction::Idle => Ok(()),
        TickAction::ServiceExecution | TickAction::SelectRoute => route_runtime::service_tick().await,
        TickAction::StartObservation => crate::start_route_observation_internal().map(|_|()),
        TickAction::QuoteBatch(cursor) => crate::quote_route_observation_batch_internal(cursor, 100).await.map(|_|()),
    };
    route_runtime::note_scheduler_result(&result)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_tick_exposes_in_flight_start_without_faking_completed_heartbeat() {
        state::init_state(state::BotState::default());
        route_runtime::set_scheduler_observability_for_test(
            500,
            Some("prior scheduler error".to_owned()),
        )
        .unwrap();

        let guard = TickGuard::enter_at(100).unwrap();
        assert!(TickGuard::enter_at(100).is_none());
        let in_flight = route_runtime::status().unwrap();
        assert_eq!(in_flight.scheduler_in_flight_since_ns, Some(100));
        assert_eq!(in_flight.last_tick_ns, 500);
        assert_eq!(in_flight.last_error.as_deref(), Some("prior scheduler error"));
        drop(guard);
        assert_eq!(route_runtime::status().unwrap().scheduler_in_flight_since_ns, None);
    }
}
