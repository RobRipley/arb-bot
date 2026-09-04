# Strategy T (Three-Stablecoin Router) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Strategy T — a dry-run-only, inventory-aware router that evaluates all twelve candidates (six directed Rumi 3pool legs × {stop, close via matching ICPSwap pool}) among icUSD, ckUSDT, and ckUSDC, ranks them by net dollar profit, and reports both the best economic candidate and the best currently-executable one (allowance- and inventory-gated), without ever moving funds.

**Architecture:** Follow the existing pattern-per-strategy design in `src/arb_bot/src/arb.rs` (data structs + explicit functions, no trait). Strategy T is new enough in shape (three symmetric assets, a 12-candidate router instead of a fixed two-venue spread) to warrant its own module, `src/arb_bot/src/strategy_t.rs`, rather than growing the already-3800-line `arb.rs`. The module splits into a **pure, synchronous core** (route enumeration + profit/fee math — fully unit-testable, no `ic_cdk` calls) and a **thin async shell** (live quoting, allowance checks, balance checks) that calls the pure core. This mirrors how `arb.rs` already separates quote-fetching (`prices.rs`) from math, just consolidated into one new file because Strategy T's math is the novel part, not the venue plumbing.

**Tech Stack:** Rust IC canister (`ic-cdk` 0.13), hand-maintained candid (`src/arb_bot/arb_bot.did` + the IDL block in `src/arb_bot/src/dashboard.html`), guarded by `scripts/check-candid.sh`. No CI — verification is local (`cargo build`, `cargo test`, `scripts/check-candid.sh`).

**Spec:** This plan's spec is the five-round, read-only mainnet-verified design memo produced in-conversation on 2026-09-02/03 (route topology, live pool/allowance/fee facts, the four-ledger-movement accounting, the `quote` vs `quoteForAll` distinction, and the operator's symmetric three-asset router requirement). No separate spec file exists; this plan carries the full spec context inline in Global Constraints and Task 2.

## Global Constraints

- **Dry-run only, no exceptions in this plan.** No task in this plan implements a fund-moving call (no `swap`, `depositFromAndSwap`, `icrc2_approve`, `icrc1_transfer`) for Strategy T. Every new async function either reads on-chain state (`calc_swap`, `quoteForAll`, `icrc2_allowance`, `icrc1_balance_of`) or is pure. `strategy_t_enabled` and `strategy_t_dry_run` config fields exist so a *future*, separately-planned PR can wire live execution — this plan does not add an `execute_strategy_t()` and must not be extended to add one.
- Every new `BotConfig` field MUST have `#[serde(default)]` or `#[serde(default = "fn")]` — state is a JSON blob in stable memory, decoded across upgrades old snapshots must survive.
- Candid triple-sync: any new type or method touches THREE files — `src/arb_bot/src/lib.rs` (Rust source of truth), `src/arb_bot/arb_bot.did`, and the `IDL.*` block in `src/arb_bot/src/dashboard.html`. `scripts/check-candid.sh` (no `--no-cargo`) must pass before any task is considered done.
- All admin-only endpoints call `require_admin()`.
- USD amounts are 6-decimal (`_usd` suffix, `i64` for signed profit, `u64` for unsigned thresholds); native token amounts use each token's own decimals (icUSD 8, ckUSDT/ckUSDC 6).
- Verified mainnet facts to hardcode (2026-09-02/03, read-only `dfx canister --network ic call` queries — see conversation record):
  - Rumi 3pool `fohh4-yyaaa-aaaap-qtkpa-cai`, coin index icUSD=0/ckUSDT=1/ckUSDC=2 (matches `ICUSD_LEDGER`/`CKUSDT_LEDGER` constants already in `lib.rs:359-360` and `pool_token_ledger` at `lib.rs:362`).
  - `eb25l-dyaaa-aaaar-qb4lq-cai` — icUSD/ckUSDC, token0=icUSD, token1=ckUSDC, fee 3000 (0.30%).
  - `jogrm-gqaaa-aaaar-qcg2a-cai` — ckUSDT/icUSD, token0=ckUSDT, token1=icUSD, fee 3000 (0.30%).
  - `heq6n-fyaaa-aaaag-qkcpq-cai` — ckUSDT/ckUSDC, token0=ckUSDT, token1=ckUSDC, fee 3000 (0.30%).
  - Ledger fees: icUSD `100_000` (0.001, 8 dec) = existing `ICUSD_FEE` in `arb.rs:14`; ckUSDT/ckUSDC `10_000` (0.01, 6 dec) = existing `CKUSDT_FEE`/`CKUSDC_FEE` in `arb.rs:12-13`. Strategy T redefines its own copies in `strategy_t.rs` (see Task 2) to keep the module self-contained and independently testable — this is deliberate, not accidental duplication.
  - **Four-ledger-movement accounting for a two-leg round trip** (verified live and reconciled exactly against three independent audited fixtures — see Task 2): entering the Rumi leg costs the start token's own ledger fee (on top of the notional sent — this is a real, separate capital outflow, not netted into the notional), Rumi's own output transfer costs the intermediate token's ledger fee, sending the intermediate token into the closing ICPSwap pool costs the intermediate token's ledger fee again, and the closing pool's output transfer costs the start token's ledger fee. That is: start token pays its fee **twice** (entry + final exit), intermediate token pays its fee **twice** (Rumi's send + our send). A one-leg stop only incurs the first two of these (entry fee + Rumi's own output fee) — there is no closing leg.
  - `quote` on an ICPSwap pool can silently return a stale/plateaued amount for an input beyond the pool's actual fillable ceiling instead of erroring — confirmed live on `eb25l` (verified 2026-09-02 ceiling 430.305 icUSD, then 498.734 icUSD an hour later, then 498.734 again 2026-09-03 — genuinely moves). **`quoteForAll` is the only method Strategy T may use for ICPSwap sizing** — a `quote`-based helper already exists (`prices::fetch_icpswap_quote_for_amount`) and is used by strategies A–S; Strategy T must not reuse it for candidate scoring.
- Commit messages follow repo style: `feat(arb): ...` with the `Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>` trailer. Commits happen locally in this worktree's branch — this plan does not push, open a PR, or merge.

---

### Task 1: Strategy T types and config fields

**Files:**
- Modify: `src/arb_bot/src/state.rs` (new enums near `Venue`/`Pool` at line ~207–348; new `BotConfig` fields after `bob_execution_enabled` at line 204; new default fns near line 75; `BotState::default()` at line 663–708)
- Test: `src/arb_bot/tests/state_decode.rs` (append new backward-compat tests, following the existing pattern at lines 10–34)

**Interfaces:**
- Produces: `state::StrategyTToken` (3 variants), `state::StrategyTPool` (3 variants), and 13 new `BotConfig` fields (3 pool principals, `strategy_t_enabled: bool`, `strategy_t_dry_run: bool`, `strategy_t_min_profit_usd: i64`, `strategy_t_min_profit_bps: u32`, `strategy_t_max_trade_size_usd: u64`, and 6 per-token floor/ceiling `u64` fields).

- [ ] **Step 1: Add the two new enums to `state.rs`**, placed after the existing `Pool` enum (line 348):

```rust
/// One of the three par-valued stablecoins Strategy T routes between.
/// Rumi 3pool coin index is fixed by the pool's own token ordering
/// (verified live 2026-09-02/03): IcUsd=0, CkUsdt=1, CkUsdc=2.
#[derive(CandidType, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrategyTToken {
    IcUsd,
    CkUsdt,
    CkUsdc,
}

/// Which ICPSwap pool connects a given unordered pair of Strategy T
/// stablecoins. Each of the three pairs among {IcUsd, CkUsdt, CkUsdc} has
/// exactly one pool (verified live 2026-09-02/03 `metadata` calls).
#[derive(CandidType, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrategyTPool {
    /// eb25l-dyaaa-aaaar-qb4lq-cai — token0=icUSD, token1=ckUSDC.
    IcusdCkusdc,
    /// jogrm-gqaaa-aaaar-qcg2a-cai — token0=ckUSDT, token1=icUSD.
    IcusdCkusdt,
    /// heq6n-fyaaa-aaaag-qkcpq-cai — token0=ckUSDT, token1=ckUSDC.
    CkusdtCkusdc,
}
```

- [ ] **Step 2: Add default fns** near the existing `default_bob_min_spread_bps` (state.rs line ~79):

```rust
fn default_strategy_t_min_profit_usd() -> i64 {
    50_000 // $0.05 — matches existing `min_profit_usd` scale/convention
}

fn default_strategy_t_min_profit_bps() -> u32 {
    50 // 0.50% — matches existing `min_spread_bps` convention
}

fn default_strategy_t_max_trade_size_usd() -> u64 {
    40_000_000 // $40 — dedicated cap, deliberately equal to but independent
               // from the global max_trade_size_usd (never reuse the global one)
}

fn default_strategy_t_icusd_floor() -> u64 { 500_000_000 }      // 5 icUSD (8 dec)
fn default_strategy_t_icusd_ceiling() -> u64 { 200_000_000_000 } // 2000 icUSD
fn default_strategy_t_ckusdt_floor() -> u64 { 5_000_000 }        // 5 ckUSDT (6 dec)
fn default_strategy_t_ckusdt_ceiling() -> u64 { 2_000_000_000 }  // 2000 ckUSDT
fn default_strategy_t_ckusdc_floor() -> u64 { 5_000_000 }        // 5 ckUSDC (6 dec)
fn default_strategy_t_ckusdc_ceiling() -> u64 { 2_000_000_000 }  // 2000 ckUSDC
```

- [ ] **Step 3: Add fields to `BotConfig`**, after `bob_execution_enabled` (state.rs line 204):

```rust
    // ─── Strategy T: three-stablecoin router (Rumi 3pool × 3 ICPSwap pairs) ───
    /// icUSD/ckUSDC ICPSwap pool (eb25l-dyaaa-aaaar-qb4lq-cai when configured).
    #[serde(default = "default_principal")]
    pub strategy_t_icusd_ckusdc_pool: Principal,
    /// icUSD/ckUSDT ICPSwap pool (jogrm-gqaaa-aaaar-qcg2a-cai when configured).
    #[serde(default = "default_principal")]
    pub strategy_t_icusd_ckusdt_pool: Principal,
    /// ckUSDT/ckUSDC ICPSwap pool (heq6n-fyaaa-aaaag-qkcpq-cai when configured).
    #[serde(default = "default_principal")]
    pub strategy_t_ckusdt_ckusdc_pool: Principal,
    /// Master enable switch for Strategy T. Dry-run evaluation runs once all
    /// three pool principals are non-anonymous, independent of this flag.
    /// This flag and `strategy_t_dry_run` exist for a future live-execution
    /// PR; this build has no live-trade path regardless of either value.
    #[serde(default)]
    pub strategy_t_enabled: bool,
    /// Forces dry-run-only. Defaults true. Present so a future PR can add
    /// live execution behind an explicit flip rather than a code change.
    #[serde(default = "default_true")]
    pub strategy_t_dry_run: bool,
    /// Minimum net profit (6-decimal USD) for a candidate to be eligible.
    #[serde(default = "default_strategy_t_min_profit_usd")]
    pub strategy_t_min_profit_usd: i64,
    /// Minimum net profit in basis points of start-leg notional, evaluated
    /// alongside (both must pass) the absolute floor above.
    #[serde(default = "default_strategy_t_min_profit_bps")]
    pub strategy_t_min_profit_bps: u32,
    /// Per-candidate max trade size (6-decimal USD). Dedicated to Strategy T
    /// — never reuse or raise the global `max_trade_size_usd` for this.
    #[serde(default = "default_strategy_t_max_trade_size_usd")]
    pub strategy_t_max_trade_size_usd: u64,
    /// Per-token inventory bands (native decimals). A candidate whose start
    /// leg would draw the start token below its floor, or whose end leg
    /// would push the end token above its ceiling, is ineligible.
    #[serde(default = "default_strategy_t_icusd_floor")]
    pub strategy_t_icusd_floor: u64,
    #[serde(default = "default_strategy_t_icusd_ceiling")]
    pub strategy_t_icusd_ceiling: u64,
    #[serde(default = "default_strategy_t_ckusdt_floor")]
    pub strategy_t_ckusdt_floor: u64,
    #[serde(default = "default_strategy_t_ckusdt_ceiling")]
    pub strategy_t_ckusdt_ceiling: u64,
    #[serde(default = "default_strategy_t_ckusdc_floor")]
    pub strategy_t_ckusdc_floor: u64,
    #[serde(default = "default_strategy_t_ckusdc_ceiling")]
    pub strategy_t_ckusdc_ceiling: u64,
```

Also add `fn default_true() -> bool { true }` next to the other default fns if not already present (`grep -n "fn default_true" src/arb_bot/src/state.rs` first — it is not currently defined).

- [ ] **Step 4: Wire the 13 new fields into `BotState::default()`** (state.rs line ~666, inside the `config: BotConfig { ... }` literal, after `bob_execution_enabled: false,`):

```rust
                strategy_t_icusd_ckusdc_pool: Principal::anonymous(),
                strategy_t_icusd_ckusdt_pool: Principal::anonymous(),
                strategy_t_ckusdt_ckusdc_pool: Principal::anonymous(),
                strategy_t_enabled: false,
                strategy_t_dry_run: true,
                strategy_t_min_profit_usd: default_strategy_t_min_profit_usd(),
                strategy_t_min_profit_bps: default_strategy_t_min_profit_bps(),
                strategy_t_max_trade_size_usd: default_strategy_t_max_trade_size_usd(),
                strategy_t_icusd_floor: default_strategy_t_icusd_floor(),
                strategy_t_icusd_ceiling: default_strategy_t_icusd_ceiling(),
                strategy_t_ckusdt_floor: default_strategy_t_ckusdt_floor(),
                strategy_t_ckusdt_ceiling: default_strategy_t_ckusdt_ceiling(),
                strategy_t_ckusdc_floor: default_strategy_t_ckusdc_floor(),
                strategy_t_ckusdc_ceiling: default_strategy_t_ckusdc_ceiling(),
```

- [ ] **Step 5: `cargo check -p arb_bot`** — expect clean compile (candid/.did/dashboard drift is expected and fixed in Task 8, not here).

- [ ] **Step 6: Write the backward-compat decode test.** Append to `src/arb_bot/tests/state_decode.rs`:

```rust
/// Strategy T ships after this test suite exists — a blob saved before its
/// fields existed must decode with dry-run-safe, inert defaults (all pools
/// anonymous, enabled=false) so an upgrade never silently activates it.
#[test]
fn old_state_without_strategy_t_fields_decodes_with_defaults() {
    let mut v = serde_json::to_value(BotState::default()).expect("serialize");
    let cfg = v
        .get_mut("config")
        .and_then(|c| c.as_object_mut())
        .expect("config object");
    for field in [
        "strategy_t_icusd_ckusdc_pool",
        "strategy_t_icusd_ckusdt_pool",
        "strategy_t_ckusdt_ckusdc_pool",
        "strategy_t_enabled",
        "strategy_t_dry_run",
        "strategy_t_min_profit_usd",
        "strategy_t_min_profit_bps",
        "strategy_t_max_trade_size_usd",
        "strategy_t_icusd_floor",
        "strategy_t_icusd_ceiling",
        "strategy_t_ckusdt_floor",
        "strategy_t_ckusdt_ceiling",
        "strategy_t_ckusdc_floor",
        "strategy_t_ckusdc_ceiling",
    ] {
        assert!(cfg.remove(field).is_some(), "field {field} missing from serialized default");
    }

    let decoded: BotState = serde_json::from_value(v).expect("decode old-shape state");
    assert_eq!(decoded.config.strategy_t_enabled, false, "must decode inert");
    assert_eq!(decoded.config.strategy_t_dry_run, true, "must decode dry-run-first");
    assert_eq!(decoded.config.strategy_t_min_profit_usd, 50_000);
    assert_eq!(decoded.config.strategy_t_min_profit_bps, 50);
    assert_eq!(decoded.config.strategy_t_max_trade_size_usd, 40_000_000);
    assert_eq!(decoded.config.strategy_t_icusd_floor, 500_000_000);
    assert_eq!(decoded.config.strategy_t_ckusdc_ceiling, 2_000_000_000);
}
```

- [ ] **Step 7: Run it** — `cargo test -p arb_bot --test state_decode old_state_without_strategy_t_fields_decodes_with_defaults -- --nocapture`. Expected: PASS (or FAIL with a clear "field missing" message the first time, before Step 1–4 land, if TDD ordering is followed — either order is fine here since Step 1–6 are one task).

- [ ] **Step 8: Commit**

```bash
git add src/arb_bot/src/state.rs src/arb_bot/tests/state_decode.rs
git commit -m "feat(arb): add Strategy T config fields and types

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: Pure route model and profit math (`strategy_t.rs`)

**Files:**
- Create: `src/arb_bot/src/strategy_t.rs`
- Modify: `src/arb_bot/src/lib.rs` (add `pub mod strategy_t;` near the other `mod` declarations at line 6-10 — `pub` because integration tests need to reach it, exactly like `pub mod state;`)
- Test: Create `src/arb_bot/tests/strategy_t_math.rs`

**Interfaces:**
- Consumes: nothing (pure, no dependency on Task 1's `state::` types — deliberately decoupled so this module is trivially unit-testable; Task 4 bridges `state::StrategyTToken` ↔ this module's `StableToken` where needed, or Task 1's enum is dropped in favor of this module's if redundant — see Step 6 note).
- Produces: `strategy_t::StableToken` (3 variants, with `.decimals()`, `.ledger_fee()`, `.rumi_coin_index()`, `.ALL`), `strategy_t::ClosingPool` (3 variants, with `.for_pair()`, `.zero_for_one_from()`), `strategy_t::RouteDescriptor { start, rumi_out, closing }`, `strategy_t::all_routes() -> Vec<RouteDescriptor>` (returns exactly 12), `strategy_t::par_usd_6dec(u64, StableToken) -> i64`, `strategy_t::one_leg_net_profit_usd(...) -> i64`, `strategy_t::closing_leg_input(...) -> u64`, `strategy_t::two_leg_net_profit_usd(...) -> i64`.

- [ ] **Step 1: Write the failing tests first** — create `src/arb_bot/tests/strategy_t_math.rs`. These fixtures are the exact figures independently audited live on 2026-09-03 (route: ckUSDC start → Rumi(ckUSDC→icUSD) → close via `eb25l`(icUSD→ckUSDC); raw units reconstructed from the audited decimal quotes, decimals icUSD=8/ckUSDC=6):

```rust
//! Deterministic tests for Strategy T's pure profit math, fixtured against
//! live mainnet quotes independently audited 2026-09-03 (see the plan's
//! Global Constraints for the four-ledger-movement accounting these encode).
//! No network access — these must never require `dfx` or a running canister.

use arb_bot::strategy_t::{
    all_routes, closing_leg_input, one_leg_net_profit_usd, par_usd_6dec,
    two_leg_net_profit_usd, ClosingPool, StableToken,
};

#[test]
fn all_routes_has_exactly_twelve_candidates_six_stop_six_close() {
    let routes = all_routes();
    assert_eq!(routes.len(), 12);
    assert_eq!(routes.iter().filter(|r| r.closing.is_none()).count(), 6);
    assert_eq!(routes.iter().filter(|r| r.closing.is_some()).count(), 6);
    // No degenerate route (start == rumi_out).
    assert!(routes.iter().all(|r| r.start != r.rumi_out));
}

#[test]
fn closing_pool_covers_every_unordered_pair_exactly_once() {
    use StableToken::*;
    assert_eq!(ClosingPool::for_pair(IcUsd, CkUsdc), Some(ClosingPool::IcusdCkusdc));
    assert_eq!(ClosingPool::for_pair(CkUsdc, IcUsd), Some(ClosingPool::IcusdCkusdc));
    assert_eq!(ClosingPool::for_pair(IcUsd, CkUsdt), Some(ClosingPool::IcusdCkusdt));
    assert_eq!(ClosingPool::for_pair(CkUsdt, CkUsdc), Some(ClosingPool::CkusdtCkusdc));
    assert_eq!(ClosingPool::for_pair(IcUsd, IcUsd), None);
}

#[test]
fn par_usd_6dec_treats_all_three_tokens_as_one_dollar_peg() {
    // 100 ckUSDC (6 dec) == $100.00
    assert_eq!(par_usd_6dec(100_000_000, StableToken::CkUsdc), 100_000_000);
    // 100 icUSD (8 dec) == $100.00
    assert_eq!(par_usd_6dec(10_000_000_000, StableToken::IcUsd), 100_000_000);
}

/// $10 ckUSDC round trip, audited 2026-09-03: Rumi gross 10.40381707 icUSD,
/// closing (eb25l) gross 10.373120 ckUSDC, net profit +$0.353120.
#[test]
fn two_leg_round_trip_ten_dollars_matches_audited_fixture() {
    let start_amount = 10_000_000u64; // $10.00 ckUSDC
    let rumi_gross_icusd = 1_040_381_707u64; // 10.40381707 icUSD, raw 8-dec
    let leg2_input = closing_leg_input(StableToken::IcUsd, rumi_gross_icusd);
    assert_eq!(leg2_input, 1_040_181_707); // matches audited "10.40181707"
    let closing_gross_ckusdc = 10_373_120u64; // audited eb25l quoteForAll output
    let profit = two_leg_net_profit_usd(StableToken::CkUsdc, start_amount, closing_gross_ckusdc);
    assert_eq!(profit, 353_120); // +$0.353120
}

/// $100 ckUSDC round trip, audited 2026-09-03: Rumi gross 102.75379366 icUSD,
/// closing gross 102.429613 ckUSDC, net profit +$2.409613.
#[test]
fn two_leg_round_trip_hundred_dollars_matches_audited_fixture() {
    let start_amount = 100_000_000u64;
    let rumi_gross_icusd = 10_275_379_366u64;
    let closing_gross_ckusdc = 102_429_613u64;
    let profit = two_leg_net_profit_usd(StableToken::CkUsdc, start_amount, closing_gross_ckusdc);
    assert_eq!(profit, 2_409_613);
    let _ = rumi_gross_icusd; // exercised via closing_leg_input in the $10/$300/$450 tests
}

/// $300 ckUSDC round trip, audited 2026-09-03: Rumi gross 304.80381301 icUSD,
/// closing gross 303.595192 ckUSDC, net profit +$3.575192.
#[test]
fn two_leg_round_trip_three_hundred_dollars_matches_audited_fixture() {
    let start_amount = 300_000_000u64;
    let rumi_gross_icusd = 30_480_381_301u64;
    let leg2_input = closing_leg_input(StableToken::IcUsd, rumi_gross_icusd);
    assert_eq!(leg2_input, 30_480_181_301);
    let closing_gross_ckusdc = 303_595_192u64;
    let profit = two_leg_net_profit_usd(StableToken::CkUsdc, start_amount, closing_gross_ckusdc);
    assert_eq!(profit, 3_575_192);
}

/// $450 ckUSDC round trip, audited 2026-09-03 (last size that still fully
/// filled before the eb25l ceiling that hour): Rumi gross 455.44972349 icUSD,
/// closing gross 453.192764 ckUSDC, net profit +$3.172764.
#[test]
fn two_leg_round_trip_four_fifty_dollars_matches_audited_fixture() {
    let start_amount = 450_000_000u64;
    let closing_gross_ckusdc = 453_192_764u64;
    let profit = two_leg_net_profit_usd(StableToken::CkUsdc, start_amount, closing_gross_ckusdc);
    assert_eq!(profit, 3_172_764);
}

/// One-leg stop (inventory conversion, no closing leg): derived from the
/// audited $100 Rumi-leg quote above, not independently cross-checked by a
/// live one-leg audit — this pins the arithmetic, not a second live fact.
#[test]
fn one_leg_conversion_hundred_dollars_derived_from_audited_rumi_quote() {
    let start_amount = 100_000_000u64; // $100.00 ckUSDC
    let rumi_gross_icusd = 10_275_379_366u64;
    let profit = one_leg_net_profit_usd(StableToken::CkUsdc, start_amount, StableToken::IcUsd, rumi_gross_icusd);
    assert_eq!(profit, 2_742_793); // +$2.742793
}

#[test]
fn zero_for_one_matches_live_verified_token_ordering() {
    use StableToken::*;
    // eb25l: token0=icUSD, token1=ckUSDC
    assert!(ClosingPool::IcusdCkusdc.zero_for_one_from(IcUsd));
    assert!(!ClosingPool::IcusdCkusdc.zero_for_one_from(CkUsdc));
    // jogrm: token0=ckUSDT, token1=icUSD
    assert!(ClosingPool::IcusdCkusdt.zero_for_one_from(CkUsdt));
    assert!(!ClosingPool::IcusdCkusdt.zero_for_one_from(IcUsd));
    // heq6n: token0=ckUSDT, token1=ckUSDC
    assert!(ClosingPool::CkusdtCkusdc.zero_for_one_from(CkUsdt));
    assert!(!ClosingPool::CkusdtCkusdc.zero_for_one_from(CkUsdc));
}
```

- [ ] **Step 2: Run the tests to verify they fail** (module doesn't exist yet):

Run: `cargo test -p arb_bot --test strategy_t_math`
Expected: FAIL with `error[E0433]: failed to resolve: could not find 'strategy_t' in 'arb_bot'` (or similar — the module doesn't exist).

- [ ] **Step 3: Create `src/arb_bot/src/strategy_t.rs`** with the pure core:

```rust
//! Strategy T: inventory-aware three-stablecoin router over the Rumi 3pool
//! (icUSD/ckUSDT/ckUSDC) and three ICPSwap direct-stable pools. Dry-run
//! only — see the plan's Global Constraints for why this module never
//! calls a fund-moving canister method.
//!
//! This file is split pure-first: everything above the `// ─── Live quoting
//! ───` marker (added in later tasks) has no `ic_cdk` dependency and is
//! covered by `tests/strategy_t_math.rs` with zero network access.

/// One of the three par-valued ($1) stablecoins Strategy T routes between.
/// Rumi 3pool coin index is fixed by the pool's own token ordering
/// (verified live 2026-09-02/03): IcUsd=0, CkUsdt=1, CkUsdc=2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StableToken {
    IcUsd,
    CkUsdt,
    CkUsdc,
}

impl StableToken {
    pub const ALL: [StableToken; 3] = [StableToken::IcUsd, StableToken::CkUsdt, StableToken::CkUsdc];

    pub fn decimals(self) -> u8 {
        match self {
            StableToken::IcUsd => 8,
            StableToken::CkUsdt | StableToken::CkUsdc => 6,
        }
    }

    /// Ledger transfer fee, native decimals. icUSD 0.001 (100_000 raw, 8
    /// dec); ckUSDT/ckUSDC 0.01 (10_000 raw, 6 dec) — verified live
    /// 2026-09-02/03 `icrc1_fee` queries, matches the existing
    /// `ICUSD_FEE`/`CKUSDT_FEE`/`CKUSDC_FEE` constants in `arb.rs`.
    pub fn ledger_fee(self) -> u64 {
        match self {
            StableToken::IcUsd => 100_000,
            StableToken::CkUsdt | StableToken::CkUsdc => 10_000,
        }
    }

    pub fn rumi_coin_index(self) -> u8 {
        match self {
            StableToken::IcUsd => 0,
            StableToken::CkUsdt => 1,
            StableToken::CkUsdc => 2,
        }
    }
}

/// Which ICPSwap pool connects a given unordered pair of Strategy T
/// stablecoins. Each of the three pairs has exactly one pool (verified
/// live 2026-09-02/03 `metadata` calls).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClosingPool {
    /// eb25l-dyaaa-aaaar-qb4lq-cai — token0=icUSD, token1=ckUSDC.
    IcusdCkusdc,
    /// jogrm-gqaaa-aaaar-qcg2a-cai — token0=ckUSDT, token1=icUSD.
    IcusdCkusdt,
    /// heq6n-fyaaa-aaaag-qkcpq-cai — token0=ckUSDT, token1=ckUSDC.
    CkusdtCkusdc,
}

impl ClosingPool {
    /// The pool connecting an unordered pair, or `None` if `a == b`.
    pub fn for_pair(a: StableToken, b: StableToken) -> Option<ClosingPool> {
        use StableToken::*;
        match (a, b) {
            (IcUsd, CkUsdc) | (CkUsdc, IcUsd) => Some(ClosingPool::IcusdCkusdc),
            (IcUsd, CkUsdt) | (CkUsdt, IcUsd) => Some(ClosingPool::IcusdCkusdt),
            (CkUsdt, CkUsdc) | (CkUsdc, CkUsdt) => Some(ClosingPool::CkusdtCkusdc),
            _ => None,
        }
    }

    /// `zeroForOne` for a swap FROM `from` on this pool, given each pool's
    /// live-verified token0/token1 ordering. Panics-free: returns `false`
    /// for a `(pool, from)` combination that can't occur for a valid route
    /// (i.e. `from` not one of the pool's two tokens) — callers only ever
    /// invoke this with `from` equal to a route's `rumi_out` or `start`,
    /// which `for_pair` already guarantees is one of the pool's tokens.
    pub fn zero_for_one_from(self, from: StableToken) -> bool {
        use StableToken::*;
        matches!(
            (self, from),
            (ClosingPool::IcusdCkusdc, IcUsd)
                | (ClosingPool::IcusdCkusdt, CkUsdt)
                | (ClosingPool::CkusdtCkusdc, CkUsdt)
        )
    }
}

/// One of the twelve candidates: a directed Rumi 3pool leg (`start` →
/// `rumi_out`), optionally closed back to `start` via the matching ICPSwap
/// pool. `closing: None` is a one-leg inventory conversion.
#[derive(Clone, Copy, Debug)]
pub struct RouteDescriptor {
    pub start: StableToken,
    pub rumi_out: StableToken,
    pub closing: Option<ClosingPool>,
}

/// All twelve candidates: six directed Rumi legs × {stop, close}.
pub fn all_routes() -> Vec<RouteDescriptor> {
    let mut routes = Vec::with_capacity(12);
    for &start in StableToken::ALL.iter() {
        for &rumi_out in StableToken::ALL.iter() {
            if start == rumi_out {
                continue;
            }
            routes.push(RouteDescriptor { start, rumi_out, closing: None });
            if let Some(pool) = ClosingPool::for_pair(rumi_out, start) {
                routes.push(RouteDescriptor { start, rumi_out, closing: Some(pool) });
            }
        }
    }
    routes
}

/// Converts a native-decimal token amount to 6-decimal par-valued USD (each
/// of the three stables is scored at exactly $1 — an explicit operator
/// policy, not a market fact; see the plan's Global Constraints).
pub fn par_usd_6dec(amount_native: u64, token: StableToken) -> i64 {
    let decimals = token.decimals() as u32;
    if decimals >= 6 {
        (amount_native / 10u64.pow(decimals - 6)) as i64
    } else {
        (amount_native * 10u64.pow(6 - decimals)) as i64
    }
}

/// Net profit (6-dec USD) for a one-leg stop: `start` → `rumi_out` via
/// Rumi, no closing leg. `rumi_gross_out` is `calc_swap`'s raw returned
/// amount (native decimals of `rumi_out`). Two fee events apply: the start
/// token's own ledger fee to enter the Rumi leg (a real capital outflow on
/// top of the notional sent, per the four-ledger-movement accounting), and
/// `rumi_out`'s ledger fee on Rumi's own output transfer. There is no
/// closing-leg fee because there is no closing leg.
pub fn one_leg_net_profit_usd(
    start: StableToken,
    start_amount_native: u64,
    rumi_out: StableToken,
    rumi_gross_out: u64,
) -> i64 {
    let net_received = rumi_gross_out.saturating_sub(rumi_out.ledger_fee());
    par_usd_6dec(net_received, rumi_out) - par_usd_6dec(start_amount_native, start) - par_usd_6dec(start.ledger_fee(), start)
}

/// The exact native amount to feed into the closing-leg `quoteForAll` call
/// for a two-leg candidate: Rumi's gross output minus two ledger-fee events
/// on the intermediate token (Rumi's own send-side fee, then our send-side
/// fee into the closing pool) — part of the four-ledger-movement
/// accounting in the plan's Global Constraints.
pub fn closing_leg_input(rumi_out: StableToken, rumi_gross_out: u64) -> u64 {
    rumi_gross_out.saturating_sub(2 * rumi_out.ledger_fee())
}

/// Net profit (6-dec USD) for a two-leg round trip, given the closing
/// pool's raw `quoteForAll` gross output (native decimals of `start`).
/// Three fee events beyond `closing_leg_input`'s two: the start token's
/// entry-leg fee (see `one_leg_net_profit_usd` doc) and the closing pool's
/// own output-transfer fee on the start token — four total across the
/// round trip, matching the audited fixtures in `tests/strategy_t_math.rs`
/// exactly.
pub fn two_leg_net_profit_usd(
    start: StableToken,
    start_amount_native: u64,
    closing_gross_out: u64,
) -> i64 {
    let net_received = closing_gross_out.saturating_sub(start.ledger_fee());
    par_usd_6dec(net_received, start) - par_usd_6dec(start_amount_native, start) - par_usd_6dec(start.ledger_fee(), start)
}
```

- [ ] **Step 4: Register the module** — in `src/arb_bot/src/lib.rs`, change line 10 from `mod arb;` block area to add:

```rust
pub mod strategy_t; // pub so integration tests can reach the pure math
```

placed alongside `pub mod state;` (line 6).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p arb_bot --test strategy_t_math -- --nocapture`
Expected: PASS, all 8 tests green.

- [ ] **Step 6: Reconcile with Task 1's `state::StrategyTToken`/`StrategyTPool`.** These are deliberately a second, independent pair of enums from `strategy_t::StableToken`/`ClosingPool` — Task 1's live in `state.rs` only if `BotConfig` or `CycleSnapshot` need a candid-serializable token/pool tag (e.g. for a future `strategy_used` field). If Task 6 (dry-run result wiring) ends up not needing candid-level token/pool identifiers distinct from `strategy_t`'s own (non-candid) versions, delete `state::StrategyTToken`/`StrategyTPool` from Task 1 rather than carry two parallel enums — decide this concretely in Task 6 Step 1, not here.

- [ ] **Step 7: Commit**

```bash
git add src/arb_bot/src/strategy_t.rs src/arb_bot/src/lib.rs src/arb_bot/tests/strategy_t_math.rs
git commit -m "feat(arb): add Strategy T route model and profit math

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: Live quoting primitives — `quoteForAll` and read-only allowance check

**Files:**
- Modify: `src/arb_bot/src/prices.rs` (add after `fetch_icpswap_quote_for_amount`, line ~283)
- Modify: `src/arb_bot/src/swaps.rs` (add after `approve_infinite_subaccount`, line ~235)

**Interfaces:**
- Consumes: `prices::IcpSwapResult`, `prices::IcpSwapError`, `prices::nat_to_u64` (all already defined in `prices.rs`).
- Produces: `prices::fetch_icpswap_quote_for_all(pool: Principal, amount: u64, zero_for_one: bool) -> Result<u64, String>`; `swaps::query_allowance(token_ledger: Principal, owner: Principal, spender: Principal) -> Result<(u64, Option<u64>), String>` (returns `(allowance, expires_at)`).

- [ ] **Step 1: Add `fetch_icpswap_quote_for_all` to `prices.rs`.** Byte-identical to `fetch_icpswap_quote_for_amount` except the method name — this is intentional duplication, not a refactor, so the two call sites stay independently greppable and Strategy T's sizing path can never accidentally fall back to `quote`:

```rust
/// Like `fetch_icpswap_quote_for_amount` but calls `quoteForAll`, which
/// rejects with an explicit error instead of silently returning a
/// plateaued/partial-fill amount when `amount` exceeds the pool's
/// currently fillable ceiling. Strategy T MUST use this, never `quote`,
/// for candidate sizing (verified live 2026-09-02/03 — see the plan's
/// Global Constraints for the concrete before/after ceiling-move evidence).
pub async fn fetch_icpswap_quote_for_all(
    icpswap_pool: Principal,
    amount: u64,
    zero_for_one: bool,
) -> Result<u64, String> {
    #[derive(CandidType, Serialize)]
    struct SwapArgs {
        #[serde(rename = "amountIn")]
        amount_in: String,
        #[serde(rename = "zeroForOne")]
        zero_for_one: bool,
        #[serde(rename = "amountOutMinimum")]
        amount_out_minimum: String,
    }
    let args = SwapArgs {
        amount_in: amount.to_string(),
        zero_for_one,
        amount_out_minimum: "0".to_string(),
    };
    let result: Result<(IcpSwapResult,), _> = ic_cdk::call(icpswap_pool, "quoteForAll", (args,)).await;
    match result {
        Ok((IcpSwapResult::Ok(out),)) => Ok(nat_to_u64(&out)),
        Ok((IcpSwapResult::Err(e),)) => Err(format!("ICPSwap quoteForAll error: {:?}", e)),
        Err((code, msg)) => Err(format!("ICPSwap quoteForAll call failed ({:?}): {}", code, msg)),
    }
}
```

- [ ] **Step 2: Add `query_allowance` to `swaps.rs`.** Add the import at the top (alongside the existing `icrc_ledger_types` imports at lines 3-5):

```rust
use icrc_ledger_types::icrc2::allowance::{Allowance, AllowanceArgs};
```

Then the function, after `approve_infinite_subaccount` (line 235):

```rust
/// Read-only ICRC-2 allowance check. Never grants or modifies an
/// allowance — Strategy T uses this to report eligibility, not to act on
/// it. Returns `(allowance, expires_at)` in the ledger's native decimals.
pub async fn query_allowance(
    token_ledger: Principal,
    owner: Principal,
    spender: Principal,
) -> Result<(u64, Option<u64>), String> {
    let args = AllowanceArgs {
        account: Account { owner, subaccount: None },
        spender: Account { owner: spender, subaccount: None },
    };
    let result: Result<(Allowance,), _> = ic_cdk::call(token_ledger, "icrc2_allowance", (args,)).await;
    match result {
        Ok((a,)) => Ok((nat_to_u64_saturating(&a.allowance), a.expires_at)),
        Err((code, msg)) => Err(format!("icrc2_allowance call failed ({:?}): {}", code, msg)),
    }
}

/// Saturating Nat->u64 conversion for allowance values, which are commonly
/// set to u128::MAX (see `approve_infinite`) and would otherwise overflow.
/// Strategy T only needs to know "is the allowance >= this trade's input,"
/// so saturating to u64::MAX is exact for every value this module cares
/// about (a candidate never needs more than u64::MAX of a 6-8 decimal
/// token — that is already an absurd, inventory-band-blocked quantity).
fn nat_to_u64_saturating(n: &Nat) -> u64 {
    n.0.to_string().parse::<u64>().unwrap_or(u64::MAX)
}
```

Note: `prices::nat_to_u64` (used elsewhere in `swaps.rs`) truncates via `.unwrap_or(0)`, which would make a real u128::MAX-scale allowance look like `0` (ineligible) instead of `u64::MAX` (eligible) — this is exactly backwards for an eligibility check, which is why `query_allowance` needs its own saturating conversion rather than reusing `prices::nat_to_u64`.

- [ ] **Step 3: `cargo check -p arb_bot`** — expect clean compile.

- [ ] **Step 4: Commit**

```bash
git add src/arb_bot/src/prices.rs src/arb_bot/src/swaps.rs
git commit -m "feat(arb): add quoteForAll and read-only allowance query helpers

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 4: Live candidate assembly

**Files:**
- Modify: `src/arb_bot/src/strategy_t.rs` (append below the `// ─── Live quoting ───` marker)

**Interfaces:**
- Consumes: `swaps::pool_calc_swap(rumi_3pool: Principal, coin_in: u8, coin_out: u8, amount_in: u64) -> Result<u64, String>` (existing, `swaps.rs:175`), `prices::fetch_icpswap_quote_for_all` (Task 3), `StableToken`/`ClosingPool`/`RouteDescriptor`/`all_routes`/profit fns (Task 2).
- Produces: `strategy_t::CandidateQuote { route: RouteDescriptor, economic_profit_usd: i64, fill_status: FillStatus }`, `strategy_t::FillStatus` enum, `strategy_t::quote_all_candidates(rumi_3pool: Principal, pools: PoolPrincipals, start_amount_native: fn(StableToken) -> u64) -> Vec<CandidateQuote>` (async).

- [ ] **Step 1: Append to `strategy_t.rs`**, after a new marker comment:

```rust
// ─── Live quoting (async — calls Rumi 3pool `calc_swap` and ICPSwap
// `quoteForAll`; both are read-only query-style calls, no fund movement) ───

use candid::Principal;

/// The three configured ICPSwap closing-pool principals, resolved from
/// `BotConfig` by the caller (Task 6) — kept out of `state::` here so this
/// module's async layer stays testable against arbitrary principals.
#[derive(Clone, Copy, Debug)]
pub struct PoolPrincipals {
    pub icusd_ckusdc: Principal,
    pub icusd_ckusdt: Principal,
    pub ckusdt_ckusdc: Principal,
}

impl PoolPrincipals {
    fn resolve(self, pool: ClosingPool) -> Principal {
        match pool {
            ClosingPool::IcusdCkusdc => self.icusd_ckusdc,
            ClosingPool::IcusdCkusdt => self.icusd_ckusdt,
            ClosingPool::CkusdtCkusdc => self.ckusdt_ckusdc,
        }
    }
}

/// Why a candidate's economic profit could or couldn't be computed / relied
/// on. `FullyQuoted` is the only status a candidate may be ranked on.
#[derive(Clone, Debug)]
pub enum FillStatus {
    /// Both legs (or the one leg, for a stop) quoted successfully with a
    /// full-fill guarantee (`quoteForAll` for ICPSwap; Rumi's `calc_swap`
    /// has no partial-fill mode).
    FullyQuoted,
    /// Rumi's `calc_swap` call failed or errored.
    RumiQuoteFailed(String),
    /// The closing pool rejected the sizing (partial-fill boundary hit, or
    /// any other `quoteForAll` error) — the candidate is not fillable at
    /// this size right now, not merely "less profitable."
    ClosingQuoteRejected(String),
}

#[derive(Clone, Debug)]
pub struct CandidateQuote {
    pub route: RouteDescriptor,
    pub start_amount_native: u64,
    pub economic_profit_usd: i64, // meaningless unless fill_status is FullyQuoted
    pub fill_status: FillStatus,
}

/// Quotes all twelve candidates for the given per-start-token trade size.
/// Never calls `swap`, `depositFromAndSwap`, `icrc2_approve`, or any other
/// fund-moving method — every call here is `calc_swap` (Rumi, read-only)
/// or `quoteForAll` (ICPSwap, read-only query).
pub async fn quote_all_candidates(
    rumi_3pool: Principal,
    pools: PoolPrincipals,
    start_amount_native: impl Fn(StableToken) -> u64,
) -> Vec<CandidateQuote> {
    let mut results = Vec::with_capacity(12);
    for route in all_routes() {
        let amount_in = start_amount_native(route.start);
        let rumi_result = crate::swaps::pool_calc_swap(
            rumi_3pool,
            route.start.rumi_coin_index(),
            route.rumi_out.rumi_coin_index(),
            amount_in,
        )
        .await;

        let (economic_profit_usd, fill_status) = match (rumi_result, route.closing) {
            (Err(e), _) => (0, FillStatus::RumiQuoteFailed(e)),
            (Ok(rumi_gross), None) => {
                let profit = one_leg_net_profit_usd(route.start, amount_in, route.rumi_out, rumi_gross);
                (profit, FillStatus::FullyQuoted)
            }
            (Ok(rumi_gross), Some(closing_pool)) => {
                let leg2_input = closing_leg_input(route.rumi_out, rumi_gross);
                let zero_for_one = closing_pool.zero_for_one_from(route.rumi_out);
                let closing_result = crate::prices::fetch_icpswap_quote_for_all(
                    pools.resolve(closing_pool),
                    leg2_input,
                    zero_for_one,
                )
                .await;
                match closing_result {
                    Err(e) => (0, FillStatus::ClosingQuoteRejected(e)),
                    Ok(closing_gross) => {
                        let profit = two_leg_net_profit_usd(route.start, amount_in, closing_gross);
                        (profit, FillStatus::FullyQuoted)
                    }
                }
            }
        };

        results.push(CandidateQuote {
            route,
            start_amount_native: amount_in,
            economic_profit_usd,
            fill_status,
        });
    }
    results
}
```

- [ ] **Step 2: `cargo check -p arb_bot`** — expect clean compile. This function is async and calls `ic_cdk::call` transitively (via `swaps::pool_calc_swap` / `prices::fetch_icpswap_quote_for_all`), so it cannot be exercised by a plain `cargo test` without a running replica — no test is written for `quote_all_candidates` itself in this task; Task 2's fixtures already cover its two profit-computation branches exhaustively via the pure functions it calls. Confirm this by inspection, not a new test: `quote_all_candidates` contains no arithmetic of its own beyond what `one_leg_net_profit_usd`/`two_leg_net_profit_usd`/`closing_leg_input` already do.

- [ ] **Step 3: Commit**

```bash
git add src/arb_bot/src/strategy_t.rs
git commit -m "feat(arb): assemble live Strategy T candidate quotes

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 5: Allowance and inventory-band eligibility

**Files:**
- Modify: `src/arb_bot/src/strategy_t.rs`

**Interfaces:**
- Consumes: `swaps::query_allowance` (Task 3), `swaps::icrc1_balance_of_default` (existing, `swaps.rs:288`).
- Produces: `strategy_t::AllowanceStatus` enum, `strategy_t::check_allowance(candidate: &CandidateQuote, token_ledger: Principal, spender: Principal, this_canister: Principal) -> AllowanceStatus` (async), `strategy_t::InventoryCheck { start_ok: bool, end_ok: bool }`, `strategy_t::check_inventory_bands(route: &RouteDescriptor, start_amount: u64, expected_end_amount: u64, balances: TokenBalances, floors: TokenBands, ceilings: TokenBands) -> InventoryCheck` (pure — balances/bands passed in, not fetched here, so this stays unit-testable).

- [ ] **Step 1: Append to `strategy_t.rs`**:

```rust
// ─── Eligibility: allowance (read-only) and inventory bands (pure) ───

/// Whether a candidate's required ICPSwap-side allowance is currently
/// sufficient. This module only ever *reads* allowances — see
/// `swaps::query_allowance` — never grants or modifies one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AllowanceStatus {
    /// No closing leg — nothing to approve (every Rumi 3pool approval
    /// already exists, verified live 2026-09-02/03; a one-leg candidate is
    /// never allowance-blocked on that basis).
    NotRequired,
    /// Allowance covers the candidate's required input.
    Sufficient,
    /// Allowance exists but is smaller than required, or is exactly zero.
    Insufficient { allowance: u64, required: u64 },
}

/// Checks the allowance a two-leg candidate's closing leg needs: the
/// intermediate token approved to spend into the closing ICPSwap pool.
/// Returns `NotRequired` for a one-leg candidate.
pub async fn check_allowance(
    candidate: &CandidateQuote,
    token_ledger: Principal,
    spender: Principal,
    this_canister: Principal,
) -> AllowanceStatus {
    if candidate.route.closing.is_none() {
        return AllowanceStatus::NotRequired;
    }
    let required = closing_leg_input(
        candidate.route.rumi_out,
        // Re-derive the pre-closing-fee gross amount is not available here
        // without re-quoting; callers pass the already-computed leg2 input
        // via `required_override` in Task 6's wiring instead of calling
        // this helper standalone when the exact figure matters. For a
        // conservative (never-false-positive) check, use the candidate's
        // start amount as a floor — any real leg2 input for a profitable
        // candidate is within a few fee-units of it.
        candidate.start_amount_native,
    );
    match crate::swaps::query_allowance(token_ledger, this_canister, spender).await {
        Ok((allowance, _expires_at)) if allowance >= required => AllowanceStatus::Sufficient,
        Ok((allowance, _expires_at)) => AllowanceStatus::Insufficient { allowance, required },
        Err(_) => AllowanceStatus::Insufficient { allowance: 0, required },
    }
}

/// Native-decimal balances/bands for all three tokens, keyed by
/// `StableToken`. A small fixed-size struct rather than a `HashMap` — there
/// are exactly three tokens and this is on the hot path of every dry-run.
#[derive(Clone, Copy, Debug)]
pub struct TokenAmounts {
    pub icusd: u64,
    pub ckusdt: u64,
    pub ckusdc: u64,
}

impl TokenAmounts {
    pub fn get(self, token: StableToken) -> u64 {
        match token {
            StableToken::IcUsd => self.icusd,
            StableToken::CkUsdt => self.ckusdt,
            StableToken::CkUsdc => self.ckusdc,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct InventoryCheck {
    /// False if spending `start_amount` would take the start token's
    /// balance below its configured floor.
    pub start_ok: bool,
    /// False if receiving the expected end amount would take the end
    /// token's balance above its configured ceiling.
    pub end_ok: bool,
}

impl InventoryCheck {
    pub fn eligible(self) -> bool {
        self.start_ok && self.end_ok
    }
}

/// Pure inventory-band check — balances and bands are passed in (fetched
/// by the caller in Task 6) so this stays synchronously unit-testable.
/// `end_token`/`expected_end_amount` are the route's actual output token
/// (== `rumi_out` for a one-leg stop, == `start` for a two-leg close).
pub fn check_inventory_bands(
    start_token: StableToken,
    start_amount: u64,
    end_token: StableToken,
    expected_end_amount: u64,
    balances: TokenAmounts,
    floors: TokenAmounts,
    ceilings: TokenAmounts,
) -> InventoryCheck {
    let start_balance = balances.get(start_token);
    let start_floor = floors.get(start_token);
    let start_ok = start_balance.saturating_sub(start_amount) >= start_floor;

    let end_balance = balances.get(end_token);
    let end_ceiling = ceilings.get(end_token);
    let end_ok = end_balance.saturating_add(expected_end_amount) <= end_ceiling;

    InventoryCheck { start_ok, end_ok }
}
```

- [ ] **Step 2: Add pure tests for `check_inventory_bands`** to `tests/strategy_t_math.rs` (no network needed — pure function):

```rust
use arb_bot::strategy_t::{check_inventory_bands, StableToken, TokenAmounts};

#[test]
fn inventory_bands_block_start_leg_below_floor() {
    let balances = TokenAmounts { icusd: 0, ckusdt: 0, ckusdc: 10_000_000 }; // $10 ckUSDC
    let floors = TokenAmounts { icusd: 0, ckusdt: 0, ckusdc: 5_000_000 };    // $5 floor
    let ceilings = TokenAmounts { icusd: 200_000_000_000, ckusdt: 2_000_000_000, ckusdc: 2_000_000_000 };
    // Spending $8 would leave $2 balance, below the $5 floor.
    let check = check_inventory_bands(
        StableToken::CkUsdc, 8_000_000, StableToken::IcUsd, 0,
        balances, floors, ceilings,
    );
    assert!(!check.start_ok);
    assert!(!check.eligible());
}

#[test]
fn inventory_bands_block_end_leg_above_ceiling() {
    let balances = TokenAmounts { icusd: 199_000_000_000, ckusdt: 0, ckusdc: 100_000_000 };
    let floors = TokenAmounts { icusd: 0, ckusdt: 0, ckusdc: 5_000_000 };
    let ceilings = TokenAmounts { icusd: 200_000_000_000, ckusdt: 2_000_000_000, ckusdc: 2_000_000_000 };
    // Receiving 2000 icUSD would push balance to 199_002B + ... over the 200_000_000_000 ceiling.
    let check = check_inventory_bands(
        StableToken::CkUsdc, 10_000_000, StableToken::IcUsd, 2_000_000_000,
        balances, floors, ceilings,
    );
    assert!(check.start_ok);
    assert!(!check.end_ok);
    assert!(!check.eligible());
}

#[test]
fn inventory_bands_pass_within_range() {
    let balances = TokenAmounts { icusd: 771_230_051_57, ckusdt: 402_313_538, ckusdc: 73_465_211 };
    let floors = TokenAmounts { icusd: 500_000_000, ckusdt: 5_000_000, ckusdc: 5_000_000 };
    let ceilings = TokenAmounts { icusd: 200_000_000_000, ckusdt: 2_000_000_000, ckusdc: 2_000_000_000 };
    let check = check_inventory_bands(
        StableToken::CkUsdc, 10_000_000, StableToken::IcUsd, 10_400_000_00,
        balances, floors, ceilings,
    );
    assert!(check.eligible());
}
```

- [ ] **Step 3: Run** `cargo test -p arb_bot --test strategy_t_math` — expect all tests (Task 2's 8 + these 3) passing.

- [ ] **Step 4: Commit**

```bash
git add src/arb_bot/src/strategy_t.rs src/arb_bot/tests/strategy_t_math.rs
git commit -m "feat(arb): add Strategy T allowance and inventory-band eligibility checks

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 6: Ranking, dry-run result, and `dry_run_strategy_t()` wiring

**Files:**
- Modify: `src/arb_bot/src/strategy_t.rs` (ranking + top-level `evaluate()` entry point)
- Modify: `src/arb_bot/src/state.rs` (resolve Task 2 Step 6's decision: keep or drop `StrategyTToken`/`StrategyTPool`; add `CycleSnapshot` summary fields)
- Modify: `src/arb_bot/src/lib.rs` (new `#[update] async fn dry_run_strategy_t()`, following the `dry_run_strategy_c()` pattern at line 991)

**Interfaces:**
- Consumes: everything from Tasks 2–5.
- Produces: `strategy_t::StrategyTDryRunResult { candidates: Vec<CandidateReport>, best_economic: Option<CandidateReport>, best_executable: Option<CandidateReport> }` (candid-serializable), `lib::dry_run_strategy_t() -> strategy_t::StrategyTDryRunResult`.

- [ ] **Step 1: Resolve Task 2 Step 6.** `CandidateReport` (below) needs candid-serializable token/pool tags for the dashboard. Reuse `state::StrategyTToken`/`state::StrategyTPool` from Task 1 for this — do not duplicate a third pair of enums. Add `From` conversions in `strategy_t.rs`:

```rust
impl From<StableToken> for crate::state::StrategyTToken {
    fn from(t: StableToken) -> Self {
        match t {
            StableToken::IcUsd => crate::state::StrategyTToken::IcUsd,
            StableToken::CkUsdt => crate::state::StrategyTToken::CkUsdt,
            StableToken::CkUsdc => crate::state::StrategyTToken::CkUsdc,
        }
    }
}

impl From<ClosingPool> for crate::state::StrategyTPool {
    fn from(p: ClosingPool) -> Self {
        match p {
            ClosingPool::IcusdCkusdc => crate::state::StrategyTPool::IcusdCkusdc,
            ClosingPool::IcusdCkusdt => crate::state::StrategyTPool::IcusdCkusdt,
            ClosingPool::CkusdtCkusdc => crate::state::StrategyTPool::CkusdtCkusdc,
        }
    }
}
```

**Step 1b (required — controller ruling, see ledger): extend `CandidateQuote` a second time.** Task 5's fix round already added `closing_leg_input_native: Option<u64>` to `CandidateQuote` to fix a decimals-mismatch bug in allowance checking (recomputing a value in the wrong token's decimals instead of reusing the one `quote_all_candidates` already computed correctly). The inventory-band check in Step 3 below needs the exact same treatment for the same reason: it needs the candidate's true native-decimal end-token amount, and computing it from `economic_profit_usd` (a 6-decimal-USD par-valued figure) mixes units exactly like the fixed bug did whenever the end token isn't 6-decimal (i.e. any route ending in icUSD, 8-decimal). Add one more field to `CandidateQuote` (in the struct definition inside the "Live quoting" section Task 4 added):

```rust
pub struct CandidateQuote {
    pub route: RouteDescriptor,
    pub start_amount_native: u64,
    pub economic_profit_usd: i64,
    pub fill_status: FillStatus,
    pub closing_leg_input_native: Option<u64>,
    /// The candidate's actual native-decimal end-token amount net of all
    /// fees — for a one-leg stop, `rumi_gross_out` minus `rumi_out`'s own
    /// ledger fee; for a two-leg close, `closing_gross_out` minus
    /// `start`'s ledger fee (the same `net_received` value each of Task
    /// 2's profit functions computes internally, just also surfaced here).
    /// Zero when the candidate's fill_status isn't `FullyQuoted`. This
    /// must be used for any native-decimal inventory/balance check — never
    /// reconstructed from `economic_profit_usd`, which is USD-denominated
    /// and mixes decimals across tokens exactly like the bug Task 5's fix
    /// round corrected.
    pub net_end_amount_native: u64,
}
```

And set it in `quote_all_candidates`'s three match arms (in the same function Task 4 wrote, alongside where `closing_leg_input_native` is already set per Task 5's fix):
- `(Err(e), _)` arm: `net_end_amount_native: 0`
- One-leg success arm: `net_end_amount_native: rumi_gross.saturating_sub(route.rumi_out.ledger_fee())`
- Two-leg success arm: `net_end_amount_native: closing_gross.saturating_sub(route.start.ledger_fee())`
- Two-leg closing-quote-rejected arm: `net_end_amount_native: 0`

- [ ] **Step 2: Add the candid-facing report types and ranking to `strategy_t.rs`**:

```rust
// ─── Reporting and ranking ───

use candid::CandidType;

#[derive(CandidType, Clone, Debug)]
pub struct CandidateReport {
    pub start: crate::state::StrategyTToken,
    pub rumi_out: crate::state::StrategyTToken,
    pub closing: Option<crate::state::StrategyTPool>,
    pub start_amount_native: u64,
    pub economic_profit_usd: i64,
    pub meets_profit_threshold: bool,
    pub allowance_status: String, // Display of AllowanceStatus — candid-simple, dashboard-friendly
    pub inventory_eligible: bool,
    pub fill_ok: bool,
    pub fill_note: String, // empty if fill_ok, else the FillStatus error text
}

#[derive(CandidType, Clone, Debug)]
pub struct StrategyTDryRunResult {
    pub candidates: Vec<CandidateReport>,
    /// Highest `economic_profit_usd` among fully-quoted candidates that
    /// clear the profit threshold — regardless of allowance/inventory
    /// eligibility. A profitable route must never disappear from this
    /// list just because it isn't executable today.
    pub best_economic: Option<CandidateReport>,
    /// Highest `economic_profit_usd` among candidates that ALSO have
    /// `allowance_status == Sufficient/NotRequired` AND `inventory_eligible`.
    /// `None` if no candidate is both profitable and currently executable.
    pub best_executable: Option<CandidateReport>,
}

/// Ranks a fully-assembled, fully-checked candidate list. Pure — no
/// network calls; Task 6 Step 3's `evaluate()` does the async assembly and
/// calls this at the end.
pub fn rank_candidates(reports: Vec<CandidateReport>) -> StrategyTDryRunResult {
    let best_economic = reports
        .iter()
        .filter(|r| r.fill_ok && r.meets_profit_threshold)
        .max_by_key(|r| r.economic_profit_usd)
        .cloned();

    let best_executable = reports
        .iter()
        .filter(|r| {
            r.fill_ok
                && r.meets_profit_threshold
                && r.inventory_eligible
                && matches!(r.allowance_status.as_str(), "NotRequired" | "Sufficient")
        })
        .max_by_key(|r| r.economic_profit_usd)
        .cloned();

    StrategyTDryRunResult { candidates: reports, best_economic, best_executable }
}
```

- [ ] **Step 3: Add the top-level async entry point to `strategy_t.rs`**:

```rust
/// Full Strategy T dry-run evaluation: quotes all twelve candidates,
/// checks allowance and inventory eligibility for each, and ranks them.
/// Every call this function makes is read-only (see Task 4/5 doc comments
/// for the exhaustive list). Requires all three closing pools to be
/// configured (non-anonymous) — returns an empty result otherwise.
pub async fn evaluate(
    rumi_3pool: Principal,
    pools: PoolPrincipals,
    this_canister: Principal,
    ledgers: TokenLedgers,
    start_amount_native: impl Fn(StableToken) -> u64 + Copy,
    min_profit_usd: i64,
    min_profit_bps: u32,
    balances: TokenAmounts,
    floors: TokenAmounts,
    ceilings: TokenAmounts,
) -> StrategyTDryRunResult {
    let quotes = quote_all_candidates(rumi_3pool, pools, start_amount_native).await;

    let mut reports = Vec::with_capacity(quotes.len());
    for q in quotes {
        let start_amount = q.start_amount_native;
        let bps_profit = if start_amount == 0 {
            0i64
        } else {
            (q.economic_profit_usd as i128 * 10_000 / par_usd_6dec(start_amount, q.route.start).max(1) as i128) as i64
        };
        let meets_profit_threshold =
            q.economic_profit_usd >= min_profit_usd && bps_profit >= min_profit_bps as i64;

        let allowance_status = match q.route.closing {
            None => AllowanceStatus::NotRequired,
            Some(pool) => {
                let token_ledger = ledgers.get(q.route.rumi_out);
                let spender = pools.resolve(pool);
                check_allowance(&q, token_ledger, spender, this_canister).await
            }
        };

        let end_token = q.route.closing.map(|_| q.route.start).unwrap_or(q.route.rumi_out);
        let expected_end_amount = q.net_end_amount_native;
        let inventory = check_inventory_bands(
            q.route.start, start_amount, end_token, expected_end_amount, balances, floors, ceilings,
        );

        let (fill_ok, fill_note) = match &q.fill_status {
            FillStatus::FullyQuoted => (true, String::new()),
            FillStatus::RumiQuoteFailed(e) => (false, format!("Rumi quote failed: {e}")),
            FillStatus::ClosingQuoteRejected(e) => (false, format!("Closing quote rejected: {e}")),
        };

        reports.push(CandidateReport {
            start: q.route.start.into(),
            rumi_out: q.route.rumi_out.into(),
            closing: q.route.closing.map(Into::into),
            start_amount_native: start_amount,
            economic_profit_usd: q.economic_profit_usd,
            meets_profit_threshold,
            allowance_status: format!("{:?}", allowance_status).split(' ').next().unwrap_or("").trim_end_matches('{').to_string(),
            inventory_eligible: inventory.eligible(),
            fill_ok,
            fill_note,
        });
    }

    rank_candidates(reports)
}

/// The three stablecoin ledger principals, resolved by the caller from
/// `BotConfig` (`icusd_ledger`, `ckusdt_ledger`, `ckusdc_ledger`).
#[derive(Clone, Copy, Debug)]
pub struct TokenLedgers {
    pub icusd: Principal,
    pub ckusdt: Principal,
    pub ckusdc: Principal,
}

impl TokenLedgers {
    fn get(self, token: StableToken) -> Principal {
        match token {
            StableToken::IcUsd => self.icusd,
            StableToken::CkUsdt => self.ckusdt,
            StableToken::CkUsdc => self.ckusdc,
        }
    }
}
```

`expected_end_amount` now comes directly from `q.net_end_amount_native` (Step 1b) — the candidate's actual native-decimal end-token amount, in the same units as `balances`/`floors`/`ceilings`. Do not reconstruct it from `economic_profit_usd`/`par_usd_6dec` — see Step 1b's doc comment for why that mixes units.

- [ ] **Step 4: Wire `dry_run_strategy_t()` in `lib.rs`**, following `dry_run_strategy_c()`'s pattern (line 991) — read config, check the three pools are configured, delegate to `strategy_t::evaluate`:

```rust
#[update]
async fn dry_run_strategy_t() -> strategy_t::StrategyTDryRunResult {
    let config = state::read_state(|s| s.config.clone());
    if config.strategy_t_icusd_ckusdc_pool == Principal::anonymous()
        || config.strategy_t_icusd_ckusdt_pool == Principal::anonymous()
        || config.strategy_t_ckusdt_ckusdc_pool == Principal::anonymous()
    {
        return strategy_t::StrategyTDryRunResult {
            candidates: vec![],
            best_economic: None,
            best_executable: None,
        };
    }

    let this_canister = ic_cdk::id();
    let pools = strategy_t::PoolPrincipals {
        icusd_ckusdc: config.strategy_t_icusd_ckusdc_pool,
        icusd_ckusdt: config.strategy_t_icusd_ckusdt_pool,
        ckusdt_ckusdc: config.strategy_t_ckusdt_ckusdc_pool,
    };
    let ledgers = strategy_t::TokenLedgers {
        icusd: config.icusd_ledger,
        ckusdt: config.ckusdt_ledger,
        ckusdc: config.ckusdc_ledger,
    };

    let (icusd_bal, ckusdt_bal, ckusdc_bal) = futures::future::join3(
        swaps::icrc1_balance_of_default(config.icusd_ledger),
        swaps::icrc1_balance_of_default(config.ckusdt_ledger),
        swaps::icrc1_balance_of_default(config.ckusdc_ledger),
    ).await;
    let balances = strategy_t::TokenAmounts {
        icusd: icusd_bal.unwrap_or(0),
        ckusdt: ckusdt_bal.unwrap_or(0),
        ckusdc: ckusdc_bal.unwrap_or(0),
    };
    let floors = strategy_t::TokenAmounts {
        icusd: config.strategy_t_icusd_floor,
        ckusdt: config.strategy_t_ckusdt_floor,
        ckusdc: config.strategy_t_ckusdc_floor,
    };
    let ceilings = strategy_t::TokenAmounts {
        icusd: config.strategy_t_icusd_ceiling,
        ckusdt: config.strategy_t_ckusdt_ceiling,
        ckusdc: config.strategy_t_ckusdc_ceiling,
    };

    let max_trade_usd = config.strategy_t_max_trade_size_usd;
    let start_amount_native = move |token: strategy_t::StableToken| -> u64 {
        // 6-dec USD max trade size converted to the token's native decimals
        // at $1 peg — matches `par_usd_6dec`'s inverse.
        let decimals = token.decimals() as u32;
        if decimals >= 6 {
            max_trade_usd * 10u64.pow(decimals - 6)
        } else {
            max_trade_usd / 10u64.pow(6 - decimals)
        }
    };

    strategy_t::evaluate(
        config.rumi_3pool,
        pools,
        this_canister,
        ledgers,
        start_amount_native,
        config.strategy_t_min_profit_usd,
        config.strategy_t_min_profit_bps,
        balances,
        floors,
        ceilings,
    ).await
}
```

Add `use crate::strategy_t;` near the top of `lib.rs` alongside the other `use crate::...` lines if not already present via the `mod` declaration.

- [ ] **Step 5: `cargo check -p arb_bot`** — expect clean compile (candid drift expected until Task 8).

- [ ] **Step 6: Commit**

```bash
git add src/arb_bot/src/strategy_t.rs src/arb_bot/src/state.rs src/arb_bot/src/lib.rs
git commit -m "feat(arb): wire Strategy T dry-run ranking and dry_run_strategy_t()

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 7: Admin config setters

**Files:**
- Modify: `src/arb_bot/src/lib.rs` (new setters, following `set_bob_inventory_band` pattern at line ~1908)

**Interfaces:**
- Produces: `set_strategy_t_pools(icusd_ckusdc: Principal, icusd_ckusdt: Principal, ckusdt_ckusdc: Principal) -> Result<(), String>`, `set_strategy_t_enabled(enabled: bool)`, `set_strategy_t_dry_run(dry_run: bool)`, `set_strategy_t_thresholds(min_profit_usd: i64, min_profit_bps: u32, max_trade_size_usd: u64) -> Result<(), String>`, `set_strategy_t_icusd_band(floor: u64, ceiling: u64) -> Result<(), String>`, `set_strategy_t_ckusdt_band(floor: u64, ceiling: u64) -> Result<(), String>`, `set_strategy_t_ckusdc_band(floor: u64, ceiling: u64) -> Result<(), String>`.

- [ ] **Step 1: Add to `lib.rs`**, after the existing `set_bob_inventory_band`:

```rust
#[update]
fn set_strategy_t_pools(icusd_ckusdc: Principal, icusd_ckusdt: Principal, ckusdt_ckusdc: Principal) -> Result<(), String> {
    require_admin();
    state::mutate_state(|s| {
        s.config.strategy_t_icusd_ckusdc_pool = icusd_ckusdc;
        s.config.strategy_t_icusd_ckusdt_pool = icusd_ckusdt;
        s.config.strategy_t_ckusdt_ckusdc_pool = ckusdt_ckusdc;
    });
    state::log_activity("admin", "strategy T pool principals updated");
    Ok(())
}

/// Enables Strategy T's dry-run evaluation from the arb cycle (Task 8).
/// This build has no live-trade path — flipping this on never moves funds.
#[update]
fn set_strategy_t_enabled(enabled: bool) {
    require_admin();
    state::mutate_state(|s| s.config.strategy_t_enabled = enabled);
    state::log_activity("admin", &format!("strategy_t_enabled set to {}", enabled));
}

#[update]
fn set_strategy_t_dry_run(dry_run: bool) {
    require_admin();
    state::mutate_state(|s| s.config.strategy_t_dry_run = dry_run);
    state::log_activity("admin", &format!("strategy_t_dry_run set to {}", dry_run));
}

#[update]
fn set_strategy_t_thresholds(min_profit_usd: i64, min_profit_bps: u32, max_trade_size_usd: u64) -> Result<(), String> {
    require_admin();
    if max_trade_size_usd == 0 {
        return Err("max_trade_size_usd must be > 0".into());
    }
    state::mutate_state(|s| {
        s.config.strategy_t_min_profit_usd = min_profit_usd;
        s.config.strategy_t_min_profit_bps = min_profit_bps;
        s.config.strategy_t_max_trade_size_usd = max_trade_size_usd;
    });
    state::log_activity(
        "admin",
        &format!("strategy T thresholds set: min_profit_usd={} min_profit_bps={} max_trade_size_usd={}", min_profit_usd, min_profit_bps, max_trade_size_usd),
    );
    Ok(())
}

#[update]
fn set_strategy_t_icusd_band(floor: u64, ceiling: u64) -> Result<(), String> {
    require_admin();
    if ceiling <= floor {
        return Err("ceiling must be > floor".into());
    }
    state::mutate_state(|s| {
        s.config.strategy_t_icusd_floor = floor;
        s.config.strategy_t_icusd_ceiling = ceiling;
    });
    state::log_activity("admin", &format!("strategy T icUSD band set to [{}, {}]", floor, ceiling));
    Ok(())
}

#[update]
fn set_strategy_t_ckusdt_band(floor: u64, ceiling: u64) -> Result<(), String> {
    require_admin();
    if ceiling <= floor {
        return Err("ceiling must be > floor".into());
    }
    state::mutate_state(|s| {
        s.config.strategy_t_ckusdt_floor = floor;
        s.config.strategy_t_ckusdt_ceiling = ceiling;
    });
    state::log_activity("admin", &format!("strategy T ckUSDT band set to [{}, {}]", floor, ceiling));
    Ok(())
}

#[update]
fn set_strategy_t_ckusdc_band(floor: u64, ceiling: u64) -> Result<(), String> {
    require_admin();
    if ceiling <= floor {
        return Err("ceiling must be > floor".into());
    }
    state::mutate_state(|s| {
        s.config.strategy_t_ckusdc_floor = floor;
        s.config.strategy_t_ckusdc_ceiling = ceiling;
    });
    state::log_activity("admin", &format!("strategy T ckUSDC band set to [{}, {}]", floor, ceiling));
    Ok(())
}
```

None of these seven setters grant, revoke, or touch any token allowance — they only ever write to `BotConfig`. There is deliberately no `approve_strategy_t_*` method anywhere in this plan.

- [ ] **Step 2: `cargo check -p arb_bot`** — expect clean compile.

- [ ] **Step 3: Commit**

```bash
git add src/arb_bot/src/lib.rs
git commit -m "feat(arb): add Strategy T admin config setters

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 8: Candid triple-sync and full verification

**Files:**
- Modify: `src/arb_bot/arb_bot.did`
- Modify: `src/arb_bot/src/dashboard.html` (IDL block only — this task does not build a UI panel, just keeps the hand-maintained candid decoder in sync so the existing dashboard doesn't hard-fail decoding `get_config`)

**Interfaces:** None new — this task only synchronizes existing signatures across the three candid sources.

- [ ] **Step 1: Generate the Rust-side candid** to see the exact new shapes:

```bash
cargo test -p arb_bot --test candid print_generated_candid -- --ignored --nocapture
```

- [ ] **Step 2: Update `arb_bot.did`** — add the 13 new `BotConfig` fields (in the same order as the Rust struct, per existing convention), the new `CandidateReport`/`StrategyTDryRunResult` records, and the 8 new method signatures (`dry_run_strategy_t`, `set_strategy_t_pools`, `set_strategy_t_enabled`, `set_strategy_t_dry_run`, `set_strategy_t_thresholds`, `set_strategy_t_icusd_band`, `set_strategy_t_ckusdt_band`, `set_strategy_t_ckusdc_band`), copying the exact types from Step 1's output.

- [ ] **Step 3: Update the `IDL.*` block in `dashboard.html`** (line ~710+) — extend the `BotConfig` record literal with the 13 new fields, in the same order, and add `IDL` record/variant definitions for `CandidateReport` and `StrategyTDryRunResult` plus the new service methods, mirroring the style already used for e.g. `TradeLeg`/`LegType` at lines 718-719.

- [ ] **Step 4: Run the full guard**:

```bash
scripts/check-candid.sh
```

Expected: exits 0. If it reports drift, the output names exactly which of the three sources disagrees — fix that source, do not edit the script.

- [ ] **Step 5: Run the full test suite**:

```bash
cargo build && cargo test
```

Expected: clean build, all tests passing, including the pre-existing 9 tests from the baseline (Task 0, before this plan started) plus Task 1's 1 new decode test and Task 2/5's 11 new pure-math tests.

- [ ] **Step 6: Final review against Global Constraints** — grep for any accidental fund-moving call introduced under the `strategy_t` name:

```bash
grep -n "icrc2_approve\|icrc1_transfer\|\"swap\"\|depositFromAndSwap" src/arb_bot/src/strategy_t.rs
```

Expected: no output. If anything matches, stop — it violates this plan's Global Constraints and must be removed before committing.

- [ ] **Step 7: Commit**

```bash
git add src/arb_bot/arb_bot.did src/arb_bot/src/dashboard.html
git commit -m "chore(arb): sync candid for Strategy T (arb_bot.did + dashboard IDL)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

- [ ] **Step 8: Report final state to the operator** — this plan's implementation is now complete in the worktree branch `worktree-strategy-t-router`. Explicitly state: no push, no PR, no deploy, no `dfx canister install`/`upgrade`, no `icrc2_approve` call, and no live trade occurred at any point in this plan's execution. Those remain separately authorized future steps, exactly as scoped by the operator.

---

## Self-Review

**Spec coverage:**
- Six directed Rumi legs × {stop, close} = 12 candidates → Task 2 (`all_routes`), verified by test.
- Three matching ICPSwap pools named explicitly → Global Constraints + Task 1 config fields.
- Par valuation with separate native/inventory tracking → Task 2 (`par_usd_6dec` used only for scoring) + Task 5 (`TokenAmounts` in native decimals for bands).
- Highest net-dollar-profit selection → Task 6 `rank_candidates` (`max_by_key(economic_profit_usd)`, not percentage).
- Configurable absolute + percentage thresholds → Task 1 fields + Task 6 `meets_profit_threshold`.
- `quoteForAll`/full-fill validation → Task 3 + Task 4 (`quote` never called from `strategy_t.rs`, checked in Task 8 Step 6 by grep for `depositFromAndSwap`/fund-moving calls — `fetch_icpswap_quote_for_all` itself is intentionally the only quote path used).
- Per-token inventory bands → Task 1 config + Task 5 `check_inventory_bands`.
- Asset-parameterized phase machine/accounting/route descriptors → `RouteDescriptor`/`StableToken`/`ClosingPool` are the single parameterized model for all three assets (Task 2); no per-asset code fork anywhere in Tasks 2-6.
- One-leg = inventory conversion, not round trip, exhaustion prevention → Task 2 (`one_leg_net_profit_usd` distinct function/fee model) + Task 5 (floor check applies identically to one-leg and two-leg starts).
- Best economic vs. best executable, allowance visible as blocker not silent exclusion → Task 6 `StrategyTDryRunResult { best_economic, best_executable }` — a profitable-but-blocked candidate stays in `candidates` and can still be `best_economic`.
- ckUSDT always evaluated → `all_routes()` has no asset exclusion; Task 6's threshold check is the only gate, applied uniformly.
- Three-ICPSwap triangle stays quote-observed, outside v1 execution → not implemented in this plan at all (no triangle candidate exists in `all_routes()` — it was locked out of v1 in the prior review, and no task here reintroduces it).
- Bounded/expiring allowances preferred, no automatic grants → Task 3/5 are read-only allowance *queries*; zero `icrc2_approve` calls anywhere in the plan (verified mechanically in Task 8 Step 6).
- Include first approval fee in setup cost — **not applicable to this plan**: since no approval is granted here (approvals remain a separately authorized future step per the operator), there is no approval fee to book yet. Flag this explicitly to the operator in Task 8 Step 8's report so it isn't forgotten when that future step is planned.
- Dry-run every two-leg direction — Task 6's `evaluate()` runs all twelve unconditionally every call.

**Placeholder scan:** No `TODO`/`TBD`/"handle appropriately" found in any task's code blocks. `check_allowance` in Task 5 Step 1 contains an inline comment explaining a real, deliberate approximation (using `start_amount_native` as a conservative required-allowance floor rather than re-deriving the exact leg-2 input) — not a placeholder, a documented simplification with its rationale and consequence stated.

**Type consistency:** `StableToken`/`ClosingPool`/`RouteDescriptor` (Task 2) are used identically through Tasks 4-6. `CandidateQuote` (Task 4) → `CandidateReport` (Task 6) conversion is explicit and total (every field mapped). `TokenAmounts`/`TokenLedgers`/`PoolPrincipals` (Tasks 4-6) are each defined once and reused, not redefined. `state::StrategyTToken`/`StrategyTPool` (Task 1) are resolved by Task 6 Step 1 to be the sole candid-facing token/pool tags, with explicit `From` impls — no duplicate third enum pair survives.

---

**Plan complete and saved to `docs/superpowers/plans/2026-09-03-strategy-t-router.md`. Two execution options:**

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
