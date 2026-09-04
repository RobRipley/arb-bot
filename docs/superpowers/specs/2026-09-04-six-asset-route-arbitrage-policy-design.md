# Six-Asset Route Arbitrage Policy Design

**Status:** Proposed for operator review

**Date:** 2026-09-04

**Scope:** `arb_bot` arbitrage routing, accounting, execution, reporting, and legacy-route retirement

**Implementation authority:** None. This document authorizes no deployment, configuration change, approval, transfer, or trade.

## 1. Decision

Replace the lettered A-R/S/T strategy model with one economic strategy:

> **Six-Asset Route Arbitrage** discovers and executes profitable paths among ICP, ckBTC, ckETH, icUSD, ckUSDT, and ckUSDC using an allowlisted graph of Rumi 3pool and ICPSwap edges.

The engine exposes three candidate classes because they have different economic explanations and profit units:

1. **Stable-par path** — begins and ends in the stable domain without traversing ICP, ckBTC, or ckETH.
2. **Stable-settled cross-asset path** — begins in a stable, traverses one or more of ICP, ckBTC, and ckETH, and ends in a stable.
3. **ICP-returning cycle** — begins and ends in ICP and may traverse one or more stables.

These are reporting and funding classes over one planner, not independent engines. A venue-specific sequence is a **route**, never a strategy letter.

The operator policy values icUSD, ckUSDT, and ckUSDC at exactly $1. This is an intentional hard-coded terminal accounting assumption, not a swap-pricing assumption. Every edge is priced from its real, size-dependent pool quote. The strategy exists to capture discrepancies among those pool prices while comparing starting and ending stable balances at operator-defined par. The engine does not claim that the tokens are economically interchangeable outside this policy.

## 2. Active and Retired Scope

### 2.1 Active assets and roles

Stable settlement assets, each valued at operator-defined $1 par:

- icUSD
- ckUSDT
- ckUSDC

Volatile pass-through assets for stable-settled routes:

- ICP
- ckBTC — mainnet ledger `mxzaz-hqaaa-aaaar-qaada-cai`
- ckETH — mainnet ledger `ss2fx-dyaaa-aaaar-qacoq-cai`

ICP is also the only volatile asset permitted as the start and end of an ICP-returning cycle. ckBTC and ckETH are pass-through and held-inventory assets only; they are not valid principal or successful terminal assets in this version.

The ckBTC and ckETH ledger principals above come from the Internet Computer's [authoritative chain-key canister registry](https://docs.internetcomputer.org/references/chain-key-canister-ids/). The implementation still queries and verifies `icrc1_symbol`, `icrc1_decimals`, and `icrc1_fee` at runtime rather than hard-coding mutable ledger metadata.

No other asset may enter a candidate.

### 2.2 Active venues and edges

The route registry is an allowlist. Only the following integrations may register edges:

- **Rumi 3pool:** both directed stable-to-stable edges for each pair among icUSD, ckUSDT, and ckUSDC.
- **ICPSwap ICP pools:** both directed edges for ICP/icUSD, ICP/ckUSDT, and ICP/ckUSDC.
- **ICPSwap direct-stable pools:** both directed edges for icUSD/ckUSDT, icUSD/ckUSDC, and ckUSDT/ckUSDC.
- **ICPSwap volatile and volatile/stable pools:** both directed edges for each allowlisted pool below:

| Pair | Pool principal |
|---|---|
| ckBTC / ICP | `xmiu5-jqaaa-aaaag-qbz7q-cai` |
| ICP / ckETH | `angxa-baaaa-aaaag-qcvnq-cai` |
| ckBTC / ckETH | `akhru-myaaa-aaaag-qcvna-cai` |
| ckETH / ckUSDC | `mvcvq-3iaaa-aaaag-qjykq-cai` |
| ckBTC / ckUSDC | `mhecj-xyaaa-aaaag-qjyjq-cai` |
| ckBTC / icUSD | `jhf2q-qyaaa-aaaar-qcg3q-cai` |
| ckUSDT / ckBTC | `ipfno-pqaaa-aaaag-qkevq-cai` |
| ckETH / icUSD | `jjhxy-liaaa-aaaar-qcg2q-cai` |

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

Pool principals, token ledgers, token ordering, decimals, fees, and supported full-fill quote semantics remain configured and runtime-verified. A supplied symbol/pair label is not admission evidence by itself. Candidate identity uses `edge_id`; parallel edges connecting the same assets remain distinguishable. Administration may update principals for these named pool slots, but it cannot admit another asset or venue type without a reviewed code and schema change.

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

Explicit admin recovery capabilities for funds already stranded inside a retired venue remain isolated from routing and scheduling. They are manual venue-withdrawal tools, not automatic swaps or drains. Recovery never registers an edge or re-enables a retired integration.

Existing on-ledger allowances are not revoked by this design. Allowance revocation is a separate live operation requiring explicit authorization and an exact inventory of targets.

This design retires BOB **arbitrage**. It does not silently alter the separately configured volume bot, including any icUSD/BOB volume setting.

The arbitrage scheduler also retires automatic residual-asset drains. It never automatically sells excess or stranded ICP, ckBTC, or ckETH. The volume bot's separate subaccount settlement/recovery behavior is unchanged.

## 3. Candidate Classes and Profit Invariants

### 3.1 Stable-par path

Requirements:

- Start asset is icUSD, ckUSDT, or ckUSDC.
- End asset is icUSD, ckUSDT, or ckUSDC.
- No edge touches ICP, ckBTC, or ckETH.
- Interior assets do not repeat.
- A literal round trip may repeat only the start asset as the final asset.

Profit is measured in six-decimal stable-par dollars:

```text
net_profit_usd_6dec =
    par_usd(net_final_stable)
  - par_usd(principal)
  - all_unaccounted_start_asset_fees
```

`par_usd` values each stable at exactly $1 after converting its native decimals. This normalization applies only to the starting principal, final settled balance, fees, and profit report. Route discovery and every intermediate amount use actual size-dependent pool quotes; the planner never substitutes `$1` for an edge price. Candidate eligibility requires both:

```text
net_profit_usd_6dec >= min_stable_profit_usd_6dec
net_profit_bps       >= min_stable_profit_bps
```

A path ending in a different stable is labeled `par_assumption=true` and reports its terminal asset. It is not described as a guaranteed same-token round trip. Per-stable inventory floors and ceilings must remain satisfied.

### 3.2 Stable-settled cross-asset path

Requirements:

- Start asset is one of the three stables.
- End asset is one of the three stables.
- At least one interior asset is ICP, ckBTC, or ckETH.
- Any subset and ordering of ICP, ckBTC, and ckETH may appear, with each asset appearing at most once.
- Interior assets do not repeat.
- A literal round trip may repeat only the start asset as the final asset.

This class contains the economic behavior previously spread across B/F-style routes and generalizes it to all allowlisted cross-asset paths. Examples include:

```text
ckUSDC -> ckBTC -> icUSD
ckUSDT -> ckBTC -> ckETH -> ckUSDC
icUSD -> ckETH -> ICP -> ckBTC -> ckUSDT
```

It uses stable-denominated capital and P&L even when multiple volatile assets appear between the endpoints.

Profit uses the same stable-par invariant and thresholds as Section 3.1. The candidate report identifies the complete pass-through sequence and the implied execution rate of every edge, but eligibility is determined by the full chained quote rather than any spot-price spread or ICP/BTC/ETH oracle. Different terminal stablecoins remain comparable only because of the explicit operator-defined par policy.

### 3.3 ICP-returning cycle

Requirements:

- Start and end asset are ICP.
- The cycle contains at least one stable asset.
- Every interior asset is icUSD, ckUSDT, or ckUSDC; ckBTC and ckETH pass-throughs are limited to Section 3.2 in this version.
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

The arbitrage engine rejects a new complete candidate that begins in the stable domain and ends in ICP, ckBTC, or ckETH, or that begins in any volatile asset and ends in the stable domain. Those are inventory conversions whose profitability requires a cost-basis or valuation policy outside normal route discovery.

Individual stable/volatile and volatile/volatile edges remain valid inside eligible complete candidates. A failed or deteriorated execution may nevertheless terminate operationally in held ICP, ckBTC, or ckETH under Section 7.3; that is an incomplete route with attributable inventory, not a successful arbitrage candidate.

## 4. Route Generation and Canonicalization

### 4.1 Permitted shapes

The planner generates venue-edge-specific routes of one to four swaps, subject to the class rules above. Four swaps permit a stable-settled path to traverse ICP, ckBTC, and ckETH once each. Longer paths are outside this version.

- A non-cyclic stable-par or stable-settled cross-asset path has no repeated asset.
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

The quote-only planner evaluates every route-size pair within inventory and venue constraints. It does not extrapolate a small quote to a larger size. Because the six-asset graph creates substantially more candidates, route enumeration is topology-first and quote execution reuses identical route prefixes within one observation where the exact edge and native input amount match. Optimization must not replace or approximate the final chained quote used for eligibility.

### 5.3 Allowances and inventory

The planner reports required allowances but never creates them. A live candidate is ineligible unless every required allowance is already sufficient.

Eligibility applies per-asset floors and ceilings to:

- the starting debit, including its ledger fee;
- every settled intermediate balance;
- the final balance and terminal stable exposure.

Unknown balances or allowances fail eligibility closed.

ICP, ckBTC, and ckETH ceilings gate additional exposure; exceeding a ceiling never triggers an automatic sale. A settled balance created by a failed route remains held even when it exceeds the configured ceiling, and new candidates that would increase that exposure become ineligible.

### 5.4 Route-aware minimum output

A generic `quote * (1 - slippage)` floor is insufficient. Each leg's minimum output must leave an executable downstream path that preserves:

```text
principal + all remaining fees + configured minimum native profit
```

If no such floor can be established before the first leg, the route is quote-only and cannot execute. After a settled leg, if the remaining route no longer supports such a floor, execution transitions to `HeldInventory` rather than forcing a loss-making conversion.

## 6. Ranking and Scheduling

Stable-profit and ICP-profit candidates are not compared by raw amount and do not require a common ICP/USD oracle.

The scheduler maintains two independent capital books:

- **Stable book:** shared inventory policy across the three $1 stables, with per-token floors and ceilings. It ranks both stable-par and stable-settled cross-asset candidates in the same profit unit while retaining their separate class labels.
- **ICP book:** ICP principal reserve and maximum exposure per route.

Within each book, candidates rank by:

1. highest net profit in the book's native profit unit;
2. highest net profit bps;
3. fewer legs;
4. deterministic `route_id` order.

The initial live design permits one globally active route at a time. If both books have eligible candidates, the least-recently-served book wins; if only one book is eligible, it may run. After any submission, settlement, failure, or held-inventory transition, all quotes are invalidated and the graph is re-quoted before another selection. A reconciled held position does not permanently retain the global route lock, but its exposure applies to later inventory eligibility.

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
Aborted
HeldInventory
```

Only `Completed`, `Aborted`, and `HeldInventory` are terminal for scheduling purposes. `Aborted` means reconciliation proved that no non-principal route inventory remains, although unavoidable fees may have produced a realized loss. `HeldInventory` is reachable only after all ambiguous submissions, delayed withdrawals, refunds, and balance deltas have been reconciled; uncertainty about whether a swap executed is not a terminal hold state.

### 7.2 Settlement proof

- An ICPSwap `depositFromAndSwap` success is submission evidence, not ledger settlement.
- Completion requires attributable balance deltas and refund accounting.
- Rumi completion also uses balance evidence consistent with its transfer semantics.
- The executor re-quotes the exact remaining route after each confirmed leg settlement.
- Duplicate callbacks, timer retries, upgrades, and traps must resume from the persisted phase rather than repeat a debit.

### 7.3 Failed or deteriorated route

If a submitted leg fails, or the exact re-quote after a settled leg no longer preserves principal and minimum profit, the executor does not drain, dump, or automatically liquidate the resulting inventory. After settlement and refund reconciliation, it enters `Aborted` when no non-principal route inventory remains; otherwise it enters `HeldInventory` and records one or more attributable held lots containing:

```text
asset
native_amount
originating_execution_id
originating_route_id
starting_stable_asset
starting_stable_cost_basis_usd_6dec
settled_leg_history
failure_or_deterioration_reason
first_held_timestamp
last_reconciled_timestamp
```

The operator may leave any resulting stable, ICP, ckBTC, or ckETH balance in the bot indefinitely. Held inventory is not counted as completed-route proceeds or realized profit. Any later conversion is a separately initiated continuation linked to the held lot and subject to fresh full-fill quotes and an explicit minimum-output policy; there is no automatic retry or liquidation loop in this design.

### 7.4 Realized P&L

Realized P&L uses before/after ledger deltas in the candidate's profit domain and records planned versus settled amounts per leg. Quoted profit is never recorded as realized profit. A `HeldInventory` route reports its original stable cost basis and current native holdings as an incomplete position without manufacturing a stablecoin P&L mark.

### 7.5 Automatic arbitrage drain deletion

The route-engine cutover deletes the arbitrage functions `drain_residual_icp` and `drain_residual_bob` and their scheduler call sites, and exposes no replacement generic drain. Loose or route-attributable ICP, ckBTC, and ckETH balances are never automatically sold merely because a cycle begins or an inventory ceiling is exceeded.

This retirement does not remove the volume bot's separate subaccount settlement and stranded-fund recovery behavior. It also does not prohibit a narrowly scoped manual withdrawal from a retired external venue where funds are already stranded; such a withdrawal cannot perform an opportunistic market swap or feed an active route.

## 8. Configuration and API Model

The new versioned route-arbitrage configuration contains:

```text
enabled
dry_run
asset_registry
active_pool_registry
stable_size_ladder
icp_size_ladder
max_route_legs
max_stable_principal_usd_6dec
max_icp_principal_e8s
min_stable_profit_usd_6dec
min_stable_profit_bps
min_icp_profit_e8s
min_icp_profit_bps
per_asset_inventory_floor
per_asset_inventory_ceiling
per_asset_additional_exposure_enabled
quote_max_age_ns
settlement_timeout_ns
```

The versioned asset registry contains the allowlisted asset role, ledger principal, expected symbol, expected decimals, ledger fee, and wallet-balance visibility for all six assets. Runtime metadata must match the configured expectation before that asset or a dependent edge is eligible. ckBTC and ckETH are enabled for balance visibility and receipt without requiring the bot to hold a pre-funded balance.

There is one master route-arbitrage enable switch plus per-profit-book enable switches. Pool registry changes and any future venue admission remain admin-only. `max_route_legs` cannot exceed four in this schema. Dry-run is the migration default.

New APIs are additive and use the new taxonomy:

- quote the complete route universe;
- inspect best candidate per profit book;
- inspect pending execution and held inventory;
- inspect all six arb-wallet ledger balances;
- configure route-arbitrage policy;
- execute a descriptive route only after live execution is separately authorized.

There is no drain API. A future held-inventory continuation API would require a separate reviewed design and explicit operator initiation.

Legacy letter dry-run and execute methods remain wire-compatible during the transition. Retired-integration methods fail closed in Stage 1. The remaining methods are labeled legacy while the new planner observes, then become fail-closed compatibility stubs at the execution cutover. No new consumer should call them.

## 9. Reporting and Dashboard

The active dashboard shows:

- best stable-book candidate;
- best ICP-book candidate;
- candidate class and descriptive asset/venue path;
- principal, quoted net profit, bps, all fees, full-fill status, inventory effect, allowance status, and quote age;
- canonical-cycle collision/rotation status;
- current durable execution phase and settlement latency;
- explicit rejection and held-inventory reasons;
- arb-wallet balances for ICP, ckBTC, ckETH, icUSD, ckUSDT, and ckUSDC;
- held lots with originating route, stable cost basis, settled history, and reconciliation timestamp.

Rumi AMM, PartyDEX, BOB arbitrage, and lettered strategies are absent from active opportunity cards. Historical views continue to render their original labels and fields as legacy data.

All stable-book candidates display the policy disclosure:

> Pool quotes determine every execution amount. icUSD, ckUSDT, and ckUSDC are valued at a hard-coded $1 only for starting-principal, terminal-balance, and profit accounting. Terminal token and inventory exposure may change.

## 10. Migration Plan

Migration is additive and staged.

### Stage 1: Retirement safety

- Remove retired integrations from automatic metadata, quote, approval, scheduler, and execution paths.
- Make legacy manual K-R, S, and Rumi-AMM-backed letter methods fail closed.
- Delete the arbitrage functions `drain_residual_icp` and `drain_residual_bob` and their scheduler call sites; do not replace them with a generic drain.
- Preserve isolated manual withdrawal from a retired external venue without admitting it to routing.
- Preserve any deployed `pending_exit` evidence as a visible legacy held incident; do not use it to trigger an automatic swap.
- Prove zero inter-canister calls for every retired path with call-counting tests.

### Stage 2: Quote-only route planner

- Add the six-asset wallet/metadata registry, all allowlisted edges, candidate generation, canonicalization, accounting, ranking, held-inventory reports, and all-six-asset balance reporting.
- Keep all new execution disabled and dry-run-only.
- Keep still-permitted legacy ICPSwap execution isolated from the new planner during observation; it cannot consume a new-planner candidate or route ID.
- Preserve existing stable-state and Candid compatibility.

### Stage 3: Observation

- Collect timestamped route-size observations across materially different pool states.
- Measure quote drift, full-fill rejections, quote latency, cycle cost, candidate rotation duplication, and expected settlement exposure.
- Do not infer realized fill performance from query-only observations.

### Stage 4: Durable executor

- Add the persisted phase machine, venue adapters, settlement reconciliation, route-aware floors, and attributable `HeldInventory` transition.
- Validate deterministic failure and upgrade/restart behavior before any live authorization.
- Define an atomic cutover: all remaining letter-based automatic and manual execution becomes fail-closed in the same release that can enable the new executor. There is never a window where both engines can execute the same opportunity.

### Stage 5: Bounded live trial

- Requires separate explicit authorization.
- Begins with one profit book, one small size cap, and one route at a time.
- Route-length and pass-through-asset expansion remain independently configurable; adding support does not automatically enable every path for live execution.
- Expansion requires realized P&L and settlement evidence, not quoted P&L.

The currently merged stable-only Strategy T work remains useful source groundwork, but its lettered configuration and report shape are superseded by this design. The pending mainnet deployment remains paused until a separately reviewed implementation plan determines the correct migration boundary.

## 11. Compatibility Rules

- Do not delete or reinterpret historical A-S snapshot fields.
- Add a versioned route observation/trade record rather than expanding letter fields.
- Retain old Candid methods until a deliberate compatibility-removal release.
- Legacy execute methods must fail closed without changing their existing wire signatures.
- Stable-memory decoding tests must cover the currently deployed schema, the merged pre-router schema, and the new schema.
- Dashboard and client code must distinguish active route records from immutable legacy records.
- Existing `pending_exit` and `pending_bob_exit` data must decode without initiating a drain and must remain inspectable as legacy incident evidence.

## 12. Verification and Acceptance

The design is implementation-ready only when a plan covers all of the following tests.

### Graph and accounting

- Exact active asset, pool, direction, and venue allowlist.
- Runtime ledger metadata and pool token-order verification for ICP, ckBTC, ckETH, icUSD, ckUSDT, and ckUSDC.
- No candidate contains a retired venue or asset.
- Exhaustive simple-path/cycle generation for one-to-four-leg shapes.
- Canonical rotation deduplication with reversal remaining distinct.
- No repeated vertices, repeated pools, immediate inverses, or embedded cycles.
- Exact native-decimal and ledger/DEX-fee fixtures for every edge direction.
- Stable-par and ICP-native profit invariants, thresholds, and rounding boundaries.
- Stable-only routes use real chained pool quotes while applying $1 par only at their accounting boundary.
- Stable-settled routes cover every permitted subset and ordering of ICP, ckBTC, and ckETH within the four-leg limit.
- ckBTC and ckETH are rejected as successful route endpoints.
- Per-stable inventory floor/ceiling enforcement, including changed terminal token.
- Volatile exposure ceilings reject new exposure without triggering a sale of existing holdings.

### Retirement

- Zero automatic calls to Rumi AMM, PartyDEX, and BOB arbitrage pools.
- Legacy automatic and manual execution fails before metadata, quote, approval, or swap calls.
- Approval setup excludes retired integrations.
- The arbitrage drain functions and their scheduler call sites are absent.
- Manual external-venue withdrawal remains isolated and cannot initiate a market swap.
- Volume-bot recovery behavior is unchanged.

### Execution, settlement, and holding

- Full-fill rejection and refund accounting.
- Delayed settlement, timeout, and out-of-order response behavior.
- Trap/restart/upgrade at every persisted phase.
- Duplicate timer/callback idempotency.
- Downstream quote deterioration and `HeldInventory` transition after exact reconciliation.
- Full refund/no-position failures transition to `Aborted`, with unavoidable fees reported accurately.
- Held lots preserve native amounts, stable cost basis, route attribution, settled legs, and failure reason across restart and upgrade.
- Held ICP, ckBTC, and ckETH remain untouched across later arb cycles; no ceiling, timer, or scheduler event automatically sells them.
- Profit-preserving minimum-output proofs.
- Planned-versus-realized P&L from attributable ledger deltas.
- Global route lock, per-book scheduling, and canonical-cycle collision prevention.

### Compatibility and presentation

- Stable-state decode/round-trip across all supported schemas.
- Candid compatibility against the deployed baseline.
- Legacy history retains original letter meaning.
- New dashboard route descriptions match the actual directed edge sequence.
- Stable-par disclosure is visible wherever a cross-stable result is called profitable.
- Wallet balances include ckBTC and ckETH even when zero, and no UI copy implies they were funded by configuration.

## 13. Non-Goals

This design does not authorize or include:

- deployment or activation;
- live swaps, transfers, approvals, or allowance revocations;
- funding the arb canister with ckBTC or ckETH;
- re-admission of Rumi AMM, PartyDEX, BOB, or any other asset/venue;
- removal or rewriting of legacy history;
- changes to the volume bot;
- concurrent live route execution;
- automatic draining, liquidation, or retry of held ICP, ckBTC, or ckETH;
- ckBTC-returning or ckETH-returning profit books;
- an ICP/USD oracle or cross-profit-domain optimizer.
