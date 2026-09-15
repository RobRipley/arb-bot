const assert = require('node:assert/strict');

(async () => {
  const {
    buildQueryCommand,
    applyLedgerFees,
    decodeNatResponse,
    rankCompletedRoute,
  } = await import('./local-route-scanner.mjs');

  // Break caught: replacing the local public query with an update, adding an
  // identity, or targeting the arb canister would make the scanner capable of
  // spending canister cycles or moving funds.
  const command = buildQueryCommand({
    pool: 'mohjv-bqaaa-aaaag-qjyia-cai',
    method: 'quoteForAll',
    args: '(record { amountIn = "100000000"; zeroForOne = true; amountOutMinimum = "0" })',
  });
  assert.deepEqual(command, {
    executable: 'icp',
    args: [
      'canister', 'call', 'mohjv-bqaaa-aaaag-qjyia-cai', 'quoteForAll',
      '(record { amountIn = "100000000"; zeroForOne = true; amountOutMinimum = "0" })',
      '--query', '--network', 'ic', '--output', 'candid',
    ],
  });
  assert(!command.args.includes('--identity'));
  assert(!command.args.includes('ucjxv-nqaaa-aaaaj-qrsaq-cai'));

  // Break caught: rejecting the standard Candid thousands separators emitted
  // by the ICP CLI and silently turning a real quote into a failed route.
  assert.equal(decodeNatResponse('(variant { ok = 30_911_070 : nat })'), 30_911_070n);

  // Break caught: overstating a local opportunity by feeding gross output
  // into the next leg instead of deducting both modeled ledger movements.
  assert.equal(applyLedgerFees({ walletBefore: 1_000_000n, sourceFee: 10_000n, grossOutput: 1_020_000n, destinationFee: 10_000n }), 1_010_000n);
  assert.throws(() => applyLedgerFees({ walletBefore: 10_000n, sourceFee: 10_000n, grossOutput: 1n, destinationFee: 0n }), /source ledger fee/);

  // Break caught: treating an incomplete or money-losing cycle as a winner.
  const ranked = rankCompletedRoute({
    routeId: 'icpswap-icp-ckusdc:Icp>CkUsdc|icpswap-icp-ckusdc:CkUsdc>Icp',
    startAsset: 'Icp',
    endAsset: 'Icp',
    principalNative: 100_000_000n,
    expectedLegs: 2,
    outputs: [125_000_000n, 101_500_000n],
  });
  assert.deepEqual(ranked, {
    routeId: 'icpswap-icp-ckusdc:Icp>CkUsdc|icpswap-icp-ckusdc:CkUsdc>Icp',
    startAsset: 'Icp',
    endAsset: 'Icp',
    principalNative: '100000000',
    finalNative: '101500000',
    netProfitNative: '1500000',
    netProfitBps: 150,
    parAssumption: false,
    complete: true,
  });
  assert.equal(rankCompletedRoute({
    routeId: 'partial', startAsset: 'Icp', principalNative: 100n, expectedLegs: 2, outputs: [125n],
  }), null);
  assert.equal(rankCompletedRoute({
    routeId: 'loss', startAsset: 'Icp', principalNative: 100n, expectedLegs: 1, outputs: [99n],
  }), null);

  // Break caught: comparing 8-decimal icUSD directly with 6-decimal ckUSDC
  // and hiding an otherwise profitable stable-settled path.
  const stableSettled = rankCompletedRoute({
    routeId: 'stable-cross', startAsset: 'IcUsd', endAsset: 'CkUsdc', principalNative: 100_000_000n,
    expectedLegs: 1, outputs: [1_010_000n],
  });
  assert.equal(stableSettled.netProfitNative, '10000');
  assert.equal(stableSettled.netProfitBps, 100);
  assert.equal(stableSettled.parAssumption, true);
})();
