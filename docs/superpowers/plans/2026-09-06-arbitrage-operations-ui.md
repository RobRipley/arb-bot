# Arbitrage Operations UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reorganize the dashboard around operator decisions, make all runtime state truthful, and let each bundled arbitrage transaction expand into its ordered execution legs.

**Architecture:** Add a bounded, additive `RouteExecutionDetailV1` read model persisted from the durable route runtime, then build the Cockpit, Markets, Ops, Ledger, and Diagnostics views on explicit per-source load states. Keep old APIs and legacy ledger history intact while presenting new route executions through stable execution identities.

**Tech Stack:** Rust, Internet Computer CDK and stable structures, Candid, embedded HTML/CSS/JavaScript, Node VM behavior tests, Cargo integration tests.

**Spec:** `docs/superpowers/specs/2026-09-06-arbitrage-operations-ui.md`

## Global Constraints

- Preserve stable-memory IDs 0 through 26; allocate a new ID for execution detail.
- Preserve existing route query signatures and add a versioned detail query.
- Keep automatic trading state unchanged during UI development, tests, review, and deployment.
- Display unavailable data as unavailable; never substitute an empty healthy state for a failed query.
- Use client, associate, and admin in visible copy; use owner only for source/API terminology.
- Never add `@rollup/rollup-darwin-*` to a package manifest.
- A quoted opportunity is an estimate; only reconciled terminal movement may be called realized profit.
- Each task receives focused tests and its own reviewable commit.

---

## File map

- `src/arb_bot/src/route_arb.rs`: public route-execution detail types and validation constants.
- `src/arb_bot/src/route_runtime.rs`: construct and update exact per-leg detail from persisted requests and reconciled settlements.
- `src/arb_bot/src/state.rs`: bounded stable detail index at MemoryId 27 and query helpers.
- `src/arb_bot/src/lib.rs`: additive `get_route_execution_detail_v1` query.
- `src/arb_bot/arb_bot.did`: generated public Candid interface.
- `src/arb_bot/src/dashboard.html`: data-state model, navigation, Cockpit, Markets, Ops, Ledger disclosure, Diagnostics, accessibility, and responsive layout.
- `src/arb_bot/tests/route_execution_detail.rs`: detail construction, ordering, settlement accounting, terminal idempotence, and historical-unavailable behavior.
- `src/arb_bot/tests/route_storage.rs`: stable detail bounds and duplicate-write behavior.
- `src/arb_bot/tests/state_decode.rs`: unchanged-state regression coverage remains in the final acceptance gate.
- `src/arb_bot/tests/dashboard_route_ui.rs`: navigation ownership and static DOM contract.
- `scripts/test-dashboard-data-state.cjs`: failed/stale/unknown query behavior.
- `scripts/test-dashboard-ledger.cjs`: expandable two-to-six-leg and legacy ledger behavior.
- `scripts/test-dashboard-runtime.cjs`: Ops toggle location, state, confirmation, and error behavior.
- `scripts/check-route-arb-acceptance.sh`: focused UI behavior tests in the route acceptance gate.

---

### Task 1: Persist an exact route-execution detail read model

**Files:**
- Modify: `src/arb_bot/src/route_arb.rs:1348`
- Modify: `src/arb_bot/src/state.rs:1168`
- Modify: `src/arb_bot/src/route_runtime.rs:8`
- Create: `src/arb_bot/tests/route_execution_detail.rs`
- Modify: `src/arb_bot/tests/route_storage.rs`

**Interfaces:**
- Consumes: `RuntimeExecution`, `RuntimeRequest`, `RuntimeSettlement`, `RouteCandidateReportV1`, `ExecutionRecordV1`.
- Produces: `RuntimeLegTrace`, `RouteExecutionDetailV1`, `RouteExecutionLegV1`, `RouteExecutionLegStatusV1`, `put_route_execution_detail`, `get_route_execution_detail`, and `find_route_execution_record`.

- [ ] **Step 1: Write failing public-shape and ordering tests**

Add `src/arb_bot/tests/route_execution_detail.rs` with fixtures for a three-leg route. Assert that leg indices are `[0, 1, 2]`, the edge and asset transition come from the selected route, quote fields remain distinct from actual settlement fields, and realized profit exists only on the parent record.

```rust
#[test]
fn detail_keeps_variable_legs_in_route_order() {
    let detail = three_leg_execution_detail();
    assert_eq!(detail.legs.len(), 3);
    assert_eq!(detail.legs.iter().map(|leg| leg.leg_index).collect::<Vec<_>>(), vec![0, 1, 2]);
    assert_eq!(detail.legs[0].edge_id, "rumi-ckusdc-icusd");
    assert_eq!(detail.legs[1].quoted_output_native, Some(1_250_000));
    assert_eq!(detail.legs[1].actual_output_credit_native, Some(1_247_000));
    assert_ne!(detail.legs[1].quoted_output_native, detail.legs[1].actual_output_credit_native);
}
```

- [ ] **Step 2: Run the new test and verify the types are missing**

Run: `RUSTFLAGS=-Awarnings cargo test -p arb_bot --test route_execution_detail`

Expected: compilation fails because `RouteExecutionDetailV1` and its builder do not exist.

- [ ] **Step 3: Define the additive public types**

Add these versioned shapes in `route_arb.rs`, using existing `Asset`, `VenueKind`, `ExecutionRecordV1`, and `ReconciliationEvidenceV1` types:

```rust
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub enum RouteExecutionLegStatusV1 {
    Quoted,
    Prepared,
    Submitted,
    AwaitingSettlement,
    Settled,
    Refunded,
    RejectedBeforeDebit,
    ReconciliationRequired,
    HeldInventory,
    Aborted,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct RouteExecutionLegV1 {
    pub leg_index: u8,
    pub status: RouteExecutionLegStatusV1,
    pub edge_id: String,
    pub pool_id: String,
    pub pool_principal: Principal,
    pub venue: VenueKind,
    pub from: Asset,
    pub to: Asset,
    pub quoted_input_native: u64,
    pub quoted_output_native: Option<u64>,
    pub minimum_output_native: u64,
    pub input_fee_native: u64,
    pub output_fee_native: u64,
    pub actual_input_debit_native: Option<u64>,
    pub actual_effective_input_native: Option<u64>,
    pub actual_output_credit_native: Option<u64>,
    pub refund_credit_native: Option<u64>,
    pub prepared_at_ns: Option<u64>,
    pub submitted_at_ns: Option<u64>,
    pub settled_at_ns: Option<u64>,
    pub reconciled_at_ns: Option<u64>,
    pub evidence: Vec<ReconciliationEvidenceV1>,
    pub incident: Option<String>,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct RouteExecutionDetailV1 {
    pub record: ExecutionRecordV1,
    pub asset_path: Vec<Asset>,
    pub legs: Vec<RouteExecutionLegV1>,
    pub detail_available: bool,
}
```

- [ ] **Step 4: Add bounded stable storage at MemoryId 27**

Add `json_storable!(RouteExecutionDetailV1)`, document MemoryId 27, and initialize:

```rust
static ROUTE_EXECUTION_DETAILS: RefCell<StableBTreeMap<String, crate::route_arb::RouteExecutionDetailV1, Memory>> =
    RefCell::new(StableBTreeMap::init(
        MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(27))),
    ));
```

Implement `validate_route_execution_detail()` with exact limits: execution ID and text validators already used by `ExecutionRecordV1`, 1–6 legs, strictly ascending unique indices starting at zero, at most 64 total evidence records, and at most 65,536 JSON bytes. `put_route_execution_detail()` must reject a changed terminal record for an existing execution ID and accept an identical retry. `get_route_execution_detail()` returns `Result<Option<RouteExecutionDetailV1>, String>` so storage failures remain distinct from a missing detail row. `find_route_execution_record()` returns the same `Result<Option<_>, String>` shape, checks the current slot, then scans the bounded terminal log newest-first; the existing 10,000-record terminal cap bounds this historical fallback.

- [ ] **Step 5: Add an upgrade-safe internal leg trace**

Add this internal runtime type and field. An execution recovered from the already-deployed pre-detail runtime has an empty trace and therefore returns `detail_available = false`; it must not infer actual per-leg values.

```rust
#[derive(Deserialize, Serialize, Clone, Debug)]
struct RuntimeLegTrace {
    leg_index: u8,
    edge: RuntimeEdge,
    quoted_input_native: u64,
    quoted_output_native: Option<u64>,
    minimum_output_native: u64,
    input_fee_native: u64,
    output_fee_native: u64,
    submitted_at_ns: Option<u64>,
    settled_at_ns: Option<u64>,
    reconciled_at_ns: Option<u64>,
    settlement: Option<RuntimeSettlement>,
    status: RouteExecutionLegStatusV1,
    incident: Option<String>,
}

pub struct RuntimeExecution {
    pub record: ExecutionRecordV1,
    pub generation: u64,
    pub original: RouteCandidateReportV1,
    pub minimum_final_native: u64,
    pub current_wallet_native: u64,
    pub request: Option<RuntimeRequest>,
    pub intent: Option<AdapterIntent>,
    pub settlements: Vec<RuntimeSettlement>,
    pub submitted_intents: Vec<(RuntimeRequest, AdapterIntent)>,
    pub realized_profit: Option<i128>,
    #[serde(default)]
    leg_traces: Vec<RuntimeLegTrace>,
}
```

- [ ] **Step 6: Persist detail at each durable execution transition**

Add this exact resolver to `src/arb_bot/src/route_arb.rs`; it uses the existing public `directed_edges()` registry and verifies every transition:

```rust
pub fn resolve_route_edges(
    edge_ids: &[String],
    asset_path: &[Asset],
) -> Result<Vec<DirectedEdge>, String> {
    if asset_path.len() != edge_ids.len() + 1 {
        return Err("route asset path does not match edge count".into());
    }
    let registry = directed_edges();
    edge_ids.iter().enumerate().map(|(index, edge_id)| {
        let edge = registry.iter().find(|edge| edge.edge_id == *edge_id)
            .cloned().ok_or_else(|| format!("unknown route edge: {edge_id}"))?;
        if edge.from != asset_path[index] || edge.to != asset_path[index + 1] {
            return Err(format!("route edge does not match asset path at leg {index}"));
        }
        Ok(edge)
    }).collect()
}
```

At execution creation, call `route_arb::resolve_route_edges(&original.venue_edges, &original.asset_path)` and create one `RuntimeLegTrace` per quoted leg. This snapshots `RuntimeEdge` metadata before any submission, so later quoted or prepared legs still have exact venue, pool, and asset fields if the registry changes. In `route_execution_detail.rs`, assert unknown edge IDs and a reversed asset transition are rejected before execution persistence or outbound calls.

Build public detail only from `RuntimeExecution.original`, `submitted_intents`, and `leg_traces`. Map `QuoteLegReportV1.gross_output` into `quoted_output_native` with checked `u128` to `u64` conversion. Match submitted requests and traces by `leg_index`; never align them by vector position. Update the affected trace before every durable transition:

- `LegPrepared`: status Prepared with the exact request values.
- accepted or unknown submission: status Submitted or ReconciliationRequired with `submitted_at_ns` captured before the outbound call.
- rejected before debit: status RejectedBeforeDebit and the typed reason, followed by Aborted only after terminal persistence.
- awaiting evidence: status AwaitingSettlement; an ambiguous response never clears the submitted timestamp or request identity.
- accepted settlement: store `RuntimeSettlement`, status Settled or Refunded, `settled_at_ns` from the latest evidence, and `reconciled_at_ns` when reconciliation supplied the proof.
- partial or unsafe terminal inventory: status HeldInventory with the attributable incident.
- terminal failure before movement: status Aborted with the attributable incident.

Write detail immediately after persisting each runtime phase and before releasing a terminal lock. If the detail write fails, return the persistence error and retain the current execution and lock.

- [ ] **Step 7: Add compatibility and capacity tests**

In `route_storage.rs`, assert a 65,537-byte encoded detail is rejected, duplicate terminal detail is idempotent, changed terminal detail is rejected, and initializing the new MemoryId 27 map over a pre-detail stable layout yields an empty map without changing existing execution records.

- [ ] **Step 8: Run focused backend tests**

Run:

```bash
RUSTFLAGS=-Awarnings cargo test -p arb_bot --test route_execution_detail
RUSTFLAGS=-Awarnings cargo test -p arb_bot --test route_storage
RUSTFLAGS=-Awarnings cargo test -p arb_bot --lib route_runtime
```

Expected: all tests pass with no lock-release or duplicate-terminal regression.

- [ ] **Step 9: Commit the durable read model**

```bash
git add src/arb_bot/src/route_arb.rs src/arb_bot/src/route_runtime.rs src/arb_bot/src/state.rs src/arb_bot/tests/route_execution_detail.rs src/arb_bot/tests/route_storage.rs
git commit -m "feat(arb-bot): persist route execution leg detail"
```

---

### Task 2: Expose the execution detail through an additive query

**Files:**
- Modify: `src/arb_bot/src/lib.rs:468`
- Modify: `src/arb_bot/arb_bot.did`
- Modify: `src/arb_bot/src/dashboard.html:847`
- Modify: `scripts/check-route-arb-acceptance.sh`

**Interfaces:**
- Consumes: `state::get_route_execution_detail(execution_id: &str) -> Result<Option<RouteExecutionDetailV1>, String>` and `state::find_route_execution_record(execution_id: &str) -> Result<Option<ExecutionRecordV1>, String>`.
- Produces: `get_route_execution_detail_v1(execution_id: String) -> Result<RouteExecutionDetailV1, String>`.

- [ ] **Step 1: Add a failing Candid/API assertion**

Extend the Candid integration test to require `get_route_execution_detail_v1` and require its result to contain `detail_available`, `record`, `asset_path`, and `legs`.

- [ ] **Step 2: Add the query without changing existing endpoints**

```rust
#[query]
fn get_route_execution_detail_v1(
    execution_id: String,
) -> Result<route_arb::RouteExecutionDetailV1, String> {
    if let Some(detail) = state::get_route_execution_detail(&execution_id)? {
        return Ok(detail);
    }
    let record = state::find_route_execution_record(&execution_id)?
        .ok_or_else(|| format!("unknown route execution: {execution_id}"))?;
    Ok(route_arb::RouteExecutionDetailV1 {
        record,
        asset_path: vec![],
        legs: vec![],
        detail_available: false,
    })
}
```

For a known historical `ExecutionRecordV1` without detail, return `Ok(RouteExecutionDetailV1 { record, asset_path: vec![], legs: vec![], detail_available: false })`; reserve `Err` for unknown IDs or storage failures.

- [ ] **Step 3: Regenerate the Candid interface**

Run the repository’s Candid generator used by `scripts/check-candid.sh`; update the dashboard IDL with the exact generated record and variant field names. Do not hand-invent a shape that differs from Rust.

- [ ] **Step 4: Verify additive compatibility**

Run:

```bash
RUSTFLAGS=-Awarnings cargo test -p arb_bot --test candid
bash scripts/check-candid.sh
```

Expected: generated drift, structural guards, and deployed-interface subtyping all pass.

- [ ] **Step 5: Commit the query**

```bash
git add src/arb_bot/src/lib.rs src/arb_bot/arb_bot.did src/arb_bot/src/dashboard.html scripts/check-route-arb-acceptance.sh
git commit -m "feat(arb-bot): expose route execution details"
```

---

### Task 3: Introduce truthful per-source loading and freshness states

**Files:**
- Modify: `src/arb_bot/src/dashboard.html:1410`
- Create: `scripts/test-dashboard-data-state.cjs`
- Modify: `scripts/test-dashboard-health.cjs`
- Modify: `scripts/check-route-arb-acceptance.sh`

**Interfaces:**
- Produces: `createSourceState()`, `markSourceFresh()`, `markSourceFailed()`, `markSourceUnavailable()`, `sourceDisplayState()`, and `routeSources`.
- Consumed by: Cockpit, Markets, Ledger, and Diagnostics tasks.

- [ ] **Step 1: Write failing failed-query tests**

Cover route lock, current execution, reservations, held positions, runtime, wallet, and health. Assert that a rejected query with a previous value yields `stale` with the value and timestamp retained; a rejected first load yields `failed`; an interface method absent from the deployed service yields `unavailable`. None of these cases may render “clear,” “none,” zero, or healthy.

```javascript
const source = createSourceState();
markSourceFresh(source, [{ execution_id: 'exec-1' }], 1000);
markSourceFailed(source, Error('query rejected'), 2000);
assert.equal(source.status, 'stale');
assert.equal(source.value[0].execution_id, 'exec-1');
assert.equal(source.lastSuccessMs, 1000);
assert.equal(source.lastAttemptMs, 2000);
```

- [ ] **Step 2: Implement a single explicit source-state model**

```javascript
function createSourceState() {
  return { status: 'loading', value: null, error: null, lastSuccessMs: null, lastAttemptMs: null };
}
function markSourceFresh(source, value, nowMs = Date.now()) {
  Object.assign(source, { status: 'fresh', value, error: null, lastSuccessMs: nowMs, lastAttemptMs: nowMs });
}
function markSourceFailed(source, error, nowMs = Date.now()) {
  Object.assign(source, {
    status: source.lastSuccessMs == null ? 'failed' : 'stale',
    error: error && error.message ? error.message : String(error),
    lastAttemptMs: nowMs,
  });
}
function markSourceUnavailable(source, reason, nowMs = Date.now()) {
  Object.assign(source, { status: 'unavailable', value: null, error: String(reason), lastAttemptMs: nowMs });
}
function sourceDisplayState(source, staleAfterMs, nowMs = Date.now()) {
  if (source.status === 'fresh' && source.lastSuccessMs != null
      && nowMs - source.lastSuccessMs >= staleAfterMs) return 'stale';
  return source.status;
}
```

Replace `safe(fn, fallback, label)` in `loadRouteData()` with updates to named source objects. A rejected query with cached data becomes stale; a rejected first load becomes failed; a method absent from the deployed interface becomes unavailable. Keep cached values only with the explicit stale label.

- [ ] **Step 3: Tie staleness to source-specific thresholds**

Define `ROUTE_RUNTIME_STALE_AFTER_MS = 30_000`, which gives the 10-second runtime timer three intervals of grace. Route quotes use the configured `max_quote_age_ns` converted to milliseconds. Do not use one global “Live” badge for unrelated sources.

- [ ] **Step 4: Run behavior tests**

Run:

```bash
node scripts/test-dashboard-data-state.cjs
node scripts/test-dashboard-health.cjs
node scripts/test-dashboard-runtime.cjs
```

Expected: all pass, including healthy → stale → recovered, rejected first-load → failed, and absent method → unavailable cases.

- [ ] **Step 5: Commit truthful data states**

```bash
git add src/arb_bot/src/dashboard.html scripts/test-dashboard-data-state.cjs scripts/test-dashboard-health.cjs scripts/check-route-arb-acceptance.sh
git commit -m "fix(dashboard): preserve unknown and stale states"
```

---

### Task 4: Reorganize navigation and establish the Cockpit hierarchy

**Files:**
- Modify: `src/arb_bot/src/dashboard.html:694`
- Modify: `src/arb_bot/src/dashboard.html:1690`
- Modify: `src/arb_bot/src/dashboard.html:2547`
- Modify: `src/arb_bot/tests/dashboard_route_ui.rs`

**Interfaces:**
- Consumes: named source states from Task 3 and existing runtime/health data.
- Produces: `renderCockpit()`, `renderMarkets()`, `renderOps()`, `renderLedger()`, and `renderDiagnostics()` with unique content ownership.

- [ ] **Step 1: Write failing navigation ownership tests**

Assert exactly five primary view IDs—`cockpit`, `markets`, `ops`, `ledger`, `diagnostics`—and one mount per view. Assert Charts content appears under Markets and Money’s wallet-readiness content remains reachable. Assert the automatic-arbitrage control exists only in Ops.

- [ ] **Step 2: Replace the primary navigation map**

```javascript
const VIEWS = [
  { id: 'cockpit', label: 'Cockpit' },
  { id: 'markets', label: 'Markets' },
  { id: 'ops', label: 'Ops' },
  { id: 'ledger', label: 'Ledger' },
  { id: 'diagnostics', label: 'Diagnostics' },
];
```

Move existing chart markup into a Markets subsection and wallet readiness into Markets. Move operational volume actions from Money into Ops. Preserve loaders by invoking them from their new owning view.

- [ ] **Step 3: Build the Cockpit’s information order**

Render, in order: bot state and heartbeat; current phase and leg progress; today’s realized profit/completed/failed counts; latest completed execution; actionable incidents. Link incidents to `goTo('ops')` or `goTo('diagnostics')` with a section anchor.

- [ ] **Step 4: Test anonymous, admin, fresh, stale, blocked, and unknown rendering**

Add static DOM markers plus JS behavior cases. Status text must distinguish Stopped, Scanning, Executing, Confirming, Reconciling, Blocked, and Unknown without relying on badge color.

- [ ] **Step 5: Run focused UI tests and commit**

```bash
RUSTFLAGS=-Awarnings cargo test -p arb_bot --test dashboard_route_ui
node scripts/test-dashboard-data-state.cjs
git add src/arb_bot/src/dashboard.html src/arb_bot/tests/dashboard_route_ui.rs
git commit -m "feat(dashboard): organize the arbitrage cockpit"
```

---

### Task 5: Put automation controls in Ops with accessible state semantics

**Files:**
- Modify: `src/arb_bot/src/dashboard.html:2444`
- Modify: `src/arb_bot/src/dashboard.html:3616`
- Modify: `scripts/test-dashboard-runtime.cjs`
- Modify: `src/arb_bot/tests/dashboard_route_ui.rs`

**Interfaces:**
- Consumes: runtime source state and existing `setRouteTrading(enabled)` mutation sequence.
- Produces: `routeAutomationState()` and an Ops-only native control.

- [ ] **Step 1: Write failing placement and accessibility tests**

Assert that the route panel contains status and a link to Ops but no mutation button. Assert Ops contains a native `<button>` with `aria-pressed`, the labels On/Off/Applying/Blocked/Unknown, and the existing confirmation copy.

- [ ] **Step 2: Reuse the Ops lever state machine without optimistic updates**

Derive On only from `live_authorized && enabled && !dry_run`. Set local state to Applying during the request, reload authoritative runtime status after success or failure, and leave first-load query failure as Unknown. Preserve the enable order: save `{ enabled: true, dry_run: false }`, then authorize. Preserve stop semantics: authorization false prevents new trades while settlement reconciliation continues.

- [ ] **Step 3: Replace clickable-div switches with native controls**

Use a native button for both route and volume controls:

```html
<button type="button" class="lever-toggle" aria-pressed="true" aria-label="Automatic arbitrage on">
  <span aria-hidden="true" class="toggle on"></span>
  <span>On</span>
</button>
```

Disable the button while Applying. Provide visible focus styling and state text.

- [ ] **Step 4: Run runtime and navigation tests**

```bash
node scripts/test-dashboard-runtime.cjs
RUSTFLAGS=-Awarnings cargo test -p arb_bot --test dashboard_route_ui
```

Expected: admin-only controls, no mutation before confirmation, configuration failure cannot authorize, stop only revokes authorization, and runtime query failure renders Unknown.

- [ ] **Step 5: Commit Ops controls**

```bash
git add src/arb_bot/src/dashboard.html scripts/test-dashboard-runtime.cjs src/arb_bot/tests/dashboard_route_ui.rs
git commit -m "feat(dashboard): move automation controls into ops"
```

---

### Task 6: Make Markets distinguish quote estimates and automate manual scan progress

**Files:**
- Modify: `src/arb_bot/src/dashboard.html:2380`
- Create: `scripts/test-dashboard-observation.cjs`
- Modify: `scripts/check-route-arb-acceptance.sh`

**Interfaces:**
- Consumes: `start_route_observation_v1`, `quote_route_observation_batch_v1(cursor, limit)`, observation counters, and configured quote-age limit.
- Produces: `runManualQuoteScan()`, `observationProgressHtml()`, and quote freshness labels.

- [ ] **Step 1: Write failing quote-label and scan-flow tests**

Cover a fresh positive quote, stale quote, no realized result, complete scan, mid-scan failure, and double-click while a batch is in flight. Assert quote cards say Estimated and never Realized.

- [ ] **Step 2: Replace two manual buttons with one bounded workflow**

`runManualQuoteScan()` starts a new observation, reloads it, and repeatedly calls the existing batch endpoint with a limit of 100 until `scan_complete`. Before each batch, verify the active view and observation ID still match. On failure, retain the current cursor and show Resume manual scan; do not restart.

- [ ] **Step 3: Render explicit progress and freshness**

Show `candidates_evaluated / total_work_items`, `quote_calls_made / required_quote_calls`, current state Ready/Running/Complete/Failed, observation time, and the stale reason. Put raw cursor and batch count in Diagnostics.

- [ ] **Step 4: Run behavior tests and commit**

```bash
node scripts/test-dashboard-observation.cjs
RUSTFLAGS=-Awarnings cargo test -p arb_bot --test dashboard_route_ui
git add src/arb_bot/src/dashboard.html scripts/test-dashboard-observation.cjs scripts/check-route-arb-acceptance.sh
git commit -m "feat(dashboard): clarify quotes and manual scans"
```

---

### Task 7: Add expandable, variable-length ledger transactions

**Files:**
- Modify: `src/arb_bot/src/dashboard.html:4036`
- Create: `scripts/test-dashboard-ledger.cjs`
- Modify: `src/arb_bot/tests/dashboard_route_ui.rs`
- Modify: `scripts/check-route-arb-acceptance.sh`

**Interfaces:**
- Consumes: terminal `ExecutionRecordV1` rows and `get_route_execution_detail_v1(execution_id)` from Task 2.
- Produces: `routeLedgerEntryHtml()`, `routeLegDetailHtml()`, `toggleLedgerExecution(executionId)`, and accessible disclosure state.

- [ ] **Step 1: Write failing two-, three-, four-, and six-leg tests**

For each count, assert one parent P&L cell, exact leg order, `Leg N of M` labels, no child P&L summation, and disclosure state. Add cases for historical detail unavailable, failed detail query, refund, rejected-before-debit, and reconciliation required.

- [ ] **Step 2: Add route transactions as the primary ledger dataset**

Render terminal route executions using their stable `execution_id`. Keep legacy `TradeLeg` groups in a clearly labeled Legacy history subsection with their existing pagination. Do not globally sort two independently paginated logs or infer a new route from adjacent legacy legs.

- [ ] **Step 3: Implement lazy disclosure loading**

The parent row button sets `aria-expanded`, retains focus, and fetches detail once per execution ID. While loading, render Loading leg details. On `detail_available=false`, render “Detailed legs were not recorded for this historical execution.” On query failure, render Unavailable with Retry; never render an empty successful expansion.

- [ ] **Step 4: Render exact per-leg fields**

Render quote, request, actual settlement, refund, timestamp, incident, and evidence groups separately. Use native token units and symbols consistently. Label missing actual values Awaiting settlement or Unavailable according to leg status.

- [ ] **Step 5: Make the disclosure responsive and accessible**

Use a native button, `aria-controls="ledger-detail-${executionId}"`, `scope="col"` headers, and screen-reader text for Leg N of M. Below the mobile breakpoint, stack fields in the same order rather than hiding columns.

- [ ] **Step 6: Run ledger and route UI tests**

```bash
node scripts/test-dashboard-ledger.cjs
RUSTFLAGS=-Awarnings cargo test -p arb_bot --test dashboard_route_ui
```

Expected: all variable-length, unavailable, failure, legacy, keyboard, and P&L cases pass.

- [ ] **Step 7: Commit ledger drill-down**

```bash
git add src/arb_bot/src/dashboard.html scripts/test-dashboard-ledger.cjs src/arb_bot/tests/dashboard_route_ui.rs scripts/check-route-arb-acceptance.sh
git commit -m "feat(dashboard): expand arbitrage transactions into legs"
```

---

### Task 8: Move operational internals into Diagnostics and verify the complete experience

**Files:**
- Modify: `src/arb_bot/src/dashboard.html:2434`
- Modify: `src/arb_bot/tests/dashboard_route_ui.rs`
- Modify: `scripts/check-route-arb-acceptance.sh`
- Modify: `docs/execution/2026-09-05-runtime-executor-contract.md`

**Interfaces:**
- Consumes: truthful source states, execution details, locks, reservations, held positions, observations, and legacy disclosures.
- Produces: Diagnostics view and complete release evidence.

- [ ] **Step 1: Write failing Diagnostics ownership tests**

Assert that mutation lock, reservation IDs, held-position IDs, evidence references, observation cursors, quote-call counts, raw execution IDs, and legacy A-S/T disclosure occur in Diagnostics. Assert Cockpit contains only actionable summaries and links.

- [ ] **Step 2: Render Diagnostics with explicit unavailable states**

Group sections as Runtime, Execution and settlement, Reservations and held inventory, Observation internals, and Legacy state. Each group displays its own freshness and failure state.

- [ ] **Step 3: Run the full route-arbitrage acceptance gate**

Run:

```bash
bash scripts/check-route-arb-acceptance.sh
node scripts/test-dashboard-data-state.cjs
node scripts/test-dashboard-observation.cjs
node scripts/test-dashboard-ledger.cjs
git diff --check
```

Expected: all Rust, Candid, stage-retirement, call-target, dashboard behavior—including all three new behavior suites—JavaScript syntax, and release Wasm checks pass; `git diff --check` prints nothing. Also verify that `scripts/check-route-arb-acceptance.sh` itself invokes the three new scripts so CI and local execution use the same gate.

- [ ] **Step 4: Perform two independent review passes**

One reviewer checks stable-memory/API compatibility, per-leg financial truth, and absence of execution-state mutations. A second reviewer checks keyboard behavior, unavailable/stale semantics, responsive information order, and that every prior feature remains reachable. Fix concrete findings and rerun only affected focused tests plus the final acceptance gate after the last source change.

- [ ] **Step 5: Update the executor contract with the proven UI state**

Record exact test output, Wasm hash and size, Candid compatibility result, historical-detail limitation, and proof boundary. Do not call source tests evidence of a deployed UI or live settlement.

- [ ] **Step 6: Commit the final organization and evidence**

```bash
git add src/arb_bot/src/dashboard.html src/arb_bot/tests/dashboard_route_ui.rs scripts/check-route-arb-acceptance.sh docs/execution/2026-09-05-runtime-executor-contract.md
git commit -m "feat(dashboard): add arbitrage diagnostics workspace"
```

---

## Delivery sequence

1. Push the reviewed branch and open one PR whose commits preserve the task boundaries above.
2. Run required remote checks once after the final commit; do not repeatedly spend CI on unchanged code.
3. Merge only when local acceptance, Candid compatibility, both independent reviews, and required remote checks are green.
4. Stop after source merge and report the merged commit plus exact local and remote checks. This implementation plan does not itself authorize a production upgrade.
5. If the user separately requests release, build the artifact from the exact merged tree and record its SHA-256.
6. Before an authorized upgrade, read current execution, mutation lock, runtime authorization, volume state, and module hash. The UI work must not alter trading configuration.
7. Upgrade only from a clear execution/lock state, then verify module hash, runtime gates, dashboard bytes, route configuration, volume configuration, and current execution state.
8. Record deployment evidence in a readable pushed artifact. Deployment does not establish a successful live trade; ledger drill-down becomes live-proven only after an actual terminal execution supplies detail.
