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

const context = vm.createContext({
  esc: value => String(value),
  routeOpt: value => Array.isArray(value) ? value[0] || null : value || null,
  routeAssetLabel: asset => Object.keys(asset)[0],
  routeNativeAmount: amount => String(amount),
  routeCandidateQuoteState: () => 'fresh',
  routeAge: () => '4s',
});
vm.runInContext(section(
  '    function routeTopCandidateGate',
  '    function routeWalletHtml',
), context);

function candidate(routeId, spreadBps, eligible, rejectionReason) {
  return {
    route_id: routeId,
    asset_path: [{ IcUsd: null }, { Icp: null }, { IcUsd: null }],
    principal_native: 40_000_000n,
    start_asset: { IcUsd: null },
    net_profit_bps: spreadBps,
    eligible,
    rejection_reason: rejectionReason ? [rejectionReason] : [],
    full_fill: true,
    quote_timestamp_ns: 1n,
  };
}

const rendered = vm.runInContext('routeTopCandidateRowsHtml(report)', vm.createContext({
  ...context,
  report: {
    observation_id: ['observation-7'],
    scan_complete: true,
    candidates: [
      candidate('route-1', 80, false, 'profit bps does not meet configured threshold'),
      candidate('route-2', 70, false, 'profit bps does not meet configured threshold'),
      candidate('route-3', 60, false, 'inventory band would be exceeded'),
      candidate('route-4', 50, true, null),
      candidate('route-5', 40, false, 'allowance is unknown'),
      candidate('route-6', 30, false, 'profit bps does not meet configured threshold'),
    ],
  },
}));

for (const expected of ['+80 bps', '+70 bps', '+60 bps', '+50 bps', '+40 bps']) {
  assert.match(rendered, new RegExp(expected.replace('+', '\\+')), `top-five spread missing ${expected}`);
}
assert.doesNotMatch(rendered, /\+30 bps/, 'the panel must show only the top five routes');
assert(rendered.indexOf('+80 bps') < rendered.indexOf('+40 bps'), 'backend ranking order must be retained');
assert.match(rendered, /Below profit threshold/, 'below-threshold routes need a direct human label');
assert.match(rendered, /Inventory blocked/, 'other eligibility gates should remain visible');
assert.doesNotMatch(rendered, /<button\b/, 'the quote-only panel must not add a force-trade control');

console.log('dashboard top-candidate behavior tests passed');
