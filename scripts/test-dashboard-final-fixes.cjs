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

console.log('PASS: final frontend review regressions are covered');
