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
        Some(o) if route_runtime::has_candidate_in_any_book(o) => TickAction::SelectRoute,
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
// ── Adaptive tick cadence ──────────────────────────────────────────────────
//
// A timer firing is three message executions (global timer, the timers
// crate's trap-isolating self-call, and its reply), each billed a flat fee on
// top of the instructions it runs. Firing every ten seconds around the clock
// therefore costs the same whether or not there is work, and on this canister
// that idle polling was the single largest cycle sink. The scheduler instead
// asks, after each tick, how long it can sleep before a tick could do
// anything, and a slow repeating watchdog guarantees it is never left without
// a timer.

/// Cadence while there is work: an execution to service, a scan to continue,
/// or a route to select. The same ten seconds as the previous fixed interval,
/// and `delay_after_tick_ns` keeps it measured from when a tick starts.
pub const ACTIVE_TICK_NS: u64 = 10_000_000_000;
/// Floor for a planned delay, so a wake-up scheduled just before the cadence
/// gate cannot degenerate into a zero-delay timer.
pub const MIN_TICK_DELAY_NS: u64 = 1_000_000_000;
/// Period of the repeating safety-net timer. A one-shot timer is lost if its
/// callback traps, a repeating one is not, so this bounds how long the
/// scheduler can stay asleep for any reason — a trap, or an operator change
/// that no wake-up hook covered.
pub const WATCHDOG_INTERVAL_NS: u64 = 300_000_000_000;
/// Returned when no tick can do anything until an operator changes
/// configuration. The watchdog and the admin wake-up hooks cover that case.
pub const NO_TICK_PLANNED_NS: u64 = u64::MAX;

/// How long until a tick could next do useful work, given the same inputs
/// `tick` decides on. Mirrors `next_action_for_profile` rather than
/// re-deriving the state machine, so the two cannot drift apart: every state
/// in which the tick would act maps to at most `ACTIVE_TICK_NS`.
pub fn next_tick_delay_ns(
    status: &RuntimeStatus,
    has_current: bool,
    observation: Option<&ObservationAccumulatorV1>,
    profile_active: bool,
    last_observation_started_ns: Option<u64>,
    now_ns: u64,
) -> u64 {
    let gates_open = profile_active && status.live_authorized && status.enabled && !status.dry_run;
    match next_action_for_profile(
        status, has_current, observation, profile_active, last_observation_started_ns, now_ns,
    ) {
        TickAction::ServiceExecution | TickAction::QuoteBatch(_) | TickAction::SelectRoute => ACTIVE_TICK_NS,
        // Both mean "no scan is running". With the gates open the only thing
        // left to wait for is the observation cadence; a start attempted
        // before then is rejected, so ticking earlier accomplishes nothing.
        TickAction::StartObservation | TickAction::Idle if gates_open => {
            let Some(last_started) = last_observation_started_ns else {
                return ACTIVE_TICK_NS;
            };
            match last_started.checked_add(ICUSD_PEG_OBSERVATION_INTERVAL_NS) {
                Some(eligible_at) if now_ns < eligible_at => (eligible_at - now_ns).max(MIN_TICK_DELAY_NS),
                Some(_) => ACTIVE_TICK_NS,
                None => NO_TICK_PLANNED_NS,
            }
        }
        TickAction::StartObservation | TickAction::Idle => NO_TICK_PLANNED_NS,
    }
}

/// The delay to arm once a tick has finished. At the fast cadence the next
/// tick is due `ACTIVE_TICK_NS` after the previous one *started*, as it was
/// on a fixed interval — arming a full interval after a tick finishes would
/// stretch every execution step by the tick's own duration and age quotes
/// between legs. A planned sleep is already measured from now.
pub fn delay_after_tick_ns(planned_delay_ns: u64, tick_elapsed_ns: u64) -> u64 {
    if planned_delay_ns == ACTIVE_TICK_NS {
        ACTIVE_TICK_NS.saturating_sub(tick_elapsed_ns).max(MIN_TICK_DELAY_NS)
    } else {
        planned_delay_ns
    }
}

/// Whether a recorded wake-up will still fire by `target_ns`. A due time well
/// in the past belongs to a one-shot whose callback trapped (the rollback
/// restores the record but the timer is gone); counting it as pending would
/// silence every later wake-up until the watchdog.
pub fn wake_is_pending(wake_due_ns: Option<u64>, now_ns: u64, target_ns: u64) -> bool {
    wake_due_ns.is_some_and(|due| due <= target_ns && due.saturating_add(ACTIVE_TICK_NS) >= now_ns)
}

/// `next_tick_delay_ns` evaluated against live state.
pub fn planned_delay_ns(now_ns: u64) -> Result<u64, String> {
    let status = route_runtime::status()?;
    let current = route_runtime::has_current()?;
    Ok(state::read_state(|s| next_tick_delay_ns(
        &status,
        current,
        s.route_observation.as_ref(),
        s.route_arb.icusd_peg_trades_profile_active(),
        s.route_observation_last_started_ns,
        now_ns,
    )))
}

thread_local! {
    static BUSY: Cell<Option<u64>> = const { Cell::new(None) };
    static WAKE_DUE: Cell<Option<u64>> = const { Cell::new(None) };
    static WATCHDOG_DUE: Cell<Option<u64>> = const { Cell::new(None) };
}
pub fn in_flight_since_ns() -> Option<u64> { BUSY.with(Cell::get) }

/// Records when the one-shot wake-up timer will fire (`None` once it has
/// fired or been cancelled).
pub fn set_wake_due_ns(due_ns: Option<u64>) { WAKE_DUE.with(|d| d.set(due_ns)); }
pub fn wake_due_ns() -> Option<u64> { WAKE_DUE.with(Cell::get) }
/// Records that the repeating watchdog was (re)armed at `now_ns`.
pub fn note_watchdog_armed(now_ns: u64) {
    WATCHDOG_DUE.with(|d| d.set(Some(now_ns.saturating_add(WATCHDOG_INTERVAL_NS))));
}
/// When the scheduler will next tick: the sooner of the wake-up timer and
/// the watchdog. Lets an observer tell a planned sleep from a dead timer —
/// `last_tick_ns` alone cannot, now that the scheduler sleeps on purpose.
pub fn next_tick_due_ns() -> Option<u64> {
    match (WAKE_DUE.with(Cell::get), WATCHDOG_DUE.with(Cell::get)) {
        (Some(wake), Some(watchdog)) => Some(wake.min(watchdog)),
        (wake, watchdog) => wake.or(watchdog),
    }
}
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
