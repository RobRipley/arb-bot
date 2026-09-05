use arb_bot::route_arb::{
    validate_route_config, wallet_rows_from_results, Asset, LedgerReadResult, RouteArbConfigV1,
    HARD_MAX_CONCURRENT_QUOTES, HARD_MAX_QUOTE_AGE_NS, HARD_MAX_RECONCILIATION_QUERIES_PER_CYCLE,
    HARD_MAX_ROUTE_LEGS, HARD_MAX_SETTLEMENT_TIMEOUT_NS, HARD_MAX_SIZE_LADDER_ENTRIES,
};

#[test]
fn policy_defaults_are_observable_but_execution_inert() {
    let config = RouteArbConfigV1::default();
    assert!(!config.enabled);
    assert!(config.dry_run);
    assert!(config.stable_book_enabled);
    assert!(config.icp_book_enabled);
    assert_eq!(config.asset_controls.len(), 6);
    assert_eq!(config.pool_controls.len(), 15);
    assert!(config.asset_controls.iter().all(|item| item.enabled));
    assert!(config.pool_controls.iter().all(|item| item.enabled));
    validate_route_config(&config).expect("defaults valid");
}

#[test]
fn resource_and_time_boundaries_fail_closed() {
    let mut config = RouteArbConfigV1::default();
    for mutate in [
        |c: &mut RouteArbConfigV1| c.max_route_legs = 0,
        |c: &mut RouteArbConfigV1| c.max_route_legs = HARD_MAX_ROUTE_LEGS + 1,
        |c: &mut RouteArbConfigV1| c.quote_max_age_ns = 0,
        |c: &mut RouteArbConfigV1| c.quote_max_age_ns = HARD_MAX_QUOTE_AGE_NS + 1,
        |c: &mut RouteArbConfigV1| c.settlement_timeout_ns = 0,
        |c: &mut RouteArbConfigV1| c.settlement_timeout_ns = HARD_MAX_SETTLEMENT_TIMEOUT_NS + 1,
        |c: &mut RouteArbConfigV1| c.reconciliation_queries_per_cycle = 0,
        |c: &mut RouteArbConfigV1| c.reconciliation_queries_per_cycle = HARD_MAX_RECONCILIATION_QUERIES_PER_CYCLE + 1,
        |c: &mut RouteArbConfigV1| c.max_concurrent_quote_calls = 0,
        |c: &mut RouteArbConfigV1| c.max_concurrent_quote_calls = HARD_MAX_CONCURRENT_QUOTES + 1,
    ] {
        config = RouteArbConfigV1::default();
        mutate(&mut config);
        assert!(validate_route_config(&config).is_err());
    }

    config = RouteArbConfigV1::default();
    config.stable_size_ladder = vec![1; HARD_MAX_SIZE_LADDER_ENTRIES as usize + 1];
    assert!(validate_route_config(&config).is_err());
}

#[test]
fn invalid_inventory_and_duplicate_controls_are_rejected() {
    let mut config = RouteArbConfigV1::default();
    config.inventory_bands[0].floor_native = 11;
    config.inventory_bands[0].ceiling_native = 10;
    assert!(validate_route_config(&config).is_err());

    let mut config = RouteArbConfigV1::default();
    config.asset_controls.push(config.asset_controls[0].clone());
    assert!(validate_route_config(&config).is_err());

    let mut config = RouteArbConfigV1::default();
    config.pool_controls.pop();
    assert!(validate_route_config(&config).is_err());
}

#[test]
fn wallet_report_contains_all_assets_and_exposes_read_failures() {
    let reads = Asset::ALL.map(|asset| {
        if asset == Asset::CkEth {
            LedgerReadResult::failed("ledger unavailable")
        } else {
            LedgerReadResult::ok(asset.symbol(), asset.decimals(), 10, 0)
        }
    });
    let rows = wallet_rows_from_results(reads);
    assert_eq!(rows.len(), 6);
    assert_eq!(rows.iter().map(|row| row.asset).collect::<Vec<_>>(), Asset::ALL);
    let eth = rows.iter().find(|row| row.asset == Asset::CkEth).unwrap();
    assert_eq!(eth.balance_native, None);
    assert!(!eth.metadata_valid);
    assert_eq!(eth.error.as_deref(), Some("ledger unavailable"));
    let btc = rows.iter().find(|row| row.asset == Asset::CkBtc).unwrap();
    assert_eq!(btc.balance_native, Some(0));
    assert!(btc.metadata_valid);
}
