# Runtime route executor: implementation contract and evidence gap

Status: proposed cross-repository design; runtime implementation incomplete.
Date: 2026-09-05 (America/Los_Angeles)

The operator requested the missing executor implementation in the arb-bot task. Dashboard fixes are independent and can ship now. This document does not authorize deployment, live configuration, approvals, transfers, or trades.

## Current proof state

At arb-bot main `25aef5ec9badd0728ceee37b3eb3b9d8a6f0f5fa`, the three public executor methods return unconditional authorization errors. The Rust state-machine helpers and five route_execution tests establish inert transition behavior, not a running executor, venue request persistence, ledger reconciliation, or upgrade-safe settlement. Task 8 in the older remaining-stages plan must not be interpreted as runtime completion.

The reported +137/+130 bps quotes were observations in the earlier task, not current executable orders. The dashboard calling retired `get_bot_health` and putting eligible quotes under Attention were independent presentation defects. Pulling a local checkout cannot change the deployed embedded dashboard.

## Binding requirement

The accepted [six-asset design](../superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md), lines 437 and 460–471, requires durable intent before submission, no replay after submission, and source-bound input, venue, output and refund evidence before settlement. This is the existing required invariant. A new Rumi receipt API is the recommended implementation approach, not an independently mandated API shape or a claim that all possible correlation approaches are impossible.

## Source evidence

These are source snapshots, not deployed-interface verification.

Rumi source `73d58622e12056a19366bc61135cc927aa39def5`:

- [transfers.rs lines 44–129](https://github.com/RumiLabsXYZ/rumi-protocol-v2/blob/73d58622e12056a19366bc61135cc927aa39def5/src/rumi_3pool/src/transfers.rs#L44): input/output helpers use no memo, construct timestamps internally, discard ledger block IDs, and return only success/failure.
- [lib.rs lines 418–561](https://github.com/RumiLabsXYZ/rumi-protocol-v2/blob/73d58622e12056a19366bc61135cc927aa39def5/src/rumi_3pool/src/lib.rs#L418): swap returns an amount; successful event is appended after transfers. Refund/claim paths are not linked to a durable caller intent.
- [types.rs lines 222–248](https://github.com/RumiLabsXYZ/rumi-protocol-v2/blob/73d58622e12056a19366bc61135cc927aa39def5/src/rumi_3pool/src/types.rs#L222): event lacks request identity and ledger transaction references.

ICPSwap source `94eeb92ad6ecc2713d38fd3bef48cd4f328a3513`:

- [SwapPool.mo line 1784](https://github.com/ICPSwap-Labs/icpswap-v3-service/blob/94eeb92ad6ecc2713d38fd3bef48cd4f328a3513/src/SwapPool.mo#L1784): one-step call returns an amount and accepts no client operation identity; pool creates its own transaction index.
- [transaction/lib.mo line 183](https://github.com/ICPSwap-Labs/icpswap-v3-service/blob/94eeb92ad6ecc2713d38fd3bef48cd4f328a3513/src/components/transaction/lib.mo#L183): OneStepSwap withdrawal completion does not retain the supplied output ledger index. A zero index cannot be assumed to identify output.
- [SwapPool.mo line 1516](https://github.com/ICPSwap-Labs/icpswap-v3-service/blob/94eeb92ad6ecc2713d38fd3bef48cd4f328a3513/src/SwapPool.mo#L1516): completed transaction leaves active history for cache. Cache synchronization can remove it. Durable historical availability must be established.
- Pool-generated eight-byte big-endian transaction-index memo can bind transfers, but receipt memo fields themselves may be absent. Refunds require their related-index linkage and independent ledger validation.

No validated complete Rumi correlation predicate or deployed ICPSwap receipt-retention contract has been established. Amount/time-only correlation is not accepted as a replacement. Implementing wrappers over these unresolved predicates would not complete the requested executor.

## Recommended cross-repository implementation

### 1. Rumi: additive durable swap receipts

Keep the existing swap wire contract intact. Add a versioned swap method accepting a caller-scoped 32-byte intent ID, input/output indices, exact input and minimum net output. Add a caller-scoped receipt query. Canonical request hashing binds method version, caller, intent, pool, ledgers, direction, amounts, and minimum.

Persist a receipt before the first ledger call. Each transfer intent stores source/destination accounts, ledger, amount, fee, stable memo and created_at_time before call issuance. Persist returned ledger transaction ID or explicit unresolved outcome. Record input, output and every refund under the same operation identity. Ledger memo support is verified rather than assumed.

A duplicate caller+intent with different request bytes rejects. A duplicate with the same bytes returns the existing receipt and never initiates a second economic operation. An ambiguous submitted transfer is reconciled from the persisted intent using exact ledger evidence; it is never blindly reconstructed or retried. Terminal receipt retention is bounded but must not silently evict receipts required by clients. Capacity is admitted before debit; exhaustion rejects before mutation. Pending receipts survive upgrade and remain queryable.

Do not change pool pricing, fee economics or existing swap callers. The bot will require the new receipt capability for routes containing Rumi. A separate future deployment of that capability is needed for actual live use; source tests cannot prove deployed availability.

### 2. ICPSwap: verified receipt and ledger adapter

Verify the pinned pools' deployed Candid and a durable history source using read-only calls. Establish bounded/resumable retrieval for active, cached and archived receipts. Capture pre-submission history identity and bind exactly one new caller-owned one-step receipt to the persisted request. Validate source input block, pool direction/effective input, output transfer using pool-generated memo, and every linked refund. Fetch records directly; never accept caller-supplied proof booleans. Missing, multiple or evicted matches remain unresolved with lock held. If durable retrieval cannot be established, report that specific adapter unsupported rather than enabling it.

### 3. Arb-bot: durable runtime orchestration

Replace unconditional endpoint stubs only once concrete adapters exist. Persist complete typed requests and operation state, not boolean proof assertions. Acquire the global account lock and reserve storage capacity before preparing a submission. Re-quote the whole selected route after lock acquisition, validate pins/metadata/fees/full-fill/allowance/unencumbered balances/inventory bands/profit, and reject stale or changed policy generation.

Persist LegSubmitted before the non-idempotent update. Every resume from submission reconciles only. Re-quote the remaining route after exact settlement, preserving original principal basis and route-wide minimum profit. Convert deteriorated or partial fills to fully reconciled held lots, reserve all lots before releasing the lock, and never auto-sell them. Unresolved evidence retains ReconciliationRequired and the lock. Completion uses verified net ledger movement for P&L.

Use bounded per-poll evidence budgets, persisted cursors and idempotent callback handling. Invalidate quote selection after mutation and enforce one active route plus canonical-cycle exclusion. Scheduler selection respects the existing stable/ICP book policy. Ship with durable live authorization false and no activation in this task; report compiled support and authorization separately.

## Acceptance before source completion

- Real orchestrator tests with deterministic ledger/pool doubles, asserting outbound call counts and persisted state across each await, timeout, callback and restart.
- Source-bound success, coincident external credit rejection, changed/missing receipt, wrong accounts/memo/ledger/block, partial fill, refund, lost response, duplicate request/callback, stale quote and changed-config cases.
- Storage capacity and held-lot reservation admitted before submission; no unreserved held funds or lock release on unknown outcome.
- Actual stable-memory serialization/upgrade round trips, Candid compatibility for both repositories, no legacy executor revival, retained volume-account serialization.
- Whole-route economics and backward floors with native fee rounding; full suite and bounded independent review.
- No deployment, approvals, funding, swaps, or live configuration during implementation.

## Decision requested

Approve extending source work into Rumi Protocol to add the receipt contract above, alongside ICPSwap evidence verification and the arb-bot runtime implementation. This is a concrete cross-repository design decision; the dashboard patch does not depend on it. Deployment and live trading remain separate decisions.
