import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);

// This module is intentionally independent of the arb canister. It only
// invokes public query calls against code-pinned pool and ledger principals.
const ASSETS = {
  IcUsd: { symbol: 'icUSD', ledger: 't6bor-paaaa-aaaap-qrd5q-cai', decimals: 8 },
  CkUsdt: { symbol: 'ckUSDT', ledger: 'cngnf-vqaaa-aaaar-qag4q-cai', decimals: 6 },
  CkUsdc: { symbol: 'ckUSDC', ledger: 'xevnm-gaaaa-aaaar-qafnq-cai', decimals: 6 },
  Icp: { symbol: 'ICP', ledger: 'ryjl3-tyaaa-aaaaa-aaaba-cai', decimals: 8 },
  CkBtc: { symbol: 'ckBTC', ledger: 'mxzaz-hqaaa-aaaar-qaada-cai', decimals: 8 },
  CkEth: { symbol: 'ckETH', ledger: 'ss2fx-dyaaa-aaaar-qacoq-cai', decimals: 18 },
};

const POOLS = [
  ['rumi-3pool', 'fohh4-yyaaa-aaaap-qtkpa-cai', 'rumi', ['IcUsd', 'CkUsdt', 'CkUsdc']],
  ['icpswap-icp-ckusdc', 'mohjv-bqaaa-aaaag-qjyia-cai', 'icpswap', ['Icp', 'CkUsdc']],
  ['icpswap-icp-icusd', 'nqxwe-hiaaa-aaaar-qb5yq-cai', 'icpswap', ['Icp', 'IcUsd']],
  ['icpswap-icp-ckusdt', 'hkstf-6iaaa-aaaag-qkcoq-cai', 'icpswap', ['Icp', 'CkUsdt']],
  ['icpswap-icusd-ckusdt', 'jogrm-gqaaa-aaaar-qcg2a-cai', 'icpswap', ['IcUsd', 'CkUsdt']],
  ['icpswap-icusd-ckusdc', 'eb25l-dyaaa-aaaar-qb4lq-cai', 'icpswap', ['IcUsd', 'CkUsdc']],
  ['icpswap-ckusdt-ckusdc', 'heq6n-fyaaa-aaaag-qkcpq-cai', 'icpswap', ['CkUsdt', 'CkUsdc']],
  ['icpswap-ckbtc-icp', 'xmiu5-jqaaa-aaaag-qbz7q-cai', 'icpswap', ['CkBtc', 'Icp']],
  ['icpswap-icp-cketh', 'angxa-baaaa-aaaag-qcvnq-cai', 'icpswap', ['Icp', 'CkEth']],
  ['icpswap-ckbtc-cketh', 'akhru-myaaa-aaaag-qcvna-cai', 'icpswap', ['CkBtc', 'CkEth']],
  ['icpswap-cketh-ckusdc', 'mvcvq-3iaaa-aaaag-qjykq-cai', 'icpswap', ['CkEth', 'CkUsdc']],
  ['icpswap-ckbtc-ckusdc', 'mhecj-xyaaa-aaaag-qjyjq-cai', 'icpswap', ['CkBtc', 'CkUsdc']],
  ['icpswap-ckbtc-icusd', 'jhf2q-qyaaa-aaaar-qcg3q-cai', 'icpswap', ['CkBtc', 'IcUsd']],
  ['icpswap-ckusdt-ckbtc', 'ipfno-pqaaa-aaaag-qkevq-cai', 'icpswap', ['CkUsdt', 'CkBtc']],
  ['icpswap-cketh-icusd', 'jjhxy-liaaa-aaaar-qcg2q-cai', 'icpswap', ['CkEth', 'IcUsd']],
].map(([id, principal, venue, assets]) => ({ id, principal, venue, assets }));

const ALLOWED_PRINCIPALS = new Set([
  ...POOLS.map(pool => pool.principal),
  ...Object.values(ASSETS).map(asset => asset.ledger),
]);
const ALLOWED_METHODS = new Set(['quoteForAll', 'calc_swap', 'metadata', 'icrc1_fee']);

export function buildQueryCommand({ pool, method, args }) {
  if (!ALLOWED_PRINCIPALS.has(pool)) throw new Error(`unrecognized local scan principal: ${pool}`);
  if (!ALLOWED_METHODS.has(method)) throw new Error(`method is not allowed in local scanner: ${method}`);
  return {
    executable: 'icp',
    args: ['canister', 'call', pool, method, args, '--query', '--network', 'ic', '--output', 'candid'],
  };
}

async function publicQuery(request) {
  const command = buildQueryCommand(request);
  const { stdout } = await execFileAsync(command.executable, command.args, { encoding: 'utf8', timeout: 45_000 });
  return stdout;
}

export function decodeNatResponse(candid) {
  const match = candid.match(/(?:ok|Ok)\s*=\s*([\d_]+)\s*:\s*nat|\b([\d_]+)\s*:\s*nat/);
  if (!match) throw new Error(`quote response did not contain nat: ${candid.trim()}`);
  return BigInt((match[1] ?? match[2]).replaceAll('_', ''));
}

function parseToken0(candid) {
  const match = candid.match(/token0\s*=\s*record\s*\{[^}]*address\s*=\s*"([^"]+)"/s);
  if (!match) throw new Error(`metadata response did not include token0 address: ${candid.trim()}`);
  const asset = Object.entries(ASSETS).find(([, detail]) => detail.ledger === match[1])?.[0];
  if (!asset) throw new Error(`metadata token0 is not in the pinned asset registry: ${match[1]}`);
  return asset;
}

function directedEdges() {
  return POOLS.flatMap(pool => pool.assets.flatMap((from, index) =>
    pool.assets.slice(index + 1).flatMap(to => [
      { id: `${pool.id}:${from}>${to}`, pool, from, to },
      { id: `${pool.id}:${to}>${from}`, pool, from: to, to: from },
    ]),
  )).sort((a, b) => a.id.localeCompare(b.id));
}

function candidateClass(path, selected) {
  if (!selected.length) return null;
  const [start] = path;
  const end = path.at(-1);
  if (path.every(asset => ['IcUsd', 'CkUsdt', 'CkUsdc'].includes(asset))) return 'stable-par';
  if (['IcUsd', 'CkUsdt', 'CkUsdc'].includes(start) && ['IcUsd', 'CkUsdt', 'CkUsdc'].includes(end)) return 'stable-settled';
  if (start === 'Icp' && end === 'Icp' && selected.length > 1 && path.slice(1, -1).every(asset => ['IcUsd', 'CkUsdt', 'CkUsdc'].includes(asset))) return 'icp-returning';
  if (start === 'CkBtc' && end === 'CkBtc' && selected.length > 1) return 'ckbtc-returning';
  if (start === 'CkEth' && end === 'CkEth' && selected.length > 1) return 'cketh-returning';
  return null;
}

function walkRoutes(start, maxLegs, edges, path = [start], selected = [], routes = []) {
  const candidate = candidateClass(path, selected);
  if (candidate) routes.push({ id: selected.map(edge => edge.id).join('|'), startAsset: start, endAsset: path.at(-1), candidate, edges: [...selected] });
  if (selected.length === maxLegs || (selected.length && path.at(-1) === start)) return routes;
  for (const edge of edges.filter(candidate => candidate.from === path.at(-1))) {
    if (selected.some(prior => prior.id === edge.id || prior.pool.id === edge.pool.id)) continue;
    const closesCycle = edge.to === start;
    if (path.includes(edge.to) && !closesCycle) continue;
    walkRoutes(start, maxLegs, edges, [...path, edge.to], [...selected, edge], routes);
  }
  return routes;
}

function sizeLadder(asset) {
  if (asset === 'Icp') return [100_000_000n, 500_000_000n, 1_000_000_000n];
  if (asset === 'CkBtc') return [100_000n, 500_000n, 1_000_000n];
  if (asset === 'CkEth') return [1_000_000_000_000_000n, 5_000_000_000_000_000n, 10_000_000_000_000_000n];
  const factor = 10n ** BigInt(ASSETS[asset].decimals - 6);
  return [1_000_000n, 5_000_000n, 10_000_000n, 40_000_000n].map(amount => amount * factor);
}

export function buildCycleWorkItems({ maxRouteLegs = 3 } = {}) {
  if (!Number.isInteger(maxRouteLegs) || maxRouteLegs < 2 || maxRouteLegs > 4) {
    throw new Error('maxRouteLegs must be an integer between 2 and 4');
  }
  const edges = directedEdges();
  const routes = Object.keys(ASSETS).flatMap(start => walkRoutes(start, maxRouteLegs, edges));
  return routes.flatMap(route => sizeLadder(route.startAsset).map((principalNative, sizeIndex) => ({
    ...route,
    sizeIndex,
    principalNative,
  }))).sort((a, b) => `${a.id}#${a.sizeIndex}`.localeCompare(`${b.id}#${b.sizeIndex}`));
}

export function rankCompletedRoute({ routeId, startAsset, endAsset = startAsset, principalNative, expectedLegs, outputs }) {
  if (!outputs.length || outputs.length !== expectedLegs || outputs.some(value => typeof value !== 'bigint')) return null;
  const finalNative = outputs.at(-1);
  const stablePath = ['IcUsd', 'CkUsdt', 'CkUsdc'].includes(startAsset) && ['IcUsd', 'CkUsdt', 'CkUsdc'].includes(endAsset);
  const toComparable = (amount, asset) => stablePath ? amount / (10n ** BigInt(ASSETS[asset].decimals - 6)) : amount;
  const comparablePrincipal = toComparable(principalNative, startAsset);
  const comparableFinal = toComparable(finalNative, endAsset);
  if (comparableFinal <= comparablePrincipal) return null;
  const netProfitNative = comparableFinal - comparablePrincipal;
  return {
    routeId,
    startAsset,
    endAsset,
    principalNative: principalNative.toString(),
    finalNative: finalNative.toString(),
    netProfitNative: netProfitNative.toString(),
    netProfitBps: Number((netProfitNative * 10_000n) / comparablePrincipal),
    parAssumption: stablePath && startAsset !== endAsset,
    complete: true,
  };
}

export function applyLedgerFees({ walletBefore, sourceFee, grossOutput, destinationFee }) {
  if (walletBefore <= sourceFee) throw new Error('source ledger fee consumes the route input');
  if (grossOutput <= destinationFee) throw new Error('destination ledger fee consumes the route output');
  return grossOutput - destinationFee;
}

async function quoteEdge(edge, amount, token0ByPool) {
  if (edge.pool.venue === 'rumi') {
    const coinIn = edge.pool.assets.indexOf(edge.from);
    const coinOut = edge.pool.assets.indexOf(edge.to);
    return decodeNatResponse(await publicQuery({ pool: edge.pool.principal, method: 'calc_swap', args: `(${coinIn} : nat8, ${coinOut} : nat8, ${amount} : nat)` }));
  }
  const token0 = token0ByPool.get(edge.pool.id);
  if (!token0) throw new Error(`missing verified token ordering for ${edge.pool.id}`);
  const zeroForOne = token0 === edge.from ? 'true' : 'false';
  return decodeNatResponse(await publicQuery({
    pool: edge.pool.principal,
    method: 'quoteForAll',
    args: `(record { amountIn = "${amount}"; zeroForOne = ${zeroForOne}; amountOutMinimum = "0" })`,
  }));
}

async function loadTokenOrderings() {
  const entries = await Promise.all(POOLS.filter(pool => pool.venue === 'icpswap').map(async pool => {
    const token0 = parseToken0(await publicQuery({ pool: pool.principal, method: 'metadata', args: '()' }));
    if (!pool.assets.includes(token0)) throw new Error(`metadata token0 does not belong to ${pool.id}`);
    return [pool.id, token0];
  }));
  return new Map(entries);
}

async function loadLedgerFees() {
  const entries = await Promise.all(Object.entries(ASSETS).map(async ([asset, detail]) => [
    asset,
    decodeNatResponse(await publicQuery({ pool: detail.ledger, method: 'icrc1_fee', args: '()' })),
  ]));
  return new Map(entries);
}

async function scanWorkItem(item, token0ByPool, ledgerFees) {
  let amount = item.principalNative;
  const outputs = [];
  for (const edge of item.edges) {
    const sourceFee = ledgerFees.get(edge.from);
    const destinationFee = ledgerFees.get(edge.to);
    if (sourceFee === undefined || destinationFee === undefined) throw new Error(`missing pinned ledger fee for ${edge.from} or ${edge.to}`);
    if (amount <= sourceFee) throw new Error('source ledger fee consumes the route input');
    const grossOutput = await quoteEdge(edge, amount - sourceFee, token0ByPool);
    amount = applyLedgerFees({ walletBefore: amount, sourceFee, grossOutput, destinationFee });
    outputs.push(amount);
  }
  return rankCompletedRoute({ routeId: item.id, startAsset: item.startAsset, endAsset: item.endAsset, principalNative: item.principalNative, expectedLegs: item.edges.length, outputs });
}

async function mapWithConcurrency(values, limit, worker) {
  const output = [];
  let cursor = 0;
  await Promise.all(Array.from({ length: Math.min(limit, values.length) }, async () => {
    while (cursor < values.length) {
      const index = cursor++;
      try { output[index] = { result: await worker(values[index]) }; }
      catch (error) { output[index] = { error: error.message }; }
    }
  }));
  return output;
}

export async function runLocalManualScan({ maxRouteLegs = 3, maxWorkItems = 25, concurrency = 4 } = {}) {
  if (!Number.isInteger(maxWorkItems) || maxWorkItems < 1) throw new Error('maxWorkItems must be a positive integer');
  if (!Number.isInteger(concurrency) || concurrency < 1 || concurrency > 8) throw new Error('concurrency must be an integer between 1 and 8');
  const allItems = buildCycleWorkItems({ maxRouteLegs });
  const selected = allItems.slice(0, maxWorkItems);
  const token0ByPool = await loadTokenOrderings();
  const ledgerFees = await loadLedgerFees();
  const completed = await mapWithConcurrency(selected, concurrency, item => scanWorkItem(item, token0ByPool, ledgerFees));
  const opportunities = completed.flatMap(item => item.result ? [item.result] : [])
    .sort((a, b) => {
      if (a.netProfitBps !== b.netProfitBps) return b.netProfitBps - a.netProfitBps;
      if (BigInt(a.netProfitNative) !== BigInt(b.netProfitNative)) return BigInt(b.netProfitNative) > BigInt(a.netProfitNative) ? 1 : -1;
      return a.routeId.localeCompare(b.routeId);
    });
  return {
    scanner: 'local-public-query-only',
    note: 'Results are indicative only. This tool models current published ledger fees but has no trade, allowance, transfer, identity, or arb-canister call path.',
    totalCycleWorkItems: allItems.length,
    scannedWorkItems: selected.length,
    failedWorkItems: completed.filter(item => item.error).length,
    opportunities,
    failures: completed.flatMap((item, index) => item.error ? [{ workItem: `${selected[index].id}#s${selected[index].sizeIndex}`, error: item.error }] : []),
  };
}
