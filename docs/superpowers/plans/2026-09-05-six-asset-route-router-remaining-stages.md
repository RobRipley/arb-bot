# Six-Asset Route Router Remaining Stages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Every production change follows test-driven development. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the retired A-S/T opportunity model with one six-asset, principal-pinned route-arbitrage engine; add bounded quote observations, durable execution/reconciliation state, and a consolidated dashboard; leave all live execution inert pending a separately authorized Stage-5 trial.

**Architecture:** A new `route_arb` module owns immutable asset/pool registries, graph generation, native-unit accounting, route ranking, quote observations, and execution state transitions. Stable storage uses new non-overlapping memory IDs for bounded observations, terminal executions, held positions, and ownership reservations, while a small versioned control record remains in `BotState`. Public APIs use additive versioned types. Existing A-S/T records and Candid methods retain their historical wire meaning and remain fail-closed. The volume bot and surviving withdrawal/recovery calls participate in the new durable mutation lock without changing their economic routing.

**Tech Stack:** Rust IC canister (`ic-cdk` 0.13), Candid, `ic-stable-structures` 0.6, embedded HTML/JavaScript dashboard, shell acceptance guards.

**Source of truth:** `docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md` at the branch base. This plan implements Stages 2-4 and the source/configuration controls needed for Stage 5. It does not authorize Stage-5 activation or any live operation.

## Global constraints

- No deployment, mainnet configuration, approvals, transfers, swaps, funding, or live activation.
- New route execution defaults to disabled and dry-run. No source path may make a fund-moving call unless both the durable executor and a separate live-authorization flag permit it; this branch never flips that flag.
- The six ledgers and fifteen pools are code-pinned. Configuration can enable a pin but cannot replace its identity.
- Every ICPSwap eligibility quote uses `quoteForAll`; every Rumi quote uses `calc_swap`; no partial observation can produce an executable winner.
- Stable accounting values icUSD, ckUSDT, and ckUSDC at $1 only at the principal/terminal/P&L boundary. Intermediate amounts always use live pool quotes and native fees.
- Preserve deployed, pre-router, and current stable-state decode; preserve legacy Candid signatures and meanings.
- New returned lists are cursor-bounded to 100. Route length is at most four, size ladders at most 16, concurrent quote calls at most 16, reconciliation queries at most 32, held/non-route open records at most 256, evidence items at most 64, encoded execution records at most 65,536 bytes, and terminal history at most 10,000.
- Do not restore automatic drains. Held stable, ICP, ckBTC, or ckETH lots remain reserved and untouched until a separately designed explicit continuation/release.
- A route or non-route mutation owns one durable global account-mutation lock through source-bound reconciliation. A legacy cycle-lock clear cannot release it.
- Update Rust exports, `arb_bot.did`, and the dashboard IDL together; `scripts/check-candid.sh` must pass after every API task.
- Each task commits only after its scoped tests pass. Final push/PR/merge is allowed when all gates are green; deployment is not.

---

### Task 1: Immutable six-asset and fifteen-pool registry

**Files:**
- Create: `src/arb_bot/src/route_arb.rs`
- Create: `src/arb_bot/tests/route_registry.rs`
- Modify: `src/arb_bot/src/lib.rs`

**Interfaces:** `Asset`, `AssetRole`, `VenueKind`, `PoolPin`, `DirectedEdge`, `asset_pins()`, `pool_pins()`, `directed_edges()`.

- [x] Write failing golden tests for all six ledger principals, symbols, decimals, roles, all fifteen pool principals/pairs, both directions, unique edge IDs, and exclusion of BOB/3USD/PartyDEX/Rumi-AMM.
- [x] Run `cargo test -p arb_bot --test route_registry` and observe unresolved-interface failure.
- [x] Implement the smallest immutable registry and deterministic directed-edge construction.
- [x] Re-run the scoped test and commit `feat(arb): add immutable six-asset route registry`.

### Task 2: Bounded graph generation and canonical identity

**Files:**
- Modify: `src/arb_bot/src/route_arb.rs`
- Create: `src/arb_bot/tests/route_graph.rs`

**Interfaces:** `CandidateClass`, `Route`, `enumerate_routes(max_legs)`, `route_id()`, `canonical_cycle_id()`.

- [x] Write failing exhaustive golden tests for the exact ordered route set and count at one through four legs, permitted endpoint classes, reachability, reverse distinction, rotation deduplication, and rejection of repeated vertices/pools/edges, embedded cycles, consecutive same-pool reversals, and a second Rumi use.
- [x] Run the scoped test and observe failure.
- [x] Implement deterministic DFS/backtracking and canonicalization without symbol-derived identity.
- [x] Re-run tests and commit `feat(arb): enumerate bounded six-asset routes`.

### Task 3: Native-unit accounting, eligibility, inventory, and ranking

**Files:**
- Modify: `src/arb_bot/src/route_arb.rs`
- Create: `src/arb_bot/tests/route_accounting.rs`

**Interfaces:** `par_usd_6dec_checked`, `net_profit_bps_checked`, `QuoteLeg`, `RouteQuote`, `CandidateEvaluation`, `ReservationTotals`, `available_native`, `evaluate_candidate`, `rank_book`.

- [x] Write failing boundary tests for decimals, checked overflow/underflow, signed bps truncation, zero principal, all ledger-fee recurrence points, stable/ICP profit domains, dual thresholds, per-asset floors/ceilings, unknown balances/allowances, held/active/non-route reservations, changed stable terminal, volatile exposure, book separation, and deterministic tie-breaks.
- [x] Run the scoped test and observe failure.
- [x] Implement checked native arithmetic and pure eligibility/ranking.
- [x] Re-run tests and commit `feat(arb): add route accounting and ranking invariants`.

### Task 4: Versioned inert policy and six-asset wallet reporting

**Files:**
- Modify: `src/arb_bot/src/route_arb.rs`
- Modify: `src/arb_bot/src/state.rs`
- Modify: `src/arb_bot/src/lib.rs`
- Modify: `src/arb_bot/arb_bot.did`
- Modify: `src/arb_bot/src/dashboard.html` (IDL only)
- Modify: `src/arb_bot/tests/state_decode.rs`
- Create: `src/arb_bot/tests/route_policy.rs`

**Interfaces:** `RouteArbConfigV1`, `RouteArbStatusV1`, `WalletAssetBalanceV1`, `get_route_arb_config_v1`, `set_route_arb_config_v1`, `get_route_arb_status_v1`, `get_route_wallet_balances_v1`.

- [x] Write failing tests for inert defaults, immutable ceilings, invalid encoded config disabling mutation, old-state decode, pin substitution impossibility, checked clock regression, and all six wallet rows including zero ckBTC/ckETH.
- [x] Implement serde-defaulted control state and admin setter validation. Wallet reporting performs only balance/metadata queries against code-pinned ledgers and marks failures explicitly.
- [x] Run scoped tests plus `scripts/check-candid.sh`; commit `feat(arb): add inert route policy and wallet reporting`.

### Task 5: Cursor-bounded quote observations

**Files:**
- Modify: `src/arb_bot/src/route_arb.rs`
- Modify: `src/arb_bot/src/state.rs`
- Modify: `src/arb_bot/src/lib.rs`
- Modify: `src/arb_bot/arb_bot.did`
- Modify: `src/arb_bot/src/dashboard.html` (IDL only)
- Create: `src/arb_bot/tests/route_observation.rs`

**Interfaces:** `start_route_observation_v1`, `quote_route_observation_batch_v1(cursor, limit)`, `get_route_observations_v1(cursor, limit)`, `get_best_route_candidates_v1`, `ObservationV1`, `ObservationMetricsV1`.

- [x] Write failing adapter tests proving ICPSwap uses full-fill semantics, Rumi uses `calc_swap`, downstream input is net of both relevant ledger movements, prefix reuse only on exact edge/native-input identity, partial/error quotes reject, and no fund-moving method is reachable.
- [x] Write failing boundedness tests for deterministic cursors, maximum page/call/concurrency/age limits, complete-universe winner gating, route/size/call counts, quote latency/cycle metrics, rotation collisions, and incomplete observations.
- [x] Implement query-only adapters and a resumable observation builder. Persist only bounded completed summaries in new stable memory IDs.
- [x] Run scoped tests, Candid guard, and Stage-1 zero-call guards; commit `feat(arb): add bounded route quote observations`.

### Task 6: Durable reservations, held positions, history, and global lock

**Files:**
- Modify: `src/arb_bot/src/route_arb.rs`
- Modify: `src/arb_bot/src/state.rs`
- Modify: `src/arb_bot/src/lib.rs`
- Modify: `src/arb_bot/arb_bot.did`
- Modify: `src/arb_bot/src/dashboard.html` (IDL only)
- Create: `src/arb_bot/tests/route_storage.rs`

**Interfaces:** `MutationOwnerV1`, `MutationLockV1`, `OwnershipReservationV1`, `HeldPositionV1`, `ExecutionRecordV1`, paginated query APIs, internal acquire/release/reserve helpers.

- [x] Write failing tests for new non-overlapping stable memory IDs, record/evidence/text/page/open-count caps, capacity reservation before mutation, checked spendable balances, per-asset freezes, owner/provenance attribution, restart/upgrade round trips, and inability of `clear_cycle_lock` to alter the durable lock.
- [x] Implement bounded stable maps/cells/logs and migration of legacy pending exits to exact reservations when provable or durable whole-asset freezes otherwise. Never initiate a drain.
- [x] Run scoped state/upgrade/Candid tests and commit `feat(arb): add durable route ownership state`.

### Task 7: Lock every surviving shared-account mutation and pin call targets

**Files:**
- Modify: `src/arb_bot/src/lib.rs`
- Modify: `src/arb_bot/src/volume.rs`
- Modify: `src/arb_bot/src/state.rs`
- Modify: `scripts/check-stage1-disposition.sh`
- Create: `src/arb_bot/tests/account_mutation_lock.rs`
- Create: `scripts/check-route-call-targets.sh`

**Interfaces:** internal mutation guard used by volume manual/timer operations, generic withdrawal, and PartyDEX recovery; immutable volume registry validation.

- [x] Write failing call-counting tests for every overlapping public/timer path, all corrupt persisted execution/reference venue pins and orderings, arbitrary withdraw/recovery principals, reservation underflow, ambiguous non-route settlement, and legacy-lock clearing.
- [x] Implement fail-before-call target validation, lock acquisition, non-route capacity reservation, source-bound release rules, and timer deferral. Preserve volume pool selection/sizing/recovery economics.
- [x] Add structural scripts enforcing call-target constants and complete mutator classification.
- [x] Run all scoped and Stage-1 tests; commit `feat(arb): serialize route-relevant account mutation`.

### Task 8: Durable route executor and adapter reconciliation

**Files:**
- Modify: `src/arb_bot/src/route_arb.rs`
- Modify: `src/arb_bot/src/swaps.rs`
- Modify: `src/arb_bot/src/lib.rs`
- Modify: `src/arb_bot/arb_bot.did`
- Modify: `src/arb_bot/src/dashboard.html` (IDL only)
- Create: `src/arb_bot/tests/route_execution.rs`

**Interfaces:** `prepare_route_execution_v1`, `advance_route_execution_v1`, `reconcile_route_execution_v1`; persisted phases `Planned` through `HeldInventory`. Preparation remains inert unless the separate live authorization field is true; defaults and migration keep it false.

- [ ] Write failing deterministic state-machine tests covering intent persisted before outbound call, immutable request fingerprint, no replay from `LegSubmitted`, exact full-fill/partial/refund conservation, coincident-credit rejection, delayed/out-of-order/lost responses, timeout to `ReconciliationRequired`, 32-query budget, trap/restart/upgrade at each phase, duplicate callback idempotency, profit-preserving backward floors, deterioration to held inventory, full-refund abort, realized ledger-delta P&L, re-quote after lock wait and every leg, and canonical collision exclusion.
- [ ] Implement venue-specific request builders and source-bound reconciliation predicates. The executor never advances on a bare DEX success or balance delta and never automatically sells a held lot.
- [ ] Keep live authorization false in every constructor/migration and expose no method that can flip it in this release; execution APIs therefore exercise validation/state preparation only and return a clear authorization error before a fund-moving call.
- [ ] Run scoped tests, Candid guard, and structural no-drain/retirement checks; commit `feat(arb): add inert durable route executor`.

### Task 9: Consolidated route-arbitrage dashboard

**Files:**
- Modify: `src/arb_bot/src/dashboard.html`
- Create: `src/arb_bot/tests/dashboard_route_ui.rs`

**Interfaces:** active route overview, stable/ICP candidate cards, observation metrics, six-asset balances/reservations, execution/held-position views, route policy controls that cannot live-authorize.

- [ ] Write failing static/behavioral checks proving active A-S/T cards and controls are absent, legacy history remains labeled, all six wallet assets render even at zero, actual directed asset/venue paths render, stable-par disclosure is visible, rejection/full-fill/allowance/inventory/quote-age/collision fields render, and execution/settlement/held states render.
- [ ] Replace the active opportunity UI and obsolete strategy levers with the consolidated router surfaces while leaving the separately scoped volume UI intact.
- [ ] Run dashboard test and `scripts/check-candid.sh`; commit `feat(arb): consolidate dashboard around route arbitrage`.

### Task 10: Whole-system acceptance and source delivery

**Files:**
- Modify: `README.md`
- Create: `scripts/check-route-arb-acceptance.sh`
- Modify: this plan (check completed boxes)

- [ ] Add one acceptance script that runs registry/graph/accounting/storage/execution/UI tests, Candid equality, Stage-1 disposition/zero-call/no-drain guards, call-target guard, release Wasm build, encoded interface checks, and a Wasm code-section/total-size report without changing the project limit.
- [ ] Run `RUSTFLAGS=-Awarnings cargo test -p arb_bot`, `scripts/check-route-arb-acceptance.sh`, and `git diff --check`; record exact results.
- [ ] Self-review the complete branch against every Section 12 acceptance bullet. Fix only correctness defects within this approved scope; cap independent review at one whole-branch pass plus one focused fix verification.
- [ ] Confirm no deployment/configuration/approval/transfer/swap occurred and that all new execution remains inert.
- [ ] Commit docs/checks, push branch, create PR, wait for required checks, and merge when green. Do not deploy.
