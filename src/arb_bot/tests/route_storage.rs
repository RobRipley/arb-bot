use arb_bot::route_arb::{
    Asset, CandidateClass, ExecutionPhaseV1, ExecutionRecordV1, HeldBasisV1, HeldLotV1,
    HeldPositionV1, MutationOwnerV1, OwnershipReservationV1, ReconciliationEvidenceV1,
    ReservationKindV1, RouteExecutionDetailV1, RouteExecutionLegStatusV1, RouteExecutionLegV1,
    VenueKind,
};

fn detail_fixture() -> RouteExecutionDetailV1 {
    RouteExecutionDetailV1 {
        record: ExecutionRecordV1 {
            execution_id: "route-storage-detail".into(),
            route_id: "route-storage-route".into(),
            canonical_cycle_id: None,
            candidate_class: CandidateClass::StablePar,
            phase: ExecutionPhaseV1::Completed,
            current_leg_index: 0,
            planned_input_native: 100,
            required_min_output_native: 99,
            quote_timestamp_ns: 1,
            submission_started_at_ns: Some(2),
            adapter_request_fingerprint: None,
            evidence: vec![],
            reconciliation_query_count: 3,
            incident: None,
            updated_at_ns: 3,
            realized_profit: Some(1),
        },
        asset_path: vec![Asset::CkUsdc, Asset::IcUsd],
        legs: vec![RouteExecutionLegV1 {
            leg_index: 0,
            status: RouteExecutionLegStatusV1::Settled,
            edge_id: "fixture-edge".into(),
            pool_id: "fixture-pool".into(),
            pool_principal: candid::Principal::anonymous(),
            venue: VenueKind::Rumi3Pool,
            from: Asset::CkUsdc,
            to: Asset::IcUsd,
            quoted_input_native: 100,
            quoted_output_native: Some(101),
            minimum_output_native: 99,
            input_fee_native: 1,
            output_fee_native: 1,
            actual_input_debit_native: Some(101),
            actual_effective_input_native: Some(100),
            actual_output_credit_native: Some(100),
            refund_credit_native: None,
            prepared_at_ns: Some(2),
            submitted_at_ns: Some(3),
            settled_at_ns: Some(4),
            reconciled_at_ns: Some(5),
            evidence: vec![ReconciliationEvidenceV1 {
                evidence_kind: "block".into(),
                source_reference: "fixture".into(),
                amount_native: 100,
                observed_at_ns: 5,
            }],
            incident: None,
        }],
        detail_available: true,
    }
}

#[test]
fn detail_capacity_rejects_65537_encoded_bytes() {
    let mut detail = detail_fixture();
    detail.legs[0].evidence = (0..5)
        .map(|index| ReconciliationEvidenceV1 {
            evidence_kind: format!("evidence-{index}"),
            source_reference: "x".repeat(16_384),
            amount_native: 1,
            observed_at_ns: index,
        })
        .collect();
    let error = arb_bot::state::put_route_execution_detail(detail).unwrap_err();
    assert!(error.contains("65,536-byte cap"));
}

#[test]
fn terminal_detail_retry_is_idempotent_but_changed_detail_is_rejected() {
    let detail = detail_fixture();
    arb_bot::state::put_route_execution_detail(detail.clone()).unwrap();
    arb_bot::state::put_route_execution_detail(detail.clone()).unwrap();
    let mut changed = detail;
    changed.legs[0].actual_output_credit_native = Some(98);
    assert!(arb_bot::state::put_route_execution_detail(changed).is_err());
    assert_eq!(
        arb_bot::state::get_route_execution_detail("route-storage-detail")
            .unwrap()
            .unwrap()
            .legs[0]
            .actual_output_credit_native,
        Some(100)
    );
}

#[test]
fn detail_map_is_empty_for_pre_detail_runtime_without_changing_current_record() {
    let record = ExecutionRecordV1 {
        execution_id: "pre-detail-current".into(),
        route_id: "pre-detail-route".into(),
        canonical_cycle_id: None,
        candidate_class: CandidateClass::StablePar,
        phase: ExecutionPhaseV1::Planned,
        current_leg_index: 0,
        planned_input_native: 0,
        required_min_output_native: 0,
        quote_timestamp_ns: 0,
        submission_started_at_ns: None,
        adapter_request_fingerprint: None,
        evidence: vec![],
        reconciliation_query_count: 0,
        incident: None,
        updated_at_ns: 0,
        realized_profit: None,
    };
    arb_bot::state::put_current_route_execution(record.clone()).unwrap();
    assert!(arb_bot::state::get_route_execution_detail("pre-detail-current")
        .unwrap()
        .is_none());
    assert_eq!(
        arb_bot::state::find_route_execution_record("pre-detail-current")
            .unwrap()
            .unwrap(),
        record
    );
}

#[test]
fn detail_index_has_total_entry_cap_but_allows_idempotent_replacement() {
    let mut first_inserted = None;
    let mut inserted = 0u64;
    for index in 0..arb_bot::state::HARD_MAX_ROUTE_EXECUTION_DETAILS {
        let mut detail = detail_fixture();
        detail.record.execution_id = format!("route-storage-capacity-{index}");
        match arb_bot::state::put_route_execution_detail(detail.clone()) {
            Ok(()) => {
                inserted += 1;
                first_inserted.get_or_insert(detail);
            }
            Err(error) => {
                assert!(error.contains("capacity"));
                break;
            }
        }
    }
    assert!(inserted >= arb_bot::state::HARD_MAX_ROUTE_EXECUTION_DETAILS - 3);
    let existing = first_inserted.expect("capacity fixture insertion");
    arb_bot::state::put_route_execution_detail(existing.clone()).unwrap();
    let mut new_detail = existing;
    new_detail.record.execution_id = "route-storage-capacity-overflow".into();
    assert!(arb_bot::state::put_route_execution_detail(new_detail)
        .unwrap_err()
        .contains("capacity"));
}

#[test]
fn durable_lock_is_exclusive_and_owner_bound() {
    arb_bot::state::release_mutation_lock_for_test();
    let first = arb_bot::state::acquire_mutation_lock(
        "route-storage-lock-1",
        MutationOwnerV1::RouteExecution,
        10,
    ).unwrap();
    assert_eq!(first.operation_id, "route-storage-lock-1");
    assert!(arb_bot::state::acquire_mutation_lock(
        "other",
        MutationOwnerV1::VolumeOperation,
        11,
    ).is_err());
    assert!(arb_bot::state::release_mutation_lock("other").is_err());
    arb_bot::state::release_mutation_lock("route-storage-lock-1").unwrap();
}

#[test]
fn held_position_creates_exact_per_asset_reservations() {
    let id = "held-storage-test";
    let held = HeldPositionV1 {
        position_id: id.into(),
        originating_execution_id: "exec-1".into(),
        originating_route_id: "route-1".into(),
        basis: HeldBasisV1::StablePar {
            start_asset: Asset::CkUsdc,
            principal_native: 2_000_000,
            principal_usd_6dec: 2_000_000,
        },
        lots: vec![HeldLotV1 {
            asset: Asset::CkBtc,
            native_amount: 75_000,
            attributable_fees_native: 50,
            reserved_native: 75_000,
        }],
        reason: "route deteriorated".into(),
        first_held_timestamp_ns: 10,
        last_reconciled_timestamp_ns: 11,
    };
    arb_bot::state::put_held_position(held).unwrap();
    assert_eq!(arb_bot::state::reservation_totals_for_asset(Asset::CkBtc).held, 75_000);
    let page = arb_bot::state::get_held_positions_page(0, 100).unwrap();
    assert!(page.iter().any(|row| row.position_id == id));
}

#[test]
fn reservations_are_bounded_and_whole_asset_freezes_spending() {
    let reservation = OwnershipReservationV1 {
        reservation_id: "freeze-storage-test".into(),
        asset: Asset::Icp,
        amount_native: 0,
        whole_asset_freeze: true,
        kind: ReservationKindV1::LegacyFreeze,
        owner: MutationOwnerV1::LegacyMigration,
        operation_id: "legacy-pending-exit".into(),
        reconciled_at_ns: 0,
        active: true,
    };
    arb_bot::state::put_ownership_reservation(reservation.clone()).unwrap();
    let totals = arb_bot::state::reservation_totals_for_asset(Asset::Icp);
    assert!(totals.whole_asset_frozen);
    assert!(arb_bot::state::spendable_native(Asset::Icp, 1_000_000).is_err());
    assert!(arb_bot::state::get_ownership_reservations_page(0, 101).is_err());

    // Reconciliation updates the indexed current-state row instead of
    // appending another durable event. Repeating this many times must remain
    // one reservation, and releasing it must remove the current row.
    for timestamp in 1..=300 {
        let mut replacement = reservation.clone();
        replacement.reconciled_at_ns = timestamp;
        arb_bot::state::put_ownership_reservation(replacement).unwrap();
    }
    let page = arb_bot::state::get_ownership_reservations_page(0, 100).unwrap();
    assert_eq!(page.iter().filter(|row| row.reservation_id == "freeze-storage-test").count(), 1);

    let mut released = reservation;
    released.active = false;
    arb_bot::state::put_ownership_reservation(released).unwrap();
    assert!(!arb_bot::state::reservation_totals_for_asset(Asset::Icp).whole_asset_frozen);
}
