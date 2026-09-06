const { readFileSync } = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');

const html = readFileSync('src/arb_bot/src/dashboard.html', 'utf8');
const stateSection = html.slice(
  html.indexOf('    function createSourceState()'),
  html.indexOf('    // ═══════ Button loading states')
);
assert(stateSection.includes('function createSourceState()'), 'dashboard must define source-state helpers');

const context = vm.createContext({ Date, console: { error() {} } });
vm.runInContext(stateSection, context);

const source = vm.runInContext('createSourceState()', context);
context.source = source;
vm.runInContext("markSourceFresh(source, [{ execution_id: 'exec-1' }], 1000)", context);

assert.equal(source.status, 'fresh');
assert.equal(source.value[0].execution_id, 'exec-1');
assert.equal(source.lastSuccessMs, 1000);

vm.runInContext("markSourceFailed(source, Error('query rejected'), 2000)", context);
assert.equal(source.status, 'stale');
assert.equal(source.value[0].execution_id, 'exec-1');
assert.equal(source.lastSuccessMs, 1000);
assert.equal(source.lastAttemptMs, 2000);

const firstFailure = vm.runInContext('createSourceState()', context);
context.firstFailure = firstFailure;
vm.runInContext("markSourceFailed(firstFailure, Error('offline'), 3000)", context);
assert.equal(firstFailure.status, 'failed');
assert.equal(firstFailure.value, null);

const unavailable = vm.runInContext('createSourceState()', context);
context.unavailable = unavailable;
vm.runInContext("markSourceUnavailable(unavailable, 'method missing', 4000)", context);
assert.equal(unavailable.status, 'unavailable');
assert.equal(unavailable.value, null);

const fresh = vm.runInContext('createSourceState()', context);
context.fresh = fresh;
vm.runInContext('markSourceFresh(fresh, { ok: true }, 5000)', context);
assert.equal(vm.runInContext('sourceDisplayState(fresh, 30000, 34999)', context), 'fresh');
assert.equal(vm.runInContext('sourceDisplayState(fresh, 30000, 35000)', context), 'stale');

Object.assign(context, {
  currentConfig: { quote_max_age_ns: 30_000_000_000n },
  authenticatedActor: null,
  ROUTE_RUNTIME_STALE_AFTER_MS: 30_000,
  variantKey: value => value && typeof value === 'object' ? Object.keys(value)[0] : '—',
  esc: String,
  ROUTE_ASSET_LABELS: { Icp: 'ICP' },
  ROUTE_ASSET_DECIMALS: { Icp: 8 },
});
const routeHelperSection = html.slice(html.indexOf('    function routeAssetKey'), html.indexOf('    function routeCandidateHtml'));
vm.runInContext(routeHelperSection, context);
assert.equal(vm.runInContext('routeRuntimePayloadTimestampMs({ Ok: { last_tick_ns: 1000000000n } })', context), 1000);
assert.equal(vm.runInContext("routeCandidateQuoteState({ quote_timestamp_ns: 60000000000n }, 100000)", context), 'stale');
assert.equal(vm.runInContext("routeCandidateQuoteState({ quote_timestamp_ns: 90000000000n }, 100000)", context), 'fresh');
assert(vm.runInContext('sourceLastSuccessLabel({ lastSuccessMs: 1000 })', context).includes('as of'));
vm.runInContext(`const nowMs = Date.now(); Object.keys(routeSources).forEach(name => markSourceFresh(routeSources[name], null, nowMs)); markSourceFresh(routeSources.candidates, { stable: [], icp: [] }, nowMs); markSourceFresh(routeSources.runtime, { Ok: { last_tick_ns: BigInt(nowMs) * 1000000n } }, nowMs);`, context);
assert.equal(vm.runInContext('routeAggregateState()', context), 'fresh');
vm.runInContext("markSourceFresh(routeSources.runtime, { Ok: { last_tick_ns: 60000000000n } }, 60000); routeSources.runtime.lastAttemptMs = Date.now();", context);
assert.equal(vm.runInContext('routeAggregateState()', context), 'stale');

const candidateSection = html.slice(html.indexOf('    function routeCandidateHtml'), html.indexOf('    function routeWalletHtml'));
vm.runInContext(candidateSection, context);
const staleCandidate = `{ asset_path: [{ Icp: null }], venue_edges: ['Rumi3Pool'], rejection_reason: [], canonical_cycle_id: [], net_profit_bps: 1n, net_profit_native: 1n, principal_native: 1n, start_asset: { Icp: null }, quote_timestamp_ns: 60000000000n, full_fill: true, allowance_status: 'ok', inventory_effect: 'ok', eligible: true }`;
assert(vm.runInContext(`routeCandidateHtml(${staleCandidate}, 'stable-returning')`, context).includes('Stale · as of'));

Object.assign(context, { window: {}, isAdmin: true, latestRouteRuntime: { compiled_support: true, live_authorized: true, enabled: true, dry_run: false, last_error: [], last_realized_profit: [], last_profit_class: [] }, openModal() {}, requireAuth: () => true, unwrapResult: value => value.Ok, authenticatedActor: {}, loadRouteData: async () => {}, renderAll() {}, toast() {}, latestRouteLock: null, latestRouteReservations: [], latestHeldPositions: [], latestRouteExecution: null, latestRouteWallet: [] });
vm.runInContext("markSourceFresh(routeSources.runtime, latestRouteRuntime, 60000); routeSources.runtime.lastAttemptMs = Date.now();", context);
const runtimeSection = html.slice(html.indexOf('    function routeTradingLabel'), html.indexOf('    function routeArbitrageHtml'));
vm.runInContext(runtimeSection, context);
const staleRuntimeHtml = vm.runInContext('routeRuntimeHtml()', context);
assert(staleRuntimeHtml.includes('Blocked · stale runtime status'));
assert(!staleRuntimeHtml.includes('onclick='));
const stoppedRuntime = `{ compiled_support: true, live_authorized: false, enabled: false, dry_run: true, last_tick_ns: 0n, last_error: [], last_realized_profit: [], last_profit_class: [] }`;
const activeRuntime = `{ compiled_support: true, live_authorized: true, enabled: true, dry_run: false, last_tick_ns: 0n, last_error: [], last_realized_profit: [], last_profit_class: [] }`;
assert.equal(vm.runInContext(`routeRuntimePayloadTimestampMs({ Ok: ${stoppedRuntime} }, 12345)`, context), 12345);
assert.equal(vm.runInContext(`routeRuntimePayloadTimestampMs({ Ok: ${activeRuntime} }, 12345)`, context), 0);
vm.runInContext(`markSourceFresh(routeSources.runtime, ${stoppedRuntime}, Date.now()); latestRouteRuntime = routeSources.runtime.value;`, context);
const stoppedRuntimeHtml = vm.runInContext('routeRuntimeHtml()', context);
assert(stoppedRuntimeHtml.includes('Start arbitrage'));
assert(stoppedRuntimeHtml.includes('onclick='));
vm.runInContext(`markSourceFresh(routeSources.runtime, ${activeRuntime}, 0); routeSources.runtime.lastAttemptMs = Date.now(); latestRouteRuntime = routeSources.runtime.value;`, context);
const activeZeroRuntimeHtml = vm.runInContext('routeRuntimeHtml()', context);
assert(activeZeroRuntimeHtml.includes('Blocked · stale runtime status'));
assert(!activeZeroRuntimeHtml.includes('onclick='));

const operationsSection = html.slice(html.indexOf('    function routeWalletHtml'), html.indexOf('    function routeTradingLabel'));
vm.runInContext(operationsSection, context);
assert.equal(vm.runInContext("diagnosticsSourceFailureLabel('failed')", context), 'Failed', 'Diagnostics must label an internal query failure');
assert.equal(vm.runInContext("diagnosticsSourceFailureLabel('unavailable')", context), 'Unavailable', 'Diagnostics must label a missing query as unavailable');
assert(html.includes('data-diagnostics-source="lock"'), 'Diagnostics must own mutation-lock source state');
assert(html.includes('data-diagnostics-source="terminalExecutions"'), 'Diagnostics must own terminal-execution source state');
vm.runInContext("markSourceFresh(routeSources.lock, [{ owner: { Bot: null }, operation_id: 'lock-1', reconciliation_required: false }], 60000); markSourceFailed(routeSources.lock, Error('offline'), Date.now()); latestRouteLock = routeSources.lock.value[0];", context);
assert(vm.runInContext('routeOperationsHtml()', context).includes('Stale · as of'));

assert(html.includes('const routeSources'), 'dashboard must expose named route source states');
assert(!html.includes('const safe = async (fn, fallback, label)'), 'route loader must not use fallback-safe queries');
assert(html.includes('ROUTE_RUNTIME_STALE_AFTER_MS = 30_000'), 'runtime staleness must be exactly 30 seconds');
assert(html.includes('max_quote_age_ns'), 'quote staleness must use configured max quote age');
assert(html.includes('sourceDisplayState'), 'rendering must consult per-source display state');
assert(html.includes('Unavailable'), 'failed and unavailable sources need explicit UI labels');
assert(html.includes('Stale'), 'cached source values need an explicit stale label');
assert(html.includes('routeRuntimePayloadTimestampMs'), 'runtime freshness must use payload heartbeat timestamp');
assert(html.includes('routeCandidateQuoteState'), 'quote freshness must use each candidate timestamp');
assert(html.includes('sourceLastSuccessLabel'), 'stale cached source values need last-success timestamps');
assert(html.includes('routeAggregateState'), 'page freshness must aggregate route source states');
assert(html.includes('Blocked · stale runtime status'), 'stale runtime must be blocked');
assert(html.includes("runtimeState === 'fresh'"), 'stale runtime must not expose an actionable mutation button');

const loadRouteDataSection = html.slice(html.indexOf('    async function loadRouteData()'), html.indexOf('    window.startRouteObservation'));
const terminalLoaderSection = html.slice(html.indexOf('    async function loadTerminalExecutionsForToday'), html.indexOf('    async function loadRouteData()'));
const runtime = { compiled_support: true, live_authorized: false, enabled: false, dry_run: true, last_tick_ns: 1000000000n };
const routeActor = {
  get_route_arb_status_v1: async () => ({ config_valid: true, execution_compiled_in: true, live_execution_authorized: false, route_count: 1 }),
  get_route_observation_v1: async () => null,
  get_best_route_candidates_v1: async () => ({ stable: [], icp: [] }),
  get_route_mutation_lock_v1: async () => [{ owner: { Bot: null }, operation_id: 'lock-1', reconciliation_required: false }],
  get_route_reservations_v1: async () => ({ Ok: [{ reservation_id: 'reservation-1', active: true }] }),
  get_held_positions_v1: async () => ({ Ok: [{ position_id: 'held-1', lots: [] }] }),
  get_current_route_execution_v1: async () => [{ execution_id: 'exec-1', route_id: 'route-1', phase: { Quoting: null } }],
  get_terminal_route_executions_v1: null,
  get_route_runtime_status_v1: async () => ({ Ok: runtime }),
};
const walletActor = {
  get_route_wallet_balances_v1: async () => [{ asset: { Icp: null }, balance_native: [1n], metadata_valid: true, error: [] }],
};
Object.assign(context, {
  anonymousActor: routeActor,
  authenticatedActor: walletActor,
  routeDataRequestPromise: null,
  latestRouteStatus: null,
  latestRouteObservation: null,
  latestRouteCandidates: null,
  latestRouteLock: null,
  latestRouteReservations: [],
  latestHeldPositions: [],
  latestRouteExecution: null,
  latestTerminalExecutions: [],
  latestRouteRuntime: null,
  latestRouteWallet: [],
  routeOpt: value => Array.isArray(value) && value.length ? value[0] : null,
  TERMINAL_EXECUTIONS_PAGE_SIZE: 100n,
  TERMINAL_EXECUTIONS_MAX: 1000n,
  terminalExecutionsIncomplete: false,
  terminalExecutionsLoadedCount: 0,
});
const terminalRows = Array.from({ length: 125 }, (_, index) => ({
  execution_id: `exec-${index}`,
  updated_at_ns: BigInt(Date.now()) * 1000000n - BigInt(index) * 1000000000n,
  phase: { Completed: null },
}));
let terminalQueryCount = 0;
routeActor.get_terminal_route_executions_v1 = async (offset, limit) => {
  terminalQueryCount += 1;
  const start = Number(offset);
  return { Ok: terminalRows.slice(start, start + Number(limit)) };
};
vm.runInContext(terminalLoaderSection, context);
vm.runInContext(loadRouteDataSection, context);
(async () => {
  await vm.runInContext('loadRouteData()', context);
  assert.equal(vm.runInContext('routeSources.terminalExecutions.status', context), 'fresh');
  assert.equal(vm.runInContext('latestTerminalExecutions.length', context), 125, 'today metrics must fetch beyond the old 20-row page');
  assert.equal(terminalQueryCount, 2, 'bounded today coverage should use two 100-row pages for 125 rows');
  assert.equal(vm.runInContext('terminalExecutionsIncomplete', context), false);
  for (const name of ['lock', 'currentExecution', 'reservations', 'heldPositions', 'runtime', 'wallet']) {
    assert.equal(vm.runInContext(`routeSources.${name}.status`, context), 'fresh', `${name} should load fresh`);
  }
  assert.equal(vm.runInContext('routeSources.runtime.lastSuccessMs', context), 1000);

  const cockpitSection = html.slice(html.indexOf('    function cockpitSourceLabel'), html.indexOf('    function renderCockpit'));
  const todayNs = BigInt(Date.now()) * 1000000n;
  Object.assign(context, {
    cockpitStates: { currentExecution: 'fresh', terminalExecutions: 'fresh', runtime: 'fresh' },
    routeSourceState: name => context.cockpitStates[name] || 'fresh',
    routeSourceError: () => '',
    sourceLastSuccessLabel: () => 'as of now',
    routePhaseLabel: () => 'Completed',
    routeAge: () => '1s',
    bi: value => typeof value === 'bigint' ? Number(value) : value,
    fmt$: value => `$${(Number(value) / 1e6).toFixed(4)}`,
    fmtTok: (value, decimals) => (Number(value) / 10 ** decimals).toFixed(decimals > 6 ? 4 : 2),
    latestRouteExecution: null,
    latestRouteRuntime: { compiled_support: true, live_authorized: true, enabled: true, dry_run: false, last_tick_ns: todayNs },
    latestTerminalExecutions: [
      { execution_id: 'stable-1', updated_at_ns: todayNs - 1000000n, candidate_class: { StablePar: null }, phase: { Completed: null }, realized_profit: [1250000n] },
      { execution_id: 'icp-1', updated_at_ns: todayNs - 2000000n, candidate_class: { IcpReturning: null }, phase: { Aborted: null }, realized_profit: [123456789n] },
    ],
    terminalExecutionsIncomplete: false,
    terminalExecutionsLoadedCount: 2,
  });
  vm.runInContext(cockpitSection, context);
  const todayMetrics = vm.runInContext('cockpitTodayResults("fresh")', context);
  assert.equal(todayMetrics.stableResult, '$1.2500', 'stable terminal profit must stay USD6');
  assert.equal(todayMetrics.icpResult, '1.2346 ICP', 'ICP terminal profit must stay ICP e8s');
  assert.equal(todayMetrics.completed, '1');
  assert.equal(todayMetrics.failed, '1');
  context.cockpitStates.currentExecution = 'stale';
  const staleCurrent = vm.runInContext('cockpitStatus()', context);
  assert.equal(staleCurrent.label, 'Scanning', 'current source degradation must not override runtime status');
  assert.match(staleCurrent.executionSource.label, /^Stale/);
  context.cockpitStates.currentExecution = 'fresh';
  context.cockpitStates.terminalExecutions = 'unavailable';
  const unavailableTerminal = vm.runInContext('cockpitStatus()', context);
  assert.equal(unavailableTerminal.label, 'Scanning', 'terminal source degradation must not override runtime status');
  assert.match(unavailableTerminal.terminalSource.label, /^Unavailable/);
  context.cockpitStates.terminalExecutions = 'fresh';
  context.cockpitStates.runtime = 'stale';
  assert.equal(vm.runInContext('cockpitStatus().label', context), 'Blocked', 'stale runtime must block independently');

  for (const name of Object.keys(routeActor)) routeActor[name] = async () => { throw Error('query rejected'); };
  walletActor.get_route_wallet_balances_v1 = async () => { throw Error('query rejected'); };
  await vm.runInContext('loadRouteData()', context);
  for (const name of ['lock', 'currentExecution', 'reservations', 'heldPositions', 'runtime', 'wallet']) {
    assert.equal(vm.runInContext(`routeSources.${name}.status`, context), 'stale', `${name} should retain stale data`);
  }
  delete routeActor.get_route_mutation_lock_v1;
  await vm.runInContext('loadRouteData()', context);
  assert.equal(vm.runInContext('routeSources.lock.status', context), 'unavailable');
  console.log('PASS: route loader preserves cached values and distinguishes unavailable methods');
})().catch(error => { console.error(error); process.exitCode = 1; });
console.log('PASS: source-state transitions and explicit freshness guards are covered');
