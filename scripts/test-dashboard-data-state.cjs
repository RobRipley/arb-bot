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

const operationsSection = html.slice(html.indexOf('    function routeWalletHtml'), html.indexOf('    function routeTradingLabel'));
vm.runInContext(operationsSection, context);
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
const runtime = { compiled_support: true, live_authorized: false, enabled: false, dry_run: true, last_tick_ns: 1000000000n };
const routeActor = {
  get_route_arb_status_v1: async () => ({ config_valid: true, execution_compiled_in: true, live_execution_authorized: false, route_count: 1 }),
  get_route_observation_v1: async () => null,
  get_best_route_candidates_v1: async () => ({ stable: [], icp: [] }),
  get_route_mutation_lock_v1: async () => [{ owner: { Bot: null }, operation_id: 'lock-1', reconciliation_required: false }],
  get_route_reservations_v1: async () => ({ Ok: [{ reservation_id: 'reservation-1', active: true }] }),
  get_held_positions_v1: async () => ({ Ok: [{ position_id: 'held-1', lots: [] }] }),
  get_current_route_execution_v1: async () => [{ execution_id: 'exec-1', route_id: 'route-1', phase: { Quoting: null } }],
  get_terminal_route_executions_v1: async () => ({ Ok: [] }),
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
});
vm.runInContext(loadRouteDataSection, context);
(async () => {
  await vm.runInContext('loadRouteData()', context);
  for (const name of ['lock', 'currentExecution', 'reservations', 'heldPositions', 'runtime', 'wallet']) {
    assert.equal(vm.runInContext(`routeSources.${name}.status`, context), 'fresh', `${name} should load fresh`);
  }
  assert.equal(vm.runInContext('routeSources.runtime.lastSuccessMs', context), 1000);
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
