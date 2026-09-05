const DASHBOARD: &str = include_str!("../src/dashboard.html");

fn rendered_region(start: &str, end: &str) -> &'static str {
    let start_at = DASHBOARD
        .find(start)
        .unwrap_or_else(|| panic!("dashboard is missing rendered section marker: {start}"));
    let body = &DASHBOARD[start_at..];
    let end_at = body
        .find(end)
        .unwrap_or_else(|| panic!("dashboard is missing rendered section end marker: {end}"));
    &body[..end_at]
}

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
    let markets = rendered_region(
        "function renderMarkets()",
        "// ═══════ Force confirm",
    );
    let money = rendered_region(
        "function renderMoney()",
        "// ═══════ Swap panel logic",
    );
    let ops = rendered_region(
        "function renderOps()",
        "// ═══════ Ledger",
    );
    let route_panel = rendered_region(
        "function routeArbitrageHtml()",
        "async function loadRouteData",
    );

    // Check the code that builds the live DOM templates, rather than a prefix
    // that happens to end before the IDL/legacy compatibility section.
    for retired in [
        "Force A",
        "Force S",
        "Strategy S</span>",
        "onclick=\"openForce(",
    ] {
        assert!(!markets.contains(retired), "retired action is rendered by Markets: {retired}");
        assert!(!ops.contains(retired), "retired action is rendered by Ops: {retired}");
    }
    for retired in [
        "id=\"swap-from-token\"",
        "id=\"swap-execute-btn\"",
        "onclick=\"confirmSwap()\"",
        "rumi_manual_swap",
    ] {
        assert!(!money.contains(retired), "retired manual swap is rendered by Money: {retired}");
    }
    for retired in [
        "leverArb(",
        "leverRumiAmm(",
        "leverStrategyS(",
        "setupTableHtml(",
        "doSetupApprovals(",
        "Run approvals",
        "cfg-arb-interval",
        "cfg-partydex-",
        "cfg-bob-",
    ] {
        assert!(!ops.contains(retired), "retired control is rendered by Ops: {retired}");
    }
    assert!(route_panel.contains("Start observation"));
    assert!(route_panel.contains("Quote-only mode cannot move funds"));
    assert!(route_panel.contains("Legacy A-S/T activity is historical only"));
}

#[test]
fn dashboard_does_not_initialize_the_removed_manual_swap_panel() {
    let admin_ui = rendered_region(
        "async function applyAdminUi(adminFlag)",
        "window.doLogin = async function",
    );
    let money = rendered_region(
        "function renderMoney()",
        "// ═══════ Swap panel logic",
    );

    assert!(!admin_ui.contains("swapUpdateBalances()"));
    assert!(!money.contains("swapTokenChanged()"));
    assert!(admin_ui.contains("volSwapUpdateBalances()"));
    assert!(money.contains("volSwapTokenChanged()"));
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
