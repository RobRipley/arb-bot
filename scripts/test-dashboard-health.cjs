const { readFileSync } = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const html = readFileSync('src/arb_bot/src/dashboard.html', 'utf8');
const section = (a, b) => html.slice(html.indexOf(a), html.indexOf(b, html.indexOf(a)));
const sourceSection = html.slice(html.indexOf('    function createSourceState()'), html.indexOf('    // ═══════ Button loading states'));
const context = vm.createContext({
  anonymousActor: { get_public_health: async () => ({has_pending_exit: true, has_pending_bob_exit: false, has_stranded_volume_funds: true}) },
  authenticatedActor: { get_bot_health: () => { throw Error('retired endpoint called'); } },
  healthRequestPromise: null, toast() {}, publicHealthRequestPromise: null, latestPublicHealth: null, publicHealthFailed: false,
  window: {}, console: { error() {} }, renderBanner() {}, renderCurrentView() {}, Date,
});
vm.runInContext(sourceSection, context);
vm.runInContext(section('function fetchHealth()', 'function wedgeConditions'), context);
vm.runInContext(section('function fetchPublicHealth()', '// ═══════ Data loaders'), context);
(async () => {
  await vm.runInContext('fetchHealth()', context);
  assert.deepEqual(Array.from(context.window._publicWedge), ['pending_exit', 'volume_stranded']);
  assert.equal(context.latestPublicHealth.has_pending_exit, true);
  assert.equal(vm.runInContext("routeSources.health.status", context), 'fresh');
  context.anonymousActor.get_public_health = async () => { throw Error('offline'); };
  await vm.runInContext('fetchHealth()', context);
  assert.equal(context.publicHealthFailed, true);
  assert.deepEqual(Array.from(context.window._publicWedge), ['pending_exit', 'volume_stranded']);
  assert.equal(vm.runInContext("routeSources.health.status", context), 'stale');
  context.authenticatedActor = null;
  context.anonymousActor.get_public_health = async () => ({has_pending_exit: false, has_pending_bob_exit: false, has_stranded_volume_funds: false});
  await vm.runInContext('fetchPublicHealth()', context);
  assert.equal(context.publicHealthFailed, false);
  assert.deepEqual(Array.from(context.window._publicWedge), []);
  assert.equal(vm.runInContext("routeSources.health.status", context), 'fresh');
  delete context.anonymousActor.get_public_health;
  await vm.runInContext('fetchPublicHealth()', context);
  assert.equal(vm.runInContext("routeSources.health.status", context), 'unavailable');
  assert(!section('function attentionItems()', 'window.dismissAttn').includes("kind: 'opportunity'"));
  assert(html.includes('Not implemented'));
  console.log('PASS: signed-in health uses supported query, failures retain incident flags, quotes are not attention items');
})().catch(e => { console.error(e); process.exitCode = 1; });
