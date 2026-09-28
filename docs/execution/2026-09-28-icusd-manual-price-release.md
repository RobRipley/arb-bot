# Configurable icUSD price release and activation record

Status: draft in progress. This record covers the user-authorized configurable icUSD valuation, initial value, combined reviewed release, deployment, and activation. At the timestamped baseline below, the canister remained on the already-approved $1.00 profile; later state is not asserted until fresh readback.

## Authorized target

On 2026-09-28, the user explicitly authorized implementing, merging, deploying, configuring, and resuming the reviewed release with an adjustable icUSD price set initially to `970000` USD6 (`$0.97`). Preserve backward-compatible behavior for existing state and configurations that omit the new setting: icUSD remains valued at `$1.00` (1,000,000 USD6) until an operator explicitly changes it.

The operator-selected price changes icUSD valuation and the resulting USD-denominated route economics only. The approved execution scope remains the same four two-leg routes:

- icUSD → ICP → ckUSDC; ckUSDC → ICP → icUSD
- icUSD → ICP → ckUSDT; ckUSDT → ICP → icUSD

Keep the 600-second persisted observation cadence, `$5`/`$20` stable size ladder, `$40` maximum principal, two-leg maximum, 20 pool-quote-call cap, concurrency two, zero stable profit thresholds, fee/full-fill/slippage/settlement/reservation/held-position safeguards, and no automatic residual drain or held-ICP sale. Legacy volume trading remains paused with all volume pools disabled.

The current implementation adds `RouteArbConfigV1.icusd_price_usd6` (Candid field `icusd_price_usd6`) and the administrative method `set_icusd_price_usd6_v1(u64)`; the config readback is `get_route_arb_config_v1`. The target argument is `970000`. A missing legacy value resolves to `1000000` USD6, and wrapped dollar ledgers remain valued at `$1.00`. The setter validates a positive price that can produce nonzero native amounts across the configured stable-size ladder, rejects a price change while a route execution is active, increments the policy generation, and invalidates the old observation. The dashboard exposes this under **Ops → icUSD peg trades profile → icUSD value → Save icUSD value**. It reads fresh config/current state before saving, shows an active-execution error from the server, resets its local observation cursor after a successful save, and distinguishes the configured accounting value from wrapped-ledger parity and actual pool quotes. These source details remain subject to final review and exact-tree validation below.

## Current live baseline before the price-setting release

Read-only verification at `2026-09-28T09:58:29Z`, using the current generated `src/arb_bot/arb_bot.did`, observed:

- Canister `ucjxv-nqaaa-aaaaj-qrsaq-cai` was Running on the already-reviewed module SHA-256 `03ee4baad1a194cdb91ffccac26cbb2689940fda4d684dcac415085833d8a99d`.
- Runtime was authorized and enabled with dry-run off, no last error, and no scheduler callback in flight.
- The four-route profile was active at the existing `$1.00` assumption; cadence was 600 seconds and the status endpoint reported the next eligible observation at `2026-09-28T10:00:52.518Z`.
- Current execution and mutation lock were null; held positions and route reservations were empty; no observation was in progress.
- Stable ladder, principal and quote/concurrency bounds matched the approved profile. Only the three required ICPSwap pools were enabled; the Rumi 3pool and all other configured route pools were disabled.
- Legacy volume was paused and its three volume pools were disabled.

These values are a timestamped baseline, not a substitute for fresh pre-deployment and post-deployment reads. In particular, do not reuse the empty-current/empty-lock result as evidence at the later deployment time.

## Safe sequencing constraint

The scheduler services an existing durable execution before applying the live-authorization and enabled/dry-run gates. Execution advancement checks both its captured route-config generation and authorization between legs. Therefore, do not change price/config generation or disable live authorization while a route is active or holds the mutation lock: a partially settled route can become held if its generation or authorization changes before the next leg.

For deployment, use the quiet interval after a normal execution reaches a terminal state and consumes its observation, when fresh reads show no current execution, no mutation lock, no observation, and no scheduler callback in flight, and the persisted cadence places the next observation in the future. In that interval disable live authorization, then re-read authorization/current/lock/observation/callback state. If an execution or lock appears, leave configuration unchanged and let the scheduler finish it before trying another quiet interval. Keep authorization off through upgrade and the `970000` USD6 setting readback. Do not issue a manual scan, prepare, advance, retry, drain, or volume action. After the candidate module and exact setting are verified alongside the unchanged route/fee/cadence bounds, resume using the separate authorization control. The sole operational writer will record each actual operation and its readback below.

## Implementation and release evidence

The price-setting source contract above is present in the draft branch. The user also requested that the separate dashboard-only ConnectSwap quote-input change tracked by PR #52 ship in the same release. PR #52 was merged to main at `1fb13d2c5d412dcca8c376e44849c40d3524f712` (reported main head `a4024f99607d255ab78e51ab507c829b34481876`). The price branch is being integrated with that main revision so one combined artifact can be validated. The price feature remains unmerged, and neither change is deployed. Combined-tree UI/compatibility checks, source review, build, and final hash must be recorded before deployment.

Before that integration, `CARGO_TARGET_DIR=/tmp/arb-bot-red-target cargo test -p arb_bot --no-fail-fast` exited 0; all test groups passed, including storage (7), stage1 retirement (3), state decode (12), strategy math (31), library (50), Candid equality (3), and Rust UI (37). The `print_generated_candid` diagnostic was ignored; it is not a test failure. The dashboard command `for file in scripts/test-dashboard-*.cjs; do node "$file" || exit $?; done` exited 0 across 12 suites, and `git diff --check` exited 0. Independent price reviewers A1 and B1 passed production-source review and confirmed the corrected regression for older clients omitting the new fields. The lead reports the stage1 disposition inventory now classifies all 111 methods, including the price setter as accounting configuration, with no route-execution or wrapped-stable mutation capability. Combined-tree UI/compatibility reruns, final Candid/runtime checks, final Wasm hash/build, merge commit, and deployment/readback evidence remain pending. This draft does not claim that either change has been deployed or that the accounting value is a market quote or proof that icUSD trades at `$0.97`.
