const { readFileSync } = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');

const html = readFileSync('src/arb_bot/src/dashboard.html', 'utf8');
const start = html.indexOf('    const ROUTE_CONFIG_BOUNDS =');
const end = html.indexOf('    // ═══════ Route arbitrage threshold controls end ═══════');
assert(start >= 0 && end > start, 'dashboard must define executable route-threshold helpers');

const context = vm.createContext({
  BigInt,
  Number,
  Object,
  Array,
  String,
  Math,
  esc: String,
  routeOpt: value => Array.isArray(value) && value.length ? value[0] : null,
  routeAssetKey: value => value && typeof value === 'object' ? Object.keys(value)[0] : '',
  routeLedgerFormatAmount(value, decimals) {
    const raw = BigInt(value).toString().padStart(decimals + 1, '0');
    const whole = decimals ? raw.slice(0, -decimals) : raw;
    const fraction = decimals ? raw.slice(-decimals).replace(/0+$/, '') : '';
    return decimals ? `${whole}.${fraction || '0'}` : whole;
  },
});
vm.runInContext(html.slice(start, end), context);
context.typedStableLadder = new BigUint64Array([1000000n, 5000000n, 40000000n]);
assert.equal(
  vm.runInContext('routeConfigFormatLadder(typedStableLadder, 6)', context),
  '1.0, 5.0, 40.0',
  'Candid BigUint64Array ladders must render as exact human values',
);

assert.equal(
  vm.runInContext("routeConfigParseNativeAmount('0.123456789012345678', 18, 'ckETH profit')", context),
  123456789012345678n,
  'ckETH values must retain every wei instead of passing through Number',
);
assert.throws(
  () => vm.runInContext("routeConfigParseNativeAmount('0.1234567890123456789', 18, 'ckETH profit')", context),
  /up to 18 decimal places/,
);
assert.throws(
  () => vm.runInContext("routeConfigParseNativeAmount('-1', 8, 'ICP profit')", context),
  /non-negative/,
);
assert.throws(
  () => vm.runInContext("routeConfigParseNativeAmount('18.446744073709551616', 18, 'ckETH ceiling')", context),
  /exceeds the maximum supported value/,
  'human-unit input must fail before Candid receives a value above nat64',
);
assert.deepEqual(
  Array.from(vm.runInContext("routeConfigParseLadder('1, 5, 10, 40', 6, 40000000n, 'Stable trade sizes')", context)),
  [1000000n, 5000000n, 10000000n, 40000000n],
);
assert.throws(
  () => vm.runInContext("routeConfigParseLadder('5, 1', 6, 40000000n, 'Stable trade sizes')", context),
  /strictly increasing/,
);
assert.throws(
  () => vm.runInContext("routeConfigParseLadder('1, 50', 6, 40000000n, 'Stable trade sizes')", context),
  /principal cap/,
);

const baseConfig = {
  enabled: true,
  dry_run: false,
  stable_book_enabled: true,
  icp_book_enabled: true,
  allow_wrapped_stable_to_icusd: [false],
  icusd_peg_trades_profile: [true],
  asset_controls: [
    { asset: { Icp: null }, enabled: true },
    { asset: { IcUsd: null }, enabled: true },
    { asset: { CkUsdc: null }, enabled: true },
    { asset: { CkUsdt: null }, enabled: true },
    { asset: { CkBtc: null }, enabled: true },
    { asset: { CkEth: null }, enabled: true },
  ],
  pool_controls: [{ pool_id: 'pool-1', enabled: true }],
  stable_size_ladder: [1000000n, 5000000n, 10000000n, 40000000n],
  icp_size_ladder: [100000000n, 500000000n, 1000000000n],
  max_route_legs: 3,
  max_quote_calls_per_observation: 4096,
  max_concurrent_quote_calls: 8,
  max_stable_principal_usd_6dec: 40000000n,
  max_icp_principal_e8s: 1000000000n,
  min_stable_profit_usd_6dec: 50000n,
  min_stable_profit_bps: 50,
  min_icp_profit_e8s: 10000n,
  min_icp_profit_bps: 50,
  inventory_bands: [
    { asset: { Icp: null }, floor_native: 0n, ceiling_native: 100000000000n },
    { asset: { IcUsd: null }, floor_native: 0n, ceiling_native: 100000000000n },
    { asset: { CkUsdc: null }, floor_native: 0n, ceiling_native: 1000000000n },
    { asset: { CkUsdt: null }, floor_native: 0n, ceiling_native: 1000000000n },
    { asset: { CkBtc: null }, floor_native: 0n, ceiling_native: 100000000n },
    { asset: { CkEth: null }, floor_native: 0n, ceiling_native: 1000000000000000000n },
  ],
  quote_max_age_ns: 30000000000n,
  settlement_timeout_ns: 3600000000000n,
  reconciliation_queries_per_cycle: 16,
  max_open_held_positions: 256,
  max_open_non_route_reservations: 256,
  max_terminal_execution_records: 10000,
  max_execution_record_bytes: 65536,
  max_reconciliation_evidence_items: 64,
  ckbtc_book: [{
    enabled: true,
    size_ladder: [100000n, 500000n, 1000000n],
    max_principal_native: 2000000n,
    min_profit_native: 10n,
    min_profit_bps: 50,
  }],
  cketh_book: [{
    enabled: true,
    size_ladder: [1000000000000000n, 5000000000000000n, 10000000000000000n],
    max_principal_native: 20000000000000000n,
    min_profit_native: 1000000000000n,
    min_profit_bps: 50,
  }],
};
context.baseConfig = baseConfig;
context.routeArbConfig = { ...baseConfig, ckbtc_book: [{ ...baseConfig.ckbtc_book[0], size_ladder: new BigUint64Array(baseConfig.ckbtc_book[0].size_ladder) }] };
context.routeReturnBook = key => context.routeOpt(context.routeArbConfig[key]);
const bookSettingsCode = html.slice(html.indexOf('    function routeBookSettingsHtml('), html.indexOf('    const ROUTE_CONFIG_BOUNDS ='));
vm.runInContext(bookSettingsCode, context);
const compactBookSettings = vm.runInContext("routeBookSettingsHtml('ckbtc_book', 'ckBTC', 8)", context);
assert.match(
  compactBookSettings,
  /ckBTC size ladder: 0\.001, 0\.005, 0\.01/,
  'the policy summary must not contradict the threshold card by calling typed-array ladders unavailable',
);
assert.match(
  compactBookSettings,
  /min profit: 0\.0000001 ckBTC and 0\.50%/,
  'the compact policy summary must state the backend profit gates use AND semantics',
);
context.values = {
  stableSizeLadder: '1, 5, 10, 35', stableMaxPrincipal: '35', stableMinProfit: '0.075', stableMinProfitBps: '30',
  icpSizeLadder: '1, 5', icpMaxPrincipal: '5', icpMinProfit: '0.0002', icpMinProfitBps: '35',
  ckbtcSizeLadder: '0.001, 0.005', ckbtcMaxPrincipal: '0.01', ckbtcMinProfit: '0.00000123', ckbtcMinProfitBps: '40',
  ckethSizeLadder: '0.001, 0.005', ckethMaxPrincipal: '0.01', ckethMinProfit: '0.000001234567890123', ckethMinProfitBps: '45',
  maxRouteLegs: '4', maxQuoteCallsPerObservation: '5000', maxConcurrentQuoteCalls: '12', quoteMaxAgeSeconds: '45',
  settlementTimeoutSeconds: '7200', reconciliationQueriesPerCycle: '24', maxOpenHeldPositions: '128',
  maxOpenNonRouteReservations: '64', maxTerminalExecutionRecords: '8000', maxExecutionRecordBytes: '32768',
  maxReconciliationEvidenceItems: '32',
  inventoryBands: {
    Icp: { floor: '1.5', ceiling: '900' }, IcUsd: { floor: '2', ceiling: '800' },
    CkUsdc: { floor: '3', ceiling: '700' }, CkUsdt: { floor: '4', ceiling: '600' },
    CkBtc: { floor: '0.01', ceiling: '0.9' }, CkEth: { floor: '0.02', ceiling: '0.8' },
  },
};
const next = vm.runInContext('routeConfigBuildUpdate(baseConfig, values)', context);
assert.equal(next.min_stable_profit_usd_6dec, 75000n);
assert.equal(next.min_stable_profit_bps, 30);
assert.equal(next.min_icp_profit_e8s, 20000n);
assert.equal(next.ckbtc_book[0].min_profit_native, 123n);
assert.equal(next.cketh_book[0].min_profit_native, 1234567890123n);
assert.equal(next.quote_max_age_ns, 45000000000n);
assert.equal(next.settlement_timeout_ns, 7200000000000n);
assert.equal(next.inventory_bands[0].floor_native, 150000000n);
assert.equal(next.inventory_bands[5].ceiling_native, 800000000000000000n);
assert.deepEqual(next.asset_controls, baseConfig.asset_controls, 'threshold edits must preserve route asset controls');
assert.deepEqual(next.pool_controls, baseConfig.pool_controls, 'threshold edits must preserve code-pinned pool controls');
assert.equal(next.enabled, true, 'threshold edits must not change live automation state');
assert.equal(next.allow_wrapped_stable_to_icusd[0], false, 'threshold edits must preserve stable-exit protection');
assert.equal(next.icusd_peg_trades_profile[0], true, 'threshold edits must preserve the independently stored peg-trades profile');
context.tooOldQuoteValues = { ...context.values, quoteMaxAgeSeconds: '60.000000001' };
assert.throws(
  () => vm.runInContext('routeConfigBuildUpdate(baseConfig, tooOldQuoteValues)', context),
  /Maximum quote age must be between 0\.000000001 and 60 seconds/,
);
context.tooLongSettlementValues = { ...context.values, settlementTimeoutSeconds: '86400.000000001' };
assert.throws(
  () => vm.runInContext('routeConfigBuildUpdate(baseConfig, tooLongSettlementValues)', context),
  /Settlement timeout must be between 0\.000000001 and 86,400 seconds/,
);

const summary = vm.runInContext('routeThresholdSummaryHtml(baseConfig)', context);
for (const label of [
  'Minimum stable profit', 'Minimum ICP profit', 'Minimum ckBTC profit', 'Minimum ckETH profit',
  'Stable trade sizes', 'ICP trade sizes', 'ckBTC trade sizes', 'ckETH trade sizes',
  'Maximum route length', 'Maximum quote calls per scan', 'Maximum concurrent quote calls',
  'Maximum quote age', 'Settlement timeout', 'Reconciliation queries per cycle',
  'Maximum held positions', 'Maximum non-route reservations', 'Terminal execution history limit',
  'Maximum execution record size', 'Reconciliation evidence limit', 'Inventory floor', 'Inventory ceiling',
]) assert(summary.includes(label), `Ops summary must expose ${label}`);
assert(summary.includes('data-route-config-tooltip'), 'every threshold explanation must use an accessible tooltip trigger');
assert(summary.includes('aria-description="A stable-returning route must satisfy'), 'tooltip explanations must be available to assistive technology');
assert(summary.includes('Both the absolute minimum and the basis-point minimum must pass'), 'profit-gate tooltip must explain AND semantics');

const editorStart = html.indexOf('    function routeConfigEditorFieldHtml(');
const editorEnd = html.indexOf('    // ═══════ Route threshold editor DOM actions ═══════');
assert(editorStart >= 0 && editorEnd > editorStart, 'dashboard must define the route-threshold editor');
vm.runInContext(html.slice(editorStart, editorEnd), context);
const editor = vm.runInContext('routeThresholdEditorHtml(baseConfig)', context);
for (const id of [
  'route-cfg-stable-min-profit', 'route-cfg-stable-min-bps', 'route-cfg-icp-min-profit', 'route-cfg-icp-min-bps',
  'route-cfg-ckbtc-min-profit', 'route-cfg-ckbtc-min-bps', 'route-cfg-cketh-min-profit', 'route-cfg-cketh-min-bps',
  'route-cfg-stable-sizes', 'route-cfg-stable-cap', 'route-cfg-icp-sizes', 'route-cfg-icp-cap',
  'route-cfg-ckbtc-sizes', 'route-cfg-ckbtc-cap', 'route-cfg-cketh-sizes', 'route-cfg-cketh-cap',
  'route-cfg-max-route-legs', 'route-cfg-max-quote-calls', 'route-cfg-concurrent-quotes', 'route-cfg-quote-age',
  'route-cfg-settlement-timeout', 'route-cfg-reconciliation-queries', 'route-cfg-held-positions',
  'route-cfg-non-route-reservations', 'route-cfg-terminal-records', 'route-cfg-record-bytes', 'route-cfg-evidence-items',
]) {
  assert(editor.includes(`id="${id}"`), `editor must expose ${id}`);
  assert(editor.includes(`aria-describedby="${id}-help"`), `${id} must connect to visible explanatory help`);
}
for (const asset of ['Icp', 'IcUsd', 'CkUsdc', 'CkUsdt', 'CkBtc', 'CkEth']) {
  assert(editor.includes(`id="route-cfg-inventory-${asset}-floor"`), `${asset} inventory floor must be editable`);
  assert(editor.includes(`id="route-cfg-inventory-${asset}-ceiling"`), `${asset} inventory ceiling must be editable`);
}
assert(editor.includes('data-route-config-tooltip'), 'editor labels must retain accessible tooltips');
assert(editor.includes('Higher is stricter'), 'profit inputs must state the direction of effect');
assert(editor.includes('Higher allows larger trades'), 'principal caps must state the direction of effect');
const opsSource = html.slice(html.indexOf('    function renderOps()'), html.indexOf('    // ═══════ Ledger'));
assert(opsSource.includes('routeThresholdSummaryHtml(routeArbConfig)'), 'Ops must render the complete threshold summary');
assert(opsSource.includes('onclick="openRouteThresholdEditor()"'), 'Ops must provide one clearly named threshold editor action');
assert(opsSource.includes('Edit all thresholds and limits'), 'the edit action must describe its complete scope');

(async () => {
const actionsStart = html.indexOf('    function routeConfigEditableFingerprint(');
const actionsEnd = html.indexOf('    function routeArbitrageHtml()');
assert(actionsStart >= 0 && actionsEnd > actionsStart, 'dashboard must define stale-safe threshold persistence');
Object.assign(context, {
  window: {},
  routeArbConfig: baseConfig,
  routeSources: { routeArbConfig: {} },
  latestRouteObservation: { observation_id: 'old' },
  latestRouteCandidates: { stable: [{}] },
  latestTopRouteCandidates: { entries: [{}] },
  routeDataRequestPromise: null,
  markSourceFresh(source, value) { source.status = 'fresh'; source.value = value; },
  markSourceUnavailable(source, reason) { source.status = 'unavailable'; source.error = reason; },
  unwrapResult(result) { if (result && Object.hasOwn(result, 'Err')) throw Error(result.Err); return result.Ok; },
  loadRouteData: async () => {},
  renderRouteAutomationViews() {},
  document: { getElementById() { return { addEventListener() {} }; }, addEventListener() {} },
});
vm.runInContext(html.slice(actionsStart, actionsEnd), context);
assert.equal(vm.runInContext('typeof routeConfigReadEditorValues', context), 'function', 'editor must read every visible field');
assert.equal(vm.runInContext('typeof routeConfigReviewRowsHtml', context), 'function', 'editor must present an exact before-and-after review');

const inputValues = {
  'route-cfg-stable-sizes': context.values.stableSizeLadder, 'route-cfg-stable-cap': context.values.stableMaxPrincipal,
  'route-cfg-stable-min-profit': context.values.stableMinProfit, 'route-cfg-stable-min-bps': context.values.stableMinProfitBps,
  'route-cfg-icp-sizes': context.values.icpSizeLadder, 'route-cfg-icp-cap': context.values.icpMaxPrincipal,
  'route-cfg-icp-min-profit': context.values.icpMinProfit, 'route-cfg-icp-min-bps': context.values.icpMinProfitBps,
  'route-cfg-ckbtc-sizes': context.values.ckbtcSizeLadder, 'route-cfg-ckbtc-cap': context.values.ckbtcMaxPrincipal,
  'route-cfg-ckbtc-min-profit': context.values.ckbtcMinProfit, 'route-cfg-ckbtc-min-bps': context.values.ckbtcMinProfitBps,
  'route-cfg-cketh-sizes': context.values.ckethSizeLadder, 'route-cfg-cketh-cap': context.values.ckethMaxPrincipal,
  'route-cfg-cketh-min-profit': context.values.ckethMinProfit, 'route-cfg-cketh-min-bps': context.values.ckethMinProfitBps,
  'route-cfg-max-route-legs': context.values.maxRouteLegs, 'route-cfg-max-quote-calls': context.values.maxQuoteCallsPerObservation,
  'route-cfg-concurrent-quotes': context.values.maxConcurrentQuoteCalls, 'route-cfg-quote-age': context.values.quoteMaxAgeSeconds,
  'route-cfg-settlement-timeout': context.values.settlementTimeoutSeconds, 'route-cfg-reconciliation-queries': context.values.reconciliationQueriesPerCycle,
  'route-cfg-held-positions': context.values.maxOpenHeldPositions, 'route-cfg-non-route-reservations': context.values.maxOpenNonRouteReservations,
  'route-cfg-terminal-records': context.values.maxTerminalExecutionRecords, 'route-cfg-record-bytes': context.values.maxExecutionRecordBytes,
  'route-cfg-evidence-items': context.values.maxReconciliationEvidenceItems,
};
for (const [asset, band] of Object.entries(context.values.inventoryBands)) {
  inputValues[`route-cfg-inventory-${asset}-floor`] = band.floor;
  inputValues[`route-cfg-inventory-${asset}-ceiling`] = band.ceiling;
}
context.document = { getElementById(id) { return Object.hasOwn(inputValues, id) ? { value: inputValues[id] } : null; } };
const readValues = vm.runInContext('routeConfigReadEditorValues()', context);
assert.equal(readValues.ckethMinProfit, '0.000001234567890123');
assert.equal(readValues.inventoryBands.CkEth.ceiling, '0.8');
inputValues['route-cfg-record-bytes'] = '65536';
const review = vm.runInContext('routeConfigReviewRowsHtml(baseConfig, routeConfigBuildUpdate(baseConfig, routeConfigReadEditorValues()))', context);
assert(review.includes('Minimum stable profit (relative)'), 'review must name changed fields in operator language');
assert(review.includes('50 bps (0.50%)'), 'review must show the current value');
assert(review.includes('30 bps (0.30%)'), 'review must show the proposed value');
assert(!review.includes('Maximum execution record size'), 'review must omit unchanged values');

const latestConfig = structuredClone(baseConfig);
latestConfig.enabled = false;
latestConfig.asset_controls[0].enabled = false;
latestConfig.allow_wrapped_stable_to_icusd = [true];
let submittedConfig = null;
let configReadCount = 0;
context.authenticatedActor = {
  async get_route_arb_config_v1() {
    configReadCount += 1;
    return configReadCount === 1 ? latestConfig : submittedConfig;
  },
  async set_route_arb_config_v1(config) { submittedConfig = config; return { Ok: null }; },
};
const saved = await vm.runInContext('routeConfigCommitChanges(baseConfig, values)', context);
assert.equal(submittedConfig.min_stable_profit_bps, 30);
assert.equal(submittedConfig.enabled, false, 'save must rebase onto the latest automation state');
assert.equal(submittedConfig.asset_controls[0].enabled, false, 'save must preserve the latest asset controls');
assert.equal(submittedConfig.allow_wrapped_stable_to_icusd[0], true, 'save must preserve the latest stable-exit setting');
assert.equal(saved.min_stable_profit_usd_6dec, 75000n);
assert.equal(context.routeSources.routeArbConfig.status, 'fresh', 'save must publish authoritative readback');
assert.equal(context.latestRouteObservation, null, 'save must invalidate old quote observations');

const conflicting = structuredClone(baseConfig);
conflicting.min_stable_profit_bps = 99;
let conflictingSetCalled = false;
context.authenticatedActor = {
  async get_route_arb_config_v1() { return conflicting; },
  async set_route_arb_config_v1() { conflictingSetCalled = true; return { Ok: null }; },
};
await assert.rejects(
  vm.runInContext('routeConfigCommitChanges(baseConfig, values)', context),
  /changed since this editor opened/,
);
assert.equal(conflictingSetCalled, false, 'stale editor must never overwrite a concurrent threshold change');

console.log('PASS: Ops exposes every route threshold and limit; exact native-unit conversions, validation, and unrelated-policy preservation are enforced');
})().catch(error => { console.error(error); process.exitCode = 1; });
