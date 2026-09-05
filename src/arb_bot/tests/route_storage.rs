use arb_bot::route_arb::{
    Asset, HeldBasisV1, HeldLotV1, HeldPositionV1, MutationOwnerV1,
    OwnershipReservationV1, ReservationKindV1,
};

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
    arb_bot::state::put_ownership_reservation(reservation).unwrap();
    let totals = arb_bot::state::reservation_totals_for_asset(Asset::Icp);
    assert!(totals.whole_asset_frozen);
    assert!(arb_bot::state::spendable_native(Asset::Icp, 1_000_000).is_err());
    assert!(arb_bot::state::get_ownership_reservations_page(0, 101).is_err());
}
