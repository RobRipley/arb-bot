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
    assert!(!route_panel.contains("Legacy A-S/T activity is historical only"), "legacy disclosure belongs in Diagnostics");
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

#[test]
fn automatic_arbitrage_control_is_ops_only_and_native() {
    let route_panel = rendered_region(
        "function routeArbitrageHtml()",
        "async function loadTerminalExecutionsForToday",
    );
    let ops = rendered_region("function renderOps()", "// ═══════ Ledger");
    let runtime = rendered_region("function routeRuntimeHtml()", "window.setRouteTrading");

    assert!(route_panel.contains("route-arbitrage-status") || route_panel.contains("routeTradingLabel"));
    assert!(route_panel.contains("goTo('ops')") || route_panel.contains("Ops"));
    assert!(!route_panel.contains("setRouteTrading("), "route panel must not mutate automatic arbitrage");

    assert!(ops.contains("data-ops-automatic-control"));
    assert!(runtime.contains("aria-pressed"), "automation control must expose pressed state");
    for label in ["On", "Off", "Applying", "Blocked", "Unknown", "Starting", "Stopping", "Stopped"] {
        assert!(ops.contains(label) || runtime.contains(label), "Ops control missing state label: {label}");
    }
    assert!(DASHBOARD.contains("Start automatic arbitrage"));
    assert!(DASHBOARD.contains("New swaps will stop. Any previously submitted swap will continue to be reconciled."));
    assert!(!DASHBOARD.contains("<div class=\"lever-toggle"), "lever controls must be native buttons");
    assert!(DASHBOARD.contains("<button type=\"button\" class=\"lever-toggle"));
}

#[test]
fn dashboard_has_exactly_five_primary_views_with_one_mount_each() {
    let nav = rendered_region("const VIEWS = [", "function renderNav()");
    let expected = [
        ("cockpit", "Cockpit"),
        ("markets", "Markets"),
        ("ops", "Ops"),
        ("ledger", "Ledger"),
        ("diagnostics", "Diagnostics"),
    ];
    assert_eq!(nav.matches("{ id:").count(), expected.len());
    for (id, label) in expected {
        assert!(nav.contains(&format!("id: '{id}'")), "missing primary view id: {id}");
        assert!(nav.contains(&format!("label: '{label}'")), "missing primary view label: {label}");
        assert_eq!(DASHBOARD.matches(&format!("id=\"view-{id}\"")).count(), 1, "view mount must be unique: {id}");
    }
    for retired in ["charts", "money"] {
        assert!(!nav.contains(&format!("id: '{retired}'")), "retired primary view remains: {retired}");
        assert_eq!(DASHBOARD.matches(&format!("id=\"view-{retired}\"")).count(), 0, "retired mount remains: {retired}");
    }
}

#[test]
fn dashboard_assigns_content_to_unique_owners() {
    let markets = rendered_region("function renderMarkets()", "// ═══════ Force confirm");
    let ops = rendered_region("function renderOps()", "// ═══════ Ledger");
    let diagnostics = rendered_region("function renderDiagnostics()", "// ═══════ Ledger");

    assert!(markets.contains("renderCharts") || markets.contains("chart-tabs"), "Markets must own charts");
    assert!(markets.contains("wallet") || markets.contains("Wallet") || markets.contains("route-wallet-grid"), "Markets must retain wallet readiness");
    assert!(ops.contains("Automatic arbitrage"), "Ops must own automatic arbitrage");
    assert!(ops.contains("Volume engine"), "Ops must own volume operations");
    assert!(diagnostics.contains("Route lock") || diagnostics.contains("Diagnostics"), "Diagnostics must own runtime diagnostics");

    assert!(!markets.contains("setRouteTrading("), "automatic arbitrage control must not render in Markets");
    assert!(!markets.contains("trigger_volume_cycle"), "volume operation must not render in Markets");
}

#[test]
fn ops_exposes_default_on_ck_stable_protection_as_a_toggle() {
    let ops = rendered_region("function renderOps()", "// ═══════ Ledger");
    let dashboard = include_str!("../src/dashboard.html");
    assert!(ops.contains("Protect ck stables"));
    assert!(ops.contains("leverStableExitProtection()"));
    assert!(ops.contains("allow_wrapped_stable_to_icusd"));
    assert!(ops.contains("'routeArbConfig'"));
    assert!(dashboard.contains("set_wrapped_stable_to_icusd_allowed_v1"));
    assert!(dashboard.contains("markSourceFailed(routeSources.routeArbConfig"));
    assert!(dashboard.contains("markSourceUnavailable(routeSources.observation"));
    assert!(dashboard.contains("markSourceUnavailable(routeSources.candidates"));
    assert!(dashboard.contains("await loadRouteData()"));
}

#[test]
fn diagnostics_owns_low_level_route_evidence_without_copying_cockpit_content() {
    let diagnostics = rendered_region("function renderDiagnostics()", "// ═══════ Ledger");
    let operations = rendered_region("function routeOperationsHtml()", "function manualQuoteScanState");
    let evidence = rendered_region("function diagnosticsEvidenceHtml", "function routeOperationsHtml");
    let diagnostics = format!("{diagnostics}\n{evidence}\n{operations}");
    let cockpit = rendered_region("function renderCockpit()", "// ═══════ Markets");

    for marker in [
        "Runtime",
        "Execution and settlement",
        "Reservations and held inventory",
        "Observation internals",
        "Legacy state",
        "data-diagnostics-source=\"runtime\"",
        "data-diagnostics-source=\"lock\"",
        "data-diagnostics-source=\"currentExecution\"",
        "data-diagnostics-source=\"terminalExecutions\"",
        "data-diagnostics-source=\"reservations\"",
        "data-diagnostics-source=\"heldPositions\"",
        "data-diagnostics-source=\"observation\"",
        "data-diagnostics-source=\"health\"",
        "data-diagnostics-source=\"config\"",
        "Raw cursor",
        "quote calls",
        "source_reference",
        "diagnosticsSourceFailureLabel",
        "Reservation ID",
        "Held position ID",
        "Legacy A-S/T activity is historical only",
    ] {
        assert!(diagnostics.contains(marker), "Diagnostics missing low-level marker: {marker}");
    }
    for marker in ["routeOperationsHtml()", "Raw cursor", "Reservation ID", "Held position ID"] {
        assert!(!cockpit.contains(marker), "Cockpit must not own low-level diagnostic detail: {marker}");
    }
    assert!(diagnostics.contains("routeSourceState('runtime'"));
    assert!(diagnostics.contains("routeSourceState('config'"));
    assert!(diagnostics.contains("routeSourceState('health'"));
}

#[test]
fn cockpit_exposes_ordered_operational_state_sections_and_truthful_labels() {
    let cockpit = rendered_region("function renderCockpit()", "// ═══════ Markets");
    for marker in [
        "data-cockpit-state",
        "data-cockpit-heartbeat",
        "data-cockpit-phase",
        "data-cockpit-realized-results",
        "data-cockpit-latest-execution",
        "data-cockpit-incidents",
        "Stopped",
        "Scanning",
        "Executing",
        "Confirming",
        "Reconciling",
        "Blocked",
        "Unknown",
    ] {
        assert!(cockpit.contains(marker), "Cockpit missing truthful state marker: {marker}");
    }
    let state_pos = cockpit.find("data-cockpit-state").unwrap();
    let phase_pos = cockpit.find("data-cockpit-phase").unwrap();
    let results_pos = cockpit.find("data-cockpit-realized-results").unwrap();
    let latest_pos = cockpit.find("data-cockpit-latest-execution").unwrap();
    let incidents_pos = cockpit.find("data-cockpit-incidents").unwrap();
    assert!(state_pos < phase_pos && phase_pos < results_pos && results_pos < latest_pos && latest_pos < incidents_pos);
}

#[test]
fn cockpit_keeps_execution_and_terminal_failures_distinct_from_empty_success() {
    let status = rendered_region("function cockpitStatus()", "function renderCockpit()");
    let cockpit = rendered_region("function renderCockpit()", "// ═══════ Markets");
    for source in ["currentExecution", "terminalExecutions"] {
        assert!(status.contains(&format!("routeSourceState('{source}'")), "Cockpit status must inspect {source} source state");
    }
    for state in ["failed", "stale", "unavailable", "Unknown", "Unavailable", "Stale"] {
        assert!(status.contains(state) || cockpit.contains(state), "Cockpit must render explicit {state} state");
    }
    assert!(cockpit.contains("data-cockpit-execution-state"));
    assert!(cockpit.contains("data-cockpit-terminal-state"));
    assert!(status.contains("cockpitSourceLabel"));
    assert!(!cockpit.contains("latestRouteExecution ? `${esc(latestRouteExecution.execution_id)} · leg"), "raw current execution ternary bypasses source state");
    assert!(cockpit.contains("cockpit.terminalSource.state === 'fresh'"), "terminal detail must be gated by fresh source state");
}

#[test]
fn cockpit_reports_today_realized_result_and_terminal_counts() {
    let cockpit = rendered_region("function renderCockpit()", "// ═══════ Markets");
    for marker in [
        "data-cockpit-today-results",
        "Today's realized result",
        "Completed",
        "Failed",
        "terminalExecutions",
        "realized_profit",
    ] {
        assert!(cockpit.contains(marker), "Cockpit missing today's terminal metric: {marker}");
    }
}

#[test]
fn cockpit_latest_terminal_card_preserves_non_completed_phases() {
    let status = rendered_region("function cockpitStatus()", "function renderCockpit()");
    let cockpit = rendered_region("function renderCockpit()", "// ═══════ Markets");
    assert!(cockpit.contains("Latest terminal execution"));
    assert!(!cockpit.contains("Latest completed execution"));
    assert!(status.contains("No terminal route execution"));
    assert!(cockpit.contains("data-cockpit-terminal-phase"));
    assert!(cockpit.contains("routePhaseLabel(latestTerminalExecutions[0].phase)"));
}

#[test]
fn ops_volume_handlers_and_balance_loader_repaint_the_new_owners() {
    let cycle = rendered_region("window.doRunVolumeCycle", "window.doVolumeRebalance");
    let rebalance = rendered_region("window.doVolumeRebalance", "// ═══════ Charts");
    assert!(cycle.contains("renderOps()"));
    assert!(rebalance.contains("renderOps()"));
    assert!(!cycle.contains("renderMarkets()"));
    assert!(!rebalance.contains("renderMarkets()"));

    let balances = rendered_region("async function loadMyBalances", "// ═══════ Market state computation");
    assert!(balances.contains("renderMarkets()"), "balance refresh must repaint Markets after its ownership move");
}

#[test]
fn primary_navigation_and_cockpit_status_chips_are_keyboard_operable() {
    let nav = rendered_region("function renderNav()", "window.goTo");
    let cockpit = rendered_region("function renderCockpit()", "// ═══════ Markets");
    assert!(nav.contains("<button"));
    assert!(nav.contains("type=\"button\""));
    assert!(!nav.contains("<div class=\"nav-item"));
    assert!(cockpit.contains("<button type=\"button\" class=\"status-chip"));
    assert!(!cockpit.contains("<div class=\"status-chip\" onclick"), "clickable status chips should use native controls");
}

#[test]
fn realized_profit_totals_preserve_candidate_units() {
    let results = rendered_region("function cockpitTodayResults", "function cockpitStatus");
    let cockpit = rendered_region("function renderCockpit()", "// ═══════ Markets");
    assert!(results.contains("StablePar") || results.contains("StableSettledCrossAsset"));
    assert!(results.contains("IcpReturning"));
    assert!(results.contains("stableRealized"));
    assert!(results.contains("icpRealized"));
    assert!(results.contains("stableResult"));
    assert!(results.contains("icpResult"));
    assert!(cockpit.contains("Stable profit") || cockpit.contains("USD realized"));
    assert!(cockpit.contains("ICP profit") || cockpit.contains("ICP realized"));
}

#[test]
fn terminal_loader_is_bounded_and_covers_today_beyond_one_page() {
    let loader = rendered_region("function loadTerminalExecutionsForToday", "async function loadRouteData");
    assert!(loader.contains("TERMINAL_EXECUTIONS_PAGE_SIZE"));
    assert!(loader.contains("TERMINAL_EXECUTIONS_MAX"));
    assert!(loader.contains("get_terminal_route_executions_v1"));
    assert!(loader.contains("terminalExecutionsIncomplete"));
    assert!(loader.contains("100") || loader.contains("20"));
    assert!(loader.contains("nextOffset") || loader.contains("offset"));
}

#[test]
fn execution_source_degradation_does_not_override_runtime_status() {
    let status = rendered_region("function cockpitStatus()", "function renderCockpit()");
    assert!(!status.contains("sourceBoundState"));
    assert!(status.contains("runtimeState"));
    assert!(status.contains("executionSource"));
    assert!(status.contains("terminalSource"));
}

#[test]
fn mobile_primary_tabs_size_to_content() {
    let responsive = rendered_region("@media (max-width: 900px)", "@media (max-width: 480px)");
    assert!(responsive.contains(".nav-item"));
    assert!(responsive.contains("width: auto") || responsive.contains("width: max-content"));
}

#[test]
fn ledger_uses_lazy_route_execution_disclosures_and_labels_legacy_history() {
    let route_helpers = rendered_region(
        "function routeLedgerStatusKey",
        "function legendHtml",
    );
    let ledger = rendered_region(
        "function renderLedger()",
        "// ═══════ Progress indicator",
    );
    for marker in [
        "function routeLedgerEntryHtml",
        "function routeLegDetailHtml",
        "async function toggleLedgerExecution",
        "get_route_execution_detail_v1",
        "routeExecutionDetails",
        "routeExecutionDetailLoads",
        "data-ledger-execution-id",
        "bindRouteLedgerDisclosureHandlers",
        "aria-expanded",
        "aria-controls",
        "Leg ${index + 1} of ${count}",
        "Awaiting settlement",
        "No evidence yet",
        "Submission evidence",
        "Settlement evidence",
        "Retry",
    ] {
        assert!(route_helpers.contains(marker), "route ledger missing marker: {marker}");
    }
    let summary = route_helpers
        .split("function routeLedgerEntryHtml")
        .nth(1)
        .unwrap()
        .split("async function toggleLedgerExecution")
        .next()
        .unwrap();
    assert!(!summary.contains("get_route_execution_detail_v1"), "summary rendering must not eagerly fetch detail");
    assert!(!summary.contains("onclick="), "route execution IDs must use delegated handlers, not inline JS");
    assert!(!route_helpers
        .split("function routeLegDetailHtml")
        .nth(1)
        .unwrap()
        .split("function routeLedgerDetailStateHtml")
        .next()
        .unwrap()
        .contains("realized_profit"), "child legs must not render a second P&L");
    for marker in [
        "routeLedgerExecutions",
        "ROUTE_LEDGER_PAGE_SIZE",
        "ROUTE_LEDGER_MAX",
        "loadRouteLedgerPage",
        "routeLedgerIncomplete",
        "Incomplete · bounded at",
        "Stale · ${esc(sourceLastSuccessLabel(source))}",
    ] {
        assert!(DASHBOARD.contains(marker), "route ledger source missing marker: {marker}");
    }
    for marker in [
        "Route execution history",
        "id=\"route-ledger-body\"",
        "aria-label=\"Route execution ledger\"",
        "Legacy trade history",
        "Legacy TradeLeg records are historical only",
        "scope=\"col\"",
    ] {
        assert!(ledger.contains(marker), "ledger missing ownership/accessibility marker: {marker}");
    }
    let route_pos = ledger.find("Route execution history").unwrap();
    let legacy_pos = ledger.find("Legacy trade history").unwrap();
    assert!(route_pos < legacy_pos, "route executions must be the primary ledger section");
    let legacy = ledger
        .split("Legacy trade history")
        .nth(1)
        .unwrap()
        .split("Activity log")
        .next()
        .unwrap();
    assert_eq!(legacy.matches("<div").count(), legacy.matches("</div>").count(), "legacy card HTML must be balanced");
}
