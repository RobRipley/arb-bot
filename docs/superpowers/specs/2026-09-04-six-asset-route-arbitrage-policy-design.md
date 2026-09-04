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

The six admitted ledger identities are code-pinned:

| Role | Asset | Mainnet ledger principal |
|---|---|---|
| stable settlement, operator-defined $1 par | icUSD | `t6bor-paaaa-aaaap-qrd5q-cai` |
| stable settlement, operator-defined $1 par | ckUSDT | `cngnf-vqaaa-aaaar-qag4q-cai` |
| stable settlement, operator-defined $1 par | ckUSDC | `xevnm-gaaaa-aaaar-qafnq-cai` |
| volatile pass-through and ICP-returning principal | ICP | `ryjl3-tyaaa-aaaaa-aaaba-cai` |
| volatile pass-through and held inventory | ckBTC | `mxzaz-hqaaa-aaaar-qaada-cai` |
| volatile pass-through and held inventory | ckETH | `ss2fx-dyaaa-aaaar-qacoq-cai` |

ICP is also the only volatile asset permitted as the start and end of an ICP-returning cycle. ckBTC and ckETH are pass-through and held-inventory assets only; they are not valid principal or successful terminal assets in this version.

The ckBTC and ckETH ledger principals above come from the Internet Computer's [authoritative chain-key canister registry](https://docs.internetcomputer.org/references/chain-key-canister-ids/); the other four match the deployed arb canister's read-only `get_config` response on 2026-09-04. The implementation still queries and verifies `icrc1_symbol`, `icrc1_decimals`, and `icrc1_fee` at runtime rather than hard-coding mutable ledger metadata. Configuration may enable or disable a code-pinned asset but may not substitute another ledger principal or redefine its expected identity without a reviewed code and schema migration.

No other asset may enter a candidate.

### 2.2 Active venues and edges

The route registry is a principal-pinned allowlist. Only the integrations in the following table may register edges, and each admitted pool contributes both directed pair edges (the Rumi 3pool contributes both directed edges for each of its three stable pairs):

| Venue | Pair or pool | Pool principal |
|---|---|---|
| Rumi 3pool | icUSD / ckUSDT / ckUSDC | `fohh4-yyaaa-aaaap-qtkpa-cai` |
| ICPSwap | ICP / ckUSDC | `mohjv-bqaaa-aaaag-qjyia-cai` |
| ICPSwap | ICP / icUSD | `nqxwe-hiaaa-aaaar-qb5yq-cai` |
| ICPSwap | ICP / ckUSDT | `hkstf-6iaaa-aaaag-qkcoq-cai` |
| ICPSwap | icUSD / ckUSDT | `jogrm-gqaaa-aaaar-qcg2a-cai` |
| ICPSwap | icUSD / ckUSDC | `eb25l-dyaaa-aaaar-qb4lq-cai` |
| ICPSwap | ckUSDT / ckUSDC | `heq6n-fyaaa-aaaag-qkcpq-cai` |
| ICPSwap | ckBTC / ICP | `xmiu5-jqaaa-aaaag-qbz7q-cai` |
| ICPSwap | ICP / ckETH | `angxa-baaaa-aaaag-qcvnq-cai` |
| ICPSwap | ckBTC / ckETH | `akhru-myaaa-aaaag-qcvna-cai` |
| ICPSwap | ckETH / ckUSDC | `mvcvq-3iaaa-aaaag-qjykq-cai` |
| ICPSwap | ckBTC / ckUSDC | `mhecj-xyaaa-aaaag-qjyjq-cai` |
| ICPSwap | ckBTC / icUSD | `jhf2q-qyaaa-aaaar-qcg3q-cai` |
| ICPSwap | ckUSDT / ckBTC | `ipfno-pqaaa-aaaag-qkevq-cai` |
| ICPSwap | ckETH / icUSD | `jjhxy-liaaa-aaaar-qcg2q-cai` |

The three ICP/stable principals above are the values returned by the deployed arb canister's read-only `get_config` query on 2026-09-04. The direct-stable and volatile-pair principals are operator-supplied admission pins. Implementation must independently verify every principal against pool metadata before enabling its edges; the table is an allowlist, not proof that a pool is presently healthy.

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

Pool principals, token ledgers, token ordering, decimals, fees, and supported full-fill quote semantics remain runtime-verified. A supplied symbol/pair label is not admission evidence by itself. Candidate identity uses `edge_id`; parallel edges connecting the same assets remain distinguishable. Administration may disable a pinned pool but may not substitute a different principal, asset, or venue type without a reviewed code and schema change.

### 2.3 Retired integrations

The following are absent from the active **arbitrage** route registry:

- Rumi AMM / 3USD-ICP arbitrage routes.
- Every PartyDEX route.
- BOB arbitrage Strategy S and all BOB pools as arbitrage edges.

Arbitrage retirement is stronger than winner-selection exclusion. Within the arbitrage engine, scheduler, and arbitrage-facing administration, a retired integration receives zero automatic:

- metadata or token-ordering calls;
- quotes or health polling;
- allowance creation or refresh;
- scheduled or manual strategy execution;
- route ranking;
- dashboard actions that could initiate trading.

Legacy letter execution methods remain temporarily for Candid compatibility, but every automatic and manual lettered executor fails closed in Stage 1 before any inter-canister call and records a `retired_route` activity event. Historical and dry-run reads may remain available under their existing signatures during migration, provided they cannot submit an approval, transfer, deposit, swap, or withdrawal. There is no observation period in which a legacy lettered executor and the new executor are both live.

Existing historical records and stable-state fields are preserved with their original meaning. They are not rewritten into the new taxonomy. Old configuration fields may remain decode-only during the compatibility period but are ignored by the route registry.

Explicit admin recovery capabilities for funds already stranded inside a retired venue remain isolated from routing and scheduling. They are manual venue-withdrawal tools, not automatic swaps or drains. Recovery never registers an edge or re-enables a retired integration, and it must acquire the global account-mutation lock whenever its destination can overlap a route-relevant account.

Existing on-ledger allowances are not revoked by this design. Allowance revocation is a separate live operation requiring explicit authorization and an exact inventory of targets.

This design retires Rumi AMM, PartyDEX, and BOB **from arbitrage only**. It does not retire or silently alter the separately configured volume bot, including its existing Rumi-AMM-backed 3USD/ICP activity or any icUSD/BOB volume setting. Those volume routes remain outside the active arbitrage graph and are affected only by the global account-mutation serialization in Section 7.3.

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
- Any graph-reachable subset and ordering of ICP, ckBTC, and ckETH may appear, with each asset appearing at most once. This does not promise a direct edge between every stable and every first or last pass-through asset; reachability is determined by the admitted pool graph and four-swap limit.
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

### 3.4 Profit-bps normalization

`net_profit_bps` always uses the candidate's **initial principal**, expressed in the same profit unit as its native profit, as the denominator. It never divides by final proceeds, gross route volume, or an oracle-converted value:

```text
stable candidate:
  net_profit_bps = trunc_toward_zero(
      checked_widen(net_profit_usd_6dec) * 10_000
      / principal_usd_6dec
  )

ICP candidate:
  net_profit_bps = trunc_toward_zero(
      checked_widen(net_profit_icp_e8s) * 10_000
      / principal_icp_e8s
  )
```

The calculation uses a signed widened integer representation sufficient to multiply before division without overflow. Principal must be greater than zero. A zero principal, failed checked conversion/multiplication, quotient outside the report field's representable range, or any other arithmetic error rejects the candidate fail closed. Negative profit remains negative and truncates toward zero; it is never converted to an unsigned value. Threshold comparison uses the exact truncated integer result, so a mathematical value below 50 bps cannot become eligible for a 50-bps threshold through rounding.

### 3.5 Rejected endpoints

The arbitrage engine rejects a new complete candidate that begins in the stable domain and ends in ICP, ckBTC, or ckETH, or that begins in any volatile asset and ends in the stable domain. Those are inventory conversions whose profitability requires a cost-basis or valuation policy outside normal route discovery.

Individual stable/volatile and volatile/volatile edges remain valid inside eligible complete candidates. A failed or deteriorated execution may nevertheless terminate operationally in held ICP, ckBTC, or ckETH under Section 7.4; that is an incomplete route with attributable inventory, not a successful arbitrage candidate.

## 4. Route Generation and Canonicalization

### 4.1 Permitted shapes

The planner generates venue-edge-specific routes of one to four swaps, subject to the class rules above. Four swaps permit a stable-settled path to traverse ICP, ckBTC, and ckETH once each. Longer paths are outside this version.

- A non-cyclic stable-par or stable-settled cross-asset path has no repeated asset.
- A cycle repeats only its start asset as its final asset.
- No candidate repeats an edge or pool.
- No candidate traverses both directions of the same physical pool consecutively. Reversing the asset direction through a different pool and different `edge_id` is permitted and is the normal two-venue stable-arbitrage shape.
- No candidate contains a smaller embedded cycle.
- Reverse directions remain distinct because they have different economics.

Repeated-vertex walks are excluded: they either contain a smaller independently evaluable cycle or add avoidable fees and settlement risk. The Rumi 3pool may appear at most once in a candidate; a second swap inside the same physical pool is replaced by the pool's direct pair edge and otherwise adds avoidable fees and settlement risk.

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

Funding eligibility uses only unencumbered balance:

```text
available_native(asset) =
    ledger_balance_native(asset)
  - durable_held_reservations_native(asset)
  - active_execution_reservations_native(asset)
```

The subtractions are checked and fail closed on underflow or inconsistent attribution. Every asset lot in a `HeldInventory` position, including a stable or ICP lot, creates a durable per-asset reservation before the global route lock is released. An implementation may instead isolate each held position in a dedicated subaccount, but it must preserve the same non-spendability invariant. Unlinked routes, volume operations, generic withdrawals, and admin tools may not consume, sweep, or count held lots as available principal. Only a separately initiated continuation linked to that held position may release or spend its reservation.

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
call_intent_id
source_and_destination_accounts
allowance_spender
ledger_created_at_time_and_memo
adapter_request_fingerprint
submission_started_at
submission_response_or_reject
expected_debits_credits_and_refunds
observed_ledger_or_pool_receipts
reconciliation_evidence
reconciliation_query_count
```

The intent is persisted before the outbound update. The executor must durably transition to `LegSubmitted`, including `call_intent_id`, complete request fingerprint, planned input, source/destination accounts, and `submission_started_at`, **before** issuing a non-idempotent inter-canister call. `ledger_created_at_time_and_memo` are supplied wherever the called ledger interface supports them; the adapter fingerprint is evidence, not a claim that a DEX update is idempotent. Any resume from `LegSubmitted`, including a lost response, timer retry, trap, or upgrade, may only reconcile the recorded intent; it must never reconstruct and resubmit the debit or swap.

Phases are:

```text
Planned
LegPrepared
LegSubmitted
AwaitingSettlement
ReconciliationRequired
LegSettled
RemainingRouteRequoted
Completed
Aborted
HeldInventory
```

Only `Completed`, `Aborted`, and `HeldInventory` are terminal for scheduling purposes. `Aborted` means reconciliation proved that no non-principal route inventory remains, although unavoidable fees may have produced a realized loss. `HeldInventory` is reachable only after all ambiguous submissions, delayed withdrawals, refunds, and balance deltas have been reconciled; uncertainty about whether a swap executed is not a terminal hold state.

`ReconciliationRequired` is a durable, non-terminal incident state entered when `settlement_timeout_ns` expires without sufficient evidence to classify the leg. It alerts the operator, continues read-only reconciliation polling with a bounded per-cycle query budget, and never resubmits the debit or swap. Quote-only observation may continue, but no new wallet-mutating route or volume operation may begin until evidence resolves the incident to `LegSettled`, `Aborted`, or `HeldInventory`. If authoritative evidence remains unavailable, the state and mutation lock remain fail-closed rather than guessing.

### 7.2 Settlement proof

- An ICPSwap `depositFromAndSwap` success is submission evidence, not ledger settlement.
- Completion requires attributable source debit, venue execution, destination credit, and refund accounting.
- Rumi completion also uses balance evidence consistent with its transfer semantics.
- The executor re-quotes the exact remaining route after each confirmed leg settlement.
- Duplicate callbacks, timer retries, upgrades, and traps must resume from the persisted phase rather than repeat a debit.
- Ledger transaction references, pool receipts, exact source/destination account deltas, and refund evidence are persisted as they are observed. A bare balance increase is never sufficient attribution, even with account-mutation exclusivity, because an external party can credit the public account.

Each execution adapter defines a source-bound reconciliation predicate before it may be enabled. For ICPSwap and Rumi, `LegSettled` requires evidence tied to the recorded request fingerprint that establishes all of the following: the exact input debit from the recorded source account; the pool, direction, and effective input accepted; the corresponding output credit from an attributable venue or settlement account; every partial-fill refund or unused-input return; and conservation of the expected gross amounts and ledger/DEX fees within exact native-unit rules. Acceptable bindings include ledger transaction identities with matching from/to accounts and created-at-time or memo where supported, plus a pool receipt or transaction-history record that binds the venue execution to the same input. An amount-only DEX response or coincident balance delta is insufficient.

If an adapter cannot obtain this source-bound evidence after a lost response, delayed withdrawal, or partial fill, the route remains `ReconciliationRequired` and the durable mutation lock remains held. The design does not guess, advance the next leg, mark profit, or downgrade uncertainty to `HeldInventory` merely because balances appear plausible.

### 7.3 Global account-mutation ownership

A single durable account-mutation lock covers **every canister-controlled operation that can mutate a route-relevant default account**, not only the route executor and volume bot. It is acquired before a transfer, approval-funded deposit, swap, withdrawal, sweep, recovery, or movement into a default account and held through settlement/refund reconciliation. Covered surfaces include scheduled and manual volume cycles, volume fund/withdraw administration, generic withdrawals, Rumi manual swaps, 3pool deposit/redeem/exchange tools, retired-venue recovery, and any compatibility endpoint that remains capable of moving funds. An endpoint may avoid the lock only if its accounts are structurally disjoint and that non-overlap is an asserted, tested invariant.

While the route executor owns the lock, every covered scheduled, manual, or admin-triggered operation defers before its first mutation. While any other covered operation owns the lock, route execution defers. Quote-only calls remain permitted. The implementation maintains an exhaustive inventory of Candid update entrypoints and timer callbacks, classifying each as fail-closed, lock-participating, read-only, or proven-account-disjoint; an unclassified mutator fails the acceptance gate.

This deliberately changes overlap scheduling because the existing volume flow transiently uses arb default accounts; it does not change the volume bot's pool selection, sizing, subaccount ownership, recovery semantics, or balances. The lock and owner survive upgrade/restart, and an unresolved `ReconciliationRequired` incident retains the lock.

Legacy `clear_cycle_lock` behavior cannot release or overwrite this durable mutation lock. Resolving a stuck reconciliation lock requires source-bound settlement evidence or a separately reviewed, explicit loss-acceptance recovery design; ordinary cycle-lock administration cannot bypass attribution safety.

### 7.4 Failed or deteriorated route

If a submitted leg fails, or the exact re-quote after a settled leg no longer preserves principal and minimum profit, the executor does not drain, dump, or automatically liquidate the resulting inventory. After settlement and refund reconciliation, it enters `Aborted` when no non-principal route inventory remains; otherwise it enters `HeldInventory` and records an attributable held position containing:

```text
originating_execution_id
originating_route_id
principal_domain =
    StablePar { start_asset, principal_native, principal_usd_6dec }
  | IcpNative { principal_icp_e8s }
  | LegacyUnknown { preserved_pending_fields }
settled_leg_history
failure_or_deterioration_reason
first_held_timestamp
last_reconciled_timestamp
lots[] = { asset, native_amount, attributable_fees_native, reserved_native }
```

The operator may leave any resulting stable, ICP, ckBTC, or ckETH balance in the bot indefinitely. Held inventory is not counted as completed-route proceeds or realized profit and its reserved lots are excluded from every unrelated spendable-balance calculation under Section 5.3. Stable-funded positions retain stable-par principal and cost basis; ICP-funded positions retain ICP-native principal and never invent a USD cost basis. `LegacyUnknown` is used only when migrated deployed evidence lacks a truthful original cost basis; it preserves the raw legacy fields and is ineligible for an automated continuation. Any later conversion is a separately initiated continuation linked to the held position and subject to fresh full-fill quotes and an explicit minimum-output policy; there is no automatic retry or liquidation loop in this design.

### 7.5 Realized P&L

Realized P&L uses before/after ledger deltas in the candidate's profit domain and records planned versus settled amounts per leg. Quoted profit is never recorded as realized profit. A `HeldInventory` route reports its tagged original principal domain and current native holdings as an incomplete position. Stable-funded holds may show the original stable-par cost basis; ICP-funded holds remain entirely ICP-native.

### 7.6 Automatic arbitrage drain deletion

Stage 1 deletes the arbitrage functions `drain_residual_icp` and `drain_residual_bob` and their scheduler call sites, before quote-only observation begins, and exposes no replacement generic drain. Loose or route-attributable ICP, ckBTC, and ckETH balances are never automatically sold merely because a cycle begins or an inventory ceiling is exceeded.

This retirement does not remove the volume bot's separate subaccount settlement and stranded-fund recovery behavior. It also does not prohibit a narrowly scoped manual withdrawal from a retired external venue where funds are already stranded; such a withdrawal cannot perform an opportunistic market swap or feed an active route.

## 8. Configuration and API Model

The new versioned route-arbitrage configuration contains:

```text
enabled
dry_run
stable_book_enabled
icp_book_enabled
asset_registry
active_pool_registry
stable_size_ladder
icp_size_ladder
max_route_legs
max_size_ladder_entries
max_quote_calls_per_observation
max_concurrent_quote_calls
max_terminal_execution_records
max_execution_record_bytes
max_reconciliation_evidence_items
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
reconciliation_queries_per_cycle
max_open_held_positions
```

The versioned asset registry contains the allowlisted asset role, **code-pinned** ledger principal from Section 2.1, immutable expected symbol/decimals, runtime ledger fee, enabled status, and wallet-balance visibility for all six assets. Runtime metadata must match the code-pinned identity and immutable expectation before that asset or a dependent edge is eligible. Administration may toggle an admitted asset but cannot rewrite its identity. ckBTC and ckETH are enabled for balance visibility and receipt without requiring the bot to hold a pre-funded balance.

There is one master route-arbitrage enable switch plus the explicit `stable_book_enabled` and `icp_book_enabled` switches. Pool registry changes and any future venue admission remain admin-only. `max_route_legs` cannot exceed four and each size ladder cannot exceed 16 entries. Dry-run is the migration default.

New APIs are additive and use the new taxonomy:

- quote a cursor-bounded batch of the route universe and report observation completeness;
- inspect best candidate per profit book;
- inspect pending execution and held inventory;
- inspect all six arb-wallet ledger balances;
- configure route-arbitrage policy;
- execute a descriptive route only after live execution is separately authorized.

There is no drain API. A future held-inventory continuation API would require a separate reviewed design and explicit operator initiation.

### 8.1 Bounded observation and query model

Topology generation produces a deterministic ordered set of route IDs. Quoting walks that set in cursor-bounded batches with configured call and concurrency limits. Candidate-list, execution-history, and held-position APIs require pagination and enforce a maximum page size of 100 records.

A live winner may be selected only from an observation that evaluated the entire enabled route-and-size universe within `quote_max_age_ns`. If the configured universe cannot complete within the quote-call, response-size, cycle, or age budget, the observation reports `incomplete` and no route may execute. It may not silently select the best result from a partial scan. Quote-prefix reuse is permitted only under the exact-match rule in Section 5.2.

The implementation plan must measure the full current graph and choose defaults below tested canister limits. Compile-time ceilings cap size ladders at 16 entries, concurrent quote calls at 16, and returned pages at 100 records. Increasing those ceilings requires a reviewed schema/resource change.

### 8.2 Bounded durable storage

Current execution metadata remains a single bounded record. Executions, their active per-asset funding reservations, reconciliation evidence, held per-asset reservations, and held positions live in dedicated indexed stable structures assigned new, non-overlapping memory IDs; they are not appended to heap `BotState`. Every execution record has a maximum encoded size of 65,536 bytes and at most 64 reconciliation-evidence items; any individual variable-length text/blob field is capped so the aggregate record remains encodable. Oversized evidence is rejected and reported rather than trapping a stable write.

A held position contains at most six coalesced asset lots, one per active asset. `max_open_held_positions` is bounded by a compile-time ceiling of 256. Before beginning a route, the executor reserves capacity for one potential held position. Reaching the ceiling disables new execution without deleting, coalescing across unrelated executions, or liquidating existing holdings.

The terminal execution log is separately paginated and capped at 10,000 records. Capacity for the current route's terminal record is reserved before its first mutation. When either the terminal-log or held-position reservation cannot be made, new live execution fails closed while quote-only observation and existing reconciliation remain available. The executor never automatically deletes history or coalesces unrelated records to regain capacity; export/pruning, if desired, requires a separately reviewed admin design.

Legacy letter dry-run and execute methods remain wire-compatible during the transition. Every lettered automatic and manual executor becomes a fail-closed compatibility stub in Stage 1. Read-only legacy observations may remain labeled legacy while the new planner observes. No new consumer should call them.

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
- held positions with originating route, tagged stable-par/ICP-native/legacy-unknown principal domain, native asset lots, settled history, and reconciliation timestamp.

Rumi AMM, PartyDEX, BOB arbitrage, and lettered strategies are absent from active opportunity cards. Historical views continue to render their original labels and fields as legacy data.

All stable-book candidates display the policy disclosure:

> Pool quotes determine every execution amount. icUSD, ckUSDT, and ckUSDC are valued at a hard-coded $1 only for starting-principal, terminal-balance, and profit accounting. Terminal token and inventory exposure may change.

## 10. Migration Plan

Migration is additive and staged.

### Stage 1: Retirement safety

- Remove retired integrations from automatic metadata, quote, approval, scheduler, and execution paths.
- Make every legacy lettered automatic and manual executor fail closed before any inter-canister call. Preserve wire signatures and read-only historical/dry-run access only where it cannot mutate funds.
- Make the non-letter legacy `manual_arb_cycle` executor fail closed. Replace legacy `setup_approvals` behavior with a fail-closed compatibility stub; future active-pool allowances require a separately named, spender-specific admin action and remain outside dry-run observation.
- Inventory every Candid update method and timer callback that can mutate a route-relevant balance. Retired and legacy execution surfaces fail closed; isolated recovery is explicitly classified; remaining active manual balance tools are marked for mandatory participation in the Stage-4 global account-mutation lock before the new executor can be enabled.
- Delete the arbitrage functions `drain_residual_icp` and `drain_residual_bob` and their scheduler call sites; do not replace them with a generic drain.
- Preserve isolated manual withdrawal from a retired external venue without admitting it to routing.
- Preserve any deployed `pending_exit` evidence as a visible legacy held incident; do not use it to trigger an automatic swap.
- Prove zero inter-canister calls for every retired path with call-counting tests.

### Stage 2: Quote-only route planner

- Add the six-asset wallet/metadata registry, all allowlisted edges, candidate generation, canonicalization, accounting, ranking, held-inventory reports, and all-six-asset balance reporting.
- Record a timestamped, read-only admission fixture for the Rumi 3pool and every ICPSwap pool: pool principal, token ledger pair, token ordering, fee tier/model, decimals, ledger fees, and full-fill quote behavior.
- Keep all new execution disabled and dry-run-only.
- Confirm all legacy lettered execution remains fail-closed throughout observation; no legacy executor may consume either a historical quote or a new-planner candidate.
- Preserve existing stable-state and Candid compatibility.

### Stage 3: Observation

- Collect timestamped route-size observations across materially different pool states.
- Measure the exact enabled route count, route-and-size count, quote-call count, response size, quote drift, full-fill rejections, quote latency, cycle cost, candidate rotation duplication, and expected settlement exposure.
- Demonstrate that a complete observation fits within configured call, concurrency, response-size, cycle, and quote-age limits; otherwise reduce the enabled universe rather than executing from a partial scan.
- Do not infer realized fill performance from query-only observations.

### Stage 4: Durable executor

- Add the persisted phase machine, per-call intent/receipt evidence, source-bound adapter reconciliation predicates, venue adapters, global account-mutation lock across the complete mutating-surface inventory, durable held-balance reservations, bounded reconciliation process, route-aware floors, `ReconciliationRequired`, and attributable `HeldInventory` transition.
- Validate deterministic failure and upgrade/restart behavior before any live authorization.
- Preserve the Stage-1 fail-closed legacy boundary in the same release that can enable the new executor. There is never a window where both engines can execute the same opportunity.

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
- Legacy pending records without a provable principal basis migrate to `LegacyUnknown`; migration must not fabricate stable or ICP cost basis.

## 12. Verification and Acceptance

The design is implementation-ready only when a plan covers all of the following tests.

### Graph and accounting

- Exact active asset, pool, direction, and venue allowlist.
- Golden asset fixture code-pins all six ledger principals from Section 2.1; enable/disable is configurable, but every attempted ledger-identity or immutable-metadata substitution is rejected absent a reviewed code/schema migration.
- Golden admission fixture pins every active pool principal in Section 2.2 and proves an admin cannot substitute a different principal without a reviewed code/schema change.
- Runtime ledger metadata and pool token-order verification for ICP, ckBTC, ckETH, icUSD, ckUSDT, and ckUSDC.
- No candidate contains a retired venue or asset.
- Exhaustive simple-path/cycle generation for one-to-four-leg shapes, with a golden fixture containing the exact ordered route-ID set and count for the admitted graph.
- Canonical rotation deduplication with reversal remaining distinct.
- No repeated vertices, repeated pools, same-pool consecutive reversals, or embedded cycles; distinct-pool reverse directions remain eligible.
- Exact native-decimal and ledger/DEX-fee fixtures for every edge direction.
- Stable-par and ICP-native profit invariants, thresholds, and rounding boundaries.
- Stable and ICP `net_profit_bps` fixtures prove initial-principal denominators, signed widened checked arithmetic, truncation toward zero, zero-principal rejection, negative-profit behavior, overflow rejection, and exact values immediately below/at/above each configured bps threshold.
- Stable-only routes use real chained pool quotes while applying $1 par only at their accounting boundary.
- Stable-settled routes cover every graph-reachable permitted subset and ordering of ICP, ckBTC, and ckETH within the four-leg limit; unreachable endpoint/order combinations are reported as absent rather than synthesized.
- ckBTC and ckETH are rejected as successful route endpoints.
- Per-stable inventory floor/ceiling enforcement, including changed terminal token.
- Volatile exposure ceilings reject new exposure without triggering a sale of existing holdings.

### Retirement

- Zero arbitrage-engine calls to Rumi AMM, PartyDEX, and BOB arbitrage pools; this assertion does not classify the independently configured volume bot's existing Rumi AMM or BOB activity as arbitrage.
- Every legacy lettered automatic and manual executor fails before metadata, quote, approval, transfer, deposit, withdrawal, or swap calls from Stage 1 onward.
- Non-letter `manual_arb_cycle` and legacy `setup_approvals` fail closed in Stage 1 before any inter-canister call.
- Exhaustive Candid-update/timer inventory classifies every route-account mutator as fail-closed, global-lock-participating, read-only, or proven-account-disjoint; an unclassified mutator fails acceptance.
- Approval setup excludes retired integrations.
- The arbitrage drain functions and their scheduler call sites are absent.
- Manual external-venue withdrawal remains isolated and cannot initiate a market swap.
- Volume-bot recovery behavior is unchanged.
- Drain deletion occurs in Stage 1 before quote-only observation; no later stage may retain or reintroduce an automatic drain.

### Execution, settlement, and holding

- Full-fill rejection and refund accounting.
- Delayed settlement, timeout, and out-of-order response behavior.
- Trap/restart/upgrade at every persisted phase.
- Duplicate timer/callback idempotency.
- Persist-before-call intent and receipt tests prove `LegSubmitted` plus the complete immutable request fingerprint is durably written before the outbound call and cover acknowledged-then-trapped ledger transfers, lost DEX responses, duplicate callbacks, delayed refunds, and restarts after the ledger duplicate window; a resume from `LegSubmitted` reconciles only and an ambiguous leg is never resubmitted.
- Adapter-specific reconciliation fixtures prove the exact source-bound predicate for full fills, partial fills, refunds, lost responses, and delayed withdrawals; a coincident amount-only external credit cannot advance the route.
- Settlement timeout enters `ReconciliationRequired`, retains the global account-mutation lock, emits an operator-visible incident, and performs only bounded read-only evidence queries.
- Route operations and every Candid/timer/admin operation that could mutate a shared default account cannot overlap; adversarial tests cover generic withdrawal, volume administration, active manual pool tools, and retired-venue recovery during route settlement. Quote-only observation remains available.
- Legacy cycle-lock administration cannot release or overwrite a route reconciliation lock.
- Downstream quote deterioration and `HeldInventory` transition after exact reconciliation.
- Full refund/no-position failures transition to `Aborted`, with unavoidable fees reported accurately.
- Held positions preserve native amounts, tagged stable-par or ICP-native principal, route attribution, settled legs, and failure reason across restart and upgrade.
- Held ICP, ckBTC, and ckETH remain untouched across later arb cycles; no ceiling, timer, or scheduler event automatically sells them.
- Held stable, ICP, ckBTC, and ckETH lots create durable per-asset reservations; after restart or upgrade, unrelated routes, volume operations, withdrawals, and admin tools may fund only from `ledger balance - held reservations - active reservations`.
- Reservation underflow, attribution mismatch, or insufficient unencumbered balance fails closed without releasing or spending a held lot.
- Profit-preserving minimum-output proofs.
- Planned-versus-realized P&L from attributable ledger deltas.
- Global route lock, per-book scheduling, and canonical-cycle collision prevention.
- Candidate, execution, and held-position queries enforce cursor pagination and the page-size ceiling.
- Held positions and execution evidence use dedicated bounded stable structures; each execution record is at most 65,536 encoded bytes with at most 64 evidence items, the terminal log contains at most 10,000 records, and capacity is reserved before submission.
- Reaching any held-position, terminal-history, or record-size ceiling prevents a new route without deleting, coalescing unrelated records, or liquidating existing holdings; quote-only observation and existing reconciliation remain available.

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
- changes to the volume bot's economic routing, sizing, recovery semantics, or subaccount ownership; the global account-mutation lock is the sole scheduling integration added by this design;
- concurrent live route execution;
- automatic draining, liquidation, or retry of held ICP, ckBTC, or ckETH;
- ckBTC-returning or ckETH-returning profit books;
- an ICP/USD oracle or cross-profit-domain optimizer.
