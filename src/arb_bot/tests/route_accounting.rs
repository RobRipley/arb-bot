use arb_bot::route_arb::{
    available_native, evaluate_candidate, net_profit_bps_checked, par_usd_6dec_checked,
    rank_book, Asset, AssetAmounts, CandidateEvaluation, InventoryBands, ProfitDomain, QuoteLeg,
    ReservationTotals, RouteQuote,
};

fn amounts(entries: &[(Asset, Option<u128>)]) -> AssetAmounts {
    let mut out = AssetAmounts::unknown();
    for (asset, value) in entries {
        out.set(*asset, *value);
    }
    out
}

#[test]
fn par_conversion_is_checked_and_floors_native_amounts() {
    assert_eq!(par_usd_6dec_checked(123_456_789, 8), Ok(1_234_567));
    assert_eq!(par_usd_6dec_checked(1_234_567, 6), Ok(1_234_567));
    assert_eq!(par_usd_6dec_checked(123, 2), Ok(1_230_000));
    assert!(par_usd_6dec_checked(u128::MAX, 2).is_err());
    assert!(par_usd_6dec_checked(1, 80).is_err());
}

#[test]
fn bps_uses_initial_principal_and_signed_truncation() {
    assert_eq!(net_profit_bps_checked(5, 1_000), Ok(50));
    assert_eq!(net_profit_bps_checked(4, 1_000), Ok(40));
    assert_eq!(net_profit_bps_checked(-4, 1_000), Ok(-40));
    assert_eq!(net_profit_bps_checked(-1, 3), Ok(-3_333));
    assert!(net_profit_bps_checked(1, 0).is_err());
    assert!(net_profit_bps_checked(i128::MAX, 1).is_err());
}

#[test]
fn available_balance_subtracts_every_reservation_and_fails_closed() {
    let reservations = ReservationTotals {
        held: amounts(&[(Asset::CkUsdc, Some(10))]),
        active: amounts(&[(Asset::CkUsdc, Some(20))]),
        non_route: amounts(&[(Asset::CkUsdc, Some(30))]),
    };
    assert_eq!(available_native(Asset::CkUsdc, Some(100), &reservations), Ok(40));
    assert!(available_native(Asset::CkUsdc, None, &reservations).is_err());
    assert!(available_native(Asset::CkUsdc, Some(59), &reservations).is_err());
}

fn stable_quote(final_amount: u128) -> RouteQuote {
    RouteQuote {
        route_id: "two-pool-stable".into(),
        canonical_cycle_id: Some("cycle".into()),
        start_asset: Asset::CkUsdc,
        end_asset: Asset::CkUsdc,
        asset_path: vec![Asset::CkUsdc, Asset::IcUsd, Asset::CkUsdc],
        principal_native: 100_000_000,
        legs: vec![
            QuoteLeg {
                edge_id: "rumi:CkUsdc>IcUsd".into(),
                from: Asset::CkUsdc,
                to: Asset::IcUsd,
                wallet_before: 100_000_000,
                entry_ledger_fee: 10_000,
                venue_input: 99_990_000,
                gross_output: 10_200_000_000,
                output_ledger_fee: 100_000,
                wallet_after: 10_199_900_000,
                dex_fee_native: 10_000,
                full_fill: true,
            },
            QuoteLeg {
                edge_id: "icpswap:IcUsd>CkUsdc".into(),
                from: Asset::IcUsd,
                to: Asset::CkUsdc,
                wallet_before: 10_199_900_000,
                entry_ledger_fee: 100_000,
                venue_input: 10_199_800_000,
                gross_output: final_amount + 10_000,
                output_ledger_fee: 10_000,
                wallet_after: final_amount,
                dex_fee_native: 300_000,
                full_fill: true,
            },
        ],
        allowance_sufficient: Some(true),
        quoted_at_ns: 1,
        size_ladder_index: 0,
    }
}

fn permissive_context() -> (AssetAmounts, ReservationTotals, InventoryBands) {
    let balances = amounts(&Asset::ALL.map(|asset| (asset, Some(1_000_000_000_000u128))));
    let reservations = ReservationTotals::default();
    let mut bands = InventoryBands::unbounded();
    for asset in Asset::ALL {
        bands.set(asset, 0, u128::MAX);
    }
    (balances, reservations, bands)
}

#[test]
fn fee_recurrence_and_dual_thresholds_drive_stable_eligibility() {
    let (balances, reservations, bands) = permissive_context();
    let evaluation = evaluate_candidate(
        &stable_quote(101_000_000),
        &balances,
        &reservations,
        &bands,
        500_000,
        50,
        0,
        0,
    );
    assert!(evaluation.eligible, "{:?}", evaluation.rejection_reason);
    assert_eq!(evaluation.profit_domain, ProfitDomain::StableParUsd6Dec);
    assert_eq!(evaluation.net_profit_native, 1_000_000);
    assert_eq!(evaluation.net_profit_bps, 100);

    let below_absolute = evaluate_candidate(
        &stable_quote(100_400_000), &balances, &reservations, &bands, 500_000, 10, 0, 0,
    );
    assert!(!below_absolute.eligible);
    assert_eq!(below_absolute.rejection_reason.as_deref(), Some("below stable absolute-profit threshold"));
}

#[test]
fn malformed_chain_partial_fill_unknown_allowance_and_inventory_fail_closed() {
    let (balances, reservations, bands) = permissive_context();
    let mut quote = stable_quote(101_000_000);
    quote.legs[1].venue_input += 1;
    assert!(!evaluate_candidate(&quote, &balances, &reservations, &bands, 1, 1, 0, 0).eligible);

    let mut quote = stable_quote(101_000_000);
    quote.legs[0].full_fill = false;
    assert!(!evaluate_candidate(&quote, &balances, &reservations, &bands, 1, 1, 0, 0).eligible);

    let mut quote = stable_quote(101_000_000);
    quote.allowance_sufficient = None;
    assert!(!evaluate_candidate(&quote, &balances, &reservations, &bands, 1, 1, 0, 0).eligible);

    let low_balance = amounts(&[(Asset::CkUsdc, Some(99_999_999))]);
    assert!(!evaluate_candidate(&stable_quote(101_000_000), &low_balance, &reservations, &bands, 1, 1, 0, 0).eligible);
}

#[test]
fn same_asset_terminal_ceiling_uses_post_debit_balance() {
    let balances = amounts(&[
        (Asset::CkUsdc, Some(1_000_000_000)),
        (Asset::IcUsd, Some(0)),
    ]);
    let reservations = ReservationTotals::default();
    let mut bands = InventoryBands::unbounded();
    bands.set(Asset::CkUsdc, 0, 1_001_000_000);
    bands.set(Asset::IcUsd, 0, u128::MAX);
    let result = evaluate_candidate(
        &stable_quote(101_000_000), &balances, &reservations, &bands, 1, 1, 0, 0,
    );
    assert!(result.eligible, "principal is removed before same-token proceeds return: {:?}", result.rejection_reason);
}

#[test]
fn icp_profit_stays_native_and_changed_stable_terminal_uses_par() {
    let (balances, reservations, bands) = permissive_context();
    let icp = RouteQuote {
        route_id: "icp-cycle".into(), canonical_cycle_id: Some("icp-cycle".into()),
        start_asset: Asset::Icp, end_asset: Asset::Icp,
        asset_path: vec![Asset::Icp, Asset::IcUsd, Asset::Icp],
        principal_native: 100_000_000,
        legs: vec![
            QuoteLeg { edge_id: "a".into(), from: Asset::Icp, to: Asset::IcUsd, wallet_before: 100_000_000, entry_ledger_fee: 10_000, venue_input: 99_990_000, gross_output: 1_000_100_000, output_ledger_fee: 100_000, wallet_after: 1_000_000_000, dex_fee_native: 1, full_fill: true },
            QuoteLeg { edge_id: "b".into(), from: Asset::IcUsd, to: Asset::Icp, wallet_before: 1_000_000_000, entry_ledger_fee: 100_000, venue_input: 999_900_000, gross_output: 101_010_000, output_ledger_fee: 10_000, wallet_after: 101_000_000, dex_fee_native: 1, full_fill: true },
        ], allowance_sufficient: Some(true), quoted_at_ns: 1, size_ladder_index: 0,
    };
    let evaluated = evaluate_candidate(&icp, &balances, &reservations, &bands, 0, 0, 500_000, 50);
    assert!(evaluated.eligible);
    assert_eq!(evaluated.profit_domain, ProfitDomain::IcpE8s);
    assert_eq!(evaluated.net_profit_native, 1_000_000);

    let changed = RouteQuote {
        route_id: "cross-stable".into(), canonical_cycle_id: None,
        start_asset: Asset::CkUsdc, end_asset: Asset::IcUsd,
        asset_path: vec![Asset::CkUsdc, Asset::IcUsd], principal_native: 100_000_000,
        legs: vec![QuoteLeg { edge_id: "c".into(), from: Asset::CkUsdc, to: Asset::IcUsd, wallet_before: 100_000_000, entry_ledger_fee: 10_000, venue_input: 99_990_000, gross_output: 10_100_100_000, output_ledger_fee: 100_000, wallet_after: 10_100_000_000, dex_fee_native: 1, full_fill: true }],
        allowance_sufficient: Some(true), quoted_at_ns: 1, size_ladder_index: 0,
    };
    let evaluated = evaluate_candidate(&changed, &balances, &reservations, &bands, 1, 1, 0, 0);
    assert!(evaluated.eligible && evaluated.par_assumption);
    assert_eq!(evaluated.net_profit_native, 1_000_000);
}

fn ranked(id: &str, profit: i128, bps: i64, legs: usize, size_index: u8) -> CandidateEvaluation {
    CandidateEvaluation {
        route_id: id.into(), canonical_cycle_id: None, start_asset: Asset::CkUsdc,
        end_asset: Asset::CkUsdc, profit_domain: ProfitDomain::StableParUsd6Dec,
        principal_native: 1, net_profit_native: profit, net_profit_bps: bps,
        leg_count: legs as u8, size_ladder_index: size_index, par_assumption: false,
        eligible: true, rejection_reason: None,
    }
}

#[test]
fn ranking_has_a_total_deterministic_order() {
    let mut candidates = vec![
        ranked("z", 10, 100, 2, 0), ranked("b", 11, 80, 4, 0),
        ranked("a", 11, 90, 4, 1), ranked("a", 11, 90, 3, 2),
        ranked("a", 11, 90, 3, 1),
    ];
    rank_book(&mut candidates);
    assert_eq!(candidates.iter().map(|item| (item.route_id.as_str(), item.net_profit_native, item.leg_count, item.size_ladder_index)).collect::<Vec<_>>(), vec![
        ("a", 11, 3, 1), ("a", 11, 3, 2), ("a", 11, 4, 1), ("b", 11, 4, 0), ("z", 10, 2, 0),
    ]);
}
