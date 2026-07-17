# Volume Bot — icUSD/BOB Pool Integration (PR #11)

> **For agentic workers:** Executed by a subagent. Read the impact map in this session's context and the strategy-S plan (`2026-07-16-strategy-s-bob.md`) first. Steps use checkbox syntax.

**Goal:** Add the icUSD/BOB ICPSwap pool to the volume bot with ping-pong + BOB-drift rebalance, ships inert (disabled + anonymous pool), stacked on `feat/strategy-s-bob`.

**Architecture:** Special-case a third `VolumePool::IcusdBob` variant (Approach B from the impact map). BOB is NOT $1-pegged, so its USD sizing/marking goes through strategy S's reference helpers (`median_stable_usd_per_icp`, the `ref_icusd_per_bob = icp_per_bob × usd_per_icp` formula) rather than the flat `×100` the ICP-paired pools use. Rebalance is a DIRECT one-hop unwind through the icUSD/BOB pool itself (BOB↔icUSD via `icpswap_swap`), not a Rumi/3pool route.

**Tech Stack:** Rust IC canister; hand-maintained candid (lib.rs / arb_bot.did / dashboard.html IDL); `scripts/check-candid.sh`.

## Global Constraints

- Every new `VolumeConfig`/`VolumeStats` field: `#[serde(default)]` or default-fn + `Default` impl entry (state is serde_json in stable memory; a missing default bricks the upgrade). No migration struct edits needed — `VolumeConfig` lives inside the `BotState` blob which appends safely.
- Additive only: add `VolumeDirection::BuyBob`/`SellBob`; do NOT rename `BuyIcp`/`SellIcp`. Add `VolumePool::IcusdBob`; existing arms unchanged.
- Pool identity/ordering comes from EXISTING `BotConfig` fields (strategy S already added them): `icpswap_icusd_bob_pool`, `icpswap_icusd_bob_icusd_is_token0`, `bob_ledger`, `bob_ledger_fee`. Do NOT add new BotConfig fields.
- Ships inert: `icusd_bob: VolumePoolConfig` defaults `enabled=false`, and pool principal defaults anonymous. No trading until an admin enables it AND sets the pool.
- BOB is a standard ICRC ledger — it honors subaccounts. Do NOT apply the `is_3usd` no-subaccount special-case to BOB; it follows the icUSD code path.
- **`check-candid.sh` does NOT cover volume types vs the dashboard IDL** (only Rust↔.did via cargo test). Manually diff the dashboard IDL block against `arb_bot.did` for every volume type touched.
- Reuse strategy S helpers in `arb.rs` — make them `pub(crate)` if they aren't already; verify exact names (`median_stable_usd_per_icp`, `stable_usd_per_icp_candidates`, `mark_icp_usd`/`mark_bob_usd`). Do NOT duplicate the reference math.
- Commit style `feat(arb): ...`, trailer `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task V1: State + config plumbing (inert)

**Files:** `state.rs`, `lib.rs` (`set_volume_config` match ~1776, `get_volume_stats` ~2086), `arb_bot.did`, `dashboard.html` (IDL).

- [ ] `VolumePool` +`IcusdBob` (state.rs ~335). `VolumeDirection` +`BuyBob`,`SellBob` (~341).
- [ ] `VolumeConfig` +`icusd_bob: VolumePoolConfig` +`icusd_bob_state: VolumePoolState` with serde-defaults + `Default` entries (~398-427). Reuse `VolumePoolConfig::default()` shape (enabled=false).
- [ ] `VolumeStats` +`icusd_bob: VolumePoolStatus` +`daily_cost_cap_usd_icusd_bob: u64` (~444), serde-defaulted.
- [ ] `set_volume_config` match +`IcusdBob => s.volume.icusd_bob = new_config` arm.
- [ ] `get_volume_stats` +`icusd_bob` field reads + include its `trade_count` in `total_trade_count`.
- [ ] Candid: `.did` + dashboard IDL for `VolumePool`, `VolumeDirection`, `VolumeStats`. Run `scripts/check-candid.sh` (Rust↔.did) AND hand-diff the dashboard IDL.
- [ ] `cargo check` + `cargo test -p arb_bot` clean. Commit: `feat(arb): volume bot icUSD/BOB state plumbing (inert)`.

### Task V2: Execute path (ping-pong)

**Files:** `volume.rs` (`execute_volume_trade` 60-188, `run_volume_cycle` 254-438), `lib.rs` (`get_bot_health` 1973-2018).

- [ ] `execute_volume_trade` pool→venue tuple (~68): +`IcusdBob` arm returning `(icpswap_icusd_bob_pool, icusd_is_token0, bob_ledger, bob_ledger_fee, 8)`. Generalize the tuple's semantics so the "other leg" isn't assumed to be ICP.
- [ ] Direction→native-amount (~92-110): for `IcusdBob`, the icUSD leg uses flat `×100` (icUSD is $1), the BOB leg must be sized via the reference price (`ref_icusd_per_bob`), NOT flat `×100`. `BuyBob` = spend icUSD to buy BOB; `SellBob` = sell BOB for icUSD.
- [ ] `min_native` (~329-344): +`IcusdBob`/`BuyBob`,`SellBob` arms computing the BOB side via reference price.
- [ ] USD-conversion outcome arms (~358-383): +2 arms for `(BuyBob|SellBob, IcusdBob)` — icUSD side flat `/100`, BOB side `amount × ref_icusd_per_bob / 1e8 / 100` (verify scaling against S's `mark_bob_usd`).
- [ ] `run_volume_cycle` pool loop array (~254) +`IcusdBob`; config/state read (~255), ICPSwap pool/ordering read (~276), `input_token` (~304), `VolumeTradeLeg` token_out (~392), state mutation (~407) all +arms. Direction toggle: `BuyBob↔SellBob`.
- [ ] `get_bot_health`: pools array (~1973) +`IcusdBob`; inner arms (~1981,1986,1993,2008) +`IcusdBob`. Price shown = icUSD-per-BOB reference.
- [ ] `cargo check`+`cargo test` clean. Commit: `feat(arb): volume bot icUSD/BOB ping-pong execution`.

### Task V3: Rebalance (BOB drift → 50/50 by value)

**Files:** `volume.rs` (`run_rebalance` 444-597).

- [ ] `run_rebalance` pool array (~448) +`IcusdBob`; pool-config/ledger/pool lookups (~449-474) +arms.
- [ ] For `IcusdBob`, a SEPARATE drift branch (do not reuse the ICP/Rumi arms): read subaccount BOB + icUSD balances; value BOB via `median_stable_usd_per_icp` × BOB/ICP quote (reuse S's `ref_icusd_per_bob`); compute `bob_as_usd` vs `icusd_usd`; target 50/50.
- [ ] Unwind is ONE hop through the icUSD/BOB pool: too much BOB → `transfer_from_subaccount(bob_ledger)` → `icpswap_swap(icpswap_icusd_bob_pool, BOB→icUSD)` → `transfer_to_subaccount(icusd_ledger)`. Too much icUSD → mirror (icUSD→BOB). No BOB/ICP, no Rumi, no 3pool. Recover-on-failure back to subaccount like the existing arms.
- [ ] `cargo check`+`cargo test` clean. Commit: `feat(arb): volume bot icUSD/BOB rebalance (direct one-hop unwind)`.

### Task V4: Approvals + dashboard

**Files:** `lib.rs` (`setup_approvals` volume block ~450-465), `dashboard.html`.

- [ ] `setup_approvals` volume vec +2 subaccount approvals (mirror existing pattern; the default-account approvals for the BOB pools already exist from strategy S task 2.7): `("Vol: icUSD→ICPSwap-icUSD-BOB", icusd_ledger, icpswap_icusd_bob_pool)`, `("Vol: BOB→ICPSwap-icUSD-BOB", bob_ledger, icpswap_icusd_bob_pool)`. Gate on `icpswap_icusd_bob_pool != anonymous`.
- [ ] Dashboard: new icUSD/BOB volume config card (copy the icUSD/ICP card), stat rows, `POOL_LABELS` +`IcusdBob: 'icUSD / BOB'`, `DIRECTION_LABELS` +BuyBob/SellBob, `saveVolumePoolConfig` ternary→lookup, `loadVolumeData` +3rd block, `inputSym` ternary (~4214)→lookup. The gate-card loop is already generic.
- [ ] `scripts/check-candid.sh` + hand-diff dashboard IDL + `cargo check` + `cargo test` + `node --check` on dashboard JS + wasm release build. Commit: `feat(arb): volume bot icUSD/BOB approvals + dashboard`.

## Verification gates

- `cargo check`, `cargo test -p arb_bot` (all decode-guard + candid tests), `scripts/check-candid.sh`, manual dashboard-IDL diff, wasm release build.
- Persona review (icp-canister-auditor + rust-migration-reviewer) on the diff; variant-QA sweep (every VolumePool match site has an IcusdBob arm; every new field serde-defaulted + Default + .did + dashboard IDL; approvals gated; rebalance recovers stranded funds).
