const { readFileSync } = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');

const html = readFileSync('src/arb_bot/src/dashboard.html', 'utf8');
const section = (start, end) => {
  const a = html.indexOf(start);
  const b = html.indexOf(end, a);
  assert.notEqual(a, -1, `missing section: ${start}`);
  assert.notEqual(b, -1, `missing section end: ${end}`);
  return html.slice(a, b);
};
const option = value => value == null ? [] : [value];

// Route freshness must come from the dedicated RouteArbConfigV1 query. A
// BotConfig-shaped cold cache must never provide a made-up route config.
const routeContext = vm.createContext({
  BigInt, Date, console: { error() {} },
  currentConfig: { max_quote_age_ns: 999_000_000_000n },
  routeArbConfig: null,
  routeSources: { routeArbConfig: { status: 'loading', value: null, error: null, lastSuccessMs: null, lastAttemptMs: null } },
  sourceDisplayState: source => source.status,
  sourceLastSuccessLabel: () => 'as of now',
  routeSourceState: name => routeContext.routeSources[name]?.status || 'loading',
  ROUTE_RUNTIME_STALE_AFTER_MS: 30_000,
  variantKey: value => value && typeof value === 'object' ? Object.keys(value)[0] : '—',
  esc: String,
  ROUTE_ASSET_LABELS: { Icp: 'ICP', CkUsdc: 'ckUSDC' },
  ROUTE_ASSET_DECIMALS: { Icp: 8, CkUsdc: 6 },
});
vm.runInContext(section('    function routeAssetKey', '    function routeCandidateHtml'), routeContext);
assert.equal(vm.runInContext('routeQuoteStaleAfterMs()', routeContext), null, 'BotConfig must not supply route quote age');
vm.runInContext('routeArbConfig = { quote_max_age_ns: 30_000_000_000n }; routeSources.routeArbConfig.status = "fresh";', routeContext);
assert.equal(vm.runInContext('routeQuoteStaleAfterMs()', routeContext), 30_000, 'RouteArbConfigV1 quote age must be used');

// Ledger details must decode candid opt values as [] / [value], use the debit
// asset for refunds, and display the additive requested input.
const ledgerContext = vm.createContext({
  BigInt, Date, console: { error() {} },
  routeOpt: value => Array.isArray(value) && value.length ? value[0] : null,
  routeAssetKey: asset => Object.keys(asset || {})[0] || '—',
  routeAssetLabel: asset => ({ Icp: 'ICP', CkUsdc: 'ckUSDC' }[Object.keys(asset || {})[0]] || '—'),
  routePhaseLabel: phase => Object.keys(phase || {})[0] || '—',
  variantKey: value => value && typeof value === 'object' ? Object.keys(value)[0] : '—',
  esc: value => String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c])),
  fmtTime: value => `time-${String(value)}`,
  pClass: value => Number(value) > 0 ? 'positive' : Number(value) < 0 ? 'negative' : '',
});
vm.runInContext(section('    function routeLedgerStatusKey', '    function legendHtml'), ledgerContext);
const legHtml = vm.runInContext(`routeLegDetailHtml({ detail_available: true, legs: [{
  leg_index: 0, status: { Settled: null }, venue: { ICPSwap: null }, pool_id: 'p',
  pool_principal: 'pool', edge_id: 'e', from: { CkUsdc: null }, to: { Icp: null },
  quoted_input_native: 1000000n, requested_input_native: [1250000n], quoted_output_native: [200000000n],
  minimum_output_native: 190000000n, input_fee_native: 10n, output_fee_native: 20n,
  actual_input_debit_native: [1250000n], actual_effective_input_native: [1249990n],
  actual_output_credit_native: [199000000n], refund_credit_native: [5000n],
  prepared_at_ns: [], submitted_at_ns: [100n], settled_at_ns: [], reconciled_at_ns: [200n],
  evidence: [], incident: []
}] })`, ledgerContext);
assert.match(legHtml, /Requested input/);
assert.match(legHtml, /1\.25 ckUSDC/, 'requested input must use the from asset decimals');
assert.match(legHtml, /0\.005 ckUSDC/, 'refund must use the from asset decimals');
assert.match(legHtml, /Prepared<\/span><span class="v">Unavailable/);
assert.match(legHtml, /Submitted<\/span><span class="v">time-100/);

// Source inventory must cover the non-route data visible in Markets/Ops/Cockpit
// and query methods must be retained as explicit unavailable/failed states.
for (const name of ['routeArbConfig', 'legacyLedger', 'summary', 'legacyBalances', 'volumeBalances', 'operatorBalances', 'volumeStats', 'prices', 'snapshots', 'activity']) {
  assert.match(html, new RegExp(`${name}\\s*:`), `missing explicit source state: ${name}`);
}
assert.match(html, /get_route_arb_config_v1/);
assert.match(html, /Promise\.allSettled|markSourceFailed\(routeSources\.(legacyBalances|volumeBalances|operatorBalances)/);
assert.doesNotMatch(html, /get_route_wallet_balances_v1[^\n]*[?][^\n]*0n/);

// ExecutionRecordV1.realized_profit is an opt int (arbitrary precision), not
// an opt int64. The Rust Candid wire regression covers [123n] and >i64 values.
assert.match(html, /realized_profit:\s*I\.Opt\(I\.Int\)/);
assert.doesNotMatch(html, /realized_profit:\s*I\.Opt\(I\.Int64\)/);
const wireTest = readFileSync('src/arb_bot/tests/dashboard_candid_wire.rs', 'utf8');
assert.match(wireTest, /encode_one[\s\S]*decode_one/);

// Cockpit hierarchy and phase precedence are contractual.
const cockpit = section('    function renderCockpit()', '    // ═══════ Markets');
const today = cockpit.indexOf('data-cockpit-today-results');
const latest = cockpit.indexOf('data-cockpit-latest-execution');
const incidents = cockpit.indexOf('data-cockpit-incidents');
const allTime = cockpit.indexOf('Net P&amp;L · all-time');
assert(today > -1 && latest > today && incidents > latest, 'Cockpit must put today, terminal, then incidents in order');
assert(incidents < allTime, 'legacy all-time P&L/equity must follow incidents');
assert.match(html, /settlement|reconciliation/i);
assert.match(html, /Leg .*of/);
assert.match(html, /data-diagnostics-manual-scan/);
assert.match(html, /runtime query|Runtime status unavailable|Unknown/);
assert.match(html, /Evaluating/);
assert.match(html, /Preparing/);
assert.match(html, /total unavailable/);
assert.doesNotMatch(html, /execution\.total_legs|execution\.leg_count|execution\.route_leg_count/);
assert.doesNotMatch(html, /routeAutomation\.state === 'Blocked'[\s\S]{0,180}quote-only/);
assert.match(html, /currentExecutionDetail/);
assert.match(html, /loadCurrentExecutionDetail/);
assert.match(html, /cockpitExecutionPhaseCopy/);
assert.match(html, /activeDetail\.asset_path|asset_path/);
assert.match(html, /flipLever\('vol',[\s\S]{0,260}resume_volume/);
assert.match(html, /!latestVolumeStats\.volume_paused/);
assert.doesNotMatch(html, /const actionable = authoritative \|\| applying/);

assert.match(html, /route-reconciliation[\s\S]{0,260}goTo\('diagnostics'\)/);

// Volume controls and lazy historical surfaces must expose source degradation.
assert.match(html, /volumeStats.*stale|stale.*volumeStats/i);
assert.match(html, /sourceState === 'loading'[\s\S]{0,500}Loading/);
assert.match(html, /sourceState === 'failed'[\s\S]{0,500}Failed/);
assert.match(html, /Stale · .*sourceLastSuccessLabel\(routeSources\.legacyLedger\)/);
assert.match(html, /Stale · .*sourceLastSuccessLabel\(routeSources\.activity\)/);

// Hostile source errors must be escaped before a source label can enter
// innerHTML (especially Cockpit's balance hero values).
const sourceSection = html.slice(html.indexOf('    function createSourceState()'), html.indexOf('    // ═══════ Button loading states'));
const sourceContext = vm.createContext({ Date, console: { error() {} }, esc: value => String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c])) });
vm.runInContext(sourceSection, sourceContext);
vm.runInContext("function routeSourceState(name, staleAfterMs = null) { return sourceDisplayState(routeSources[name], staleAfterMs); }", sourceContext);
vm.runInContext("routeSources.legacyBalances.status = 'failed'; routeSources.legacyBalances.error = '<img src=x onerror=alert(1)>';", sourceContext);
assert.equal(vm.runInContext("sourceStateLabel('legacyBalances')", sourceContext), 'Failed · &lt;img src=x onerror=alert(1)&gt;');
// Active execution detail supplies the authoritative route leg count and phase copy.
const phaseStart = html.indexOf('    function routeExecutionLegCount');
const phaseEnd = html.indexOf('    function cockpitTodayResults', phaseStart);
const phaseContext = vm.createContext({
  routeExecutionDetails: new Map(),
  routeSources: { candidates: { value: null }, currentExecutionDetail: { executionId: 'active-1', status: 'fresh', value: { detail_available: true, asset_path: ['a', 'b', 'c', 'a'], legs: [{}, {}, {}] } } },
  routeOpt: value => Array.isArray(value) ? (value.length ? value[0] : null) : value,
  variantKey: value => value && typeof value === 'object' ? Object.keys(value)[0] : String(value),
});
vm.runInContext(html.slice(phaseStart, phaseEnd), phaseContext);
assert.equal(vm.runInContext("cockpitExecutionLegLabel({ execution_id: 'active-1', route_id: 'route-1', current_leg_index: 1, phase: { LegSubmitted: null } })", phaseContext), 'Leg 2 of 3');
assert.equal(vm.runInContext("cockpitExecutionPhaseCopy({ execution_id: 'active-1', route_id: 'route-1', current_leg_index: 1, phase: { LegSubmitted: null } })", phaseContext), 'Submitting leg 2 of 3');

// Volume master direction and Applying lock are exercised through the real VM helpers.
const leverStart = html.indexOf('    async function flipLever');
const leverEnd = html.indexOf('    // ═══════ Ops — Setup', leverStart);
const volumeCalls = [];
const leverContext = vm.createContext({
  window: {}, state: { levers: {} }, authenticatedActor: { resume_volume: async () => volumeCalls.push('resume'), pause_volume: async () => volumeCalls.push('pause') },
  latestVolumeStats: { volume_paused: true }, currentConfig: { paused: false, rumi_amm_paused: false, bob_execution_enabled: false },
  requireAuth: () => true, routeSourceState: () => 'fresh', sourceStateText: () => 'Fresh', requireFreshVolumeStats: () => true, renderOps: () => {}, loadConfig: async () => {}, loadVolumeData: async () => {}, fetchHealth: async () => {}, toast: () => {}, checkRes: value => value,
  Promise, console,
});
vm.runInContext(html.slice(leverStart, leverEnd), leverContext);
vm.runInContext('window.leverVolume()', leverContext).then(() => {
  assert.deepEqual(volumeCalls, ['resume'], 'paused volume must call resume exactly once');
  return vm.runInContext('latestVolumeStats.volume_paused = false; window.leverVolume()', leverContext);
}).then(() => {
  assert.deepEqual(volumeCalls, ['resume', 'pause'], 'running volume must call pause exactly once');
}).catch(error => { console.error(error); process.exitCode = 1; });


// Current-detail reads are bounded, deduplicated, and generation-safe.
const detailLoaderStart = html.indexOf('    async function loadCurrentExecutionDetail');
const detailLoaderEnd = html.indexOf('    async function loadRouteData', detailLoaderStart);
(async () => {
  let calls = 0;
  const pending = [];
  const detailContext = vm.createContext({
    routeSources: { currentExecutionDetail: { status: 'loading', value: null, error: null, lastSuccessMs: null, lastAttemptMs: null } },
    routeExecutionDetails: new Map(), routeExecutionDetailLoads: new Map(), currentExecutionDetailGeneration: 0,
    routeRuntimeQueryGeneration: 1, anonymousActor: { get_route_execution_detail_v1: id => { calls += 1; return new Promise(resolve => pending.push({ id, resolve })); } },
    state: { activeView: 'cockpit' }, renderCockpit: () => {}, markSourceFresh: (source, value) => Object.assign(source, { status: 'fresh', value, error: null, lastSuccessMs: 1 }),
    markSourceFailed: (source, error) => Object.assign(source, { status: 'failed', error: String(error) }), markSourceUnavailable: (source, error) => Object.assign(source, { status: 'unavailable', value: null, error }),
    Date, Promise,
  });
  vm.runInContext(html.slice(detailLoaderStart, detailLoaderEnd), detailContext);
  const first = vm.runInContext("loadCurrentExecutionDetail({ execution_id: 'active-1' }, 1)", detailContext);
  const duplicate = vm.runInContext("loadCurrentExecutionDetail({ execution_id: 'active-1' }, 1)", detailContext);
  assert.equal(calls, 1, 'active detail query must deduplicate in-flight reads');
  pending[0].resolve({ Ok: { detail_available: true, asset_path: ['a', 'b', 'a'], legs: [{}, {}] } });
  await first; await duplicate;
  assert.equal(vm.runInContext("routeSources.currentExecutionDetail.status", detailContext), 'fresh');
  const old = vm.runInContext("loadCurrentExecutionDetail({ execution_id: 'active-old' }, 1)", detailContext);
  const current = vm.runInContext("loadCurrentExecutionDetail({ execution_id: 'active-new' }, 1)", detailContext);
  pending[1].resolve({ Ok: { detail_available: true, asset_path: ['a', 'b'], legs: [{}] } });
  await current;
  pending[2].resolve({ Ok: { detail_available: true, asset_path: ['a', 'b', 'c'], legs: [{}, {}] } });
  await old;
  assert.equal(vm.runInContext("routeSources.currentExecutionDetail.executionId", detailContext), 'active-new', 'late old detail must not repaint current execution');
})().catch(error => { console.error(error); process.exitCode = 1; });

const leverHtmlStart = html.indexOf('    function leverHtml');
const leverHtmlEnd = html.indexOf('    // ═══════ Ops — Setup', leverHtmlStart);
const applyingContext = vm.createContext({
  state: { levers: { vol: 'applying' } }, routeSources: { volumeStats: { error: null, lastSuccessMs: 1 } },
  routeSourceState: () => 'fresh', sourceLastSuccessLabel: () => 'as of now', sourceStateText: () => 'Fresh',
  esc: value => String(value),
});
vm.runInContext(html.slice(leverHtmlStart, leverHtmlEnd), applyingContext);
const applyingMarkup = vm.runInContext("leverHtml('vol', 'Volume engine', 'Master run/pause', true, 'leverVolume()')", applyingContext);
assert.match(applyingMarkup, /disabled/);
assert.doesNotMatch(applyingMarkup, /onclick=\"leverVolume\(\)\"/);

console.log('PASS: final frontend review regressions are covered');
