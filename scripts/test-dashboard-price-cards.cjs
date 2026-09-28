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
  state: { activeView: 'cockpit', activePriceInverted: new Set(), linkedStablePairs: new Set() },
  window: {},
  STABLE_ROUTE_LINKS: [
    { key: 'icusd-ckusdt', topId: 'icpswap-icusd-ckusdt', bottomId: 'rumi-3pool-icusd-ckusdt' },
    { key: 'icusd-ckusdc', topId: 'icpswap-icusd-ckusdc', bottomId: 'rumi-3pool-icusd-ckusdc' },
    { key: 'ckusdt-ckusdc', topId: 'icpswap-ckusdt-ckusdc', bottomId: 'rumi-3pool-ckusdt-ckusdc' },
  ],
  TOKEN_INFO: {
    icp: { decimals: 8, label: 'ICP' },
    ckusdc: { decimals: 6, label: 'ckUSDC' },
    ckusdt: { decimals: 6, label: 'ckUSDT' },
    icusd: { decimals: 8, label: 'icUSD' },
  },
  ACTIVE_ROUTE_PROBE_NATIVE: { icp: 100_000_000n, ckusdc: 1_000_000n, ckusdt: 1_000_000n, icusd: 100_000_000n },
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
  { ...price, id: 'stable-icusd-ckusdt-icpswap', group: 'Stable routes', input: 'icusd', output: 'ckusdc', inputNative: 100_000_000n, outputNative: 998000n, inputDecimals: 8, outputDecimals: 6, venue: 'ICPSwap' },
  { ...price, id: 'stable-icusd-ckusdc-icpswap', group: 'Stable routes', input: 'icusd', output: 'ckusdc', inputNative: 100_000_000n, outputNative: 998000n, inputDecimals: 8, outputDecimals: 6, venue: 'ICPSwap' },
  { ...price, id: 'stable-ckusdt-ckusdc-icpswap', group: 'Stable routes', input: 'icusd', output: 'ckusdc', inputNative: 100_000_000n, outputNative: 998000n, inputDecimals: 8, outputDecimals: 6, venue: 'ICPSwap' },
  { ...price, id: 'stable-icusd-ckusdt-rumi', group: 'Stable routes', input: 'icusd', output: 'ckusdc', inputNative: 100_000_000n, outputNative: 998000n, inputDecimals: 8, outputDecimals: 6, venue: 'Rumi 3pool' },
  { ...price, id: 'stable-icusd-ckusdc-rumi', group: 'Stable routes', input: 'icusd', output: 'ckusdc', inputNative: 100_000_000n, outputNative: 998000n, inputDecimals: 8, outputDecimals: 6, venue: 'Rumi 3pool' },
  { ...price, id: 'stable-ckusdt-ckusdc-rumi', group: 'Stable routes', input: 'icusd', output: 'ckusdc', inputNative: 100_000_000n, outputNative: 998000n, inputDecimals: 8, outputDecimals: 6, venue: 'Rumi 3pool' },
  { ...price, id: 'icpswap-icusd-ckusdt', group: 'Stable routes', input: 'icusd', output: 'ckusdt', inputNative: 100_000_000n, outputNative: 950000n, inputDecimals: 8, outputDecimals: 6, venue: 'ICPSwap' },
  { ...price, id: 'rumi-3pool-icusd-ckusdt', group: 'Stable routes', input: 'icusd', output: 'ckusdt', inputNative: 100_000_000n, outputNative: 960000n, inputDecimals: 8, outputDecimals: 6, venue: 'Rumi 3pool' },
  { ...price, id: 'icpswap-icusd-ckusdc', group: 'Stable routes', input: 'icusd', output: 'ckusdc', inputNative: 100_000_000n, outputNative: 950000n, inputDecimals: 8, outputDecimals: 6, venue: 'ICPSwap' },
  { ...price, id: 'rumi-3pool-icusd-ckusdc', group: 'Stable routes', input: 'icusd', output: 'ckusdc', inputNative: 100_000_000n, outputNative: 960000n, inputDecimals: 8, outputDecimals: 6, venue: 'Rumi 3pool' },
  { ...price, id: 'icpswap-ckusdt-ckusdc', group: 'Stable routes', input: 'ckusdt', output: 'ckusdc', inputNative: 1_000_000n, outputNative: 1_000_000n, inputDecimals: 6, outputDecimals: 6, venue: 'ICPSwap' },
  { ...price, id: 'rumi-3pool-ckusdt-ckusdc', group: 'Stable routes', input: 'ckusdt', output: 'ckusdc', inputNative: 1_000_000n, outputNative: 1_000_000n, inputDecimals: 6, outputDecimals: 6, venue: 'Rumi 3pool' },
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
assert.match(grouped, /stable-route-column/g, 'stable routes should be paired in three vertical columns');
assert.equal((grouped.match(/aria-label="Connect/g) || []).length, 3, 'all compatible stable pairs should have independent off-by-default connect controls');
assert.doesNotMatch(html, /id: 'icpswap-ckusdt-ckbtc'/, 'the unusable ckBTC/ckUSDT pool must not be presented as a live market quote');
assert.match(html, /window\.toggleActiveRoutePrice/, 'the card toggle must be exposed to click handlers');

context.window.toggleStableRouteLink('icusd-ckusdc');
assert(context.state.linkedStablePairs.has('icusd-ckusdc'), 'connecting a pair should enable only that path');
assert(context.state.activePriceInverted.has('rumi-3pool-icusd-ckusdc'), 'the bottom card should reverse to consume the top output');
let linkedEstimate = vm.runInContext("stableRouteEstimateHtml(STABLE_ROUTE_LINKS[1], routeSources.activePrices.value.find(p => p.id === 'icpswap-icusd-ckusdc'), routeSources.activePrices.value.find(p => p.id === 'rumi-3pool-icusd-ckusdc'))", context);
assert.match(linkedEstimate, /1 icUSD<\/strong> → 0\.95 ckUSDC → <strong>0\.989583 icUSD/);
assert.match(linkedEstimate, /Estimated round trip: <strong class="negative">−0\.010416 icUSD/);
assert.match(linkedEstimate, /not a fresh quote or executable return/);

context.window.toggleActiveRoutePrice('icpswap-icusd-ckusdc');
assert(context.state.activePriceInverted.has('icpswap-icusd-ckusdc'), 'inverting the top card should change its direction');
assert(!context.state.activePriceInverted.has('rumi-3pool-icusd-ckusdc'), 'the bottom card should stay in the opposite direction while linked');
linkedEstimate = vm.runInContext("stableRouteEstimateHtml(STABLE_ROUTE_LINKS[1], routeSources.activePrices.value.find(p => p.id === 'icpswap-icusd-ckusdc'), routeSources.activePrices.value.find(p => p.id === 'rumi-3pool-icusd-ckusdc'))", context);
assert.match(linkedEstimate, /1 ckUSDC<\/strong> → 1\.052631 icUSD → <strong>1\.010526 ckUSDC/);
assert.match(linkedEstimate, /Estimated round trip: <strong class="positive">\+0\.010526 ckUSDC/);
const linkedTop = vm.runInContext("stableLinkedTopPriceBadge(STABLE_ROUTE_LINKS[1], routeSources.activePrices.value.find(p => p.id === 'icpswap-icusd-ckusdc'), routeSources.activePrices.value.find(p => p.id === 'rumi-3pool-icusd-ckusdc'))", context);
let linkedBottom = vm.runInContext("stableLinkedBottomPriceBadge(STABLE_ROUTE_LINKS[1], routeSources.activePrices.value.find(p => p.id === 'icpswap-icusd-ckusdc'), routeSources.activePrices.value.find(p => p.id === 'rumi-3pool-icusd-ckusdc'))", context);
assert.match(linkedTop, /1 ckUSDC = 1\.052631 icUSD/);
assert.match(linkedBottom, /1\.052631 icUSD = 1\.010526 ckUSDC/);
context.window.toggleActiveRoutePrice('rumi-3pool-icusd-ckusdc');
assert(!context.state.activePriceInverted.has('icpswap-icusd-ckusdc'), 'inverting the bottom card should keep the connected directions opposite');
assert(context.state.activePriceInverted.has('rumi-3pool-icusd-ckusdc'), 'the clicked bottom card should invert');

context.window.toggleStableRouteLink('icusd-ckusdc');
assert(!context.state.linkedStablePairs.has('icusd-ckusdc'), 'disconnect should disable the path');
context.state.activePriceInverted.delete('icpswap-icusd-ckusdc');
context.state.activePriceInverted.delete('rumi-3pool-icusd-ckusdc');
context.window.toggleActiveRoutePrice('icpswap-icusd-ckusdc');
assert(context.state.activePriceInverted.has('icpswap-icusd-ckusdc'), 'disconnected cards should keep independent inversion controls');
assert(!context.state.activePriceInverted.has('rumi-3pool-icusd-ckusdc'), 'disconnected inversion should not affect the other card');

console.log('PASS: stable quote links chain compatible cards, keep directions connected, and label scaled round-trip estimates');
