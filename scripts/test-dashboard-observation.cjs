const { readFileSync } = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');

const html = readFileSync('src/arb_bot/src/dashboard.html', 'utf8');

function section(start, end) {
  const from = html.indexOf(start);
  assert.notEqual(from, -1, `dashboard is missing ${start}`);
  const to = html.indexOf(end, from);
  assert.notEqual(to, -1, `dashboard is missing ${end}`);
  return html.slice(from, to);
}

function candidateContext(nowMs = 100_000) {
  const context = vm.createContext({
    Date: class extends Date { static now() { return nowMs; } },
    BigInt,
    console: { error() {} },
    currentConfig: { quote_max_age_ns: 30_000_000_000n },
    ROUTE_ASSET_LABELS: { Icp: 'ICP', IcUsd: 'icUSD' },
    ROUTE_ASSET_DECIMALS: { Icp: 8, IcUsd: 8 },
    variantKey: value => value && typeof value === 'object' ? Object.keys(value)[0] : '—',
    esc: value => String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c])),
  });
  vm.runInContext(section(
    '    function routeAssetKey',
    '    function routeWalletHtml',
  ), context);
  return context;
}

const freshContext = candidateContext();
const freshCandidate = {
  asset_path: [{ Icp: null }, { IcUsd: null }, { Icp: null }],
  venue_edges: ['ICPSwap ICP/icUSD', 'ICPSwap icUSD/ICP'],
  eligible: true,
  net_profit_bps: 137,
  net_profit_native: 1_250_000n,
  principal_native: 40_000_000n,
  start_asset: { Icp: null },
  quote_timestamp_ns: 90_000_000_000n,
  full_fill: true,
  allowance_status: 'confirmed',
  inventory_effect: 'within band',
  rejection_reason: [],
  canonical_cycle_id: [],
};
const freshHtml = vm.runInContext("routeCandidateHtml(candidate, 'stable-returning')", vm.createContext({
  ...freshContext,
  candidate: freshCandidate,
}));
assert.match(freshHtml, /Estimated/);
assert.doesNotMatch(freshHtml, /Realized/);
assert.match(freshHtml, /Fresh/);

const staleContext = candidateContext();
const staleHtml = vm.runInContext("routeCandidateHtml(candidate, 'stable-returning')", vm.createContext({
  ...staleContext,
  candidate: { ...freshCandidate, quote_timestamp_ns: 60_000_000_000n },
}));
assert.match(staleHtml, /Stale/);
assert.match(staleHtml, /configured quote age/);

const scanSection = section(
  '    function manualQuoteScanStillActive',
  '    function cockpitSourceLabel',
);

function scanContext(actor, initialView = 'markets') {
  const messages = [];
  const renders = [];
  const context = vm.createContext({
    window: {},
    state: { activeView: initialView },
    authenticatedActor: actor,
    latestRouteObservation: null,
    manualQuoteScan: { phase: 'ready', observationId: null, cursor: null, batchCount: 0, error: null, startedAtNs: null },
    manualQuoteScanGeneration: 0,
    requireAuth: () => true,
    unwrapResult: result => {
      if (result && Object.prototype.hasOwnProperty.call(result, 'Err')) throw new Error(result.Err);
      return result && result.Ok;
    },
    loadRouteData: async () => {},
    renderCurrentView: () => renders.push('render'),
    renderDiagnostics: () => renders.push('diagnostics'),
    toast: (message, kind) => messages.push({ message, kind }),
    console: { error() {} },
    setTimeout,
    clearTimeout,
  });
  vm.runInContext(scanSection, context);
  context.messages = messages;
  context.renders = renders;
  return context;
}

(async () => {
  let batchCalls = 0;
  const actor = {
    start_route_observation_v1: async () => ({ Ok: {
      observation_id: 'obs-1', next_cursor: 0n, scan_complete: false,
    } }),
    quote_route_observation_batch_v1: async cursor => {
      batchCalls += 1;
      assert.equal(cursor, BigInt((batchCalls - 1) * 100));
      return { Ok: { observation: { observation_id: 'obs-1', next_cursor: BigInt(batchCalls * 100), scan_complete: batchCalls === 2 } } };
    },
  };
  const complete = scanContext(actor);
  await vm.runInContext('window.runManualQuoteScan()', complete);
  assert.equal(batchCalls, 2);
  assert.equal(vm.runInContext('manualQuoteScan.phase', complete), 'complete');

  let rejectBatch;
  let releaseBatch;
  const inFlight = new Promise(resolve => { releaseBatch = resolve; });
  const overlapActor = {
    start_route_observation_v1: async () => ({ Ok: { observation_id: 'obs-2', next_cursor: 0n, scan_complete: false } }),
    quote_route_observation_batch_v1: async () => {
      rejectBatch = true;
      await inFlight;
      return { Ok: { observation: { observation_id: 'obs-2', next_cursor: 100n, scan_complete: true } } };
    },
  };
  const overlap = scanContext(overlapActor);
  const first = vm.runInContext('window.runManualQuoteScan()', overlap);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(rejectBatch, true);
  const second = await vm.runInContext('window.runManualQuoteScan()', overlap);
  assert.equal(second, false);
  vm.runInContext('state.activeView = "ops"', overlap);
  releaseBatch();
  await first;
  assert.equal(vm.runInContext('manualQuoteScan.phase', overlap), 'aborted');

  let failedStarts = 0;
  let failedBatches = 0;
  const failedActor = {
    start_route_observation_v1: async () => { failedStarts += 1; return { Ok: { observation_id: 'obs-3', next_cursor: 0n, scan_complete: false } }; },
    quote_route_observation_batch_v1: async () => {
      failedBatches += 1;
      if (failedBatches === 1) throw new Error('temporary provider failure');
      return { Ok: { observation: { observation_id: 'obs-3', next_cursor: 100n, scan_complete: true } } };
    },
  };
  const failed = scanContext(failedActor);
  await vm.runInContext('window.runManualQuoteScan()', failed);
  assert.equal(vm.runInContext('manualQuoteScan.phase', failed), 'failed');
  assert.equal(vm.runInContext('manualQuoteScan.cursor', failed), 0n);
  assert.equal(vm.runInContext('manualQuoteScan.observationId', failed), 'obs-3');
  assert.equal(vm.runInContext('manualQuoteScan.batchCount', failed), 0);
  await vm.runInContext('window.runManualQuoteScan()', failed);
  assert.equal(failedStarts, 1, 'resume must not start a new observation');
  assert.equal(vm.runInContext('manualQuoteScan.phase', failed), 'complete');

  const abortActor = {
    start_route_observation_v1: async () => ({ Ok: { observation_id: 'obs-4', next_cursor: 0n, scan_complete: false } }),
    quote_route_observation_batch_v1: async () => ({ Ok: { observation: { observation_id: 'obs-4', next_cursor: 100n, scan_complete: false } } }),
  };
  const aborted = scanContext(abortActor);
  vm.runInContext('state.activeView = "ops"', aborted);
  const abortedResult = await vm.runInContext('window.runManualQuoteScan()', aborted);
  assert.equal(abortedResult, false);
  assert.equal(vm.runInContext('manualQuoteScan.phase', aborted), 'aborted');
  assert.notEqual(vm.runInContext('manualQuoteScan.phase', aborted), 'complete');

  console.log('dashboard observation behavior tests passed');
})().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
