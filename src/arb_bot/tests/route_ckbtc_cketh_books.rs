//! Focused coverage for the ckBTC-returning and ckETH-returning profit
//! books: default-off behavior, size ladders/caps, min-profit + bps
//! thresholds, quote-universe/budget growth, the omitted-field merge used by
//! `set_route_arb_config_v1`, and execution-endpoint validation.

use arb_bot::route_arb::{
    build_work_universe, evaluate_candidate, prepare_execution, resolve_incoming_book_fields,
    validate_route_config, AssetAmounts, AssetReturnBookConfigV1, CandidateClass, InventoryBands,
    ProfitDomain, QuoteLeg, ReservationTotals, RouteArbConfigV1, RouteCandidateReportV1, RouteQuote,
};

fn permissive_context() -> (AssetAmounts, ReservationTotals, InventoryBands) {
    let balances = AssetAmounts::zero();
    let reservations = ReservationTotals::default();
    let bands = InventoryBands::unbounded();
    (balances, reservations, bands)
}

#[test]
fn ckbtc_and_cketh_books_are_disabled_by_default_and_contribute_no_work_items() {
    let config = RouteArbConfigV1::default();
    assert!(!config.ckbtc_book_resolved().enabled);
    assert!(!config.cketh_book_resolved().enabled);

    let universe = build_work_universe(&config).expect("default config validates");
    assert!(universe.items.iter().all(|item| !matches!(
        item.route.candidate_class,
        CandidateClass::CkBtcReturning | CandidateClass::CkEthReturning
    )));
}

#[test]
fn enabling_ckbtc_book_admits_native_principal_work_items_from_its_own_ladder() {
    let mut config = RouteArbConfigV1::default();
    config.ckbtc_book = Some(AssetReturnBookConfigV1 {
        enabled: true,
        size_ladder: vec![10_000, 20_000, 30_000],
        max_principal_native: 100_000,
        min_profit_native: 1,
        min_profit_bps: 1,
    });
    validate_route_config(&config).expect("enabled ckBTC book with a valid ladder must validate");

    let universe = build_work_universe(&config).unwrap();
    let ckbtc_items: Vec<_> = universe
        .items
        .iter()
        .filter(|item| item.route.candidate_class == CandidateClass::CkBtcReturning)
        .collect();
    assert!(!ckbtc_items.is_empty(), "enabling the book must admit ckBTC-returning work items");
    // Native size ladders are used as-is (unlike the stable ladder, which is
    // USD6-denominated and converted via native_stable_principal).
    let principals: std::collections::BTreeSet<_> =
        ckbtc_items.iter().map(|item| item.principal_native).collect();
    assert_eq!(principals, [10_000, 20_000, 30_000].into_iter().collect());
    // Every distinct route gets one work item per ladder rung.
    let distinct_routes: std::collections::BTreeSet<_> =
        ckbtc_items.iter().map(|item| item.route.route_id.as_str()).collect();
    assert_eq!(ckbtc_items.len(), distinct_routes.len() * 3);

    // ckETH stays untouched (still disabled) alongside the enabled ckBTC book.
    assert!(universe
        .items
        .iter()
        .all(|item| item.route.candidate_class != CandidateClass::CkEthReturning));
}

#[test]
fn validate_route_config_rejects_invalid_ckbtc_and_cketh_ladders() {
    let base = RouteArbConfigV1::default();

    let mut empty_ladder = base.clone();
    empty_ladder.ckbtc_book = Some(AssetReturnBookConfigV1 {
        enabled: true,
        size_ladder: vec![],
        ..AssetReturnBookConfigV1::default()
    });
    assert!(validate_route_config(&empty_ladder).is_err());

    let mut decreasing_ladder = base.clone();
    decreasing_ladder.ckbtc_book = Some(AssetReturnBookConfigV1 {
        enabled: true,
        size_ladder: vec![100, 100],
        ..AssetReturnBookConfigV1::default()
    });
    assert!(validate_route_config(&decreasing_ladder).is_err());

    let mut over_cap = base.clone();
    over_cap.cketh_book = Some(AssetReturnBookConfigV1 {
        enabled: true,
        size_ladder: vec![100, 200],
        max_principal_native: 150,
        ..AssetReturnBookConfigV1::default()
    });
    assert!(validate_route_config(&over_cap).is_err(), "200 exceeds the 150 cap");

    let mut zero_entry = base;
    zero_entry.cketh_book = Some(AssetReturnBookConfigV1 {
        enabled: true,
        size_ladder: vec![0, 100],
        ..AssetReturnBookConfigV1::default()
    });
    assert!(validate_route_config(&zero_entry).is_err());
}

#[test]
fn enabling_a_book_increases_required_quote_calls_by_exactly_its_own_share() {
    let baseline = RouteArbConfigV1::default();
    let baseline_universe = build_work_universe(&baseline).unwrap();

    let mut with_ckbtc = baseline.clone();
    with_ckbtc.ckbtc_book = Some(AssetReturnBookConfigV1 {
        enabled: true,
        ..AssetReturnBookConfigV1::default()
    });
    let with_ckbtc_universe = build_work_universe(&with_ckbtc).unwrap();

    let ckbtc_only_calls: u64 = with_ckbtc_universe
        .items
        .iter()
        .filter(|item| item.route.candidate_class == CandidateClass::CkBtcReturning)
        .map(|item| item.route.edges.len() as u64)
        .sum();
    assert!(ckbtc_only_calls > 0);
    assert_eq!(
        with_ckbtc_universe.required_quote_calls,
        baseline_universe.required_quote_calls + ckbtc_only_calls
    );
}

fn ckbtc_quote(principal: u128, final_amount: u128) -> RouteQuote {
    use arb_bot::route_arb::Asset;
    RouteQuote {
        route_id: "ckbtc-ckusdc-ckbtc".into(),
        canonical_cycle_id: Some("cycle".into()),
        start_asset: Asset::CkBtc,
        end_asset: Asset::CkBtc,
        asset_path: vec![Asset::CkBtc, Asset::CkUsdc, Asset::CkBtc],
        principal_native: principal,
        legs: vec![
            QuoteLeg {
                edge_id: "icpswap-ckbtc-ckusdc:CkBtc>CkUsdc".into(),
                from: Asset::CkBtc,
                to: Asset::CkUsdc,
                wallet_before: principal,
                entry_ledger_fee: 10,
                venue_input: principal - 10,
                gross_output: (principal - 10) * 100_000,
                output_ledger_fee: 1_000,
                wallet_after: (principal - 10) * 100_000 - 1_000,
                dex_fee_native: 1,
                full_fill: true,
            },
            QuoteLeg {
                edge_id: "icpswap-ckbtc-ckusdc:CkUsdc>CkBtc".into(),
                from: Asset::CkUsdc,
                to: Asset::CkBtc,
                wallet_before: (principal - 10) * 100_000 - 1_000,
                entry_ledger_fee: 1_000,
                venue_input: (principal - 10) * 100_000 - 2_000,
                gross_output: final_amount + 10,
                output_ledger_fee: 10,
                wallet_after: final_amount,
                dex_fee_native: 1,
                full_fill: true,
            },
        ],
        allowance_sufficient: Some(true),
        quoted_at_ns: 1,
        size_ladder_index: 0,
    }
}

#[test]
fn ckbtc_returning_profit_is_a_native_satoshi_diff_gated_by_absolute_and_bps_thresholds() {
    let (balances, reservations, bands) = permissive_context();
    let mut balances = balances;
    balances.set(arb_bot::route_arb::Asset::CkBtc, Some(u128::MAX / 2));
    balances.set(arb_bot::route_arb::Asset::CkUsdc, Some(u128::MAX / 2));

    // 100_000 sats principal, 101_000 sats final: 1_000 sats profit, 100 bps.
    let eligible = evaluate_candidate(
        &ckbtc_quote(100_000, 101_000),
        &balances,
        &reservations,
        &bands,
        0, 0, 0, 0,
        500, 50,
        0, 0,
    );
    assert!(eligible.eligible, "{:?}", eligible.rejection_reason);
    assert_eq!(eligible.profit_domain, ProfitDomain::CkBtcSats);
    assert_eq!(eligible.net_profit_native, 1_000);
    assert_eq!(eligible.net_profit_bps, 100);

    // Same shape, 100 sats profit (10 bps) — fails both the absolute and bps
    // gate when both are set above that.
    let below = evaluate_candidate(
        &ckbtc_quote(100_000, 100_100),
        &balances,
        &reservations,
        &bands,
        0, 0, 0, 0,
        500, 50,
        0, 0,
    );
    assert!(!below.eligible);
    assert_eq!(below.rejection_reason.as_deref(), Some("below ckBTC absolute-profit threshold"));

    // Profit clears the absolute floor but not bps.
    let below_bps = evaluate_candidate(
        &ckbtc_quote(100_000, 100_600),
        &balances,
        &reservations,
        &bands,
        0, 0, 0, 0,
        500, 100,
        0, 0,
    );
    assert!(!below_bps.eligible);
    assert_eq!(below_bps.rejection_reason.as_deref(), Some("below ckBTC bps threshold"));
}

#[test]
fn resolve_incoming_book_fields_preserves_current_value_when_omitted_but_honors_explicit_updates() {
    let mut current = RouteArbConfigV1::default();
    current.ckbtc_book = Some(AssetReturnBookConfigV1 {
        enabled: true,
        size_ladder: vec![42],
        max_principal_native: 42,
        min_profit_native: 1,
        min_profit_bps: 1,
    });

    // An old-shaped caller (field decodes as None) must not silently disable
    // or clobber the currently stored, already-activated book.
    let mut old_caller_request = current.clone();
    old_caller_request.ckbtc_book = None;
    old_caller_request.cketh_book = None;
    let resolved = resolve_incoming_book_fields(old_caller_request, &current);
    assert_eq!(resolved.ckbtc_book, current.ckbtc_book);
    assert_eq!(resolved.cketh_book, Some(current.cketh_book_resolved()));

    // A caller that explicitly supplies a new value still takes effect.
    let mut explicit_request = current.clone();
    explicit_request.ckbtc_book = Some(AssetReturnBookConfigV1 {
        enabled: false,
        ..AssetReturnBookConfigV1::default()
    });
    let resolved_explicit = resolve_incoming_book_fields(explicit_request.clone(), &current);
    assert_eq!(resolved_explicit.ckbtc_book, explicit_request.ckbtc_book);
}

#[test]
fn prepare_execution_accepts_native_returning_ckbtc_and_cketh_endpoints_and_rejects_mismatches() {
    let ckbtc_candidate =
        RouteCandidateReportV1::fixture("ckbtc-route", CandidateClass::CkBtcReturning, 100, true);
    assert!(prepare_execution(&ckbtc_candidate, "exec-ckbtc", 1).is_ok());

    let cketh_candidate =
        RouteCandidateReportV1::fixture("cketh-route", CandidateClass::CkEthReturning, 100, true);
    assert!(prepare_execution(&cketh_candidate, "exec-cketh", 1).is_ok());

    // A candidate tagged CkBtcReturning whose endpoints do not actually match
    // ckBTC->ckBTC must be rejected, not silently accepted as some other
    // profit domain.
    let mut mismatched = ckbtc_candidate;
    mismatched.end_asset = arb_bot::route_arb::Asset::CkEth;
    assert!(prepare_execution(&mismatched, "exec-mismatch", 1).is_err());
}
