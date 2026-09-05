use arb_bot::route_arb::{
    build_quote_from_outputs, build_work_universe, checked_quote_age, quote_method,
    accumulate_observation_batch, Asset, CandidateClass, LedgerFeeTable, ObservationAccumulatorV1,
    QuoteMethod, RouteArbConfigV1, RouteCandidateReportV1,
};

#[test]
fn default_observation_universe_is_complete_and_bounded() {
    let config = RouteArbConfigV1::default();
    let universe = build_work_universe(&config).expect("valid defaults");
    assert_eq!(config.max_route_legs, 3, "four-leg support is available but expands only by config");
    assert_eq!(universe.route_count, 246);
    assert_eq!(universe.items.len(), 972);
    assert_eq!(universe.required_quote_calls, 2_644);
    assert!(universe.required_quote_calls <= u64::from(config.max_quote_calls_per_observation));
    assert!(universe.items.windows(2).all(|pair| pair[0].work_id < pair[1].work_id));
    assert!(universe.items.iter().all(|item| item.principal_native > 0));
}

#[test]
fn disabled_assets_and_pools_remove_dependent_routes_instead_of_repointing() {
    let mut config = RouteArbConfigV1::default();
    config.asset_controls.iter_mut().find(|item| item.asset == Asset::CkBtc).unwrap().enabled = false;
    config.pool_controls.iter_mut().find(|item| item.pool_id == "icpswap-icp-ckusdc").unwrap().enabled = false;
    let universe = build_work_universe(&config).unwrap();
    assert!(universe.items.iter().all(|item| !item.route.asset_path.contains(&Asset::CkBtc)));
    assert!(universe.items.iter().all(|item| item.route.edges.iter().all(|edge| edge.pool_id != "icpswap-icp-ckusdc")));
}

#[test]
fn adapters_are_explicitly_full_fill_and_rumi_uses_native_indices() {
    let edges = arb_bot::route_arb::directed_edges();
    let rumi = edges.iter().find(|edge| edge.pool_id == "rumi-3pool" && edge.from == Asset::CkUsdc && edge.to == Asset::IcUsd).unwrap();
    assert_eq!(quote_method(rumi, None).unwrap(), QuoteMethod::RumiCalcSwap { coin_in: 2, coin_out: 0 });

    let icpswap = edges.iter().find(|edge| edge.pool_id == "icpswap-ckbtc-ckusdc" && edge.from == Asset::CkBtc).unwrap();
    assert_eq!(quote_method(icpswap, Some(Asset::CkBtc)).unwrap(), QuoteMethod::IcpSwapQuoteForAll { zero_for_one: true });
    assert_eq!(quote_method(icpswap, Some(Asset::CkUsdc)).unwrap(), QuoteMethod::IcpSwapQuoteForAll { zero_for_one: false });
    assert!(quote_method(icpswap, Some(Asset::Icp)).is_err());
}

#[test]
fn chained_quote_deducts_both_intermediate_ledger_movements() {
    let route = arb_bot::route_arb::enumerate_routes(2).unwrap().into_iter().find(|route| {
        route.asset_path == vec![Asset::CkUsdc, Asset::IcUsd, Asset::CkUsdc]
    }).unwrap();
    let mut fees = LedgerFeeTable::zero();
    fees.set(Asset::CkUsdc, 10_000);
    fees.set(Asset::IcUsd, 100_000);
    let quote = build_quote_from_outputs(&route, 100_000_000, &fees, &[10_200_000_000, 101_010_000], 7, 0).unwrap();
    assert_eq!(quote.legs[0].venue_input, 99_990_000);
    assert_eq!(quote.legs[0].wallet_after, 10_199_900_000);
    assert_eq!(quote.legs[1].venue_input, 10_199_800_000);
    assert_eq!(quote.legs[1].wallet_after, 101_000_000);
    assert!(quote.legs.iter().all(|leg| leg.full_fill));
}

#[test]
fn quote_age_rejects_clock_regression_and_exact_expiry() {
    assert_eq!(checked_quote_age(100, 90, 11), Ok(10));
    assert!(checked_quote_age(100, 90, 10).is_err());
    assert!(checked_quote_age(89, 90, 10).is_err());
}

fn report(id: &str, class: CandidateClass, profit: i64, eligible: bool) -> RouteCandidateReportV1 {
    RouteCandidateReportV1::fixture(id, class, profit, eligible)
}

#[test]
fn observation_accumulator_only_publishes_winners_after_complete_scan() {
    let mut state = ObservationAccumulatorV1::new("obs".into(), 10, 0, 3, 6, true);
    accumulate_observation_batch(&mut state, 0, vec![
        report("stable-low", CandidateClass::StablePar, 10, true),
        report("icp-best", CandidateClass::IcpReturning, 30, true),
    ], 4, 2).unwrap();
    assert_eq!(state.next_cursor, 2);
    assert!(state.best_stable_candidate.is_none());
    assert!(state.best_icp_candidate.is_none());

    accumulate_observation_batch(&mut state, 2, vec![
        report("stable-best", CandidateClass::StableSettledCrossAsset, 20, true),
    ], 2, 1).unwrap();
    assert!(state.scan_complete);
    assert_eq!(state.best_stable_candidate.as_ref().unwrap().route_id, "stable-best");
    assert_eq!(state.best_icp_candidate.as_ref().unwrap().route_id, "icp-best");
    assert!(accumulate_observation_batch(&mut state, 2, vec![], 0, 0).is_err());
}

#[test]
fn completed_observations_are_durably_page_bounded() {
    let mut first = ObservationAccumulatorV1::new("obs-storage-1".into(), 10, 0, 1, 1, true);
    first.scan_complete = true;
    first.completed_at_ns = Some(11);
    arb_bot::state::append_route_observation(first.clone()).expect("append completed observation");

    assert!(arb_bot::state::append_route_observation(
        ObservationAccumulatorV1::new("incomplete".into(), 12, 0, 1, 1, true)
    ).is_err());
    assert!(arb_bot::state::get_route_observations_page(0, 101).is_err());
    let page = arb_bot::state::get_route_observations_page(0, 100).unwrap();
    assert_eq!(page.last().unwrap().observation_id, first.observation_id);
}

#[test]
fn live_observation_adapter_contains_no_fund_moving_calls() {
    let source = include_str!("../src/route_arb.rs");
    let start = source.find("pub async fn quote_observation_items").unwrap();
    let adapter = &source[start..];
    for forbidden in ["depositFromAndSwap", "icrc2_transfer_from", "icrc1_transfer", "pool_swap("] {
        assert!(!adapter.contains(forbidden), "observation adapter must not reach {forbidden}");
    }
    assert!(source.contains("fetch_icpswap_quote_for_all"));
    assert!(source.contains("pool_calc_swap"));
}
