# PR #22 Stage 1 (Retirement Safety) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement Stage 1 ("Retirement safety") of the six-asset route-arbitrage policy spec — fail-close every legacy lettered strategy executor and every other retiring surface before any inter-canister call, delete the automatic residual-asset drains, freeze legacy `pending_exit`/`pending_bob_exit` incidents from funding any surviving mutation, and remove the now-retired trading actions from the dashboard. This is dry-run-only groundwork: it changes what happens when a legacy method is called, it does not build the new six-asset engine (Stage 2+) and does not enable any new execution path.

**Architecture:** Every one of the 57 methods this stage retires already starts with `require_admin()` (verified against the current source). The mechanical pattern is: keep that line, replace everything after it with a fail-closed body shaped to that method's own existing return type (trap for a type with no room for a "retired" value; construct an explicit "retired" value for a type that already has one, e.g. `arb::DryRunResult`'s `message` field). No method's Candid signature changes. Two functions (`drain_residual_icp`, `drain_residual_bob`) and the automatic arb-cycle timer registration are deleted/disabled outright rather than stubbed, since the spec calls for removing the automatic-drain capability entirely, not leaving it reachable-but-inert.

**Tech Stack:** Rust IC canister (`ic-cdk` 0.13), hand-maintained candid, guarded by `scripts/check-candid.sh`. This stage should produce **zero** `.did`/dashboard-IDL diff — wire signatures are preserved exactly — which is itself a strong automated check that this stage stayed in scope.

**Spec:** `docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md` (commit `8e188ed27b5c158ebf35141532ab5df2a95f9865`, SHA-256 `8f03e00a2c7a0a537fc49d8ed86defcf5d8d590d7d5664e5bba08c22d10a7904`), Sections 2.3, 10 (Stage 1 + the disposition table), and 11 (pending-exit migration). Section 10's disposition table is the binding authority for which of the 86 current public methods falls into which bucket — it was independently verified, method-name-exhaustively, against the current `lib.rs` before this plan was written (86/86 match, zero missing, zero extra).

## Global Constraints

- **No implementation, deployment, configuration, approvals, transfers, or trades on the live canister.** This plan only touches source, tests, and docs in this worktree/branch. Nothing here deploys anything.
- **Preserve every Candid wire signature exactly.** A fail-closed method keeps its exact parameter types and exact return type. `scripts/check-candid.sh` must report **zero drift** at the end of this plan — if it reports a diff, something in this plan was implemented wrong, not something to "fix" in the `.did`.
- **Keep `require_admin()` as the first line of every retired method.** Do not change who is authorized to call a method, only what happens when they do.
- **Scope boundary, stated explicitly so it isn't rediscovered mid-task:** this stage does NOT delete the internal strategy-execution helper functions inside `arb.rs` (e.g. `run_specific_strategy`, the A-S price-fetch/execute helpers) that the 14 `execute_strategy_*` methods used to call — only the entry points are fail-closed. Once the entry points are fail-closed and the automatic timer is disabled, those helpers become unreachable and the compiler will emit new `dead_code` warnings for them, exactly like the three pre-existing dead-code warnings already in this codebase (`pool_token_ledger`, `pool_token_decimals`, `THREE_USD_FEE`). Accept these warnings; do not delete arb.rs's internals in this stage. The two named exceptions are `drain_residual_icp` and `drain_residual_bob`, which the spec explicitly calls out for deletion (Task 6).
- **Dashboard scope boundary, also explicit:** this stage removes the trading *actions* for the five named legacy Rumi/3pool tools (`rumi_quote`, `rumi_manual_swap`, `pool_deposit`, `pool_redeem`, `pool_exchange`, and by extension their `pool_quote_*` quote-preview counterparts) and the `setup_approvals()` trigger, per the spec's explicit Stage-1 bullet. It does NOT remove the A-S strategy opportunity cards from the dashboard — that's the spec's Section 9 (new reporting shape), which only makes sense once Stage 2's engine exists to replace those cards. Those cards will show a "retired" message if force-executed or dry-run in this stage (since the underlying methods now return a retired `DryRunResult`/trap) — that is safe, if not maximally tidy, and is an accepted, named tradeoff for keeping this stage bounded.
- **Ledger fee/decimal/asset-parameterization lessons from Strategy T's review history remain binding wherever new logic touches native amounts** (Task 7's freeze logic touches ICP/BOB native amounts) — never assume decimals or fees are interchangeable across assets.
- Commit messages: `feat(arb): ...` / `chore(arb): ...` / `test(arb): ...`, ending with `Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>`. Local commits only in this plan's execution — pushing/PR/merge happens per the controller's normal SDD flow, not deployment.

---

### Task 1: Stage-1 disposition inventory as an enforced, automated check

**Files:**
- Create: `scripts/check-stage1-disposition.sh`
- Modify: `README.md` (document the new script alongside `check-candid.sh`)

**Interfaces:**
- Produces: a script that extracts every `#[update]`/`#[query]` function name from `src/arb_bot/src/lib.rs` and checks it against a checked-in classification, exiting non-zero if any current method is unclassified (the spec's own acceptance criterion: "an unclassified mutator fails the acceptance gate").

This task exists first because every later task's correctness is checked against it: once a method is reclassified as fail-closed in Tasks 2-5, this script confirms nothing was missed and nothing new crept in unclassified.

- [ ] **Step 1: Write the script.**

```bash
#!/usr/bin/env bash
#
# check-stage1-disposition.sh — enforces PR #22 Section 10's Stage-1
# Candid-method disposition as a standing, automated check.
#
# Every public #[update]/#[query] method in lib.rs must appear in exactly
# one of the five disposition buckets below. A method present in the code
# but absent from every bucket list fails this check — per the spec's own
# acceptance criterion, an unclassified mutator is not acceptable.
#
# Usage: scripts/check-stage1-disposition.sh
# Exit status is non-zero if any current method is unclassified, or if a
# bucket lists a method that no longer exists in the code (a stale entry
# is itself a drift signal worth surfacing, not silently ignored).

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RUST_LIB="$ROOT/src/arb_bot/src/lib.rs"

if [[ ! -f "$RUST_LIB" ]]; then
  echo "FATAL: expected source not found: $RUST_LIB" >&2
  exit 2
fi

# Every current #[update]/#[query] fn name, one per line, sorted.
actual_methods() {
  awk '
    /^#\[(update|query)\]/ { want = 1; next }
    want && /^(async )?fn [a-z_0-9]+/ {
      match($0, /^(async )?fn [a-z_0-9]+/)
      name = substr($0, RSTART, RLENGTH)
      sub(/^(async )?fn /, "", name)
      print name
      want = 0
    }
  ' "$RUST_LIB" | sort -u
}

# ── Fail-closed compatibility stubs before any inter-canister call ──
FAIL_CLOSED='
get_prices
get_bot_health
setup_approvals
manual_arb_cycle
execute_strategy_a
execute_strategy_b
execute_strategy_c
execute_strategy_d
execute_strategy_f
execute_strategy_k
execute_strategy_l
execute_strategy_m
execute_strategy_n
execute_strategy_o
execute_strategy_p
execute_strategy_q
execute_strategy_r
execute_strategy_s
dry_run_arb_cycle
dry_run_strategy_b
dry_run_strategy_c
dry_run_strategy_d
dry_run_strategy_f
dry_run_strategy_k
dry_run_strategy_l
dry_run_strategy_m
dry_run_strategy_n
dry_run_strategy_o
dry_run_strategy_p
dry_run_strategy_q
dry_run_strategy_r
dry_run_strategy_t
rumi_quote
rumi_manual_swap
pool_deposit
pool_redeem
pool_quote_deposit
pool_quote_redeem
pool_exchange
pool_quote_exchange
set_config
set_rumi_amm_paused
set_slippage_bps
set_arb_interval_secs
set_icp_inventory_band
set_bob_inventory_band
set_strategy_t_ckusdc_band
set_strategy_t_ckusdt_band
set_strategy_t_dry_run
set_strategy_t_enabled
set_strategy_t_icusd_band
set_strategy_t_pools
set_strategy_t_thresholds
set_bob_execution_enabled
set_bob_params
set_bob_pools
backfill_trade_legs
'

# ── Preserved local/query compatibility, no fund-moving or retired-venue call ──
READ_ONLY='
is_admin
add_admin
remove_admin
get_config
get_trade_history
get_trade_legs
get_activity_log
get_errors
get_snapshots
get_summary
get_public_health
get_volume_stats
get_volume_trades
cycles_balance
http_request
pause
resume
clear_cycle_lock
'

# ── Preserved volume configuration, no direct balance mutation ──
VOLUME_CONFIG='
set_volume_config
set_volume_global
pause_volume
resume_volume
'

# ── Surviving volume operation (Stage-4 global lock participant) ──
VOLUME_OPERATION='
volume_swap
fund_volume_subaccount
withdraw_volume_subaccount
trigger_volume_cycle
trigger_volume_rebalance
'

# ── Surviving generic/recovery operation (Stage-4 global lock participant) ──
GENERIC_RECOVERY='
withdraw
recover_partydex_balance
'

all_classified() {
  printf '%s\n%s\n%s\n%s\n%s\n' \
    "$FAIL_CLOSED" "$READ_ONLY" "$VOLUME_CONFIG" "$VOLUME_OPERATION" "$GENERIC_RECOVERY" \
    | grep -v '^[[:space:]]*$' | sort -u
}

ACTUAL="$(actual_methods)"
CLASSIFIED="$(all_classified)"

fail=0

echo "== Stage 1 disposition inventory =="

UNCLASSIFIED="$(comm -23 <(printf '%s\n' "$ACTUAL") <(printf '%s\n' "$CLASSIFIED"))"
if [[ -n "$UNCLASSIFIED" ]]; then
  fail=1
  echo "FAIL: method(s) in lib.rs with no Stage-1 disposition:" >&2
  printf '  %s\n' $UNCLASSIFIED >&2
fi

STALE="$(comm -13 <(printf '%s\n' "$ACTUAL") <(printf '%s\n' "$CLASSIFIED"))"
if [[ -n "$STALE" ]]; then
  fail=1
  echo "FAIL: method(s) classified here that no longer exist in lib.rs (stale entry — update this script):" >&2
  printf '  %s\n' $STALE >&2
fi

if [[ "$fail" -eq 0 ]]; then
  n=$(printf '%s\n' "$ACTUAL" | grep -c .)
  echo "PASS: all $n public methods have an exact Stage-1 disposition (fail-closed: $(printf '%s\n' "$FAIL_CLOSED" | grep -c .), read-only: $(printf '%s\n' "$READ_ONLY" | grep -c .), volume-config: $(printf '%s\n' "$VOLUME_CONFIG" | grep -c .), volume-op: $(printf '%s\n' "$VOLUME_OPERATION" | grep -c .), generic-recovery: $(printf '%s\n' "$GENERIC_RECOVERY" | grep -c .))."
fi

exit "$fail"
```

- [ ] **Step 2: Make it executable and run it now, before any other task's code changes** — it should already PASS against the current, unmodified codebase (this task only adds the checker, it doesn't change any method yet):

```bash
chmod +x scripts/check-stage1-disposition.sh
scripts/check-stage1-disposition.sh
```

Expected: `PASS: all 86 public methods have an exact Stage-1 disposition (fail-closed: 57, read-only: 18, volume-config: 4, volume-op: 5, generic-recovery: 2).`

- [ ] **Step 3: Document it in `README.md`**, in the same section that documents `check-candid.sh`, one paragraph: this script enforces that every public method has an explicit Stage-1 retirement disposition, run it alongside `check-candid.sh` before any PR touching `lib.rs`'s public method set.

- [ ] **Step 4: Commit**

```bash
git add scripts/check-stage1-disposition.sh README.md
git commit -m "chore(arb): add automated Stage-1 disposition inventory check

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: Fail-close batch A — trap-based group (29 methods)

**Files:**
- Modify: `src/arb_bot/src/lib.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: no interface change — every method in this batch keeps its exact existing signature; only the body after `require_admin();` changes.

This batch covers every retiring method whose return type has **no field that can carry a "retired" message** without either lying (returning a fabricated real-looking value) or changing the wire type. For these, `ic_cdk::trap` is the correct fail-closed mechanism — it is already used elsewhere in this exact file (`set_config`'s ICP-band validation) for exactly this "this call cannot proceed" situation.

**The exact pattern**, worked on three representative examples — apply the same transformation (keep `require_admin();`, delete everything else in the body, add one `ic_cdk::trap(...)` line with a message naming the method) to every other method in the list below:

```rust
// BEFORE (execute_strategy_a, and identically execute_strategy_b through _s — 14 total):
#[update]
async fn execute_strategy_a() {
    require_admin();
    state::log_activity("admin", &format!("Force-execute strategy A by {}", ic_cdk::api::caller()));
    arb::run_specific_strategy("A").await;
}

// AFTER:
#[update]
async fn execute_strategy_a() {
    require_admin();
    ic_cdk::trap("retired: execute_strategy_a is retired under the Stage-1 lettered-strategy retirement (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md)");
}
```

```rust
// BEFORE (get_prices):
#[update]
async fn get_prices() -> PriceInfo {
    require_admin();
    // ... mixed price lookups across active and retired venues ...
}

// AFTER:
#[update]
async fn get_prices() -> PriceInfo {
    require_admin();
    ic_cdk::trap("retired: get_prices is retired under Stage-1 (mixed retired-venue price lookups) — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}
```

```rust
// BEFORE (rumi_quote):
#[update]
async fn rumi_quote(token_in: Principal, amount: u64) -> PoolQuote {
    require_admin();
    // ... quotes against the retiring Rumi AMM ...
}

// AFTER:
#[update]
async fn rumi_quote(token_in: Principal, amount: u64) -> PoolQuote {
    require_admin();
    ic_cdk::trap("retired: rumi_quote is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md");
}
```

Note `token_in` becomes an unused parameter after this change — prefix it `_token_in` (and similarly for any other now-unused parameter in this batch) to keep the build warning-free; do not remove the parameter, since that would change the Candid signature.

**Apply the identical pattern (keep `require_admin();`, one `ic_cdk::trap("retired: <method_name> is retired under Stage-1 ...")` line, prefix now-unused parameters with `_`) to every remaining method in this batch:**

- `manual_arb_cycle()`
- `execute_strategy_b`, `execute_strategy_c`, `execute_strategy_d`, `execute_strategy_f`, `execute_strategy_k`, `execute_strategy_l`, `execute_strategy_m`, `execute_strategy_n`, `execute_strategy_o`, `execute_strategy_p`, `execute_strategy_q`, `execute_strategy_r`, `execute_strategy_s` (13 more, same shape as the worked example above — 14 total including `_a`)
- `get_bot_health() -> state::BotHealthReport`
- `pool_deposit(coin_index: u8, amount: u64, min_lp_out: u64)`
- `pool_redeem(coin_index: u8, lp_amount: u64, min_out: u64)`
- `pool_exchange(coin_in: u8, coin_out: u8, amount_in: u64, min_out: u64)`
- `pool_quote_deposit(coin_index: u8, amount: u64) -> PoolQuote`
- `pool_quote_redeem(coin_index: u8, lp_amount: u64) -> PoolQuote`
- `pool_quote_exchange(coin_in: u8, coin_out: u8, amount_in: u64) -> PoolQuote`
- `rumi_manual_swap(token_in: Principal, amount: u64, min_out: u64)`
- `backfill_trade_legs(legs: Vec<TradeLeg>)`
- `set_config(config: BotConfigInput)`
- `set_strategy_t_enabled(enabled: bool)`
- `set_strategy_t_dry_run(dry_run: bool)`

That's 14 (`execute_strategy_*`) + `manual_arb_cycle` + `get_prices` (worked above) + `get_bot_health` + 6 `pool_*` + `rumi_quote` (worked above) + `rumi_manual_swap` + `backfill_trade_legs` + `set_config` + `set_strategy_t_enabled` + `set_strategy_t_dry_run` = 29 methods total (14 + 15 individually-named ones).

- [ ] **Step 1: Apply the transformation to all 29 methods listed above.**

- [ ] **Step 2: `cargo build -p arb_bot`** — expect clean compile, zero new warnings (any "unused variable" warning means a parameter needs the `_` prefix per the note above; any "unused import" warning means something like `state::log_activity`'s import can be checked but should NOT be removed globally since other, non-retired code in this file still uses it — only remove an import if `cargo build` says the import itself is now fully unused across the whole file, which is unlikely here since `state::log_activity` is used extensively elsewhere).

- [ ] **Step 3: `scripts/check-stage1-disposition.sh`** — still passes (this task doesn't change which bucket a method is in, only what it does; Task 1's script only checks presence-in-a-bucket, not behavior).

- [ ] **Step 4: `scripts/check-candid.sh`** — must still report zero drift. This is the key correctness signal for this task: if it fails, a signature was accidentally changed.

- [ ] **Step 5: Commit**

```bash
git add src/arb_bot/src/lib.rs
git commit -m "feat(arb): fail-close 29 legacy trap-shaped methods for Stage 1 retirement

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: Fail-close batch B — `Result<(), String>`-returning group (13 methods)

**Files:**
- Modify: `src/arb_bot/src/lib.rs`

**Interfaces:** No signature change. Every method in this batch already returns `Result<(), String>`, which has a natural, honest "retired" value: `Err("retired: ...".to_string())`.

**The exact pattern**, worked on one example:

```rust
// BEFORE (set_rumi_amm_paused):
#[update]
fn set_rumi_amm_paused(paused: bool) -> Result<(), String> {
    require_admin();
    state::mutate_state(|s| { s.config.rumi_amm_paused = paused; });
    state::log_activity(
        "admin",
        &format!("rumi_amm_paused set to {} by {}", paused, ic_cdk::api::caller()),
    );
    Ok(())
}

// AFTER:
#[update]
fn set_rumi_amm_paused(_paused: bool) -> Result<(), String> {
    require_admin();
    Err("retired: set_rumi_amm_paused is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string())
}
```

**Apply the identical pattern (keep `require_admin();`, one `Err("retired: <method_name> is retired under Stage-1 ...".to_string())` return, prefix now-unused parameters with `_`) to every method in this batch:**

- `set_rumi_amm_paused(paused: bool)` (worked above)
- `set_slippage_bps(slippage_bps: u64)`
- `set_arb_interval_secs(interval_secs: u64)`
- `set_icp_inventory_band(floor_e8s: u64, ceiling_e8s: u64)`
- `set_bob_inventory_band(floor_e8s: u64, ceiling_e8s: u64)`
- `set_strategy_t_pools(icusd_ckusdc: Principal, icusd_ckusdt: Principal, ckusdt_ckusdc: Principal)`
- `set_strategy_t_thresholds(min_profit_usd: i64, min_profit_bps: u32, max_trade_size_usd: u64)`
- `set_strategy_t_icusd_band(floor: u64, ceiling: u64)`
- `set_strategy_t_ckusdt_band(floor: u64, ceiling: u64)`
- `set_strategy_t_ckusdc_band(floor: u64, ceiling: u64)`
- `set_bob_pools(bob_icp_pool: Principal, icusd_bob_pool: Principal)`
- `set_bob_params(max_trade_size_usd: u64, min_spread_bps: u64)`
- `set_bob_execution_enabled(enabled: bool)`

13 methods total.

- [ ] **Step 1: Apply the transformation to all 13 methods.**

- [ ] **Step 2: `cargo build -p arb_bot`** — clean, no new warnings.

- [ ] **Step 3: `scripts/check-stage1-disposition.sh`** and **`scripts/check-candid.sh`** — both still pass, zero drift.

- [ ] **Step 4: Commit**

```bash
git add src/arb_bot/src/lib.rs
git commit -m "feat(arb): fail-close 13 legacy Result-returning setters for Stage 1 retirement

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 4: Fail-close batch C — `DryRunResult`-shaped group (14 methods)

**Files:**
- Modify: `src/arb_bot/src/lib.rs`

**Interfaces:** No signature change. `arb::DryRunResult` already derives `Default` and has a `message: String` field (`src/arb_bot/src/arb.rs:411-431`) — the natural, non-trapping fail-closed value is `arb::DryRunResult { message: "retired: ...".to_string(), ..Default::default() }`, which also surfaces cleanly to any existing dashboard code reading `.message`. `strategy_t::StrategyTDryRunResult` (used only by `dry_run_strategy_t`) has no `Default`/message field — construct it explicitly with empty/`None` fields.

**The exact pattern**, worked on two examples:

```rust
// BEFORE (dry_run_arb_cycle):
#[update]
async fn dry_run_arb_cycle() -> arb::DryRunResult {
    require_admin();
    // ... 60 lines of live quoting against Rumi AMM / ICPSwap / retired venues ...
}

// AFTER:
#[update]
async fn dry_run_arb_cycle() -> arb::DryRunResult {
    require_admin();
    arb::DryRunResult {
        message: "retired: dry_run_arb_cycle is retired under Stage-1 — see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md".to_string(),
        ..Default::default()
    }
}
```

```rust
// BEFORE (dry_run_strategy_t):
#[update]
async fn dry_run_strategy_t() -> strategy_t::StrategyTDryRunResult {
    require_admin();
    // ... live six-way Strategy T quoting ...
}

// AFTER:
#[update]
async fn dry_run_strategy_t() -> strategy_t::StrategyTDryRunResult {
    require_admin();
    // Strategy T's own report type carries no message field; an empty
    // result set is the honest "retired, nothing evaluated" value —
    // never a fabricated candidate.
    strategy_t::StrategyTDryRunResult {
        candidates: Vec::new(),
        best_economic: None,
        best_executable: None,
    }
}
```

**Apply the `arb::DryRunResult { message: "retired: <method_name> is retired under Stage-1 ...".to_string(), ..Default::default() }` pattern to every remaining method in this batch** (all return `arb::DryRunResult` exactly like `dry_run_arb_cycle` above):

- `dry_run_arb_cycle()` (worked above)
- `dry_run_strategy_b()`
- `dry_run_strategy_c()`
- `dry_run_strategy_d()`
- `dry_run_strategy_f()`
- `dry_run_strategy_k()`
- `dry_run_strategy_l()`
- `dry_run_strategy_m()`
- `dry_run_strategy_n()`
- `dry_run_strategy_o()`
- `dry_run_strategy_p()`
- `dry_run_strategy_q()`
- `dry_run_strategy_r()`
- `dry_run_strategy_t()` (worked above, different shape — the only one)

14 methods total.

- [ ] **Step 1: Apply the transformation to all 14 methods.**

- [ ] **Step 2: `cargo build -p arb_bot`** — clean, no new warnings.

- [ ] **Step 3: `cargo test -p arb_bot`** — the existing `strategy_t_math` and `state_decode` suites must still pass unmodified; this task doesn't touch `strategy_t.rs`'s own types, only how `lib.rs` calls into them.

- [ ] **Step 4: `scripts/check-stage1-disposition.sh`** and **`scripts/check-candid.sh`** — both still pass, zero drift.

- [ ] **Step 5: Commit**

```bash
git add src/arb_bot/src/lib.rs
git commit -m "feat(arb): fail-close 14 legacy dry-run methods for Stage 1 retirement

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 5: Fail-close `setup_approvals` (1 method)

**Files:**
- Modify: `src/arb_bot/src/lib.rs`

**Interfaces:** No signature change (`setup_approvals() -> String`). This is its own task rather than folded into batch A because the spec explicitly calls it out by name ("Replace legacy `setup_approvals` behavior with a fail-closed compatibility stub; future active-pool allowances require a separately named, spender-specific admin action") — worth its own small, clearly-labeled commit.

- [ ] **Step 1: Apply the transformation**:

```rust
// BEFORE:
#[update]
async fn setup_approvals() -> String {
    require_admin();
    let config = state::read_state(|s| s.config.clone());
    // ... grants icrc2_approve on icUSD/ckUSDT/ckUSDC to rumi_amm and rumi_3pool ...
}

// AFTER:
#[update]
async fn setup_approvals() -> String {
    require_admin();
    "retired: setup_approvals is retired under Stage-1 — active-pool allowances now require a separately named, spender-specific admin action (see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md); no approval was granted".to_string()
}
```

- [ ] **Step 2: `cargo build -p arb_bot`, `scripts/check-stage1-disposition.sh`, `scripts/check-candid.sh`** — all pass, zero drift.

- [ ] **Step 3: Commit**

```bash
git add src/arb_bot/src/lib.rs
git commit -m "feat(arb): fail-close setup_approvals for Stage 1 retirement

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 6: Delete the automatic drains and disable the automatic arb-cycle timer

**Files:**
- Modify: `src/arb_bot/src/arb.rs` (delete `drain_residual_icp`, `drain_residual_bob`, and their two call sites)
- Modify: `src/arb_bot/src/lib.rs` (`setup_timer()`)

**Interfaces:** `arb::run_arb_cycle` keeps its own signature (it's not a Candid method, and by the end of this plan it is unreachable from every Candid entry point and never invoked by a timer — deleting it is out of scope per this plan's Global Constraints), but the two `drain_residual_*` functions are deleted entirely, per the spec's explicit instruction ("Delete the arbitrage functions `drain_residual_icp` and `drain_residual_bob` and their scheduler call sites ... and exposes no replacement generic drain").

- [ ] **Step 1: Read `src/arb_bot/src/arb.rs` around lines 570-600 and 3330-3900 to find the exact current call sites and function bodies** (line numbers shift as earlier tasks in this plan don't touch `arb.rs`, so they should still be close to: call sites at `arb.rs:579` and `arb.rs:592` inside `run_arb_cycle`; function bodies at `arb.rs:3334` (`drain_residual_icp`) and `arb.rs:3617` (`drain_residual_bob`)).

- [ ] **Step 2: Delete both function bodies in full** (`async fn drain_residual_icp(...) { ... }` and `async fn drain_residual_bob(...) { ... }`, including their doc comments).

- [ ] **Step 3: Remove their two call sites inside `run_arb_cycle`.** The exact removal depends on the surrounding control flow you find in Step 1 — read the immediate context (a handful of lines before/after each `if let Err(e) = drain_residual_icp(&config).await { ... }` / `drain_residual_bob` call) and remove the whole `if let Err(e) = ... { ... }` block cleanly, leaving the surrounding function's control flow otherwise intact (do not leave a dangling `else` or an unreachable block). Since `run_arb_cycle` becomes unreachable by the end of this plan anyway (see Step 4), the goal here is simply that `arb.rs` compiles once the two functions no longer exist — do not otherwise restructure `run_arb_cycle`.

- [ ] **Step 4: In `src/arb_bot/src/lib.rs`, make `setup_timer()` not register the automatic arb-cycle timer** — this is "the legacy automatic arbitrage timer callback" the spec requires to fail closed; the simplest, most directly verifiable way to guarantee it makes zero calls is to never schedule it at all:

```rust
// BEFORE:
fn setup_timer() {
    ARB_TIMER_ID.with(|id| {
        if let Some(prev) = id.borrow_mut().take() {
            ic_cdk_timers::clear_timer(prev);
        }
    });
    let interval = state::read_state(|s| s.config.arb_interval_secs).max(1);
    let new_id = ic_cdk_timers::set_timer_interval(
        std::time::Duration::from_secs(interval),
        || ic_cdk::spawn(arb::run_arb_cycle()),
    );
    ARB_TIMER_ID.with(|id| *id.borrow_mut() = Some(new_id));
}

// AFTER:
/// Retired under Stage-1 of the six-asset route-arbitrage policy: the
/// automatic arb-cycle timer is the "legacy automatic arbitrage timer
/// callback" the spec requires to fail closed. Never registering it is
/// the most directly verifiable way to guarantee it makes zero calls —
/// there is no callback left to invoke. `ARB_TIMER_ID` is left in place
/// (harmless — `clear_timer` on a never-set id is a no-op) rather than
/// removed, to keep this diff minimal; a later stage may remove it
/// entirely once the new engine's own scheduling exists.
fn setup_timer() {
    ARB_TIMER_ID.with(|id| {
        if let Some(prev) = id.borrow_mut().take() {
            ic_cdk_timers::clear_timer(prev);
        }
    });
}
```

Do **not** modify `setup_volume_timer()` — the volume bot's own timer is explicitly out of scope for this stage (Global Constraints).

- [ ] **Step 5: `cargo build -p arb_bot`** — expect clean compile. `arb::run_arb_cycle` and everything it exclusively calls will now generate new `dead_code` warnings — expected and accepted per this plan's Global Constraints; do not silence them by adding `#[allow(dead_code)]` (that would hide a real signal for a future cleanup pass) and do not delete `run_arb_cycle` itself (out of scope, see Global Constraints).

- [ ] **Step 6: Write a test proving the automatic timer is disabled.** Since `setup_timer()` is a private, non-async function with no return value, the most direct test is structural: add to `src/arb_bot/tests/` a new test file asserting the function body contains no `set_timer_interval` call for the arb cycle. Create `src/arb_bot/tests/stage1_retirement.rs`:

```rust
//! Structural regression tests for PR #22 Stage 1 retirement — see
//! docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md.
//! These are grep-based structural checks, not live-canister tests,
//! matching this codebase's existing testing conventions (no
//! ic_cdk::call mocking infrastructure exists).

use std::fs;

fn lib_rs_source() -> String {
    fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("read lib.rs")
}

fn arb_rs_source() -> String {
    fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/arb.rs"))
        .expect("read arb.rs")
}

/// Extracts the body of `fn setup_timer() { ... }` (brace-matched) from
/// lib.rs, for structural assertions about what it does and does not do.
fn setup_timer_body(source: &str) -> String {
    let start = source.find("fn setup_timer() {").expect("find setup_timer");
    let body_start = start + source[start..].find('{').unwrap();
    let bytes = source.as_bytes();
    let mut depth = 0i32;
    let mut end = body_start;
    for (i, &b) in bytes[body_start..].iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = body_start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    source[body_start..end].to_string()
}

#[test]
fn setup_timer_never_registers_the_automatic_arb_cycle() {
    let body = setup_timer_body(&lib_rs_source());
    assert!(
        !body.contains("set_timer_interval"),
        "setup_timer() must never register a periodic timer under Stage-1 retirement; found set_timer_interval in its body"
    );
    assert!(
        !body.contains("run_arb_cycle"),
        "setup_timer() must never reference run_arb_cycle under Stage-1 retirement"
    );
}

#[test]
fn drain_residual_functions_are_deleted() {
    let source = arb_rs_source();
    assert!(
        !source.contains("fn drain_residual_icp"),
        "drain_residual_icp must be deleted under Stage-1 retirement, not merely unreachable"
    );
    assert!(
        !source.contains("fn drain_residual_bob"),
        "drain_residual_bob must be deleted under Stage-1 retirement, not merely unreachable"
    );
}
```

- [ ] **Step 7: Run it** — `cargo test -p arb_bot --test stage1_retirement` — expect both tests to pass.

- [ ] **Step 8: `scripts/check-stage1-disposition.sh` and `scripts/check-candid.sh`** — both still pass (this task adds no new public method and removes none).

- [ ] **Step 9: Commit**

```bash
git add src/arb_bot/src/arb.rs src/arb_bot/src/lib.rs src/arb_bot/tests/stage1_retirement.rs
git commit -m "feat(arb): delete automatic residual-asset drains and disable the automatic arb-cycle timer

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 7: `pending_exit`/`pending_bob_exit` Stage-1 freeze

**Files:**
- Modify: `src/arb_bot/src/state.rs` (new freeze-check helper)
- Modify: `src/arb_bot/src/lib.rs` (wire the freeze check into every surviving mutating method that could touch ICP or BOB)
- Test: `src/arb_bot/tests/state_decode.rs` (append)

**Interfaces:**
- Consumes: `state::BotState.pending_exit: Option<PendingExit>`, `state::BotState.pending_bob_exit: Option<PendingBobExit>` (both already exist, both currently `None` on the live canister — verified live 2026-09-04/05).
- Produces: `state::legacy_route_freeze_reason(asset: LegacyFreezeAsset) -> Option<String>` (pure, testable — `None` means not frozen; `Some(reason)` means the caller must reject).

Per spec §11: "Before any Stage-1 timer or wallet-mutating method is enabled, migration converts each `pending_exit` into an ICP `LegacyUnknown` ownership reservation and each `pending_bob_exit` into a BOB legacy non-route reservation, or proves the referenced funds are in a structurally disjoint account... A zero/unknown amount also freezes the affected asset rather than treating zero as no exposure." The **full** durable per-asset reservation ledger (arbitrary lots, bounded stable storage, `available_native` formula) is Stage 4 scope (§10 lists "durable held-balance reservations" under Stage 4) — there is no live execution yet to create a lot for it to hold. What Stage 1 needs, and what this task builds, is the narrower, fully-specified piece: **if `pending_exit` or `pending_bob_exit` is present, freeze that asset from every Stage-1-surviving mutating method** (`withdraw`, `volume_swap`, `fund_volume_subaccount`, `withdraw_volume_subaccount`, `trigger_volume_cycle`, `trigger_volume_rebalance`, `recover_partydex_balance`) until an operator resolves it. A "zero/unknown amount also freezes" — so this is a presence check (`is_some()`), not an amount check.

- [ ] **Step 1: Add the freeze-check type and function to `state.rs`**, near `PendingExit`/`PendingBobExit`:

```rust
/// Which asset a Stage-1 legacy-incident freeze check is being asked
/// about. Covers exactly the two assets `pending_exit`/`pending_bob_exit`
/// can encumber.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyFreezeAsset {
    Icp,
    Bob,
}

/// Returns `Some(reason)` if `asset` is frozen by an unresolved legacy
/// `pending_exit`/`pending_bob_exit` incident, `None` if it's clear to
/// spend. Per the spec (Section 11): presence alone freezes the asset —
/// a zero or unknown amount is NOT treated as "no exposure." This checks
/// presence only; it does not attempt to prove the referenced funds are
/// in a structurally disjoint account (that proof, and the full durable
/// reservation ledger for an arbitrary future held position, is Stage 4
/// scope — there is no live execution yet to create one).
pub fn legacy_route_freeze_reason(state: &BotState, asset: LegacyFreezeAsset) -> Option<String> {
    match asset {
        LegacyFreezeAsset::Icp if state.pending_exit.is_some() => Some(
            "asset frozen: unresolved legacy pending_exit incident (Stage-1 retirement freeze, see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md Section 11)".to_string(),
        ),
        LegacyFreezeAsset::Bob if state.pending_bob_exit.is_some() => Some(
            "asset frozen: unresolved legacy pending_bob_exit incident (Stage-1 retirement freeze, see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md Section 11)".to_string(),
        ),
        _ => None,
    }
}
```

- [ ] **Step 2: Write the tests first (TDD)** — append to a new `src/arb_bot/tests/legacy_freeze.rs`:

```rust
//! Tests for the Stage-1 legacy pending_exit/pending_bob_exit freeze —
//! see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md Section 11.

use arb_bot::state::{BotState, LegacyFreezeAsset, PendingBobExit, PendingExit, Pool, BobPool, legacy_route_freeze_reason};

#[test]
fn clear_state_freezes_nothing() {
    let state = BotState::default();
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Icp).is_none());
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Bob).is_none());
}

#[test]
fn pending_exit_freezes_icp_only() {
    let mut state = BotState::default();
    state.pending_exit = Some(PendingExit {
        entry_pool: Pool::IcpswapCkusdc,
        intended_exit_pool: Pool::IcpswapIcusd,
        timestamp: 0,
        icp_amount: 0, // zero amount must still freeze — presence alone matters
    });
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Icp).is_some());
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Bob).is_none());
}

#[test]
fn pending_bob_exit_freezes_bob_only() {
    let mut state = BotState::default();
    state.pending_bob_exit = Some(PendingBobExit {
        entry_pool: BobPool::BobIcp,
        bob_amount: 0, // zero amount must still freeze
    });
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Bob).is_some());
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Icp).is_none());
}

#[test]
fn both_pending_incidents_freeze_both_assets() {
    let mut state = BotState::default();
    state.pending_exit = Some(PendingExit {
        entry_pool: Pool::IcpswapCkusdc,
        intended_exit_pool: Pool::IcpswapIcusd,
        timestamp: 0,
        icp_amount: 500_000_000,
    });
    state.pending_bob_exit = Some(PendingBobExit { entry_pool: BobPool::IcusdBob, bob_amount: 100 });
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Icp).is_some());
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Bob).is_some());
}
```

Check the exact field names/types of `PendingExit`, `PendingBobExit`, `Pool`, `BobPool` in `state.rs` before writing this file — the plan's Step 1 code above was written to match what already exists there, but confirm field-for-field before compiling (e.g. `PendingExit.icp_amount` has a `#[serde(default)]` per the existing struct — verify the literal above compiles against the real struct, adjusting only if a field name genuinely differs from what's shown here).

- [ ] **Step 3: Run the tests to verify they fail** (the function doesn't exist yet): `cargo test -p arb_bot --test legacy_freeze` — expect a compile error naming `legacy_route_freeze_reason`/`LegacyFreezeAsset` as undefined.

- [ ] **Step 4: Implement Step 1's code in `state.rs`, then re-run** — expect all 4 tests to pass.

- [ ] **Step 5: Wire the freeze check into every surviving method that can move ICP or BOB.** For each of the methods below, add a freeze check as the first statement after `require_admin()` (or, for `volume_swap`/`fund_volume_subaccount`/`withdraw_volume_subaccount`/`trigger_volume_cycle`/`trigger_volume_rebalance`, after whatever their current first gate is — read each method first to place the check correctly relative to existing logic, then return/trap using that method's own existing error-reporting convention):

  - `withdraw(token_ledger: Principal, ...)`: freeze check applies only when `token_ledger` is the ICP ledger or the BOB ledger (read `config.icp_ledger`/`config.bob_ledger` to compare) — this method already has no `Result` return (`()`), so use `ic_cdk::trap(&reason)` if frozen.
  - `volume_swap(icp_to_icusd: bool, ...)`: this always touches ICP (it's the ICP/icUSD volume pool) — check `LegacyFreezeAsset::Icp` unconditionally; trap if frozen (method returns `()`).
  - `fund_volume_subaccount(token_ledger: Principal, ...)` / `withdraw_volume_subaccount(token_ledger: Principal, ...)`: both return `Result<(), String>` — check whichever of ICP/BOB `token_ledger` matches (same comparison as `withdraw`), return `Err(reason)` if frozen.
  - `trigger_volume_cycle() -> String`: touches **both ICP and BOB** — it drives the ICP/icUSD and 3USD/ICP volume pools (ICP) but also sweeps stranded BOB via `transfer_to_subaccount` and, when the icUSD/BOB leg is enabled, its SellBob leg spends `bob_ledger` directly (`volume.rs`'s `run_volume_cycle`). Check `LegacyFreezeAsset::Icp` unconditionally; if not frozen, ALSO check `LegacyFreezeAsset::Bob` unconditionally (check Icp first, then Bob; use whichever fires) — a `pending_bob_exit` incident must block this method too, not just a `pending_exit` one. If either is frozen, return that reason string directly (this method already returns a `String` status message, so no trap needed — matches its existing "return a status string" convention).
  - `trigger_volume_rebalance() -> ()`: touches **both ICP and BOB** — `run_rebalance`'s `rebalance_icusd_bob` sells BOB. Same Icp-then-Bob check as `trigger_volume_cycle` above; trap if either is frozen (method returns `()`).
  - `recover_partydex_balance(pool: Principal) -> Result<(u64, u64), String>`: PartyDEX recovery settles in ICP (per the spec's own note that PartyDEX routes are ICP-bridge routes) — check `LegacyFreezeAsset::Icp`; return `Err(reason)` if frozen.

  For each, read the method's current body first so the check is inserted correctly (right after `require_admin()`, before any inter-canister call) and uses that method's own established error-return idiom — do not introduce a new error-handling style inconsistent with the rest of that method.

- [ ] **Step 6: `cargo build -p arb_bot`** — clean compile.

- [ ] **Step 7: `cargo test -p arb_bot`** — full suite, including the new `legacy_freeze` tests, passes.

- [ ] **Step 8: `scripts/check-stage1-disposition.sh` and `scripts/check-candid.sh`** — both pass, zero drift (this task changes method bodies only, not signatures — `withdraw`, `volume_swap`, etc. keep their exact existing candid types).

- [ ] **Step 9: Commit**

```bash
git add src/arb_bot/src/state.rs src/arb_bot/src/lib.rs src/arb_bot/tests/legacy_freeze.rs
git commit -m "feat(arb): freeze ICP/BOB spending on unresolved legacy pending_exit incidents

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 8: Remove retired trading actions from the dashboard

**Files:**
- Modify: `src/arb_bot/src/dashboard.html`

**Interfaces:** No Candid change — this is UI-only. Per this plan's Global Constraints, scope is exactly the five named legacy Rumi/3pool tools' trading actions plus the `setup_approvals()` trigger — not the A-S strategy opportunity cards (deferred, see Global Constraints).

- [ ] **Step 1: Locate the manual-swap panel's execute action** (around `dashboard.html:3161-3172`, the block calling `rumi_manual_swap`/`pool_deposit`/`pool_redeem`/`pool_exchange`). Read the surrounding function (the button's click handler) in full, then replace the body of that handler with a call to the existing `toast(...)` helper reporting the tools are retired, and `return;` before any of the `authenticatedActor.*` calls — e.g.:

```javascript
// Before the first authenticatedActor.rumi_manual_swap/pool_deposit/pool_redeem/pool_exchange call in this handler, add:
toast('Manual Rumi/3pool swaps are retired (Stage 1 of the six-asset route-arbitrage policy) — this action is disabled', 'error');
return;
```

Read the actual handler function's signature/name at that location first (it wasn't captured verbatim in this plan) and place this guard as the first statement inside it, after any input-validation the handler already does but before its first `await authenticatedActor....` call.

- [ ] **Step 2: Locate the manual-swap panel's quote-preview action** (around `dashboard.html:3123-3132`, calling `rumi_quote`/`pool_quote_deposit`/`pool_quote_redeem`). Since these now trap when called, apply the same guard-and-return pattern at the start of that handler too, so the UI shows a clear "retired" toast instead of an uncaught trap error.

- [ ] **Step 3: Locate the `setup_approvals()` trigger button's click handler** (around `dashboard.html:3652`). Add the same guard: report via `toast(...)` that `setup_approvals` is retired, and `return;` before the `await authenticatedActor.setup_approvals()` call.

- [ ] **Step 4: Confirm no other dashboard code path calls any of `rumi_quote`, `rumi_manual_swap`, `pool_deposit`, `pool_redeem`, `pool_exchange`, `pool_quote_deposit`, `pool_quote_redeem`, `pool_quote_exchange`, `setup_approvals`** — `grep -n` each name in `dashboard.html` and confirm every remaining reference is either the `I.Func`/`IDL` type declaration (unchanged — Candid types don't change) or inside one of the three guarded handlers from Steps 1-3.

- [ ] **Step 5: `cargo build -p arb_bot`** — dashboard.html is `include_str!`-embedded, so run `cargo clean -p arb_bot` first per this repo's documented gotcha (dashboard.html changes aren't always picked up by incremental builds), then build. Expect clean compile.

- [ ] **Step 6: `scripts/check-candid.sh`** — must still pass; this task doesn't touch any `IDL.*`/`I.Service` declaration, only handler bodies, so the candid-surface checks are unaffected.

- [ ] **Step 7: Commit**

```bash
git add src/arb_bot/src/dashboard.html
git commit -m "feat(arb): disable retired Rumi/3pool manual-swap and setup_approvals dashboard actions

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 9: Zero-inter-canister-call verification for every fail-closed method

**Files:**
- Create: `scripts/check-stage1-zero-calls.sh`

**Interfaces:** Produces an automated check directly implementing the spec's own acceptance criterion: "Prove zero inter-canister calls for every retired path with call-counting tests." Given this codebase has no `ic_cdk::call` mocking infrastructure (confirmed throughout its history — every existing test is either a pure unit test or a structural/grep-based check), the achievable, honest version of "prove zero calls" is structural: for each fail-closed method, extract its function body (brace-matched, same technique as Task 6's test) and assert it contains no `ic_cdk::call(`, no `.await` on anything other than nothing (trapped/early-returning bodies have no `.await` at all), and no reference to any of the venue-adapter modules (`prices::`, `swaps::`, `partydex::`, `arb::run_specific_strategy`).

- [ ] **Step 1: Write the script**, reusing Task 1's `actual_methods`-style extraction but scoped to just the fail-closed list:

```bash
#!/usr/bin/env bash
#
# check-stage1-zero-calls.sh — proves every Stage-1 fail-closed method's
# CURRENT body makes zero inter-canister calls, structurally: it contains
# no ic_cdk::call, no venue-adapter reference, and (since every fail-closed
# body is a single trap/early-return with no await) no `.await`.
#
# This is a structural proxy for "zero inter-canister calls," not a live
# call-count assertion — this codebase has no ic_cdk::call mocking
# infrastructure. See the Stage-1 plan's Task 9 for why this is the
# honest, achievable version of the spec's call-counting requirement.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RUST_LIB="$ROOT/src/arb_bot/src/lib.rs"

FAIL_CLOSED_METHODS=(
  get_prices get_bot_health setup_approvals manual_arb_cycle
  execute_strategy_a execute_strategy_b execute_strategy_c execute_strategy_d
  execute_strategy_f execute_strategy_k execute_strategy_l execute_strategy_m
  execute_strategy_n execute_strategy_o execute_strategy_p execute_strategy_q
  execute_strategy_r execute_strategy_s
  dry_run_arb_cycle dry_run_strategy_b dry_run_strategy_c dry_run_strategy_d
  dry_run_strategy_f dry_run_strategy_k dry_run_strategy_l dry_run_strategy_m
  dry_run_strategy_n dry_run_strategy_o dry_run_strategy_p dry_run_strategy_q
  dry_run_strategy_r dry_run_strategy_t
  rumi_quote rumi_manual_swap pool_deposit pool_redeem
  pool_quote_deposit pool_quote_redeem pool_exchange pool_quote_exchange
  set_config set_rumi_amm_paused set_slippage_bps set_arb_interval_secs
  set_icp_inventory_band set_bob_inventory_band
  set_strategy_t_ckusdc_band set_strategy_t_ckusdt_band set_strategy_t_dry_run
  set_strategy_t_enabled set_strategy_t_icusd_band set_strategy_t_pools
  set_strategy_t_thresholds set_bob_execution_enabled set_bob_params set_bob_pools
  backfill_trade_legs
)

# Extracts the brace-matched body of `(async )?fn <name>(...) ... {` from lib.rs.
extract_body() { # $1=fn name
  awk -v fn="fn $1(" '
    index($0, fn) && !found { found = 1 }
    found {
      buf = buf $0 "\n"
      n = gsub(/\{/, "{", $0); depth += n
      n = gsub(/\}/, "}", $0); depth -= n
      if (depth <= 0 && buf ~ /\{/) { print buf; exit }
    }
  ' "$RUST_LIB"
}

fail=0
checked=0

echo "== Stage 1 zero-inter-canister-call structural check =="

for m in "${FAIL_CLOSED_METHODS[@]}"; do
  body="$(extract_body "$m")"
  if [[ -z "$body" ]]; then
    echo "FAIL: could not locate fn $m in $RUST_LIB" >&2
    fail=1
    continue
  fi
  checked=$((checked + 1))
  if echo "$body" | grep -qE '\bic_cdk::call\(|\.await\b|\bprices::|\bswaps::|\bpartydex::|run_specific_strategy\('; then
    echo "FAIL: $m's body still references a live call path:" >&2
    echo "$body" | grep -E '\bic_cdk::call\(|\.await\b|\bprices::|\bswaps::|\bpartydex::|run_specific_strategy\(' | sed 's/^/    /' >&2
    fail=1
  fi
done

if [[ "$fail" -eq 0 ]]; then
  echo "PASS: all $checked fail-closed methods' bodies are free of any live call path."
fi

exit "$fail"
```

- [ ] **Step 2: Make it executable and run it**: `chmod +x scripts/check-stage1-zero-calls.sh && scripts/check-stage1-zero-calls.sh` — run this AFTER Tasks 2-5 have landed (it will fail if run before them, since those methods still have live bodies until then — that's expected and correct; if you're executing this plan task-by-task in order, Tasks 2-5 are already done by the time you reach Task 9).

Expected: `PASS: all 57 fail-closed methods' bodies are free of any live call path.` (57, matching Task 1's `FAIL_CLOSED` list exactly — `setup_approvals` is included in this count too, since its retired body is a bare string literal that legitimately contains no live-call markers. If the count you observe differs from 57, that is a real signal to investigate — do not edit this expected number to match a wrong result; find and fix the actual discrepancy instead.)

- [ ] **Step 3: Commit**

```bash
git add scripts/check-stage1-zero-calls.sh
git commit -m "chore(arb): add structural zero-inter-canister-call check for Stage 1 retirement

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:** Every Stage-1 bullet in §10 is covered: retired-integration removal from automatic paths (Tasks 2-6 collectively fail-close every path that could reach Rumi AMM/PartyDEX/BOB-arb), every lettered executor fail-closed (Tasks 2-4), `manual_arb_cycle` fail-closed (Task 2), `setup_approvals` fail-closed (Task 5), the five Rumi/3pool public tools fail-closed (Task 2) plus their dashboard actions removed (Task 8), the exhaustive mutating-entrypoint inventory (Task 1, and it's the exact table independently verified against the live codebase before this plan was written), the automatic drains deleted (Task 6), isolated manual withdrawal preserved (`withdraw`/`recover_partydex_balance` untouched except for Task 7's freeze gate), `pending_exit` preserved as inspectable legacy evidence without triggering a drain (Task 7 — freezes spending, never triggers anything), and zero-inter-canister-call proof for every retired path (Task 9).

**Placeholder scan:** Tasks 2-5's batched method lists give 2-3 fully worked before/after examples each plus an exhaustive enumeration of every remaining target and the exact one-line difference each needs — this is the deliberate "batch small same-shape work" pattern for genuinely mechanical, identically-shaped edits, not an placeholder. No task says "add appropriate error handling" or similar without showing the actual code.

**Type consistency:** `LegacyFreezeAsset`/`legacy_route_freeze_reason` (Task 7) are defined once and consumed identically at each of the 6 call sites in the same task. `arb::DryRunResult`'s `Default`/`message` field (Task 4) and `PoolQuote`/`PriceInfo`/`BotHealthReport`'s lack of one (Task 2) were confirmed by reading the actual struct definitions before this plan was written, not assumed.

---

**Plan complete and saved to `docs/superpowers/plans/2026-09-05-route-arb-stage1-retirement-safety.md`. Two execution options:**

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
