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

assert(html.includes('const routeSources'), 'dashboard must expose named route source states');
assert(!html.includes('const safe = async (fn, fallback, label)'), 'route loader must not use fallback-safe queries');
assert(html.includes('ROUTE_RUNTIME_STALE_AFTER_MS = 30_000'), 'runtime staleness must be exactly 30 seconds');
assert(html.includes('max_quote_age_ns'), 'quote staleness must use configured max quote age');
assert(html.includes('sourceDisplayState'), 'rendering must consult per-source display state');
assert(html.includes('Unavailable'), 'failed and unavailable sources need explicit UI labels');
assert(html.includes('Stale'), 'cached source values need an explicit stale label');

const loadRouteDataSection = html.slice(html.indexOf('    async function loadRouteData()'), html.indexOf('    window.startRouteObservation'));
const runtime = { compiled_support: true, live_authorized: false, enabled: false, dry_run: true };
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
