const DASHBOARD: &str = include_str!("../src/dashboard.html");

#[test]
fn dashboard_exposes_the_consolidated_six_asset_route_surface() {
    for required in [
        "Route arbitrage",
        "route-wallet-grid",
        "route-stable-candidate",
        "route-icp-candidate",
        "route-observation-metrics",
        "route-reservations",
        "route-execution-state",
        "route-held-positions",
        "icUSD, ckUSDT, and ckUSDC are valued at $1 only for terminal profit accounting",
        "ckBTC",
        "ckETH",
        "Full fill",
        "Allowance",
        "Inventory",
        "Quote age",
        "Collision",
    ] {
        assert!(DASHBOARD.contains(required), "missing dashboard marker: {required}");
    }
}

#[test]
fn dashboard_does_not_render_retired_lettered_strategy_actions() {
    let visible_app = DASHBOARD
        .split("const idlFactory")
        .next()
        .expect("dashboard must contain an IDL factory");

    assert!(!visible_app.contains("Force A"));
    assert!(!visible_app.contains("Force S"));
    assert!(!visible_app.contains("Strategy S</span>"));
    assert!(DASHBOARD.contains("Legacy A-S/T activity is historical only"));
}

#[test]
fn volume_controls_remain_available_as_a_separate_engine() {
    for required in [
        "Volume engine",
        "Run cycle",
        "Rebalance",
        "trigger_volume_cycle",
        "trigger_volume_rebalance",
    ] {
        assert!(DASHBOARD.contains(required), "volume UI regressed: {required}");
    }
}
