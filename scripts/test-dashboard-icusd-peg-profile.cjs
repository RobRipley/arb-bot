const { readFileSync } = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');

const html = readFileSync('src/arb_bot/src/dashboard.html', 'utf8');
const start = html.indexOf('    // ═══════ icUSD peg trades profile UI ═══════');
const end = html.indexOf('    // ═══════ icUSD peg trades profile UI end ═══════');
assert(start >= 0 && end > start, 'dashboard must provide executable icUSD peg profile UI behavior');

const vmContext = vm.createContext({
  window: {},
  state: { levers: {} },
  routeDataRequestPromise: null,
  manualQuoteScanGeneration: 9,
  manualQuoteScan: { phase: 'failed', observationId: 'old-observation', cursor: 400n, batchCount: 4, error: 'provider failure', startedAtNs: 100n },
  latestRouteObservation: { observation_id: 'old-observation' },
  latestRouteStatus: { icusd_peg_trades_profile_active: true },
  latestRouteRuntime: { enabled: false, dry_run: true, live_authorized: false },
  routeArbConfig: { icusd_price_usd6: [970000n], stable_size_ladder: [1000000n] },
  routeSources: { status: {}, observation: {} },
  statusState: 'fresh',
  routeOpt: value => Array.isArray(value) && value.length ? value[0] : null,
  routeSourceState: () => vmContext.statusState,
  routeSourceError: () => '',
  markSourceUnavailable: (source, error) => Object.assign(source, { status: 'unavailable', value: null, error }),
  sourceStateText: state => state,
  sourceLastSuccessLabel: () => 'just now',
  fmtTime: value => `time:${value}`,
  bi: value => String(value),
  esc: String,
  requireAuth: () => true,
  unwrapResult: result => { if (result && Object.hasOwn(result, 'Err')) throw new Error(result.Err); return result.Ok; },
  openModal: modal => { vmContext.modal = modal; },
  toast: (message, kind) => vmContext.toasts.push([message, kind]),
  renderRouteAutomationViews() { vmContext.renderCount += 1; },
  loadRouteData: async () => { vmContext.refreshCount += 1; },
  authenticatedActor: {
    async set_icusd_peg_trades_profile_v1(active) {
      vmContext.calls.push(['profile', active]);
      return { Ok: { icusd_peg_trades_profile: [active], enabled: false, dry_run: true } };
    },
  },
  calls: [], toasts: [], renderCount: 0, refreshCount: 0,
});
vm.runInContext(html.slice(start, end), vmContext);

assert.equal(
  vm.runInContext("icusdPegTradesProfileStatusHtml('fresh', {icusd_peg_trades_profile_active:true, observation_interval_secs:600, next_observation_eligible_at_ns:[123n]}, {enabled:false,dry_run:true})", vmContext),
  '<div class="route-list-row"><strong>Profile active · execution stopped · dry-run</strong><span>Configured interval: 600 seconds · next eligible time: time:123</span></div><div class="route-wallet-meta">The scheduler checks this profile only while route execution is authorized and enabled, with dry-run off. Exactly four icUSD↔ICP↔ckUSDC/ckUSDT trades are in scope. Stable route sizing and profit comparisons use the configured icUSD accounting value of $0.97 and ckUSDC/ckUSDT at $1.00 after fees; actual route amounts and proceeds come from pool quotes. Activating this profile does not authorize live trades. No automatic drain runs; stranded ICP remains held until separately resolved. An already durable execution may still be serviced and reconciled.</div>',
  'an active profile must disclose its cadence, stopped execution, fixed scope, and held-ICP behavior',
);
assert.match(
  vm.runInContext("icusdPegTradesProfileStatusHtml('loading', null, null)", vmContext),
  /Profile status loading/,
  'profile status must remain unknown while the authoritative status is loading',
);
assert.match(
  vm.runInContext("icusdPegTradesProfileStatusHtml('fresh', {icusd_peg_trades_profile_active:false, observation_interval_secs:0}, {enabled:false,dry_run:true})", vmContext),
  /Profile inactive · no new route selections/,
  'an inactive profile must disclose that new generic scans and selections stop while existing execution can be reconciled',
);
assert.match(
  vm.runInContext("icusdPegTradesProfileStatusHtml('fresh', {icusd_peg_trades_profile_active:true, observation_interval_secs:600, next_observation_eligible_at_ns:[]}, {enabled:true,dry_run:false,live_authorized:true})", vmContext),
  /separate route execution is on/,
  'the profile must distinguish its stopped activation from a later change through the separate route execution control',
);

vm.runInContext('window.setIcusdPegTradesProfile(true)', vmContext);
assert.match(vmContext.modal.body, /exactly four/);
assert.match(vmContext.modal.body, /600 seconds/);
assert.match(vmContext.modal.body, /authorized and enabled, with dry-run off/);
assert.match(vmContext.modal.body, /after fees/);
assert.match(vmContext.modal.body, /stranded ICP remains held/);
assert.match(vmContext.modal.body, /icUSD accounting value of \$0\.97/);
assert.match(vmContext.modal.body, /actual route amounts and proceeds come from pool quotes/);
assert.match(
  vm.runInContext('icusdPegTradesProfileSummaryHtml(routeArbConfig)', vmContext),
  /Configured scope: exactly four.*authorized and enabled, with dry-run off.*configured icUSD accounting value of \$0\.97.*zero absolute and relative thresholds/i,
  'the Ops profile summary must keep its route/gate/threshold promises while displaying the configured icUSD value',
);
assert.equal(vmContext.calls.length, 0, 'opening the confirmation must not call the canister');
(async () => {
  let releaseRead;
  vmContext.routeDataRequestPromise = new Promise(resolve => { releaseRead = resolve; });
  const confirmation = vmContext.modal.onConfirm();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(vmContext.calls.length, 0, 'profile mutation must wait for an in-flight status/config read to finish');
  releaseRead();
  await confirmation;
  assert.deepEqual(vmContext.calls, [['profile', true]], 'profile activation must call only the dedicated profile method');
  assert.equal(vmContext.refreshCount, 1, 'the UI must refresh authoritative state after applying the profile');
  assert.equal(vmContext.renderCount, 2, 'the UI must repaint before and after the authoritative refresh');
  assert.equal(vmContext.state.levers.pegTradesProfile, undefined, 'applying state must clear after completion');
  assert.equal(vmContext.manualQuoteScanGeneration, 10, 'profile change must abort any loop still running on the old observation generation');
  assert.equal(vmContext.manualQuoteScan.phase, 'ready', 'successful profile selection must discard a failed local scan that belongs to the previous backend generation');
  assert.equal(vmContext.manualQuoteScan.observationId, null, 'the next scan must start a fresh backend observation');
  assert.equal(vmContext.manualQuoteScan.cursor, null, 'the next scan must not resume the previous generation cursor');
  assert.equal(vmContext.latestRouteObservation, null, 'the old observation must not remain visible as current after the selector resets it');
  console.log('PASS: icUSD peg profile status is truthful and activation uses its dedicated stopped/dry-run path');
})().catch(error => { console.error(error); process.exitCode = 1; });
