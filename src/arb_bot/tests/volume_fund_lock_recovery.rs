//! Recovery for the durable account-mutation lock left behind by a failed
//! admin `fund_volume_subaccount` call.
//!
//! A failed fund transfer marks its lock `reconciliation_required`, and no
//! ordinary path may release that (see `release_mutation_lock`). Because every
//! volume cycle and every route execution must first acquire the same lock, one
//! failed fund call otherwise wedges the whole bot permanently. The recovery
//! path is deliberately narrow: it only covers `volume-fund-*` locks, because
//! that transfer moves value solely between the canister's own default account
//! and its own volume subaccount, so a lost or ambiguous outcome cannot strand
//! or mis-attribute route-owned inventory. Withdraw, cycle and rebalance locks
//! stay unreleasable.

use arb_bot::route_arb::MutationOwnerV1;
use arb_bot::state;

fn hold_reconciliation_lock(operation_id: &str, owner: MutationOwnerV1) {
    state::release_mutation_lock_for_test();
    state::acquire_mutation_lock(operation_id, owner, 10).unwrap();
    state::mark_mutation_lock_reconciliation_required(operation_id).unwrap();
}

#[test]
fn failed_volume_fund_lock_is_released_by_its_exact_operation_id() {
    hold_reconciliation_lock("volume-fund-1790830642186099775", MutationOwnerV1::VolumeOperation);

    state::release_failed_volume_fund_lock("volume-fund-1790830642186099775").unwrap();

    assert!(state::get_mutation_lock().is_none(), "lock must be cleared");
    // And the bot can take the lock again afterwards.
    state::acquire_mutation_lock("volume-cycle-1", MutationOwnerV1::VolumeOperation, 11).unwrap();
}

#[test]
fn release_requires_the_operation_id_to_match_the_held_lock() {
    hold_reconciliation_lock("volume-fund-111", MutationOwnerV1::VolumeOperation);

    let error = state::release_failed_volume_fund_lock("volume-fund-222").unwrap_err();

    assert!(error.contains("volume-fund-111"), "error names the held lock: {error}");
    assert!(state::get_mutation_lock().is_some(), "wrong id must not release");
}

#[test]
fn release_with_no_lock_held_is_an_error() {
    state::release_mutation_lock_for_test();

    assert!(state::release_failed_volume_fund_lock("volume-fund-1").is_err());
    assert!(state::get_mutation_lock().is_none());
}

#[test]
fn only_volume_fund_locks_are_recoverable() {
    for id in [
        "volume-withdraw-5",
        "volume-cycle-5",
        "volume-rebalance-5",
        "route-execution-5",
        // Prefix must be exact, not merely contained.
        "x-volume-fund-5",
        "volume-fund",
    ] {
        hold_reconciliation_lock(id, MutationOwnerV1::VolumeOperation);
        assert!(
            state::release_failed_volume_fund_lock(id).is_err(),
            "{id} must stay locked"
        );
        assert!(state::get_mutation_lock().is_some(), "{id} lock must be retained");
    }
}

#[test]
fn route_owned_locks_are_never_recoverable_even_with_a_fund_looking_id() {
    hold_reconciliation_lock("volume-fund-7", MutationOwnerV1::RouteExecution);

    assert!(state::release_failed_volume_fund_lock("volume-fund-7").is_err());
    assert!(state::get_mutation_lock().is_some());
}

#[test]
fn a_live_fund_lock_without_reconciliation_flag_is_not_touched() {
    // An in-flight fund call holds its lock without the flag; only that call's
    // own completion may release it. The recovery path must not race it.
    state::release_mutation_lock_for_test();
    state::acquire_mutation_lock("volume-fund-9", MutationOwnerV1::VolumeOperation, 10).unwrap();

    assert!(state::release_failed_volume_fund_lock("volume-fund-9").is_err());
    assert!(state::get_mutation_lock().is_some());

    // The normal owner-bound release still works for it.
    state::release_mutation_lock("volume-fund-9").unwrap();
}

#[test]
fn ordinary_release_still_refuses_reconciliation_required_locks() {
    hold_reconciliation_lock("volume-fund-3", MutationOwnerV1::VolumeOperation);

    assert!(state::release_mutation_lock("volume-fund-3").is_err());
    assert!(state::get_mutation_lock().is_some());
}

#[test]
fn recovery_endpoint_is_admin_gated_and_audited() {
    let source = include_str!("../src/lib.rs");
    let start = source
        .find("fn release_failed_volume_fund_lock")
        .expect("lib.rs must expose release_failed_volume_fund_lock");
    let tail = &source[start..];
    let end = tail.find("\n#[").unwrap_or(tail.len());
    let body = &tail[..end];

    let admin = body.find("require_admin()").expect("endpoint must require admin");
    let release = body
        .find("state::release_failed_volume_fund_lock")
        .expect("endpoint must delegate to the state-level release");
    assert!(admin < release, "admin check must precede any state change");
    assert!(body.contains("log_activity"), "release must be written to the activity log");

    // The legacy cycle-lock escape hatch must remain unable to touch the
    // durable lock.
    let clear = source.split("fn clear_cycle_lock").nth(1).unwrap().split("\n#[").next().unwrap();
    assert!(!clear.contains("release_failed_volume_fund_lock"));
    assert!(!clear.contains("release_mutation_lock"));
}
