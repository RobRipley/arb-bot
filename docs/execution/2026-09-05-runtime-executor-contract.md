# Runtime route executor: implemented source contract and proof boundary

Status: source implementation present on `codex/route-runtime-executor`; source tests and acceptance checks are the applicable proof. This supersedes the earlier Stage 2–4 description that treated the executor as inert stubs: the user explicitly expanded this source task to implement the executor. The source now contains activation capability, while defaults remain off and this document does not authorize deployment, live configuration, approvals, transfers, swaps, funding, or trading.

Date: 2026-09-05 (America/Los_Angeles)

## What the branch implements

The arb-bot now contains a durable, typed route runtime in `src/arb_bot/src/route_runtime.rs`, venue adapters in `route_rumi.rs` and `route_icpswap.rs`, and a bounded scheduler in `route_scheduler.rs`. The public prepare, advance, and reconcile methods are admin-gated and delegate to this runtime. The runtime persists the complete request and adapter intent before submission, records `LegSubmitted` before the non-idempotent call, and resumes submitted work through reconciliation only. It never replays a submitted update from a lost response.

Preparation re-quotes the selected whole route while holding the durable account-mutation lock. Each later leg is re-quoted from the original principal basis and route-wide final-profit floor. Policy-generation changes, stale quotes, changed route identity, insufficient allowance/full-fill proof, invalid pins, capacity exhaustion, and lock ownership changes fail closed. A route has one active execution at a time; canonical-cycle collisions and held inventory block reuse.

Settlements require typed, source-bound evidence. The runtime does not advance on a bare venue success, an amount-only reply, or a coincident wallet credit. Partial fills, authenticated refunds, deteriorated remaining routes, and other fully reconciled but non-completing outcomes become held inventory with reservations. Held lots are never automatically sold or released. Terminal finalization is checkpointed so a storage error can be retried without another submission. Reconciliation query budgets and evidence/storage bounds remain enforced.

The scheduler is bounded and serialized. It services an existing execution before starting new observation, scans the complete observation universe before selection, and stays idle while live authorization is absent, policy execution is disabled, or dry-run is true.

## Authorization and live-proof boundary

The source defaults remain `enabled = false`, `dry_run = true`, and `live_authorized = false`. The runtime reports `compiled_support` separately from `live_authorized`; compiled support is source capability only. An admin-gated authorization setter exists in the source API, but no authorization, deployment, configuration change, approval, funding, swap, or trade was performed for this delivery. Passing source tests therefore does not establish a deployed canister, an enabled timer, a live configuration, or executable orders.

The Rumi adapter requires the pinned pool to expose the versioned receipt capability and to allowlist the bot as a receipt client before preparation. The current Rumi receipt-client allowlist is empty from the bot's perspective, so Rumi routes require separate operator enablement and deployed capability verification. The adapter persists a caller-scoped intent and accepts only an exact receipt binding of request identity, source/output/refund ledgers, accounts, amounts, fees, memo, and confirmed block status. A missing, unavailable, conflicting, or incomplete receipt remains unresolved.

The ICPSwap adapter uses the pinned `depositFromAndSwap` call and captures a bounded pre-submission history cutoff. It binds a single post-cutoff receipt using pool identity, caller-owned accounts, direction, amount, pool memo/index, output/refund records, and ledger evidence. The adapter treats unavailable, missing, duplicate, capped, evicted, malformed, partial, and refund-incomplete receipt data as pending rather than settled. Because the pinned ICPSwap receipt/history availability is not live-verified here, an unavailable receipt leaves the durable reconciliation fence and account lock in place.

An unresolved fence has no automatic recovery or blind retry path. `LegSubmitted`, `AwaitingSettlement`, and `ReconciliationRequired` resume by read-only reconciliation of the persisted intent. The lock remains held until exact settlement, a fully reconciled held position, or a definitive pre-debit rejection is persisted. Any future recovery or receipt-capability rollout is separate operator work; this source document does not make it a required gate beyond the existing source-bound settlement invariant.

## Source evidence and tests

The implementation is covered by focused Rust tests for:

- runtime persistence before submission, lost responses without replay, restart/upgrade behavior, stale quotes, changed policy generation, pre-debit rejection, partial fills, refunds, route-wide floors, held-lot reservation, terminal checkpoint retry, and default authorization/capacity behavior (`route_runtime` and `route_execution`);
- exact Rumi receipt identity, source transfer, fee/memo/block binding, delayed or missing receipts, and refund accounting (`route_rumi`);
- ICPSwap receipt cutoff, bounded retrieval, duplicate/evicted/malformed/partial/refund cases, exact pool memo/index binding, and conservative pending outcomes (`route_icpswap`);
- scheduler ordering, complete-scan gating, and reconciliation while new trading is disabled (`route_scheduler`);
- route registry, graph, accounting, policy, storage, lock, Candid, legacy-freeze, and dashboard behavior.

The repository acceptance command is `scripts/check-route-arb-acceptance.sh`. It runs the focused suites, the `route_runtime` library tests, Candid and Stage-1 structural guards, dashboard checks, a release Wasm build, executable-code-size inspection, and `git diff --check`. The full-bot `RUSTFLAGS=-Awarnings cargo test -p arb_bot` passed with zero failures, the equivalent library runtime target `cargo test -p arb_bot --lib route_runtime` passed 11/11, and the acceptance command completed successfully. These commands prove this checkout only; they do not prove deployment or live venue receipt availability.

## Required invariant and remaining proof

The accepted six-asset design requires durable intent before submission, no replay after submission, and source-bound input, venue, output, and refund evidence before settlement. The current branch implements and unit-tests those source transitions with deterministic doubles. It does not establish the deployed Rumi receipt API or allowlist, the deployed ICPSwap receipt/history contract, a canister install, or live settlement. Those are separate deployment and operator decisions.
