# Four-Asset Route Arbitrage Policy Design

**Status:** Proposed for operator review

**Date:** 2026-09-04

**Scope:** `arb_bot` arbitrage routing, accounting, execution, reporting, and legacy-route retirement

**Implementation authority:** None. This document authorizes no deployment, configuration change, approval, transfer, or trade.

## 1. Decision

Replace the lettered A-R/S/T strategy model with one economic strategy:

> **Four-Asset Route Arbitrage** discovers and executes profitable paths among ICP, icUSD, ckUSDT, and ckUSDC using an allowlisted graph of Rumi 3pool and ICPSwap edges.

The engine exposes three candidate classes because they have different economic explanations and profit units:

1. **Stable-par path** — begins and ends in the stable domain without traversing ICP.
2. **Stable-funded ICP path** — begins in a stable, traverses ICP, and ends in a stable.
3. **ICP-returning cycle** — begins and ends in ICP and may traverse one or more stables.

These are reporting and funding classes over one planner, not independent engines. A venue-specific sequence is a **route**, never a strategy letter.

The operator policy values icUSD, ckUSDT, and ckUSDC at exactly $1. This is an intentional hard-coded accounting assumption. The engine does not claim that the tokens are economically interchangeable outside this policy.

## 2. Active and Retired Scope

### 2.1 Active assets

- ICP
- icUSD
- ckUSDT
- ckUSDC

No other asset may enter a candidate.

### 2.2 Active venues and edges

The route registry is an allowlist. Only the following integrations may register edges:

- **Rumi 3pool:** both directed stable-to-stable edges for each pair among icUSD, ckUSDT, and ckUSDC.
- **ICPSwap ICP pools:** both directed edges for ICP/icUSD, ICP/ckUSDT, and ICP/ckUSDC.
- **ICPSwap direct-stable pools:** both directed edges for icUSD/ckUSDT, icUSD/ckUSDC, and ckUSDT/ckUSDC.

Each directed edge has a stable identity independent of token symbols:

```text
edge_id
venue
pool_principal
token_in
token_out
token_ordering
fee_model
quote_adapter
execution_adapter
settlement_adapter
```

Pool principals and token ordering remain configured and runtime-verified. Candidate identity uses `edge_id`; parallel edges connecting the same assets remain distinguishable. Administration may update principals for these named pool slots, but it cannot admit another asset or venue type without a reviewed code and schema change.

### 2.3 Retired integrations

The following are absent from the active route registry:

- Rumi AMM / 3USD-ICP arbitrage routes.
- Every PartyDEX route.
- BOB arbitrage Strategy S and all BOB pools as arbitrage edges.

Retirement is stronger than winner-selection exclusion. A retired integration receives zero automatic:

- metadata or token-ordering calls;
- quotes or health polling;
- allowance creation or refresh;
- scheduled or manual strategy execution;
- route ranking;
- dashboard actions that could initiate trading.

Legacy letter execution methods remain temporarily for Candid compatibility. Methods backed by a retired integration fail closed immediately. At the route-engine execution cutover, every remaining letter method also becomes a fail-closed stub before any inter-canister call and records a `retired_route` activity event. No letter method may bypass either integration retirement or the new engine.

Existing historical records and stable-state fields are preserved with their original meaning. They are not rewritten into the new taxonomy. Old configuration fields may remain decode-only during the compatibility period but are ignored by the route registry.

Explicit admin recovery capabilities for already-stranded venue funds remain isolated from routing and scheduling. Recovery never registers an edge or re-enables a retired integration.

Existing on-ledger allowances are not revoked by this design. Allowance revocation is a separate live operation requiring explicit authorization and an exact inventory of targets.

This design retires BOB **arbitrage**. It does not silently alter the separately configured volume bot, including any icUSD/BOB volume setting.

## 3. Candidate Classes and Profit Invariants

### 3.1 Stable-par path

Requirements:

- Start asset is icUSD, ckUSDT, or ckUSDC.
- End asset is icUSD, ckUSDT, or ckUSDC.
- No edge touches ICP.
- Interior assets do not repeat.
- A literal round trip may repeat only the start asset as the final asset.

Profit is measured in six-decimal stable-par dollars:

```text
net_profit_usd_6dec =
    par_usd(net_final_stable)
  - par_usd(principal)
  - all_unaccounted_start_asset_fees
```

`par_usd` values each stable at exactly $1 after converting its native decimals. Candidate eligibility requires both:

```text
net_profit_usd_6dec >= min_stable_profit_usd_6dec
net_profit_bps       >= min_stable_profit_bps
```

A path ending in a different stable is labeled `par_assumption=true` and reports its terminal asset. It is not described as a guaranteed same-token round trip. Per-stable inventory floors and ceilings must remain satisfied.

### 3.2 Stable-funded ICP path

Requirements:

- Start asset is one of the three stables.
- End asset is one of the three stables.
- At least one edge enters ICP and a later edge exits ICP.
- Interior assets do not repeat.
- A literal round trip may repeat only the start asset as the final asset.

This class contains the economic behavior previously spread across B/F-style routes: buy ICP through the cheapest eligible path and sell it through a more valuable eligible path, with stable-denominated capital and P&L.

Profit uses the same stable-par invariant and thresholds as Section 3.1. The candidate report additionally identifies the ICP buy and sell edges and their implied ICP prices, but eligibility is determined by the full chained quote rather than a spot-price spread.

### 3.3 ICP-returning cycle

Requirements:

- Start and end asset are ICP.
- The cycle contains at least one stable asset.
- Interior assets do not repeat.
- ICP appears only at the start and end.

Profit is measured in ICP e8s:

```text
net_profit_icp_e8s =
    net_final_icp_e8s
  - principal_icp_e8s
  - all_unaccounted_start_icp_fees_e8s
```

Candidate eligibility requires both:

```text
net_profit_icp_e8s >= min_icp_profit_e8s
net_profit_bps     >= min_icp_profit_bps
```

No ICP/USD price is required to establish profitability. Any USD rendering is informational and cannot change eligibility.

### 3.4 Rejected endpoints

The arbitrage engine rejects a complete candidate that begins in the stable domain and ends in ICP, or begins in ICP and ends in the stable domain. Those are inventory conversions whose profitability requires an ICP valuation policy outside this design.

Individual stable-to-ICP and ICP-to-stable edges remain valid inside eligible complete candidates.

## 4. Route Generation and Canonicalization

### 4.1 Permitted shapes

The planner generates venue-edge-specific routes of one to four swaps, subject to the class rules above.

- A non-cyclic stable-par or stable-funded path has no repeated asset.
- A cycle repeats only its start asset as its final asset.
- No candidate repeats an edge or pool.
- No candidate contains an immediate inverse swap.
- No candidate contains a smaller embedded cycle.
- Reverse directions remain distinct because they have different economics.

Repeated-vertex walks are excluded: they either contain a smaller independently evaluable cycle or add avoidable fees and settlement risk.

### 4.2 Canonical cycle identity

Every literal cycle receives a `canonical_cycle_id` calculated from the lexicographically minimal rotation of its directed `edge_id` sequence. Reversal is not treated as equivalent.

Example rotations:

```text
ICP -> ckUSDC -> icUSD -> ICP
ckUSDC -> icUSD -> ICP -> ckUSDC
icUSD -> ICP -> ckUSDC -> icUSD
```

If all three use the same directed venue edges, they share one `canonical_cycle_id`. The planner may assess each permitted funding rotation against inventory and fees, but the scheduler may select at most one rotation of that cycle during an observation/settlement window.

Non-cyclic stable-domain paths receive a directed `route_id` from their complete edge sequence and are not rotation-canonicalized.

### 4.3 Candidate record

Every quote candidate contains at least:

```text
route_id
canonical_cycle_id: optional
candidate_class
venue_edges
asset_path
start_asset
end_asset
profit_domain
principal_native
gross_and_net_amount_per_leg
net_profit_native
net_profit_bps
par_assumption
ledger_and_dex_fees
full_fill_status
allowance_status
inventory_impact
quote_timestamps
rejection_reason
```

Descriptive route IDs replace letters in new observations, trades, logs, and dashboard displays.

## 5. Quoting, Fees, and Eligibility

### 5.1 Quote adapters

- ICPSwap candidate sizing uses `quoteForAll` or an equivalent full-input guarantee. Plain `quote` is never an eligibility source near a liquidity boundary.
- Rumi 3pool uses `calc_swap` with its native output and fee semantics.
- Each downstream quote uses the preceding leg's output after every ledger movement required before the downstream venue can receive it.

The planner never assumes that fee semantics are symmetric between venues. Native token decimals and ledger fees are edge inputs, not strategy-level constants.

### 5.2 Size search

Each profit domain has its own configured size ladder and hard maximum:

- stable principal sizes in six-decimal par dollars, converted to the start token's native decimals;
- ICP principal sizes in e8s.

The quote-only planner evaluates every route-size pair within inventory and venue constraints. It does not extrapolate a small quote to a larger size.

### 5.3 Allowances and inventory

The planner reports required allowances but never creates them. A live candidate is ineligible unless every required allowance is already sufficient.

Eligibility applies per-asset floors and ceilings to:

- the starting debit, including its ledger fee;
- every settled intermediate balance;
- the final balance and terminal stable exposure.

Unknown balances or allowances fail eligibility closed.

### 5.4 Route-aware minimum output

A generic `quote * (1 - slippage)` floor is insufficient. Each leg's minimum output must leave an executable downstream path that preserves:

```text
principal + all remaining fees + configured minimum native profit
```

If no such floor can be established, the route is quote-only and cannot execute.

## 6. Ranking and Scheduling

Stable-profit and ICP-profit candidates are not compared by raw amount and do not require a common ICP/USD oracle.

The scheduler maintains two independent capital books:

- **Stable book:** shared inventory policy across the three $1 stables, with per-token floors and ceilings.
- **ICP book:** ICP principal reserve and maximum exposure per route.

Within each book, candidates rank by:

1. highest net profit in the book's native profit unit;
2. highest net profit bps;
3. fewer legs;
4. deterministic `route_id` order.

The initial live design permits one globally active route at a time. If both books have eligible candidates, the least-recently-served book wins; if only one book is eligible, it may run. After any submission, settlement, failure, or recovery transition, all quotes are invalidated and the graph is re-quoted before another selection.

The scheduler also enforces the canonical-cycle exclusion: no second funding rotation of the same cycle can run until the first reaches a terminal state and a fresh observation window begins.

## 7. Execution and Settlement Architecture

The system shares a route planner but uses venue-specific adapters. The current ICPSwap and Rumi helpers are not treated as interchangeable completion primitives.

### 7.1 Durable route state

Before each update call, persist:

```text
execution_id
route_id
canonical_cycle_id
candidate_class
principal_and_profit_domain
current_leg_index
edge_and_pool
planned_input
required_min_output
pre_call_balances
expected_fees
quote_timestamp
phase
retry_count
```

Phases are:

```text
Planned
LegPrepared
LegSubmitted
AwaitingSettlement
LegSettled
RemainingRouteRequoted
Completed
RecoveryRequired
Recovered
```

Only `Completed`, `RecoveryRequired`, and `Recovered` are terminal for scheduling purposes.

### 7.2 Settlement proof

- An ICPSwap `depositFromAndSwap` success is submission evidence, not ledger settlement.
- Completion requires attributable balance deltas and refund accounting.
- Rumi completion also uses balance evidence consistent with its transfer semantics.
- The executor re-quotes the exact remaining route after each confirmed leg settlement.
- Duplicate callbacks, timer retries, upgrades, and traps must resume from the persisted phase rather than repeat a debit.

### 7.3 Downstream edge disappearance

If the next leg no longer preserves principal and minimum profit, the executor does not dump at zero minimum output. It enters `RecoveryRequired`, records the stranded asset and amount, alerts the operator, and exposes explicit choices supported by the recovery adapter. Recovery cannot silently convert the position at a loss.

### 7.4 Realized P&L

Realized P&L uses before/after ledger deltas in the candidate's profit domain and records planned versus settled amounts per leg. Quoted profit is never recorded as realized profit.

## 8. Configuration and API Model

The new versioned route-arbitrage configuration contains:

```text
enabled
dry_run
active_pool_registry
stable_size_ladder
icp_size_ladder
max_stable_principal_usd_6dec
max_icp_principal_e8s
min_stable_profit_usd_6dec
min_stable_profit_bps
min_icp_profit_e8s
min_icp_profit_bps
per_asset_inventory_floor
per_asset_inventory_ceiling
quote_max_age_ns
settlement_timeout_ns
```

There is one master route-arbitrage enable switch plus per-profit-book enable switches. Pool registry changes and any future venue admission remain admin-only. Dry-run is the migration default.

New APIs are additive and use the new taxonomy:

- quote the complete route universe;
- inspect best candidate per profit book;
- inspect pending execution/recovery;
- configure route-arbitrage policy;
- execute a descriptive route only after live execution is separately authorized.

Legacy letter dry-run and execute methods remain wire-compatible during the transition. Retired-integration methods fail closed in Stage 1. The remaining methods are labeled legacy while the new planner observes, then become fail-closed compatibility stubs at the execution cutover. No new consumer should call them.

## 9. Reporting and Dashboard

The active dashboard shows:

- best stable-book candidate;
- best ICP-book candidate;
- candidate class and descriptive asset/venue path;
- principal, quoted net profit, bps, all fees, full-fill status, inventory effect, allowance status, and quote age;
- canonical-cycle collision/rotation status;
- current durable execution phase and settlement latency;
- explicit rejection and recovery reasons.

Rumi AMM, PartyDEX, BOB arbitrage, and lettered strategies are absent from active opportunity cards. Historical views continue to render their original labels and fields as legacy data.

Stable-par candidates display the policy disclosure:

> icUSD, ckUSDT, and ckUSDC are valued at a hard-coded $1 for route accounting. Terminal token and inventory exposure may change.

## 10. Migration Plan

Migration is additive and staged.

### Stage 1: Retirement safety

- Remove retired integrations from automatic metadata, quote, approval, scheduler, and execution paths.
- Make legacy manual K-R, S, and Rumi-AMM-backed letter methods fail closed.
- Preserve isolated admin recovery.
- Prove zero inter-canister calls for every retired path with call-counting tests.

### Stage 2: Quote-only route planner

- Add the active edge registry, candidate generation, canonicalization, accounting, ranking, and new reports.
- Keep all new execution disabled and dry-run-only.
- Keep still-permitted legacy ICPSwap execution isolated from the new planner during observation; it cannot consume a new-planner candidate or route ID.
- Preserve existing stable-state and Candid compatibility.

### Stage 3: Observation

- Collect timestamped route-size observations across materially different pool states.
- Measure quote drift, full-fill rejections, quote latency, cycle cost, candidate rotation duplication, and expected settlement exposure.
- Do not infer realized fill performance from query-only observations.

### Stage 4: Durable executor

- Add the persisted phase machine, venue adapters, settlement reconciliation, route-aware floors, and recovery controls.
- Validate deterministic failure and upgrade/restart behavior before any live authorization.
- Define an atomic cutover: all remaining letter-based automatic and manual execution becomes fail-closed in the same release that can enable the new executor. There is never a window where both engines can execute the same opportunity.

### Stage 5: Bounded live trial

- Requires separate explicit authorization.
- Begins with one profit book, one small size cap, and one route at a time.
- Expansion requires realized P&L and settlement evidence, not quoted P&L.

The currently merged stable-only Strategy T work remains useful source groundwork, but its lettered configuration and report shape are superseded by this design. The pending mainnet deployment remains paused until a separately reviewed implementation plan determines the correct migration boundary.

## 11. Compatibility Rules

- Do not delete or reinterpret historical A-S snapshot fields.
- Add a versioned route observation/trade record rather than expanding letter fields.
- Retain old Candid methods until a deliberate compatibility-removal release.
- Legacy execute methods must fail closed without changing their existing wire signatures.
- Stable-memory decoding tests must cover the currently deployed schema, the merged pre-router schema, and the new schema.
- Dashboard and client code must distinguish active route records from immutable legacy records.

## 12. Verification and Acceptance

The design is implementation-ready only when a plan covers all of the following tests.

### Graph and accounting

- Exact active asset, pool, direction, and venue allowlist.
- No candidate contains a retired venue or asset.
- Exhaustive simple-path/cycle generation for one-to-four-leg shapes.
- Canonical rotation deduplication with reversal remaining distinct.
- No repeated vertices, repeated pools, immediate inverses, or embedded cycles.
- Exact native-decimal and ledger/DEX-fee fixtures for every edge direction.
- Stable-par and ICP-native profit invariants, thresholds, and rounding boundaries.
- Per-stable inventory floor/ceiling enforcement, including changed terminal token.

### Retirement

- Zero automatic calls to Rumi AMM, PartyDEX, and BOB arbitrage pools.
- Legacy automatic and manual execution fails before metadata, quote, approval, or swap calls.
- Approval setup excludes retired integrations.
- Admin recovery remains callable without route admission.

### Execution and failure recovery

- Full-fill rejection and refund accounting.
- Delayed settlement, timeout, and out-of-order response behavior.
- Trap/restart/upgrade at every persisted phase.
- Duplicate timer/callback idempotency.
- Downstream quote deterioration and `RecoveryRequired` transition.
- Profit-preserving minimum-output proofs.
- Planned-versus-realized P&L from attributable ledger deltas.
- Global route lock, per-book scheduling, and canonical-cycle collision prevention.

### Compatibility and presentation

- Stable-state decode/round-trip across all supported schemas.
- Candid compatibility against the deployed baseline.
- Legacy history retains original letter meaning.
- New dashboard route descriptions match the actual directed edge sequence.
- Stable-par disclosure is visible wherever a cross-stable result is called profitable.

## 13. Non-Goals

This design does not authorize or include:

- deployment or activation;
- live swaps, transfers, approvals, or allowance revocations;
- re-admission of Rumi AMM, PartyDEX, BOB, or any other asset/venue;
- removal or rewriting of legacy history;
- changes to the volume bot;
- concurrent live route execution;
- an ICP/USD oracle or cross-profit-domain optimizer.
