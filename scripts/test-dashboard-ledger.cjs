const { readFileSync } = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');

const html = readFileSync('src/arb_bot/src/dashboard.html', 'utf8');

function routeLedgerSection() {
  const start = html.indexOf('    function routeLedgerStatusKey');
  assert.notEqual(start, -1, 'dashboard must define routeLedgerEntryHtml');
  const end = html.indexOf('    function legendHtml', start);
  assert.notEqual(end, -1, 'route ledger helpers must end before legacy legend');
  return html.slice(start, end);
}

function option(value) {
  return value == null ? [] : [value];
}

function record(id = 'route-2') {
  return {
    execution_id: id,
    route_id: 'ckUSDC → icUSD → ICP → ckUSDC',
    phase: { Completed: null },
    planned_input_native: 40_000_000n,
    updated_at_ns: 1_700_000_000_000_000_000n,
    candidate_class: { StablePar: null },
    realized_profit: option(550_000n),
  };
}

function leg(index, count, overrides = {}) {
  const assets = [
    { CkUsdc: null },
    { IcUsd: null },
    { Icp: null },
    { CkUsdc: null },
    { CkUsdt: null },
    { CkBtc: null },
    { CkEth: null },
  ];
  return {
    leg_index: index,
    status: { Settled: null },
    venue: { ICPSwap: null },
    pool_id: `pool-${index + 1}`,
    pool_principal: { toText: () => `aaaaa-aa-${index + 1}` },
    edge_id: `edge-${index + 1}`,
    from: assets[index],
    to: assets[index + 1],
    quoted_input_native: 10_000_000n + BigInt(index),
    quoted_output_native: option(9_900_000n + BigInt(index)),
    minimum_output_native: 9_800_000n,
    input_fee_native: 10n,
    output_fee_native: 11n,
    actual_input_debit_native: option(10_000_100n + BigInt(index)),
    actual_effective_input_native: option(10_000_000n + BigInt(index)),
    actual_output_credit_native: option(9_900_001n + BigInt(index)),
    refund_credit_native: [],
    prepared_at_ns: option(1_700_000_000_100_000_000n + BigInt(index)),
    submitted_at_ns: option(1_700_000_000_200_000_000n + BigInt(index)),
    settled_at_ns: option(1_700_000_000_300_000_000n + BigInt(index)),
    reconciled_at_ns: option(1_700_000_000_400_000_000n + BigInt(index)),
    evidence: [{ evidence_kind: 'receipt', source_reference: `tx-${index + 1}`, amount_native: 9_900_001n, observed_at_ns: 1_700_000_000_300_000_000n }],
    incident: [],
    ...overrides,
    _count: count,
  };
}

function detail(id, count, overrides = {}) {
  return {
    record: record(id),
    detail_available: true,
    asset_path: Array.from({ length: count + 1 }, (_, i) => leg(i, count).from).concat([{ CkUsdc: null }]).slice(0, count + 1),
    legs: Array.from({ length: count }, (_, i) => leg(i, count)),
    ...overrides,
  };
}

function makeContext(actor, details = new Map()) {
  const elements = new Map();
  const context = vm.createContext({
    BigInt,
    Date,
    console: { error() {} },
    anonymousActor: actor,
    routeExecutionDetails: new Map(),
    routeExecutionDetailLoads: new Map(),
    state: { expandedRows: new Set() },
    leg,
    routeOpt: option,
    routeAssetKey: asset => Object.keys(asset || {})[0] || '—',
    routeAssetLabel: asset => ({ Icp: 'ICP', IcUsd: 'icUSD', CkUsdc: 'ckUSDC', CkUsdt: 'ckUSDT', CkBtc: 'ckBTC', CkEth: 'ckETH' }[Object.keys(asset || {})[0]] || '—'),
    routePhaseLabel: phase => Object.keys(phase || {})[0] || '—',
    variantKey: value => value && typeof value === 'object' ? Object.keys(value)[0] : '—',
    esc: value => String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c])),
    fmtTime: value => `time-${String(value)}`,
    fmtTokAmt: (value, symbol) => `${String(value)} ${symbol}`,
    bi: value => typeof value === 'bigint' ? Number(value) : value,
    pClass: value => Number(value) > 0 ? 'positive' : Number(value) < 0 ? 'negative' : '',
    fmt$: value => `$${String(value)}`,
    document: {
      getElementById(id) {
        if (!elements.has(id)) {
          const element = {
            id,
            hidden: false,
            innerHTML: '',
            textContent: '',
            attrs: { 'aria-expanded': 'false' },
            getAttribute(name) { return this.attrs[name] || null; },
            setAttribute(name, value) { this.attrs[name] = String(value); },
            querySelector() { return this; },
            focus() { this.focused = true; },
          };
          elements.set(id, element);
        }
        return elements.get(id);
      },
      querySelector(selector) {
        const match = selector.match(/ledger-detail-(.+)/);
        return match ? elements.get(`ledger-detail-${match[1]}`) || null : null;
      },
    },
    renderLedgerTable() {},
  });
  context.details = details;
  vm.runInContext(routeLedgerSection(), context);
  return { context, elements };
}

const noEagerCalls = [];
const actor = {
  get_route_execution_detail_v1: async id => {
    noEagerCalls.push(id);
    return { Ok: detail(id, 3) };
  },
};
const { context, elements } = makeContext(actor);

for (const count of [2, 3, 4, 6]) {
  context.entry = record(`exec-${count}`);
  context.executionDetail = detail(`exec-${count}`, count);
  const entry = vm.runInContext('routeLedgerEntryHtml(entry)', context);
  assert.match(entry, /route-execution-summary-exec-/);
  assert.match(entry, /aria-expanded="false"/);
  assert.match(entry, /aria-controls="ledger-detail-exec-/);
  assert.match(entry, /Realized P&amp;L/);
  assert.doesNotMatch(entry, /Leg 1 of/);
  const rendered = vm.runInContext('routeLegDetailHtml(executionDetail)', context);
  for (let index = 1; index <= count; index += 1) assert.match(rendered, new RegExp(`Leg ${index} of ${count}`));
  assert.equal((rendered.match(/Realized P&amp;L/g) || []).length, 0, 'P&L belongs only on the parent row');
  assert.match(rendered, /Venue/);
  assert.match(rendered, /Pool/);
  assert.match(rendered, /Quoted input/);
  assert.match(rendered, /Requested input/);
  assert.match(rendered, /Actual output/);
  assert.match(rendered, /Refund/);
  assert.match(rendered, /Submission evidence/);
  assert.match(rendered, /Settlement evidence/);
}

context.entry = { ...record('stable-pnl'), candidate_class: { StablePar: null }, realized_profit: option(1_234_567n) };
const stablePnl = vm.runInContext('routeLedgerEntryHtml(entry)', context);
assert.match(stablePnl, /USD/);
assert.match(stablePnl, /raw atoms \(asset unavailable\)/);
context.entry = { ...record('icp-pnl'), candidate_class: { IcpReturning: null }, realized_profit: option(123_456_789n) };
const icpPnl = vm.runInContext('routeLedgerEntryHtml(entry)', context);
assert.match(icpPnl, /ICP/);
assert.doesNotMatch(icpPnl, /\$123/);

context.executionDetail = detail('decimal-exec', 2, {
  legs: [leg(0, 2, { from: { CkBtc: null }, to: { CkEth: null }, evidence: [{ evidence_kind: 'receipt', source_reference: 'tx-decimal', amount_native: 123456789n, observed_at_ns: 1n }] }), leg(1, 2)],
});
const decimalHtml = vm.runInContext('routeLegDetailHtml(executionDetail)', context);
assert.match(decimalHtml, /ckBTC/);
assert.match(decimalHtml, /ckETH/);
assert.match(decimalHtml, /raw atoms/);
assert.doesNotMatch(decimalHtml, /amount_native[^<]*native/);

const specialId = vm.runInContext('routeLedgerEntryHtml({ ...entry, execution_id: "exec\' & <" })', context);
assert.doesNotMatch(specialId, /onclick=.*exec/);
assert.match(specialId, /data-ledger-execution-id=/);

const unavailable = vm.runInContext('routeLegDetailHtml({ detail_available: false, legs: [] })', context);
assert.match(unavailable, /Detailed legs were not recorded for this historical execution/);
assert.match(unavailable, /Evidence unavailable for historical record/);

const rejected = vm.runInContext("routeLegDetailHtml({ detail_available: true, legs: [leg(0, 1, { status: { RejectedBeforeDebit: null }, evidence: [] })] })", context);
assert.match(rejected, /No evidence required/);

const failed = vm.runInContext("routeLedgerDetailStateHtml('failed', 'temporary outage')", context);
assert.match(failed, /Unavailable/);
assert.match(failed, /Retry/);

const loading = vm.runInContext("routeLedgerDetailStateHtml('loading', '')", context);
assert.match(loading, /Loading leg details/);

context.entry = record('keyboard-exec');
const summary = vm.runInContext('routeLedgerEntryHtml(entry)', context);
assert.match(summary, /<button[^>]+type="button"/);
assert.match(summary, /aria-expanded="false"/);
assert.match(summary, /aria-controls="ledger-detail-keyboard-exec"/);

(async () => {
  assert.equal(noEagerCalls.length, 0, 'rendering summary rows must not fetch details');
  await vm.runInContext("toggleLedgerExecution('exec-3')", context);
  assert.deepEqual(noEagerCalls, ['exec-3']);
  assert.match(elements.get('ledger-detail-exec-3').innerHTML, /Leg 1 of 3/);
  assert.equal(elements.get('ledger-disclosure-exec-3').focused, true, 'disclosure retains focus after expansion');
  assert.equal(elements.get('ledger-disclosure-exec-3').getAttribute('aria-expanded'), 'true');
  await vm.runInContext("toggleLedgerExecution('exec-3')", context);
  assert.equal(noEagerCalls.length, 1, 'cached detail must not refetch');

  let attempts = 0;
  const retryActor = {
    get_route_execution_detail_v1: async id => {
      attempts += 1;
      if (attempts === 1) return { Err: 'temporary provider failure' };
      return { Ok: detail(id, 2) };
    },
  };
  const retryFixture = makeContext(retryActor);
  retryFixture.elements.set('ledger-disclosure-retry-exec', { id: 'ledger-disclosure-retry-exec', hidden: false, textContent: '', attrs: { 'aria-expanded': 'false' }, getAttribute(name) { return this.attrs[name] || null; }, setAttribute(name, value) { this.attrs[name] = String(value); }, focus() { this.focused = true; } });
  retryFixture.elements.set('ledger-detail-retry-exec', { id: 'ledger-detail-retry-exec', hidden: false, innerHTML: '', querySelector() { return this; } });
  await vm.runInContext("toggleLedgerExecution('retry-exec')", retryFixture.context);
  assert.equal(attempts, 1);
  assert.match(retryFixture.elements.get('ledger-detail-retry-exec').innerHTML, /Unavailable/);
  assert.match(retryFixture.elements.get('ledger-detail-retry-exec').innerHTML, /Retry/);
  await vm.runInContext("toggleLedgerExecution('retry-exec')", retryFixture.context);
  await vm.runInContext("toggleLedgerExecution('retry-exec')", retryFixture.context);
  assert.equal(attempts, 1, 'collapse and reopen must keep failed detail cached');
  await vm.runInContext("toggleLedgerExecution('retry-exec', true)", retryFixture.context);
  assert.equal(attempts, 2);
  assert.match(retryFixture.elements.get('ledger-detail-retry-exec').innerHTML, /Leg 1 of 2/);

  const unavailableFixture = makeContext(null);
  unavailableFixture.elements.set('ledger-disclosure-unavailable-exec', { id: 'ledger-disclosure-unavailable-exec', hidden: false, textContent: '', attrs: { 'aria-expanded': 'false' }, getAttribute(name) { return this.attrs[name] || null; }, setAttribute(name, value) { this.attrs[name] = String(value); }, focus() { this.focused = true; } });
  unavailableFixture.elements.set('ledger-detail-unavailable-exec', { id: 'ledger-detail-unavailable-exec', hidden: false, innerHTML: '', querySelector() { return this; } });
  await vm.runInContext("toggleLedgerExecution('unavailable-exec')", unavailableFixture.context);
  assert.match(unavailableFixture.elements.get('ledger-detail-unavailable-exec').innerHTML, /Unavailable/);
  await vm.runInContext("toggleLedgerExecution('unavailable-exec')", unavailableFixture.context);
  await vm.runInContext("toggleLedgerExecution('unavailable-exec')", unavailableFixture.context);
  assert.match(unavailableFixture.elements.get('ledger-detail-unavailable-exec').innerHTML, /Unavailable/);
  console.log('dashboard ledger behavior tests passed');
})().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
