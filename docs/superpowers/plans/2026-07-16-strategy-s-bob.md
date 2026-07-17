# Strategy S (icUSD/BOB) + ICP Inventory Band — Implementation Plan

> **For agentic workers:** Executed by orchestrated subagents, one per task group. Steps use checkbox (`- [ ]`) syntax for tracking. Read the spec at `docs/superpowers/specs/2026-07-16-strategy-s-bob-design.md` first.

**Goal:** Add a configurable ICP inventory band (PR1) and strategy S — icUSD/BOB triangular arb on ICPSwap with band-based execution, USD-pinned accounting, and dry-run-first gating (PR2).

**Architecture:** Follow the existing pattern-per-strategy design in `src/arb_bot/src/arb.rs` (no trait; data structs + match dispatch). PR1 turns the hardcoded `ICP_RESERVE` into a floor/ceiling band read from `BotConfig`. PR2 adds a new `BobTarget` evaluator/executor pair where BOB is the transit asset (the role ICP plays in strategies A–R) and endpoints are icUSD and banded ICP inventory.

**Tech Stack:** Rust IC canister (ic-cdk), hand-maintained candid (`src/arb_bot/arb_bot.did` + IDL block in `src/arb_bot/src/dashboard.html`), guarded by `scripts/check-candid.sh`. No CI. Wasm build: see `reference_arb_bot_repo_facts` memory / README for the exact command.

## Global Constraints

- Every new `BotConfig` / `BotState` / `CycleSnapshot` field MUST have `#[serde(default)]` or `#[serde(default = "fn")]` — state is CBOR-decoded across upgrades and old snapshots live in StableLogs. A missing default bricks the upgrade.
- Candid triple-sync: any signature/type change must be applied to `lib.rs`, `arb_bot.did`, AND the IDL block in `dashboard.html`; `scripts/check-candid.sh` must pass.
- All admin endpoints call `require_admin()`.
- All USD amounts are 6-decimal i64/u64 (`_usd` suffix); ICP/BOB amounts are 8-decimal e8s.
- Strategy S must be inert unless BOTH `icpswap_icusd_bob_pool` and `icpswap_bob_icp_pool` are non-anonymous; execution additionally requires `bob_execution_enabled == true`.
- Existing strategies A–R behavior must be byte-for-byte preserved except the drain reserve change specified in Task 1.2.
- Verified mainnet facts to hardcode as defaults: BOB ledger `7pail-xaaaa-aaaas-aabmq-cai` (fee `1_000_000`, 8 dec); ICPSwap BOB/ICP pool `ybilh-nqaaa-aaaag-qkhzq-cai` (fee 3000 pips, token0 = BOB, token1 = ICP).
- Commit messages follow repo style: `feat(arb): ...` / `chore(arb): ...`, with the Claude Code co-author trailer.

---

## PR1 — Configurable ICP inventory band (branch `feat/icp-inventory-band`, base `main`)

### Task 1.1: Band config fields + setter

**Files:**
- Modify: `src/arb_bot/src/state.rs` (BotConfig ~line 40–114; Default impl ~line 466+; default fns ~line 18–33)
- Modify: `src/arb_bot/src/lib.rs` (admin setters — follow `set_arb_interval_secs` at lib.rs:1789)
- Modify: `src/arb_bot/arb_bot.did`, `src/arb_bot/src/dashboard.html` (IDL block + config display)

**Interfaces:**
- Produces: `config.icp_inventory_floor_e8s: u64` (default `200_000_000`), `config.icp_inventory_ceiling_e8s: u64` (default `2_000_000_000`), update method `set_icp_inventory_band(floor_e8s: u64, ceiling_e8s: u64) -> Result<(), String>`.

- [ ] Add to `BotConfig`:

```rust
/// ICP inventory band (e8s). Floor: minimum working balance the drain
/// always leaves (fee buffer + strategy-S top-up trigger). Ceiling: the
/// drain skims any balance above this to the best stable pool.
#[serde(default = "default_icp_inventory_floor")]
pub icp_inventory_floor_e8s: u64,
#[serde(default = "default_icp_inventory_ceiling")]
pub icp_inventory_ceiling_e8s: u64,
```

with `fn default_icp_inventory_floor() -> u64 { 200_000_000 }` and `fn default_icp_inventory_ceiling() -> u64 { 2_000_000_000 }`, and the same values in the `Default` impl for `BotState`.

- [ ] Add one admin setter (single method so the pair can't pass through an invalid intermediate state):

```rust
#[update]
fn set_icp_inventory_band(floor_e8s: u64, ceiling_e8s: u64) -> Result<(), String> {
    require_admin();
    if floor_e8s < 100_000_000 { return Err("floor must be >= 1 ICP".into()); }
    if ceiling_e8s <= floor_e8s { return Err("ceiling must be > floor".into()); }
    state::mutate_state(|s| {
        s.config.icp_inventory_floor_e8s = floor_e8s;
        s.config.icp_inventory_ceiling_e8s = ceiling_e8s;
    });
    state::log_activity("admin", &format!("icp inventory band set to [{}, {}] e8s", floor_e8s, ceiling_e8s));
    Ok(())
}
```

- [ ] Update `arb_bot.did` (BotConfig record + new method) and the dashboard IDL block + config panel display. Run `scripts/check-candid.sh` — expect pass.
- [ ] `cargo check -p arb_bot` — expect clean.

### Task 1.2: Drain uses the band

**Files:**
- Modify: `src/arb_bot/src/arb.rs` (`ICP_RESERVE` at :34–35; `drain_residual_icp` reserve math at :2417–2445)

**Interfaces:**
- Consumes: Task 1.1 config fields.
- Produces: drain semantics — skim above ceiling in steady state; recover down to floor when a `pending_exit` (failed leg 2) exists, still capped to that trade's leg-1 ICP amount.

- [ ] In `drain_residual_icp`, replace `let reserved = ICP_RESERVE.saturating_add(volume_stranded);` with:

```rust
// Steady state: only skim inventory above the band ceiling. During a
// pending_exit recovery (stranded leg-2 ICP), drain down to the floor —
// the leg1_cap below still limits the drain to what that trade put here.
let has_pending = state::read_state(|s| s.pending_exit.is_some());
let band_reserve = if has_pending {
    config.icp_inventory_floor_e8s
} else {
    config.icp_inventory_ceiling_e8s
};
let reserved = band_reserve.saturating_add(volume_stranded);
```

- [ ] Delete the now-unused `ICP_RESERVE` const (or keep it ONLY if something else references it — grep first; if kept, add a comment that the drain no longer uses it).
- [ ] Audit every other `ICP_RESERVE` reference (grep the whole crate) and decide floor vs ceiling per call site; document each decision in the commit message.
- [ ] Check `src/arb_bot/tests/` for an existing harness; if unit-testable seams exist (pure fns), add band-math tests; otherwise state so in the commit body.
- [ ] `cargo check -p arb_bot`, `scripts/check-candid.sh`, then commit both tasks: `feat(arb): configurable ICP inventory band replacing fixed 1-ICP drain reserve`.

---

## PR2 — Strategy S (branch `feat/strategy-s-bob`, base `feat/icp-inventory-band`)

### Task 2.1: BOB config plumbing

**Files:**
- Modify: `src/arb_bot/src/state.rs`, `src/arb_bot/src/lib.rs`, `arb_bot.did`, `dashboard.html` (IDL block)

**Interfaces:**
- Produces (all on `BotConfig`, all serde-defaulted):
  - `bob_ledger: Principal` — default `7pail-xaaaa-aaaas-aabmq-cai`
  - `bob_ledger_fee: u64` — default `1_000_000`
  - `icpswap_bob_icp_pool: Principal` — default `ybilh-nqaaa-aaaag-qkhzq-cai`
  - `icpswap_icusd_bob_pool: Principal` — default `Principal::anonymous()` (pool doesn't exist yet; this is the strategy's master gate)
  - `bob_max_trade_size_usd: u64` — default `50_000_000` ($50, 6-dec)
  - `bob_min_spread_bps: u64` — default `150`
  - `bob_execution_enabled: bool` — default `false`
  - Update methods: `set_bob_pools(bob_icp_pool: Principal, icusd_bob_pool: Principal) -> Result<(), String>`, `set_bob_params(max_trade_size_usd: u64, min_spread_bps: u64) -> Result<(), String>`, `set_bob_execution_enabled(enabled: bool) -> Result<(), String>` — all admin, all logged, modeled on `set_rumi_amm_paused` (lib.rs:312).
- Also on `BotState` (serde-defaulted): `bob_icp_ordering_resolved: bool`, `icusd_bob_ordering_resolved: bool`, and resolved-ordering bools on `BotConfig`: `icpswap_bob_icp_icp_is_token0: bool` (default false — verified token0 = BOB), `icpswap_icusd_bob_icusd_is_token0: bool` (default false).

- [ ] Add fields, default fns, `Default` impl entries, setters, candid triple-sync, dashboard config display. `scripts/check-candid.sh` + `cargo check` pass. Commit: `feat(arb): BOB strategy config plumbing (inert)`.

### Task 2.2: Token-ordering resolution for the two BOB pools

**Files:**
- Modify: `src/arb_bot/src/arb.rs` (ordering-resolution block at ~:270–330), `src/arb_bot/src/prices.rs` (`fetch_icpswap_token_ordering` at ~:318–341)

**Interfaces:**
- Consumes: Task 2.1 fields.
- Produces: at cycle start (and in `run_specific_strategy`), when the pools are configured and not yet resolved, resolve `icpswap_bob_icp_icp_is_token0` by probing with `config.icp_ledger` and `icpswap_icusd_bob_icusd_is_token0` by probing with `config.icusd_ledger`.

- [ ] Read `fetch_icpswap_token_ordering` — it takes `(pool, ledger)` and answers "is this ledger token0". If it is already token-agnostic, reuse it directly for the icUSD/BOB pool with `icusd_ledger`; if anything in it assumes ICP specifically, generalize the signature without changing existing call sites' behavior.
- [ ] Wire both resolutions into the existing resolution block following the `icusd_token_ordering_resolved` pattern exactly (persist the flag so it resolves once).
- [ ] `cargo check`; commit: `feat(arb): resolve token ordering for BOB pools`.

### Task 2.3: Reference pricing + best-stable-quote helper

**Files:**
- Modify: `src/arb_bot/src/arb.rs` (new helpers near the venue dispatch section, ~:44–180)

**Interfaces:**
- Consumes: existing `fetch_icpswap_quote_for_amount`, `fetch_rumi_quote_for_amount`, `fetch_virtual_price_cached`, `stable_to_usd_6dec`.
- Produces:
  - `async fn best_stable_usd_per_icp(config: &BotConfig, icp_amount_e8s: u64) -> Option<StableQuote>` where `struct StableQuote { pool: state::Pool, usd_out_6dec: u64, usd_per_icp_6dec: u64 }` — quotes every configured non-PartyDEX stable/ICP pool for selling `icp_amount_e8s`, converts each to 6-dec USD (virtual-price-adjusted for Rumi 3USD, $1 peg for the rest — same math as `drain_residual_icp`'s candidate block at arb.rs:2500–2545), returns the best. Do NOT refactor the drain to use this in this PR (behavior-preservation constraint); duplication is accepted and noted.
  - `async fn best_stable_icp_per_usd(config: &BotConfig, usd_6dec: u64) -> Option<TopUpQuote>` — the mirror direction (buying ICP with a stable), `struct TopUpQuote { pool: state::Pool, stable_in_amount: u64, icp_out_e8s: u64 }`, quoting stable→ICP on the same candidate set. Used by the reverse-direction top-up leg.
  - `fn mark_icp_usd(icp_e8s: u64, usd_per_icp_6dec: u64) -> i64` — the USD mark for ICP legs: `(icp_e8s as u128 * usd_per_icp_6dec as u128 / 100_000_000) as i64`.
- [ ] Implement, `cargo check`, commit: `feat(arb): best-stable quote helpers + ICP USD marks for strategy S`.

### Task 2.4: Strategy S evaluator (dry-run)

**Files:**
- Modify: `src/arb_bot/src/arb.rs`

**Interfaces:**
- Consumes: Tasks 2.1–2.3.
- Produces:
  - `struct BobTarget { icusd_bob_pool: Principal, icusd_is_token0: bool, bob_icp_pool: Principal, bob_icp_icp_is_token0: bool, bob_ledger: Principal, bob_fee: u64, icusd_ledger: Principal, icusd_fee: u64, icp_ledger: Principal }` (built inline in `run_arb_cycle` like the other targets).
  - `enum BobDirection { Forward /* icUSD→BOB→ICP */, Reverse /* ICP→BOB→icUSD */ }`
  - `struct BobDryRun { should_trade: bool, direction: Option<BobDirection>, input_amount: u64, bob_amount: u64, output_amount: u64, expected_profit_usd: i64, spread_bps: u32, usd_per_icp_6dec: u64, pool_price_icusd_per_bob_8dec: u64, ref_price_icusd_per_bob_8dec: u64 }`
  - `async fn find_optimal_bob(config: &BotConfig, target: &BobTarget) -> Result<BobDryRun, String>`
- [ ] Evaluation logic, mirroring the `NUM_CANDIDATES = 4` size-laddering of `find_optimal_cross_pool_forward` (arb.rs:1812+):
  1. Fetch `usd_per_icp` once via `best_stable_usd_per_icp` at a 1-ICP probe size; bail with `Ok(no-trade)` if unavailable.
  2. Compute the pool price and reference price at probe size: pool = icUSD out per BOB in (and inverse) from the icUSD/BOB pool quote; reference = (ICP out per BOB from BOB/ICP quote) × `usd_per_icp` (icUSD treated as $1 for the reference, matching `stable_to_usd_6dec`).
  3. For each candidate size (bob_max_trade_size_usd × 1/4, 2/4, 3/4, 4/4, converted to input units): quote the full 2-leg route in whichever direction the deviation favors and compute `expected_profit_usd`:
     - Forward: `mark_icp_usd(icp_out_net) - icusd_in_usd - fees_usd` where `icp_out_net` subtracts `ICP_FEE * 2` (transfer + next-hop approval spend, same convention as arb.rs:2043) and `fees_usd` includes icUSD + BOB ledger fees marked to USD.
     - Reverse: `icusd_out_usd - mark_icp_usd(icp_in_gross) - fees_usd`.
  4. `spread_bps` = pool-vs-reference deviation in bps. `should_trade` requires: spread ≥ `bob_min_spread_bps`, profit > 0, profit ≥ `min_profit_usd` (global), and best candidate profit maximal.
- [ ] Log a `[S]` dry-run line matching the format at arb.rs:1622.
- [ ] `cargo check`, commit: `feat(arb): strategy S evaluator (dry-run)`.

### Task 2.5: Execution + band top-up + stranded-BOB recovery

**Files:**
- Modify: `src/arb_bot/src/arb.rs`, `src/arb_bot/src/state.rs`

**Interfaces:**
- Consumes: Tasks 2.1–2.4; existing `icpswap_swap` (swaps.rs:76), `append_trade_leg`, `LegType`.
- Produces:
  - `LegType::TopUp` variant (candid-append-safe; update .did + dashboard IDL).
  - `state.pending_bob_exit: Option<PendingBobExit>` with `struct PendingBobExit { entry_pool: BobPool, bob_amount: u64 }` and `enum BobPool { IcusdBob, BobIcp }` — all `#[serde(default)]` on the BotState field.
  - `async fn execute_bob(config: &BotConfig, target: &BobTarget, dry_run: &BobDryRun)`
  - `async fn drain_residual_bob(config: &BotConfig) -> Result<(), String>` called in `run_arb_cycle` right after `drain_residual_icp` (arb.rs:335).
- [ ] `execute_bob` Forward: leg 1 icUSD→BOB on the icUSD/BOB pool (slippage-bounded via `dry_run` expectations, same convention as arb.rs:2044); record `TradeLeg` (Leg1, sold icUSD at face USD, bought BOB marked at reference); set `pending_bob_exit { entry_pool: IcusdBob, bob_amount }`; leg 2 BOB→ICP on BOB/ICP; record Leg2 (sold BOB marked, bought ICP marked via `mark_icp_usd`, unlike A–R's zero-marks — this is deliberate, per spec §3); clear `pending_bob_exit`; log `[S] COMPLETE` with net profit. On leg-2 failure: return; `drain_residual_bob` recovers next cycle.
- [ ] `execute_bob` Reverse: compute `icp_needed`; read live ICP balance; if `balance - icp_needed < config.icp_inventory_floor_e8s`, prepend TopUp leg: `best_stable_icp_per_usd` for the shortfall, execute stable→ICP on the winning pool, record `TradeLeg` (TopUp, sold stable at face USD, bought ICP marked). Then leg 1 ICP→BOB (set `pending_bob_exit { entry_pool: BobIcp, bob_amount }`), leg 2 BOB→icUSD, marks as above, clear pending, log.
- [ ] `drain_residual_bob`: read BOB balance; if ≤ `config.bob_ledger_fee * 10` (dust) return Ok. Otherwise sell it: prefer the pool that is NOT `pending_bob_exit.entry_pool` (both pools as fallback candidates, quote both, best USD/ICP-marked output wins, never re-enter the entry pool — mirror of drain_residual_icp's entry-pool exclusion at arb.rs:2451–2466); record a Drain leg with reference marks; clear `pending_bob_exit` on success or when all candidates fail (mirroring arb.rs:2660–2663 semantics).
- [ ] `cargo check`, commit: `feat(arb): strategy S execution, ICP top-up leg, stranded-BOB drain`.

### Task 2.6: Cycle wiring, snapshot, health, run_specific_strategy

**Files:**
- Modify: `src/arb_bot/src/arb.rs` (`run_arb_cycle` ~:340–800, `run_specific_strategy`), `src/arb_bot/src/state.rs` (`CycleSnapshot` :166+), `src/arb_bot/src/lib.rs` (health/balances endpoint), candid triple-sync.

**Interfaces:**
- Consumes: everything above.
- Produces: snapshot fields (ALL `#[serde(default)]` — old snapshots must decode): `bob_pool_price_icusd_per_bob: u64`, `bob_ref_price_icusd_per_bob: u64`, `spread_s_bps: i64`, `balance_bob: u64`, `balance_icp_e8s: u64` (if not already present — grep first). `run_specific_strategy("S")` support. Health endpoint includes BOB balance and `pending_bob_exit`.
- [ ] Gate: `let has_bob = config.icpswap_icusd_bob_pool != Principal::anonymous() && config.icpswap_bob_icp_pool != Principal::anonymous();` Dry-run S whenever `has_bob`; include S's `expected_profit_usd` in the best-strategy selection ONLY when `config.bob_execution_enabled` (otherwise dry-run log + snapshot only).
- [ ] Follow the exact per-letter pattern at arb.rs:703–715 (profit collection) and arb.rs:946+ (force-execute in `run_specific_strategy`).
- [ ] `cargo check` + `scripts/check-candid.sh`, commit: `feat(arb): wire strategy S into cycle, snapshot, health`.

### Task 2.7: Approvals + dashboard

**Files:**
- Modify: `src/arb_bot/src/lib.rs` (`setup_approvals` :341+), `src/arb_bot/src/dashboard.html`

**Interfaces:** consumes Task 2.1 config.
- [ ] `setup_approvals` additions, gated like the existing per-pool blocks:

```rust
// Strategy S approvals (if BOB pools are configured)
if config.icpswap_bob_icp_pool != Principal::anonymous() {
    approvals.push(("BOB→ICPSwap-BOB-ICP", config.bob_ledger, config.icpswap_bob_icp_pool));
    approvals.push(("ICP→ICPSwap-BOB-ICP", config.icp_ledger, config.icpswap_bob_icp_pool));
}
if config.icpswap_icusd_bob_pool != Principal::anonymous() {
    approvals.push(("icUSD→ICPSwap-icUSD-BOB", config.icusd_ledger, config.icpswap_icusd_bob_pool));
    approvals.push(("BOB→ICPSwap-icUSD-BOB", config.bob_ledger, config.icpswap_icusd_bob_pool));
}
```

- [ ] Dashboard: strategy S card following the K–R pattern (see commit `edc1169` for the shape): spread, pool vs reference price, dry-run badge when `bob_execution_enabled` is false, BOB balance in the balances section, band + BOB knobs in the config panel. No new logo asset required (text label "BOB" is fine; venue logo CSS per `reference_dashboard_logos` memory if trivial).
- [ ] `scripts/check-candid.sh` + `cargo check`, commit: `feat(arb): strategy S approvals + dashboard`.

---

## Verification gates (both PRs)

1. `cargo check -p arb_bot` and the repo's wasm release build command complete clean.
2. `scripts/check-candid.sh` passes.
3. Persona review: `icp-canister-auditor` + `rust-migration-reviewer` on the full diff; fix CONFIRMED findings.
4. Variant-QA matrix (see spec; run as source audit):
   - Every new config field: serde default ✓ Default impl ✓ setter ✓ .did ✓ dashboard IDL ✓ dashboard display ✓
   - Strategy S appears in: gating ✓ dry-run ✓ snapshot ✓ selection (flag-gated) ✓ run_specific_strategy ✓ approvals ✓ dashboard ✓
   - Drain paths: pending_exit recovery unchanged (cap + entry-pool exclusion) ✓ skim-above-ceiling ✓ volume_stranded still respected ✓ drain_residual_bob never re-enters entry pool ✓
   - LegType::TopUp decodes alongside old logs ✓ old CycleSnapshots decode ✓
5. PRs: PR1 base `main`; PR2 base PR1's branch; bodies list deploy checklist (cargo clean → deploy → setup_approvals → set_bob_pools once the icUSD/BOB pool exists → observe dry-run → set_bob_execution_enabled(true)).
