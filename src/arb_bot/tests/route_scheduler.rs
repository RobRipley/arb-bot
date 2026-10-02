use arb_bot::route_runtime::RuntimeStatus;
use arb_bot::route_scheduler::{
    next_action, next_action_for_profile, next_tick_delay_ns, TickAction, ACTIVE_TICK_NS,
    ICUSD_PEG_OBSERVATION_INTERVAL_NS, MIN_TICK_DELAY_NS, NO_TICK_PLANNED_NS, WATCHDOG_INTERVAL_NS,
};
use arb_bot::route_arb::{ObservationAccumulatorV1,RouteCandidateReportV1,CandidateClass};
fn status()->RuntimeStatus{RuntimeStatus{compiled_support:true,live_authorized:true,enabled:true,dry_run:false,last_error:None,last_tick_ns:0,scheduler_in_flight_since_ns:None,scheduler_next_tick_due_ns:None,last_realized_profit:None,last_profit_class:None}}
fn observation()->ObservationAccumulatorV1{ObservationAccumulatorV1::new("obs".into(),1,0,972,2644,true)}
#[test]
fn disabled_trading_stays_idle_but_existing_attempt_always_reconciles(){
    for s in [RuntimeStatus{live_authorized:false,..status()},RuntimeStatus{enabled:false,..status()},RuntimeStatus{dry_run:true,..status()}] {
        assert_eq!(next_action(&s,false,None),TickAction::Idle);
        assert_eq!(next_action(&s,true,None),TickAction::ServiceExecution);
    }
}
#[test]
fn scheduler_scans_whole_universe_before_selecting_and_restarts_no_winner_scan(){
    let s=status(); assert_eq!(next_action(&s,false,None),TickAction::StartObservation);
    let mut o=observation();o.next_cursor=100;
    assert_eq!(next_action(&s,false,Some(&o)),TickAction::QuoteBatch(100));
    o.scan_complete=true;
    assert_eq!(next_action(&s,false,Some(&o)),TickAction::StartObservation);
    o.best_stable_candidate=Some(RouteCandidateReportV1::fixture("route",CandidateClass::StablePar,10,true));
    assert_eq!(next_action(&s,false,Some(&o)),TickAction::SelectRoute);
    assert_eq!(next_action(&s,true,Some(&o)),TickAction::ServiceExecution);
}

/// One completed scan per book lane the runtime rotates over, each with a
/// winner in that lane only.
fn single_book_winners() -> [(&'static str, ObservationAccumulatorV1); 4] {
    let mut done = observation();
    done.scan_complete = true;
    let (mut stable, mut icp, mut ckbtc, mut cketh) = (done.clone(), done.clone(), done.clone(), done);
    stable.best_stable_candidate = Some(RouteCandidateReportV1::fixture("stable-route", CandidateClass::StablePar, 10, true));
    icp.best_icp_candidate = Some(RouteCandidateReportV1::fixture("icp-route", CandidateClass::IcpReturning, 10, true));
    ckbtc.best_ckbtc_candidate = Some(RouteCandidateReportV1::fixture("ckbtc-route", CandidateClass::CkBtcReturning, 10, true));
    cketh.best_cketh_candidate = Some(RouteCandidateReportV1::fixture("cketh-route", CandidateClass::CkEthReturning, 10, true));
    [("stable", stable), ("icp", icp), ("ckbtc", ckbtc), ("cketh", cketh)]
}

#[test]
fn a_winner_in_any_single_book_is_handed_to_selection() {
    // A scan whose only winner is in the ckBTC or ckETH book must not be
    // treated as "no winner": that restarts the observation and overwrites
    // the winner before the runtime's four-lane rotation ever sees it.
    for (book, o) in single_book_winners() {
        assert_eq!(next_action(&status(), false, Some(&o)), TickAction::SelectRoute, "{book} winner");
        assert_eq!(next_action(&status(), true, Some(&o)), TickAction::ServiceExecution, "{book} winner");
    }
}

#[test]
fn startup_ignores_legacy_observations_until_profile_is_explicitly_selected() {
    let mut stale = observation();
    stale.scan_complete = true;
    stale.best_stable_candidate = Some(RouteCandidateReportV1::fixture("legacy", CandidateClass::StablePar, 1_000_000, true));
    assert_eq!(
        next_action_for_profile(&status(), false, Some(&stale), false, None, 1_000),
        TickAction::Idle
    );
    assert_eq!(
        next_action_for_profile(&status(), true, Some(&stale), false, None, 1_000),
        TickAction::ServiceExecution,
        "durable execution reconciliation remains available while profile is inactive"
    );
}

#[test]
fn profile_waits_six_hundred_seconds_between_observations() {
    let last_started = 10_000_000_000;
    assert_eq!(ICUSD_PEG_OBSERVATION_INTERVAL_NS, 600_000_000_000);
    assert_eq!(
        next_action_for_profile(&status(), false, None, true, Some(last_started), last_started + ICUSD_PEG_OBSERVATION_INTERVAL_NS - 1),
        TickAction::Idle
    );
    assert_eq!(
        next_action_for_profile(&status(), false, None, true, Some(last_started), last_started + ICUSD_PEG_OBSERVATION_INTERVAL_NS),
        TickAction::StartObservation
    );
}

// ── Adaptive tick cadence ──────────────────────────────────────────────────
//
// Every timer firing costs cycles whether or not it finds work, so the
// scheduler only ticks at the fast cadence while there is something to do and
// otherwise sleeps until the observation cadence gate opens. These tests pin
// the delay the scheduler asks for in each state.

const T0: u64 = 1_000_000_000_000;
const SECOND: u64 = 1_000_000_000;

fn winner() -> RouteCandidateReportV1 {
    RouteCandidateReportV1::fixture("route", CandidateClass::StablePar, 10, true)
}

#[test]
fn cadence_constants_keep_the_fast_tick_and_a_slow_watchdog() {
    assert_eq!(ACTIVE_TICK_NS, 10 * SECOND, "fast cadence is unchanged while there is work");
    assert_eq!(WATCHDOG_INTERVAL_NS, 300 * SECOND);
    assert!(MIN_TICK_DELAY_NS >= SECOND && MIN_TICK_DELAY_NS <= ACTIVE_TICK_NS);
    assert!(WATCHDOG_INTERVAL_NS < ICUSD_PEG_OBSERVATION_INTERVAL_NS);
}

#[test]
fn an_execution_in_flight_always_ticks_at_the_fast_cadence() {
    // Even with every gate off: a previous debit must keep reconciling.
    for s in [
        status(),
        RuntimeStatus { live_authorized: false, ..status() },
        RuntimeStatus { enabled: false, ..status() },
        RuntimeStatus { dry_run: true, ..status() },
    ] {
        for profile_active in [true, false] {
            assert_eq!(
                next_tick_delay_ns(&s, true, None, profile_active, Some(T0), T0 + SECOND),
                ACTIVE_TICK_NS
            );
        }
    }
}

#[test]
fn an_unfinished_scan_and_a_pending_selection_tick_at_the_fast_cadence() {
    let mut o = observation();
    o.next_cursor = 100;
    assert_eq!(next_tick_delay_ns(&status(), false, Some(&o), true, Some(T0), T0 + SECOND), ACTIVE_TICK_NS);
    o.scan_complete = true;
    o.best_stable_candidate = Some(winner());
    assert_eq!(next_tick_delay_ns(&status(), false, Some(&o), true, Some(T0), T0 + SECOND), ACTIVE_TICK_NS);
}

#[test]
fn a_winner_in_any_single_book_is_selected_at_the_fast_cadence_before_the_gate() {
    // Well inside the 600 s cadence window: the winner is selected now, not
    // slept on until the next observation replaces it.
    let now = T0 + 95 * SECOND;
    for (book, o) in single_book_winners() {
        assert_eq!(
            next_action_for_profile(&status(), false, Some(&o), true, Some(T0), now),
            TickAction::SelectRoute,
            "{book} winner"
        );
        assert_eq!(
            next_tick_delay_ns(&status(), false, Some(&o), true, Some(T0), now),
            ACTIVE_TICK_NS,
            "{book} winner"
        );
    }
}

#[test]
fn a_finished_scan_with_no_winner_sleeps_until_the_cadence_gate_opens() {
    let mut o = observation();
    o.scan_complete = true;
    let now = T0 + 95 * SECOND;
    assert_eq!(
        next_tick_delay_ns(&status(), false, Some(&o), true, Some(T0), now),
        ICUSD_PEG_OBSERVATION_INTERVAL_NS - 95 * SECOND,
        "no tick is useful before the next observation is eligible"
    );
}

#[test]
fn no_observation_sleeps_until_eligible_then_starts_promptly() {
    let s = status();
    assert_eq!(
        next_tick_delay_ns(&s, false, None, true, Some(T0), T0 + 400 * SECOND),
        200 * SECOND
    );
    // Close to the gate the delay is exact, but never below the floor.
    assert_eq!(
        next_tick_delay_ns(&s, false, None, true, Some(T0), T0 + ICUSD_PEG_OBSERVATION_INTERVAL_NS - 3 * SECOND),
        3 * SECOND
    );
    assert_eq!(
        next_tick_delay_ns(&s, false, None, true, Some(T0), T0 + ICUSD_PEG_OBSERVATION_INTERVAL_NS - 1),
        MIN_TICK_DELAY_NS
    );
    // Already eligible (or never observed): start at the next fast tick.
    assert_eq!(
        next_tick_delay_ns(&s, false, None, true, Some(T0), T0 + ICUSD_PEG_OBSERVATION_INTERVAL_NS),
        ACTIVE_TICK_NS
    );
    assert_eq!(
        next_tick_delay_ns(&s, false, None, true, Some(T0), T0 + 10 * ICUSD_PEG_OBSERVATION_INTERVAL_NS),
        ACTIVE_TICK_NS
    );
    assert_eq!(next_tick_delay_ns(&s, false, None, true, None, T0), ACTIVE_TICK_NS);
}

#[test]
fn closed_gates_plan_no_tick_and_leave_wakeup_to_the_watchdog_and_admin_hooks() {
    let mut scanning = observation();
    scanning.next_cursor = 100;
    for s in [
        RuntimeStatus { live_authorized: false, ..status() },
        RuntimeStatus { enabled: false, ..status() },
        RuntimeStatus { dry_run: true, ..status() },
    ] {
        assert_eq!(next_tick_delay_ns(&s, false, None, true, Some(T0), T0 + SECOND), NO_TICK_PLANNED_NS);
        assert_eq!(next_tick_delay_ns(&s, false, Some(&scanning), true, Some(T0), T0 + SECOND), NO_TICK_PLANNED_NS);
    }
    // Profile not selected: nothing may start, whatever is cached.
    assert_eq!(next_tick_delay_ns(&status(), false, Some(&scanning), false, None, T0), NO_TICK_PLANNED_NS);
    assert_eq!(next_tick_delay_ns(&status(), false, None, false, None, T0), NO_TICK_PLANNED_NS);
}

#[test]
fn cadence_timestamp_overflow_plans_no_tick_instead_of_spinning() {
    assert_eq!(
        next_tick_delay_ns(&status(), false, None, true, Some(u64::MAX - 5), u64::MAX - 1),
        NO_TICK_PLANNED_NS
    );
}

#[test]
fn planned_delay_never_skips_work_the_tick_would_do() {
    // Property: whenever the tick would act right now, the scheduler asks to
    // be woken at the fast cadence or sooner — it never sleeps through work.
    let mut scanning = observation();
    scanning.next_cursor = 7;
    let mut done = observation();
    done.scan_complete = true;
    let mut won = done.clone();
    won.best_icp_candidate = Some(winner());
    // A winner only in a ckBTC/ckETH book is selected like any other lane.
    let mut won_btc = done.clone();
    won_btc.best_ckbtc_candidate = Some(winner());
    let mut won_eth = done.clone();
    won_eth.best_cketh_candidate = Some(winner());
    let observations = [None, Some(&scanning), Some(&done), Some(&won), Some(&won_btc), Some(&won_eth)];
    let statuses = [
        status(),
        RuntimeStatus { live_authorized: false, ..status() },
        RuntimeStatus { enabled: false, ..status() },
        RuntimeStatus { dry_run: true, ..status() },
    ];
    let last_started = [None, Some(T0)];
    let nows = [T0, T0 + SECOND, T0 + ICUSD_PEG_OBSERVATION_INTERVAL_NS - 1, T0 + ICUSD_PEG_OBSERVATION_INTERVAL_NS, T0 + 2 * ICUSD_PEG_OBSERVATION_INTERVAL_NS];
    for s in &statuses {
        for has_current in [false, true] {
            for o in observations {
                for profile_active in [false, true] {
                    for last in last_started {
                        for now in nows {
                            let action = next_action_for_profile(s, has_current, o, profile_active, last, now);
                            let delay = next_tick_delay_ns(s, has_current, o, profile_active, last, now);
                            let eligible = last.map_or(true, |l| now >= l + ICUSD_PEG_OBSERVATION_INTERVAL_NS);
                            let acts_now = match action {
                                TickAction::Idle => false,
                                // A start attempt before the cadence gate is rejected, so it is not work.
                                TickAction::StartObservation => eligible,
                                _ => true,
                            };
                            if acts_now {
                                assert!(delay <= ACTIVE_TICK_NS, "{action:?} must not be slept through (delay {delay})");
                            }
                            // With the gates open the only thing worth waiting for is
                            // the cadence gate, so the scheduler must be awake again
                            // within one fast tick of it — never "no tick planned".
                            let gates_open = profile_active && s.live_authorized && s.enabled && !s.dry_run;
                            if gates_open {
                                let gate = last.map_or(now, |l| (l + ICUSD_PEG_OBSERVATION_INTERVAL_NS).max(now));
                                assert!(
                                    now + delay <= gate + ACTIVE_TICK_NS,
                                    "gates open: slept past the cadence gate ({action:?}, delay {delay})"
                                );
                            }
                            assert!(delay >= MIN_TICK_DELAY_NS, "delay {delay} below floor for {action:?}");
                        }
                    }
                }
            }
        }
    }
}

// ── Timer wiring (source-level, like the other lib.rs structural guards) ───

fn lib_fn_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source.find(signature).unwrap_or_else(|| panic!("missing `{signature}`"));
    let tail = &source[start..];
    let end = tail.find("\n}\n").map(|n| n + 2).unwrap_or(tail.len());
    &tail[..end]
}

#[test]
fn scheduler_runs_on_a_repeating_watchdog_plus_one_shot_wakeups() {
    let source = include_str!("../src/lib.rs");
    let setup = lib_fn_body(source, "fn setup_route_runtime_timer()");
    assert!(
        setup.contains("set_timer_interval") && setup.contains("WATCHDOG_INTERVAL_NS"),
        "the safety net must be a repeating timer: a one-shot is lost if its callback traps"
    );
    assert!(setup.contains("arm_route_wake("), "an upgrade must not wait a full watchdog period for its first tick");
    assert!(
        !source.contains("Duration::from_secs(10), ||"),
        "the fixed ten-second polling interval must not come back"
    );

    let arm = lib_fn_body(source, "fn arm_route_wake(");
    assert!(arm.contains("ic_cdk_timers::set_timer("), "wake-ups are one-shot timers");
    assert!(arm.contains("clear_route_wake()"), "at most one wake-up may be pending");

    let run = lib_fn_body(source, "fn run_route_scheduler_tick()");
    let busy = run.find("in_flight_since_ns().is_some()").expect("an in-flight tick must not be re-entered");
    let tick = run.find("route_scheduler::tick().await").expect("the timer must run the scheduler tick");
    let rearm = run.find("rearm_route_scheduler(started_ns)").expect("a finished tick must plan the next one");
    assert!(busy < tick && tick < rearm);

    let rearm_body = lib_fn_body(source, "fn rearm_route_scheduler(");
    assert!(rearm_body.contains("planned_delay_ns("));
    assert!(
        rearm_body.contains("delay_after_tick_ns(") && rearm_body.contains("< route_scheduler::WATCHDOG_INTERVAL_NS"),
        "longer sleeps are left to the watchdog instead of a second timer"
    );
}

/// Every `#[update]` endpoint in lib.rs, as (name, body).
fn update_endpoints(source: &str) -> Vec<(String, &str)> {
    let mut found = Vec::new();
    let mut rest = source;
    while let Some(at) = rest.find("\n#[update]\n") {
        let after = &rest[at + "\n#[update]\n".len()..];
        let end = after.find("\n}\n").map(|n| n + 2).unwrap_or(after.len());
        let body = &after[..end];
        let name = body
            .split("fn ").nth(1).and_then(|tail| tail.split('(').next())
            .unwrap_or_else(|| panic!("unparseable endpoint: {}", &body[..body.len().min(80)]))
            .trim().to_string();
        found.push((name, body));
        rest = &after[end..];
    }
    found
}

#[test]
fn every_endpoint_that_can_create_scheduler_work_wakes_it() {
    let source = include_str!("../src/lib.rs");
    // Deny by default: any update endpoint that reaches something the
    // scheduler decides on must wake it. A new endpoint that touches these
    // and forgets the guard fails here, instead of silently leaving the
    // scheduler asleep until the watchdog.
    const SCHEDULER_INPUTS: [&str; 7] = [
        "route_runtime::",                        // authorization, executions
        "store_route_arb_config",                 // route config + generation
        "route_arb_config_generation",
        "s.route_arb",
        "start_route_observation_internal",
        "quote_route_observation_batch_internal",
        "release_failed_volume_fund_lock",        // frees the lock a selection waits on
    ];
    let endpoints = update_endpoints(source);
    assert!(endpoints.len() > 40, "endpoint scan found only {}", endpoints.len());
    let mut guarded = Vec::new();
    for (name, body) in &endpoints {
        let touches = SCHEDULER_INPUTS.iter().any(|needle| body.contains(needle));
        let wakes = body.contains("let _wake = RouteSchedulerWake;");
        assert!(!touches || wakes, "{name} touches scheduler inputs but does not wake the route scheduler");
        if wakes {
            let admin = body.find("require_admin()").unwrap_or_else(|| panic!("{name} must stay admin-gated"));
            let wake = body.find("let _wake = RouteSchedulerWake;").unwrap();
            assert!(admin < wake, "{name}: an unauthorized caller must not be able to wake the scheduler");
            guarded.push(name.as_str());
        }
    }
    for expected in [
        "set_route_arb_config_v1", "set_icusd_price_usd6_v1", "set_wrapped_stable_to_icusd_allowed_v1",
        "set_icusd_peg_trades_profile_v1", "start_route_observation_v1", "quote_route_observation_batch_v1",
        "set_route_runtime_authorized_v1", "prepare_route_execution_v1", "advance_route_execution_v1",
        "reconcile_route_execution_v1", "release_failed_volume_fund_lock",
    ] {
        assert!(guarded.contains(&expected), "{expected} must wake the route scheduler");
    }

    let wake_impl = lib_fn_body(source, "impl Drop for RouteSchedulerWake");
    assert!(wake_impl.contains("wake_route_scheduler()"));
    let wake = lib_fn_body(source, "fn wake_route_scheduler()");
    assert!(wake.contains("wake_is_pending("), "a wake-up must never push an already-sooner tick later");
}

#[test]
fn a_wake_that_never_fired_does_not_suppress_later_wakeups() {
    use arb_bot::route_scheduler::wake_is_pending;
    let now = T0;
    let target = now + ACTIVE_TICK_NS;
    assert!(!wake_is_pending(None, now, target));
    assert!(wake_is_pending(Some(now + 3 * SECOND), now, target), "a sooner wake is kept");
    assert!(wake_is_pending(Some(target), now, target));
    assert!(!wake_is_pending(Some(target + 1), now, target), "a later wake is brought forward");
    assert!(wake_is_pending(Some(now - 2 * SECOND), now, target), "a wake just past due is about to fire");
    // A one-shot whose callback trapped leaves its due time behind with no
    // timer. Treating that as pending would mute every wake-up hook.
    assert!(!wake_is_pending(Some(now - ACTIVE_TICK_NS - 1), now, target));
    assert!(!wake_is_pending(Some(1), now, target));
}

#[test]
fn active_cadence_stays_on_the_ten_second_grid_whatever_the_tick_took() {
    use arb_bot::route_scheduler::delay_after_tick_ns;
    // A tick that took 4s is followed 6s later, as on a fixed 10s interval —
    // not 10s later, which would age quotes between execution legs.
    assert_eq!(delay_after_tick_ns(ACTIVE_TICK_NS, 0), ACTIVE_TICK_NS);
    assert_eq!(delay_after_tick_ns(ACTIVE_TICK_NS, 4 * SECOND), 6 * SECOND);
    assert_eq!(delay_after_tick_ns(ACTIVE_TICK_NS, 9_500_000_000), MIN_TICK_DELAY_NS);
    assert_eq!(delay_after_tick_ns(ACTIVE_TICK_NS, 85 * SECOND), MIN_TICK_DELAY_NS, "a long tick is followed promptly");
    // Planned sleeps are measured from now and are not shortened.
    assert_eq!(delay_after_tick_ns(200 * SECOND, 85 * SECOND), 200 * SECOND);
    assert_eq!(delay_after_tick_ns(3 * SECOND, 4 * SECOND), 3 * SECOND);
    assert_eq!(delay_after_tick_ns(NO_TICK_PLANNED_NS, 4 * SECOND), NO_TICK_PLANNED_NS);
}

#[test]
fn a_tick_that_dies_mid_flight_still_leaves_a_fast_wakeup_behind() {
    let source = include_str!("../src/lib.rs");
    let run = lib_fn_body(source, "fn run_route_scheduler_tick()");
    let guard = run.find("RearmOnAbort").expect("the tick must be covered by an abort guard");
    let tick = run.find("route_scheduler::tick().await").unwrap();
    assert!(guard < tick, "the guard must exist before the first await");
    let drop_impl = lib_fn_body(source, "impl Drop for RearmOnAbort");
    assert!(
        drop_impl.contains("arm_route_wake(route_scheduler::ACTIVE_TICK_NS)"),
        "an aborted tick must be retried at the fast cadence, as the fixed interval did"
    );
    assert!(
        !drop_impl.contains("planned_delay_ns") && !drop_impl.contains("route_runtime::"),
        "the abort path runs in a cleanup callback: it must not read state that could trap and strand the busy flag"
    );
}

#[test]
fn dashboard_watchdog_period_matches_the_canister() {
    let html = include_str!("../src/dashboard.html");
    let line = html
        .lines()
        .find(|line| line.contains("const SCHEDULER_WATCHDOG_INTERVAL_MS ="))
        .expect("dashboard must declare the watchdog period it assumes");
    let digits: String = line.split('=').nth(1).unwrap().chars().filter(char::is_ascii_digit).collect();
    assert_eq!(
        digits.parse::<u64>().unwrap(),
        WATCHDOG_INTERVAL_NS / 1_000_000,
        "the dashboard's stall detection assumes the canister's watchdog period"
    );
}
