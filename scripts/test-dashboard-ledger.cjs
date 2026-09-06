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
            attrs: { 'aria-expanded': 'false', ...(id.startsWith('ledger-disclosure-') ? { 'data-ledger-execution-id': id.slice('ledger-disclosure-'.length) } : {}) },
            getAttribute(name) { return this.attrs[name] || null; },
            setAttribute(name, value) { this.attrs[name] = String(value); },
            querySelector() { return this; },
            querySelectorAll(selector) {
              if (selector !== '[data-ledger-execution-id]') return [];
              return Array.from(elements.values()).filter(candidate => candidate.getAttribute && candidate.getAttribute('data-ledger-execution-id'));
            },
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

for (const count of [2, 3, 4, 5, 6]) {
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
  assert.match(rendered, /Requested input<\/span><span class="v">Unavailable · not separately recorded/);
  assert.equal((rendered.match(/Requested input/g) || []).length, count, 'requested input must not duplicate quoted input');
  assert.match(rendered, /Actual output/);
  assert.match(rendered, /Refund/);
  assert.match(rendered, /Submission evidence/);
  assert.match(rendered, /Settlement evidence/);
}

context.executionDetail = detail('five-leg', 5);
const fiveLegHtml = vm.runInContext('routeLegDetailHtml(executionDetail)', context);
assert.deepEqual(
  Array.from(fiveLegHtml.matchAll(/Leg (\d+) of 5/g), match => Number(match[1])),
  [1, 2, 3, 4, 5],
  'five-leg details must retain canonical leg order',
);

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

context.executionDetail = detail('evidence-all', 1, {
  legs: [leg(0, 1, { evidence: [{ evidence_kind: 'icpswap_source_bound_terminal_transfers_v1', source_reference: "source'<&", amount_native: 7n, observed_at_ns: 1n }] })],
});
const uncategorizedEvidence = vm.runInContext('routeLegDetailHtml(executionDetail)', context);
assert.match(uncategorizedEvidence, /icpswap_source_bound_terminal_transfers_v1/);
assert.match(uncategorizedEvidence, /source&#39;&lt;&amp;/);
assert.match(uncategorizedEvidence, /data-ledger-copy-evidence=/);
assert.doesNotMatch(uncategorizedEvidence, /No evidence yet/);

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

const dashboardSource = readFileSync('src/arb_bot/src/dashboard.html', 'utf8');
for (const marker of ['routeLedgerRequestGeneration', 'routeLedgerRequestedPage', 'routeLedgerCurrentDisclosureButton', 'querySelectorAll', 'copyLedgerEvidenceReference', 'data-ledger-copy-evidence', 'Copy failed']) {
  assert.match(dashboardSource, new RegExp(marker), `historical ledger race guard is missing ${marker}`);
}

(async () => {
  assert.equal(noEagerCalls.length, 0, 'rendering summary rows must not fetch details');
  await vm.runInContext("toggleLedgerExecution('exec-3')", context);
  assert.deepEqual(noEagerCalls, ['exec-3']);
  assert.match(elements.get('ledger-detail-exec-3').innerHTML, /Leg 1 of 3/);
  assert.equal(elements.get('ledger-disclosure-exec-3').focused, true, 'disclosure retains focus after expansion');
  assert.equal(elements.get('ledger-disclosure-exec-3').getAttribute('aria-expanded'), 'true');
  await vm.runInContext("toggleLedgerExecution('exec-3')", context);
  assert.equal(noEagerCalls.length, 1, 'cached detail must not refetch');

  const copyFeedback = { textContent: '' };
  const copyContainer = { querySelector(selector) { return selector === '[data-ledger-copy-feedback]' ? copyFeedback : null; } };
  context.copyButton = { parentNode: copyContainer };
  context.navigator = { clipboard: { writeText: async value => { context.copiedEvidence = value; } } };
  await vm.runInContext("copyLedgerEvidenceReference(copyButton, \"source'<&\")", context);
  assert.equal(context.copiedEvidence, "source'<&");
  assert.equal(copyFeedback.textContent, 'Copied');
  context.navigator = { clipboard: { writeText: async () => { throw new Error('clipboard blocked'); } } };
  await vm.runInContext("copyLedgerEvidenceReference(copyButton, 'source-failure')", context);
  assert.equal(copyFeedback.textContent, 'Copy failed');
  const fallbackTextarea = { style: {}, setAttribute() {}, select() {} };
  context.document.createElement = () => fallbackTextarea;
  context.document.body = { appendChild() {}, removeChild() {} };
  context.document.execCommand = command => command === 'copy';
  context.navigator = {};
  await vm.runInContext("copyLedgerEvidenceReference(copyButton, 'source-fallback')", context);
  assert.equal(copyFeedback.textContent, 'Copied', 'clipboard fallback should report success');

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

  let releaseRerenderDetail;
  const rerenderDetailPromise = new Promise(resolve => { releaseRerenderDetail = resolve; });
  const rerenderFixture = makeContext({
    get_route_execution_detail_v1: async () => rerenderDetailPromise,
  });
  const makeDisclosure = id => ({
    id: `ledger-disclosure-${id}`,
    attrs: { 'aria-expanded': 'false', 'data-ledger-execution-id': id },
    textContent: '',
    getAttribute(name) { return this.attrs[name] || null; },
    setAttribute(name, value) { this.attrs[name] = String(value); },
    focus() { this.focused = true; },
  });
  const oldDisclosure = makeDisclosure('rerender-exec');
  const oldDetail = { id: 'ledger-detail-rerender-exec', hidden: false, innerHTML: '', querySelector() { return this; } };
  let currentDisclosure = oldDisclosure;
  rerenderFixture.elements.set('ledger-disclosure-rerender-exec', oldDisclosure);
  rerenderFixture.elements.set('ledger-detail-rerender-exec', oldDetail);
  rerenderFixture.elements.set('route-ledger-body', { querySelectorAll() { return [currentDisclosure]; } });
  const pendingRerenderDetail = vm.runInContext("toggleLedgerExecution('rerender-exec')", rerenderFixture.context);
  await Promise.resolve();
  const newDisclosure = makeDisclosure('rerender-exec');
  const newDetail = { id: 'ledger-detail-rerender-exec', hidden: false, innerHTML: '', querySelector() { return this; } };
  currentDisclosure = newDisclosure;
  rerenderFixture.elements.set('ledger-disclosure-rerender-exec', newDisclosure);
  rerenderFixture.elements.set('ledger-detail-rerender-exec', newDetail);
  releaseRerenderDetail({ Ok: detail('rerender-exec', 2) });
  await pendingRerenderDetail;
  assert.equal(oldDisclosure.focused, undefined, 'async rerender must not focus detached disclosure');
  assert.equal(newDisclosure.focused, true, 'async rerender must focus current disclosure');

  const paginationElements = new Map();
  const paginationRequests = [];
  const paginationContext = vm.createContext({
    BigInt,
    Date,
    Number,
    Object,
    Array,
    Promise,
    console: { error() {} },
    window: {},
    routeSources: { routeLedgerExecutions: { status: 'fresh', value: [], error: null, lastSuccessMs: 1, lastAttemptMs: 1 } },
    anonymousActor: {
      get_terminal_route_executions_v1(offset, limit) {
        return new Promise(resolve => paginationRequests.push({ offset: Number(offset), limit: Number(limit), resolve }));
      },
    },
    routeSourceError: source => source.error ? `: ${source.error}` : '',
    sourceLastSuccessLabel: () => 'as of test',
    markSourceFresh: (source, value) => Object.assign(source, { status: 'fresh', value, error: null }),
    markSourceFailed: (source, error) => Object.assign(source, { status: 'failed', error: String(error) }),
    markSourceUnavailable: (source, reason) => Object.assign(source, { status: 'unavailable', error: String(reason) }),
    routeLedgerEntryHtml: row => `<tr>${row.execution_id}</tr>`,
    bindRouteLedgerDisclosureHandlers() {},
    esc: value => String(value),
    document: {
      getElementById(id) {
        if (!paginationElements.has(id)) paginationElements.set(id, { innerHTML: '', outerHTML: '', textContent: '', disabled: false });
        return paginationElements.get(id);
      },
    },
  });
  const paginationStart = dashboardSource.indexOf('    function routeLedgerSourceHtml');
  const paginationEnd = dashboardSource.indexOf('    window.ledgerPrev', paginationStart);
  assert.notEqual(paginationStart, -1);
  assert.notEqual(paginationEnd, -1);
  vm.runInContext(`let routeLedgerPage = 0; let routeLedgerHasMore = false; let routeLedgerIncomplete = false; let routeLedgerLoadedCount = 0; let routeLedgerRequestedPage = 0; let routeLedgerRequestGeneration = 0; let routeLedgerRequestPromise = null; let routeLedgerRequestPage = null; const ROUTE_LEDGER_PAGE_SIZE = 25n; const ROUTE_LEDGER_MAX = 10000n;\n${dashboardSource.slice(paginationStart, paginationEnd)}`, paginationContext);
  const firstPage = vm.runInContext('loadRouteLedgerPage(0)', paginationContext);
  assert.equal(paginationRequests.length, 1, 'initial route page should issue one request');
  paginationRequests[0].resolve({ Ok: Array.from({ length: 25 }, (_, index) => ({ execution_id: `page-0-${index}`, updated_at_ns: BigInt(index) })) });
  await firstPage;
  const nextPage = vm.runInContext('window.routeLedgerNext()', paginationContext);
  const duplicateNext = vm.runInContext('loadRouteLedgerPage(1)', paginationContext);
  assert.equal(paginationRequests.length, 2, 'background rerender must dedupe the requested page');
  const previousPage = vm.runInContext('window.routeLedgerPrev()', paginationContext);
  assert.equal(paginationRequests.length, 3, 'rapid Next then Prev should issue only the new requested page');
  paginationRequests[2].resolve({ Ok: [{ execution_id: 'page-0-returned', updated_at_ns: 3n }] });
  await previousPage;
  paginationRequests[1].resolve({ Ok: [{ execution_id: 'stale-page-1', updated_at_ns: 4n }] });
  await Promise.all([nextPage, duplicateNext]);
  assert.equal(paginationContext.routeSources.routeLedgerExecutions.value[0].execution_id, 'page-0-returned', 'stale page response must not overwrite newer requested rows');
  console.log('dashboard ledger behavior tests passed');
})().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
