use arb_bot::route_arb::{
    consume_reconciliation_queries, mark_awaiting_settlement, mark_reconciliation_required, prepare_execution,
    prepare_leg, record_remaining_route_requote, reconcile_settlement,
    persist_leg_submission, Asset, CandidateClass, ExecutionPhaseV1,
    RouteCandidateReportV1, SettlementProofV1,
};

fn candidate() -> RouteCandidateReportV1 {
    let mut candidate = RouteCandidateReportV1::fixture(
        "stable-route", CandidateClass::StableSettledCrossAsset, 100_000, true,
    );
    candidate.start_asset = Asset::CkUsdc;
    candidate.end_asset = Asset::IcUsd;
    candidate.asset_path = vec![Asset::CkUsdc, Asset::Icp, Asset::IcUsd];
    candidate.full_fill = true;
    candidate.principal_native = 10_000_000;
    candidate.quote_timestamp_ns = 100;
    candidate
}

#[test]
fn intent_is_persisted_before_any_submission_and_never_replayed() {
    let mut record = prepare_execution(&candidate(), "exec-1", 101).unwrap();
    assert_eq!(record.phase, ExecutionPhaseV1::Planned);
    prepare_leg(&mut record, 9_990_000, 10_050_000, 102).unwrap();
    assert_eq!(record.phase, ExecutionPhaseV1::LegPrepared);
    persist_leg_submission(&mut record, "fingerprint-1", 103).unwrap();
    assert_eq!(record.phase, ExecutionPhaseV1::LegSubmitted);
    assert!(persist_leg_submission(&mut record, "fingerprint-1", 104).is_err());
    mark_awaiting_settlement(&mut record, 104).unwrap();
    assert_eq!(record.phase, ExecutionPhaseV1::AwaitingSettlement);
}

#[test]
fn coincident_credit_or_partial_fill_cannot_advance_as_a_settled_leg() {
    let mut record = prepare_execution(&candidate(), "exec-2", 101).unwrap();
    prepare_leg(&mut record, 9_990_000, 10_050_000, 102).unwrap();
    persist_leg_submission(&mut record, "fingerprint-2", 103).unwrap();
    mark_awaiting_settlement(&mut record, 104).unwrap();

    let bare_credit = SettlementProofV1 {
        request_fingerprint: "fingerprint-2".into(), planned_input_native: 9_990_000,
        effective_input_native: 9_990_000, gross_output_native: 10_100_000,
        refund_native: 0, source_debit_bound: false, venue_execution_bound: false,
        output_credit_bound: true, refund_bound: true, fully_reconciled: false,
    };
    assert!(reconcile_settlement(&mut record, &bare_credit, 105).is_err());
    assert_eq!(record.phase, ExecutionPhaseV1::AwaitingSettlement);

    let partial = SettlementProofV1 {
        source_debit_bound: true, venue_execution_bound: true, output_credit_bound: true,
        refund_bound: true, fully_reconciled: true, effective_input_native: 5_000_000,
        refund_native: 4_990_000, ..bare_credit
    };
    reconcile_settlement(&mut record, &partial, 106).unwrap();
    assert_eq!(record.phase, ExecutionPhaseV1::HeldInventory);
}

#[test]
fn timeout_is_checked_and_deterioration_holds_inventory() {
    let mut record = prepare_execution(&candidate(), "exec-3", 101).unwrap();
    prepare_leg(&mut record, 9_990_000, 10_050_000, 102).unwrap();
    persist_leg_submission(&mut record, "fingerprint-3", 103).unwrap();
    mark_awaiting_settlement(&mut record, 104).unwrap();
    assert!(mark_reconciliation_required(&mut record, 108, 6).is_err());
    mark_reconciliation_required(&mut record, 109, 6).unwrap();
    assert_eq!(record.phase, ExecutionPhaseV1::ReconciliationRequired);

    let proof = SettlementProofV1 {
        request_fingerprint: "fingerprint-3".into(), planned_input_native: 9_990_000,
        effective_input_native: 9_990_000, gross_output_native: 10_100_000,
        refund_native: 0, source_debit_bound: true, venue_execution_bound: true,
        output_credit_bound: true, refund_bound: true, fully_reconciled: true,
    };
    reconcile_settlement(&mut record, &proof, 111).unwrap();
    assert_eq!(record.phase, ExecutionPhaseV1::LegSettled);
    record_remaining_route_requote(&mut record, false, 112).unwrap();
    assert_eq!(record.phase, ExecutionPhaseV1::HeldInventory);
}

#[test]
fn public_executor_delegates_to_durable_runtime_with_admin_guard() {
    let source = include_str!("../src/lib.rs");
    for method in ["prepare_route_execution_v1", "advance_route_execution_v1", "reconcile_route_execution_v1"] {
        let tail = source.split(&format!("fn {method}")).nth(1).unwrap();
        let body = tail.split("\n#[").next().unwrap();
        assert!(body.contains("require_admin()"));
        assert!(body.contains("route_runtime::"));
    }
}

#[test]
fn stable_exit_toggle_is_admin_gated_and_mutates_only_its_policy_field() {
    let source = include_str!("../src/lib.rs");
    let tail = source
        .split("fn set_wrapped_stable_to_icusd_allowed_v1")
        .nth(1)
        .expect("field-specific stable-exit endpoint");
    let body = tail.split("\n#[").next().unwrap();
    assert!(body.contains("require_admin()"));
    assert!(body.contains("allow_wrapped_stable_to_icusd"));
    assert!(body.contains("route_arb_config_generation"));
    assert!(body.contains("route_observation = None"));
    assert!(!body.contains("s.route_arb = config"));
}

#[test]
fn full_route_config_writer_preserves_the_field_with_a_dedicated_setter() {
    let source = include_str!("../src/lib.rs");
    let tail = source.split("fn set_route_arb_config_v1").nth(1).unwrap();
    let body = tail.split("\n#[").next().unwrap();
    assert!(body.contains(
        "config.allow_wrapped_stable_to_icusd = s.route_arb.allow_wrapped_stable_to_icusd"
    ));
}

#[test]
fn full_refund_aborts_and_reconciliation_queries_never_exceed_hard_cap() {
    let mut record = prepare_execution(&candidate(), "exec-refund", 101).unwrap();
    prepare_leg(&mut record, 9_990_000, 10_050_000, 102).unwrap();
    persist_leg_submission(&mut record, "fingerprint-refund", 103).unwrap();
    mark_awaiting_settlement(&mut record, 104).unwrap();
    let proof = SettlementProofV1 {
        request_fingerprint: "fingerprint-refund".into(), planned_input_native: 9_990_000,
        effective_input_native: 0, gross_output_native: 0, refund_native: 9_990_000,
        source_debit_bound: true, venue_execution_bound: true, output_credit_bound: true,
        refund_bound: true, fully_reconciled: true,
    };
    reconcile_settlement(&mut record, &proof, 105).unwrap();
    assert_eq!(record.phase, ExecutionPhaseV1::Aborted);

    let mut other = prepare_execution(&candidate(), "exec-budget", 101).unwrap();
    consume_reconciliation_queries(&mut other, 31, 32).unwrap();
    consume_reconciliation_queries(&mut other, 1, 32).unwrap();
    assert!(consume_reconciliation_queries(&mut other, 1, 32).is_err());
    assert!(consume_reconciliation_queries(&mut other, 1, 33).is_err());
}
