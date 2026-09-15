# Local manual route scanner

Run pool discovery from your machine instead of asking the arb canister to
perform a route observation:

```sh
node scripts/manual-route-scan.mjs
```

The default run is deliberately small: 25 work items, three legs, and four
concurrent local requests. To select a different bounded sample:

```sh
node scripts/manual-route-scan.mjs --max-work-items 100 --concurrency 4
```

Use a complete current route universe only when you intentionally want the
larger amount of public RPC traffic:

```sh
node scripts/manual-route-scan.mjs --full --concurrency 4
```

The scanner calls only these public query methods against the code-pinned Rumi
and ICPSwap pools and the six code-pinned ledgers:

- `metadata` to verify ICPSwap token ordering
- `calc_swap` for the Rumi 3pool quote
- `quoteForAll` for the ICPSwap full-fill quote
- `icrc1_fee` to model the two ledger fees per route leg

It never calls the arb canister, uses no identity, and contains no allowance,
transfer, approval, swap, or execution method. Its JSON report is an
indicative, fee-modeled candidate ranking, not a trading instruction. Stable
paths that change stablecoin are ranked using the same par assumption the
route policy uses; this is marked with `parAssumption: true` in the report.
