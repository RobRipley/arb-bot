const { readFileSync } = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');

const html = readFileSync('src/arb_bot/src/dashboard.html', 'utf8');
const idlStart = html.indexOf('    const Direction = IDL.Variant(');
const idlEnd = html.indexOf('    const icrc1IdlFactory =', idlStart);
assert(idlStart >= 0 && idlEnd > idlStart, 'dashboard must define its route Candid service');
const idlType = (kind, value) => ({ kind, value });
const fakeIDL = new Proxy({
  Func: (args, rets, annotations) => ({ kind: 'Func', args, rets, annotations }),
  Record: fields => ({ kind: 'Record', value: fields }),
  Service: methods => ({ kind: 'Service', value: methods }),
}, { get: (target, kind) => target[kind] || (['Null', 'Bool', 'Nat', 'Nat8', 'Nat16', 'Nat32', 'Nat64', 'Int', 'Int64', 'Text', 'Principal'].includes(kind)
  ? idlType(kind)
  : value => idlType(kind, value)) });
const idlContext = vm.createContext({ IDL: fakeIDL });
vm.runInContext(html.slice(idlStart, idlEnd) + '\nthis.__idlFactory = idlFactory;', idlContext);
const service = idlContext.__idlFactory({ IDL: fakeIDL });
const routeConfigRecord = service.value.get_route_arb_config_v1.rets[0];
assert.equal(routeConfigRecord.value.icusd_price_usd6.kind, 'Opt', 'configured price must remain additive/optional for legacy callers');
assert.equal(routeConfigRecord.value.icusd_price_usd6.value.kind, 'Nat64', 'USD6 price must use the confirmed nat64 wire type');
const priceSetter = service.value.set_icusd_price_usd6_v1;
assert.equal(priceSetter.args[0].kind, 'Nat64');
assert.equal(priceSetter.rets[0].kind, 'Variant');

const start = html.indexOf('    // ═══════ icUSD accounting price UI ═══════');
const end = html.indexOf('    // ═══════ icUSD accounting price UI end ═══════');
assert(start >= 0 && end > start, 'dashboard must provide executable icUSD accounting-price UI behavior');

const context = vm.createContext({
  window: {},
  state: { levers: {} },
  routeDataRequestPromise: null,
  routeIcusdPriceMutationInFlight: false,
  routeArbConfig: { icusd_price_usd6: [1000000n], stable_size_ladder: [1000000n, 5000000n] },
  routeSources: { routeArbConfig: { status: 'fresh' }, currentExecution: { status: 'fresh' } },
  latestRouteExecution: null,
  manualQuoteScanGeneration: 5,
  manualQuoteScan: { phase: 'failed', observationId: 'old-observation', cursor: 300n, batchCount: 3, error: 'provider failed', startedAtNs: 100n },
  latestRouteObservation: { observation_id: 'old-observation' },
  routeSourceState: name => context.routeSources[name].status,
  routeOpt: value => Array.isArray(value) && value.length ? value[0] : null,
  routeTradingLabel: () => 'Disabled',
  esc: String,
  requireAuth: () => true,
  openModal: modal => { context.modal = modal; },
  toast: (message, kind) => context.toasts.push([message, kind]),
  unwrapResult: result => { if (result && Object.hasOwn(result, 'Err')) throw new Error(result.Err); return result.Ok; },
  markSourceFresh: (source, value) => Object.assign(source, { status: 'fresh', value }),
  markSourceUnavailable: (source, error) => Object.assign(source, { status: 'unavailable', error }),
  loadRouteData: async () => { context.events.push('readback'); },
  renderRouteAutomationViews: () => context.events.push('render'),
  sourceStateText: name => context.routeSources[name].status,
  document: { getElementById: id => context.inputs[id] || null },
  inputs: { 'icusd-price-usd6': { value: '0.97' } },
  freshCurrentExecution: [], setterError: null,
  calls: [], events: [], toasts: [],
  authenticatedActor: {
    async get_current_route_execution_v1() {
      context.events.push('fresh_current');
      return context.freshCurrentExecution;
    },
    async get_route_arb_config_v1() {
      context.events.push('fresh_config');
      return context.routeArbConfig;
    },
    async set_icusd_price_usd6_v1(price) {
      context.calls.push(['set_price', price]);
      context.events.push('set_price');
      if (context.setterError) return { Err: context.setterError };
      return { Ok: { icusd_price_usd6: [price], stable_size_ladder: [1000000n, 5000000n] } };
    },
  },
});
vm.runInContext(html.slice(start, end), context);

assert.equal(vm.runInContext('icusdPriceCurrentUsd6({icusd_price_usd6:[], stable_size_ladder:[1000000n]})', context), 1000000n,
  'legacy config without a stored price must display the $1.00 default');
assert.equal(vm.runInContext("icusdPriceParseUsd6('0.97', routeArbConfig)", context), 970000n,
  'manual decimal input must convert exactly to USD6');
assert.equal(vm.runInContext("icusdPriceParseUsd6('0.970001', routeArbConfig)", context), 970001n,
  'six decimal places must retain exact micro-dollar precision');
assert.equal(vm.runInContext('icusdPriceFormatUsd6(970000n)', context), '$0.97');
assert.throws(() => vm.runInContext("icusdPriceParseUsd6('0', routeArbConfig)", context), /must be positive/);
assert.throws(() => vm.runInContext("icusdPriceParseUsd6('NaN', routeArbConfig)", context), /valid USD amount/);
assert.throws(() => vm.runInContext("icusdPriceParseUsd6('0.9700001', routeArbConfig)", context), /up to 6 decimal places/);
assert.throws(() => vm.runInContext("icusdPriceParseUsd6('100000000.000001', routeArbConfig)", context), /smallest stable size/);
const nat64BoundConfig = {
  icusd_peg_trades_profile: [false],
  stable_size_ladder: [200_000_000_000n, 400_000_000_000n],
  max_stable_principal_usd_6dec: 400_000_000_000n,
};
context.nat64BoundConfig = nat64BoundConfig;
assert.equal(
  vm.runInContext("icusdPriceParseUsd6('18446744073709.551615', nat64BoundConfig)", context),
  18_446_744_073_709_551_615n,
  'a valid nat64 price must remain editable when the true minimum-size bound exceeds nat64::MAX',
);
assert.throws(
  () => vm.runInContext("icusdPriceParseUsd6('18446744073709.551616', nat64BoundConfig)", context),
  /smallest stable size/,
  'prices above nat64::MAX must remain rejected even when the configuration-derived bound is larger',
);
const editor = vm.runInContext('icusdPriceEditorHtml(routeArbConfig, true, null)', context);
assert.match(editor, /value="1\.00"/);
assert.match(editor, /icUSD value/);
assert.match(editor, /ckUSDC\/ckUSDT at \$1\.00/);
assert.match(editor, /Stable route sizing and profit comparisons use the configured icUSD accounting value of \$1\.00/);
assert.match(editor, /actual route amounts.*pool quotes/);
assert.match(
  vm.runInContext("icusdPriceEditorHtml({icusd_price_usd6:[970000n],stable_size_ladder:[1000000n]}, 'fresh', null)", context),
  /Configured icUSD accounting value: \$0\.97/,
  'the editor must display the authoritative configured icUSD accounting price, not a hardcoded peg',
);
const marketDisclosure = vm.runInContext('icusdPegMarketDisclosureHtml({icusd_price_usd6:[970000n],stable_size_ladder:[1000000n]})', context);
assert.match(marketDisclosure, /configured icUSD accounting value of \$0\.97/,
  'the Markets disclosure must use the configured icUSD accounting price');
assert.match(marketDisclosure, /ckUSDC\/ckUSDT at \$1\.00/,
  'only wrapped stablecoins retain fixed one-dollar accounting');
assert.match(marketDisclosure, /actual route amounts and proceeds come from pool quotes/,
  'the Markets disclosure must distinguish accounting value from actual pool quotes');
assert.doesNotMatch(marketDisclosure, /icUSD.*valued at \$1 only for terminal profit accounting/,
  'the Markets disclosure must not claim icUSD is fixed at one dollar');

// Active or stale execution state must block the admin action before opening a write confirmation.
context.latestRouteExecution = { execution_id: 'exec-active' };
vm.runInContext('window.saveIcusdAccountingPrice()', context);
assert.equal(context.modal, undefined);
assert.match(context.toasts.at(-1)[0], /Wait for the current route execution to finish/);
assert.equal(context.calls.length, 0);
context.latestRouteExecution = null;
context.routeSources.currentExecution.status = 'stale';
vm.runInContext('window.saveIcusdAccountingPrice()', context);
assert.match(context.toasts.at(-1)[0], /Current execution status is stale/);
assert.equal(context.calls.length, 0);
context.routeSources.currentExecution.status = 'fresh';

vm.runInContext('window.saveIcusdAccountingPrice()', context);
assert.match(context.modal.body, /icUSD.*\$0\.97/);
assert.match(context.modal.body, /pool quotes/);
assert.equal(context.calls.length, 0, 'opening confirmation must not write configuration');

(async () => {
  const staleModal = context.modal;
  const originalConfig = context.routeArbConfig;
  context.routeArbConfig = {
    icusd_price_usd6: [990000n],
    stable_size_ladder: [1000000n, 5000000n],
  };
  await staleModal.onConfirm();
  assert.equal(context.calls.length, 0,
    'a confirmation opened for an older current value must not overwrite a newer admin setting');
  assert.match(context.toasts.at(-1)[0], /changed since this confirmation/,
    'a changed current price must require a fresh review rather than silently applying the stale modal');
  context.routeArbConfig = originalConfig;
  vm.runInContext('window.saveIcusdAccountingPrice()', context);
  context.events.length = 0;

  let releaseRead;
  context.routeDataRequestPromise = new Promise(resolve => { releaseRead = resolve; });
  const confirmation = context.modal.onConfirm();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(context.calls.length, 0, 'price mutation must wait for the in-flight authoritative read');
  context.freshCurrentExecution = [{ execution_id: 'exec-started-during-confirmation' }];
  releaseRead();
  await confirmation;
  assert.equal(context.calls.length, 0, 'a fresh current-execution query immediately before mutation must block an execution that began after confirmation opened');
  assert.match(context.toasts.at(-1)[0], /Wait for the current route execution to finish/);

  context.routeDataRequestPromise = null;
  context.freshCurrentExecution = [];
  vm.runInContext('window.saveIcusdAccountingPrice()', context);
  const retryConfirmation = context.modal.onConfirm();
  await retryConfirmation;
  assert.deepEqual(context.calls, [['set_price', 970000n]], 'the dedicated admin setter must receive exact USD6');
  assert.deepEqual(context.events.filter(value => ['fresh_current', 'fresh_config', 'set_price', 'readback'].includes(value)), ['fresh_current', 'fresh_config', 'readback', 'fresh_current', 'fresh_config', 'set_price', 'readback'],
    'fresh current execution and config must be queried after the existing read drains and immediately before the setter');
  assert.equal(context.routeIcusdPriceMutationInFlight, false);
  assert.equal(context.manualQuoteScan.phase, 'ready', 'successful price update must discard a failed local scan from the old observation generation');
  assert.equal(context.manualQuoteScan.observationId, null);
  assert.equal(context.manualQuoteScan.cursor, null);
  assert.equal(context.manualQuoteScanGeneration, 6, 'successful price update must abort any older scan loop');
  assert.equal(context.latestRouteObservation, null);
  assert.equal(context.calls.some(call => call[0] === 'scan'), false, 'saving the price must not start a scan or execution');

  context.setterError = 'cannot change icUSD accounting price while a route execution is active';
  context.inputs['icusd-price-usd6'].value = '0.96';
  vm.runInContext('window.saveIcusdAccountingPrice()', context);
  await context.modal.onConfirm();
  assert.match(context.toasts.at(-1)[0], /cannot change icUSD accounting price while a route execution is active/,
    'the authoritative server guard error must be shown to the admin without hiding it behind a generic message');
  console.log('PASS: icUSD accounting-price input, validation, active-execution gate, read serialization, and observation invalidation');
})().catch(error => { console.error(error); process.exitCode = 1; });
