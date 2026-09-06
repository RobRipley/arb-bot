const { readFileSync } = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');

const html = readFileSync('src/arb_bot/src/dashboard.html', 'utf8');
const section = (start, end) => {
  const from = html.indexOf(start);
  const to = html.indexOf(end, from);
  assert.notEqual(from, -1, `missing ${start}`);
  assert.notEqual(to, -1, `missing end ${end}`);
  return html.slice(from, to);
};

const context = vm.createContext({
  BigInt,
  state: { activeView: 'cockpit', activePriceInverted: new Set() },
  window: {},
  TOKEN_INFO: {
    icp: { decimals: 8, label: 'ICP' },
    ckusdc: { decimals: 6, label: 'ckUSDC' },
    icusd: { decimals: 8, label: 'icUSD' },
  },
  ACTIVE_ROUTE_PROBE_NATIVE: { icp: 100_000_000n, ckusdc: 1_000_000n, icusd: 100_000_000n },
  ACTIVE_ROUTE_PRICE_REFRESH_MS: 30_000,
  routeSources: { activePrices: { value: [] } },
  routeSourceState: () => 'fresh',
  sourceStampHtml: () => '',
  sourceStateLabel: () => 'Unavailable',
  VENUES: {},
  venueLogo: () => '',
  esc: String,
  renderCockpit: () => { context.renderCount = (context.renderCount || 0) + 1; },
});

vm.runInContext(section('    function formatActiveRouteRate', '    async function loadActiveRoutePrices'), context);
vm.runInContext(section('    function priceBadge', '    const ROUTE_ASSET_LABELS'), context);

const price = {
  id: 'icp-ckusdc', group: 'ICP markers', input: 'icp', output: 'ckusdc', venue: 'ICPSwap',
  inputNative: 100_000_000n, outputNative: 2700200n, inputDecimals: 8, outputDecimals: 6,
};
context.routeSources.activePrices.value = [
  price,
  { ...price, id: 'stable', group: 'Stable routes', input: 'icusd', output: 'ckusdc', inputNative: 100_000_000n, outputNative: 998000n, inputDecimals: 8, outputDecimals: 6 },
  { ...price, id: 'btc', group: 'ckBTC / ckETH' },
];

const forward = vm.runInContext('activeRoutePriceBadge(routeSources.activePrices.value[0])', context);
assert.match(forward, /<button type="button" class="price-badge/);
assert.match(forward, /1 ICP = 2\.7002 ckUSDC/);
assert.match(forward, /Click to invert/);
assert.match(forward, /aria-pressed="false"/);

context.window.toggleActiveRoutePrice('icp-ckusdc');
const inverse = vm.runInContext('activeRoutePriceBadge(routeSources.activePrices.value[0])', context);
assert.match(inverse, /ckUSDC → ICP/);
assert.match(inverse, /1 ckUSDC = 0\.370343 ICP/);
assert.match(inverse, /Reciprocal of 1 ICP probe/);
assert.match(inverse, /aria-pressed="true"/);
assert.equal(context.renderCount, 1, 'card click must repaint the Cockpit');

const grouped = vm.runInContext('cockpitPriceBadgesHtml()', context);
assert(grouped.indexOf('Stable routes') < grouped.indexOf('ckBTC / ckETH'), 'stable routes must precede ckBTC/ckETH');
assert.match(html, /\.price-badge-row \{ display: grid; grid-template-columns: repeat\(auto-fill, minmax\(220px, 1fr\)\)/, 'cards must use a consistent grid');
assert.match(html, /window\.toggleActiveRoutePrice/, 'the card toggle must be exposed to click handlers');

console.log('PASS: quote cards use a fixed grid, stable routes precede ckBTC/ckETH, and cards invert their displayed rate');
