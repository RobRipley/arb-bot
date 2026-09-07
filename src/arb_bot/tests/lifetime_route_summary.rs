//! Tests for the durable all-time route-execution summary (LifetimeRouteSummaryV1)
//! — see docs/superpowers/specs/... Cockpit P&L audit follow-up. Covers folding on
//! new terminal completions, retry/idempotency, and post-upgrade backfill.
//!
//! These tests share process-global thread_local stable structures with any other
//! test in this same binary (see route_storage.rs's use of *_for_test resets for
//! precedent), so every assertion here is a DELTA against a `before` snapshot
//! rather than an absolute value — safe regardless of whether the Rust test
//! harness reuses a worker thread that already ran another test in this file.

use arb_bot::route_arb::{Asset, CandidateClass, ExecutionPhaseV1, ExecutionRecordV1};
use arb_bot::state;

fn record(
    execution_id: &str,
    phase: ExecutionPhaseV1,
    candidate_class: CandidateClass,
    realized_profit: Option<i128>,
) -> ExecutionRecordV1 {
    ExecutionRecordV1 {
        execution_id: execution_id.into(),
        route_id: "lifetime-summary-route".into(),
        canonical_cycle_id: None,
        candidate_class,
        phase,
        current_leg_index: 0,
        planned_input_native: 1_000_000,
        required_min_output_native: 999_000,
        quote_timestamp_ns: 10,
        submission_started_at_ns: Some(11),
        adapter_request_fingerprint: None,
        evidence: vec![],
        reconciliation_query_count: 0,
        incident: None,
        updated_at_ns: 12,
        realized_profit,
        start_asset: Some(Asset::CkUsdc),
    }
}

#[test]
fn new_terminal_completions_fold_exactly_once_across_all_phases_and_classes() {
    let before = state::get_lifetime_route_summary();

    state::complete_current_route_execution(record(
        "lifetime-fold-stable-par",
        ExecutionPhaseV1::Completed,
        CandidateClass::StablePar,
        Some(500),
    ))
    .unwrap();
    state::complete_current_route_execution(record(
        "lifetime-fold-stable-cross",
        ExecutionPhaseV1::Completed,
        CandidateClass::StableSettledCrossAsset,
        Some(300),
    ))
    .unwrap();
    state::complete_current_route_execution(record(
        "lifetime-fold-icp-returning",
        ExecutionPhaseV1::Completed,
        CandidateClass::IcpReturning,
        Some(-20),
    ))
    .unwrap();
    state::complete_current_route_execution(record(
        "lifetime-fold-aborted",
        ExecutionPhaseV1::Aborted,
        CandidateClass::StablePar,
        None,
    ))
    .unwrap();
    state::complete_current_route_execution(record(
        "lifetime-fold-held",
        ExecutionPhaseV1::HeldInventory,
        CandidateClass::IcpReturning,
        None,
    ))
    .unwrap();

    let after = state::get_lifetime_route_summary();

    // All three Completed-phase records above (StablePar, StableSettledCrossAsset,
    // IcpReturning) count toward "Completed"; the two stable-book candidate
    // classes additionally bucket into the same USD6 profit sum below.
    assert_eq!(after.completed_count, before.completed_count + 3);
    assert_eq!(after.aborted_count, before.aborted_count + 1);
    assert_eq!(after.held_inventory_count, before.held_inventory_count + 1);
    assert_eq!(
        after.stable_realized_profit_usd6,
        before.stable_realized_profit_usd6 + 800
    );
    assert_eq!(
        after.icp_realized_profit_e8s,
        before.icp_realized_profit_e8s - 20
    );
    // None-profit terminal records (Aborted/HeldInventory here) must not
    // perturb either profit bucket beyond the two completions above.
    assert_eq!(after.folded_through, before.folded_through + 5);
}

#[test]
fn retrying_the_same_terminal_execution_does_not_double_count() {
    let unique = record(
        "lifetime-retry-once",
        ExecutionPhaseV1::Completed,
        CandidateClass::StablePar,
        Some(777),
    );

    let before = state::get_lifetime_route_summary();
    state::complete_current_route_execution(unique.clone()).unwrap();
    let after_first = state::get_lifetime_route_summary();
    assert_eq!(after_first.completed_count, before.completed_count + 1);
    assert_eq!(
        after_first.stable_realized_profit_usd6,
        before.stable_realized_profit_usd6 + 777
    );

    // An immediate retry with an identical terminal record (same execution_id,
    // same phase, as a real canister-call retry would send) must be a total
    // no-op on the lifetime summary — not just "no error", but zero change.
    state::complete_current_route_execution(unique).unwrap();
    let after_retry = state::get_lifetime_route_summary();
    assert_eq!(after_retry, after_first);
}

#[test]
fn post_upgrade_backfill_reconstructs_lifetime_totals_from_the_existing_terminal_log() {
    // Establish ground truth: fold in a couple of fresh completions the normal
    // way (eager hook inside complete_current_route_execution).
    state::complete_current_route_execution(record(
        "lifetime-backfill-a",
        ExecutionPhaseV1::Completed,
        CandidateClass::StablePar,
        Some(111),
    ))
    .unwrap();
    state::complete_current_route_execution(record(
        "lifetime-backfill-b",
        ExecutionPhaseV1::Completed,
        CandidateClass::IcpReturning,
        Some(222),
    ))
    .unwrap();
    let ground_truth = state::get_lifetime_route_summary();

    // Simulate the mainnet post-upgrade scenario this feature ships into: a
    // durable terminal log with pre-existing records, paired with a summary
    // cell reset to its (pre-feature) default — folded_through back to 0.
    // (get_lifetime_route_summary() always folds before returning, so the
    // reset's zeroed watermark is only ever observable by reading the cell
    // directly — the read below is what actually exercises the backfill.)
    state::reset_lifetime_route_summary_for_test();

    // The very next read (mirroring the eager post_upgrade() call in lib.rs,
    // or a lazy first query) must fold the entire existing terminal log back
    // in and land on exactly the same totals — nothing lost, nothing doubled.
    let backfilled = state::get_lifetime_route_summary();
    assert_eq!(backfilled, ground_truth);

    // A second read after backfill is a pure no-op (folded_through already
    // caught up to the terminal log's length).
    let again = state::get_lifetime_route_summary();
    assert_eq!(again, backfilled);
}
