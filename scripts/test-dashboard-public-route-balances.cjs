const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');

const html = fs.readFileSync('src/arb_bot/src/dashboard.html', 'utf8');
const code = html.slice(
  html.indexOf('    function resetRouteWalletForPublicReload()'),
  html.indexOf('    async function loadVolumeBalances(config)'),
);

const routeSources = {
  legacyBalances: { status: 'loading', value: null, lastSuccessMs: null },
  wallet: { status: 'loading', value: null, lastSuccessMs: null },
};
const balances = new Map([
  ['three', 30n], ['ckusdc', 60n], ['ckusdt', 61n], ['icusd', 80n],
  ['icp', 800n], ['bob', 90n], ['ckbtc', 100n],
]);
let releaseBalances;
const balanceGate = new Promise(resolve => { releaseBalances = resolve; });
let balanceCalls = 0;
const context = vm.createContext({
  routeSources,
  authenticatedActor: {},
  isAdmin: false,
  latestRouteWallet: [],
  balanceRequestGeneration: 0,
  balanceRequestPromise: null,
  TOKEN_INFO: {
    icusd: { getLedger: () => 'icusd' }, ckusdt: { getLedger: () => 'ckusdt' },
    ckbtc: { getLedger: () => 'ckbtc' }, cketh: { getLedger: () => 'cketh' },
  },
  Principal: { fromText: text => text },
  canisterId: 'bot',
  state: { activeView: 'cockpit' },
  createBalanceActor: async ledger => ({
    icrc1_balance_of: async () => {
      balanceCalls += 1;
      await balanceGate;
      if (!balances.has(ledger)) throw Error(`${ledger} unavailable`);
      return balances.get(ledger);
    },
  }),
  markSourceFresh(source, value) { Object.assign(source, { status: 'fresh', value, lastSuccessMs: Date.now(), error: null }); },
  markSourceFailed(source, error) { Object.assign(source, { status: source.lastSuccessMs == null ? 'failed' : 'stale', error: String(error) }); },
  markSourceUnavailable(source, error) { Object.assign(source, { status: 'unavailable', value: null, error: String(error) }); },
  renderMarkets() {}, renderCockpit() {}, console,
});
vm.runInContext(code, context);

(async () => {
  const config = {
    three_usd_ledger: { toText: () => 'three' }, ckusdc_ledger: { toText: () => 'ckusdc' },
    icp_ledger: { toText: () => 'icp' }, bob_ledger: { toText: () => 'bob' },
  };
  const loadBalances = vm.runInContext('loadBalances', context);
  const first = loadBalances(config);
  const overlapping = loadBalances(config);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(balanceCalls, 8, 'overlapping refreshes must share one ledger request');
  releaseBalances();
  await Promise.all([first, overlapping]);
  assert.equal(routeSources.wallet.status, 'fresh');
  assert.equal(context.latestRouteWallet.length, 6);
  const btc = context.latestRouteWallet.find(row => Object.hasOwn(row.asset, 'CkBtc'));
  const eth = context.latestRouteWallet.find(row => Object.hasOwn(row.asset, 'CkEth'));
  assert.equal(btc.balance_native.length, 1);
  assert.equal(btc.balance_native[0], 100n);
  assert.equal(eth.balance_native.length, 0);
  assert.match(eth.error[0], /cketh unavailable/);
  assert(context.latestRouteWallet.every(row => row.public_read === true));
  assert.equal(context.balanceRequestPromise, null);
  console.log('PASS: public route wallet supports non-admin sessions, deduplicates refreshes, and isolates per-asset failures');
})().catch(error => { console.error(error); process.exitCode = 1; });
