# Arbitrage Operations UI Design

## Goal

Make the dashboard answer three operator questions immediately: is the bot working, what is it doing or waiting for, and what has it earned? Preserve detailed diagnostic evidence without making backend implementation details the primary navigation.

## Information architecture

The primary navigation is Cockpit, Markets, Ops, Ledger, and Diagnostics.

- **Cockpit** is the default operational overview. It shows bot state, current phase, heartbeat freshness, realized results, the latest completed executions, and actionable incidents.
- **Markets** contains quoted opportunities, manual quote observation, quote progress, wallet readiness, and the existing charts. Quoted values are always labeled estimated and carry an explicit freshness state.
- **Ops** contains automatic-arbitrage and volume-engine controls, risk limits, pool configuration, administrators, and recovery actions. Automatic arbitrage is the first control and uses the same control language as the volume engine.
- **Ledger** contains financial history. A bundled arbitrage transaction expands into an ordered, variable-length list of its legs. New route executions and historical legacy trades are identified honestly; the UI does not infer a three-or-more-leg route from adjacent legacy rows.
- **Diagnostics** contains mutation locks, reservations, held-position identifiers, raw execution evidence, observation counters, and legacy A-S/T disclosures.

The existing Charts content moves under Markets. Wallet and funding readiness from Money moves under Markets; operational volume actions and configuration move under Ops. No existing reachable feature may silently disappear during the navigation change.

## Truthful state model

Every independently loaded source has one of five states: `loading`, `fresh`, `stale`, `failed`, or `unavailable`. A failed query may retain the last successful value only when the UI labels it stale and displays its last-success timestamp. A failure must never become an empty collection that renders as “clear,” “none,” zero, or healthy.

The automatic-arbitrage control has five visible states: On, Off, Applying, Blocked, and Unknown. “On” means all three backend gates are true: `live_authorized`, `enabled`, and `dry_run == false`. Runtime query failure is Unknown. A stale heartbeat while the gates are on is Blocked. The control remains admin-only, requires the existing confirmation before enabling, and never changes optimistically.

The Cockpit execution state uses concrete phases: Scanning, Evaluating, Submitting leg N of M, Confirming settlement, Reconciling, Blocked, Stopped, and Unknown. It displays `last_tick_ns` as an age. The runtime timer fires every 10 seconds, so an enabled bot with no successful tick for 30 seconds is stale and displays Blocked rather than healthy.

## Quotes and observations

Candidate profit is labeled “Estimated profit” or “Quoted spread.” Realized profit is labeled “Realized result.” Quote age is visible next to the estimate and becomes stale according to the configured quote-age limit rather than an unrelated UI constant.

Manual observation is one operator action: “Run manual quote scan.” The UI automatically advances bounded batches until the scan completes, the user leaves the page, or a request fails. It shows `candidates_evaluated / total_work_items` and `quote_calls_made / required_quote_calls`. Backend batch size remains bounded; cursor and batch controls are available only in Diagnostics.

## Ledger transaction drill-down

Each automatic-arbitrage transaction is a compact parent row showing completion time, route, total input, terminal status, and aggregate realized profit. The row uses a native disclosure button with `aria-expanded` and `aria-controls`.

Expanding a row shows an ordered list of any number of legs. Each leg displays:

- leg N of M and status;
- venue, pool, edge identity, and from/to assets;
- quoted input, quoted output, minimum accepted output, and ledger fees;
- actual input debit, effective input, actual output credit, and refund credit;
- prepared, submitted, settled, or reconciled timestamps;
- source-bound evidence references with copy affordances;
- the incident on the affected leg, including rejected-before-debit, awaiting settlement, reconciliation required, partial/held inventory, aborted, and refunded outcomes.

Actual values are never reconstructed from quote values. “No evidence yet,” “evidence unavailable for historical record,” and “no evidence required” are distinct states.

Legacy `TradeLeg` rows remain available and may disclose the individual legacy legs already present in the bundle. They are labeled Legacy because their adjacency-based grouping cannot establish arbitrary route identity. New route executions use an execution ID and durable per-leg detail.

## Route execution detail API

Existing route query methods remain unchanged. Add an additive query:

```candid
get_route_execution_detail_v1 : (text) -> (variant {
  Ok : RouteExecutionDetailV1;
  Err : text;
}) query;
```

`RouteExecutionDetailV1` contains the existing `ExecutionRecordV1`, route asset path, and an ordered vector of `RouteExecutionLegV1`. Each leg separates quote, request, settlement, and evidence fields. Detail is persisted during execution because completed observations can rotate or disappear. Historical execution records created before this feature return a clear `detail_available = false` result.

Use a new stable-memory ID; never reuse IDs 0 through 26. Detail writes are bounded to 65,536 encoded bytes, at most six legs, and at most 64 evidence records. The detail write is idempotent by execution ID and terminal phase. A detail persistence failure retains the execution lock and cannot fabricate a completed ledger entry.

## Accessibility and responsive behavior

Critical toggles are native buttons with `aria-pressed` or native checkbox/switch semantics. Every control is keyboard-operable, has a visible focus state, and communicates status in text rather than color alone. Disclosure buttons retain focus after expansion. On narrow screens, leg fields stack in their desktop reading order and retain the “Leg N of M” label.

## Acceptance criteria

1. A failed route lock, execution, reservation, held-position, health, balance, or ledger query cannot render a healthy empty state.
2. Cockpit distinguishes stopped, scanning, executing, reconciling, blocked, stale, and unknown.
3. Ops contains the sole primary automatic-arbitrage control, consistent with volume controls.
4. Markets presents quoted results as estimates and manual observation as one progress-tracked action.
5. Ledger displays terminal route executions and expands two-, three-, four-, five-, or six-leg transactions in exact order without double-counting P&L.
6. Historical records without detail say detail is unavailable; they never show inferred actual settlement.
7. Diagnostics retains locks, reservations, held positions, evidence, cursors, raw IDs, and legacy disclosures.
8. Admin, anonymous, loading, failed, stale, and narrow-screen states have deterministic test coverage.
9. Existing route execution, settlement, stable-state decode, Candid compatibility, and full route-arbitrage acceptance checks remain green.
