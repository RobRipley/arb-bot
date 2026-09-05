# rumi-arb-bot

Internet Computer canister that runs arbitrage and volume strategies across
Rumi, ICPSwap, and PartyDEX pools, with an embedded HTML admin dashboard.

## Build

```sh
# Type-check / build the canister wasm (deploy artifact)
cargo build --target wasm32-unknown-unknown --release -p arb_bot

# Full canister build via dfx
dfx build arb_bot
```

> The dashboard (`src/arb_bot/src/dashboard.html`) is `include_str!`-embedded.
> If a build must pick up dashboard edits, `cargo clean` first — `include_str!`
> output is cached across incremental builds.

## Candid drift guard

The canister's Candid interface is kept in sync **by hand** across three
sources — there is no `export_candid!`-driven `.did` generation in the build:

1. **Rust** — `src/arb_bot/src/lib.rs` (`#[update]`/`#[query]` fns) and
   `src/arb_bot/src/state.rs` (`BotConfig` / `CycleSnapshot` structs)
2. **`.did`** — `src/arb_bot/arb_bot.did`
3. **Dashboard** — the `IDL.*` / `I.Service({...})` blocks in
   `src/arb_bot/src/dashboard.html`

A mismatch (wrong field name/type/order, or a missing method) produces a
**silent candid decode trap on mainnet** that nothing catches at build time.
Run the local guard before deploying anything that touches an endpoint,
`BotConfig` field, or `CycleSnapshot` field:

```sh
scripts/check-candid.sh
```

It does three things:

- **Rust ↔ `.did` ↔ dashboard** — a fast grep-diff of the highest-drift
  surfaces (`execute_strategy_*` / `dry_run_strategy_*` method sets, `BotConfig`
  fields, `BotConfigInput` fields and opt-ness, `CycleSnapshot` fields, plus a
  dashboard-internal check that `STRATEGY_T_OPT_FIELDS` matches the
  `BotConfigInput` IDL definition) across all three sources.
- **Rust ↔ `.did`** — a `cargo test` (`src/arb_bot/tests/candid.rs`) compares
  the candid service generated from the live Rust signatures against the
  committed `arb_bot.did` using candid's own subtyping machinery
  (`service_equal`). Field/method ordering and type names don't matter — only
  structure. This is the rigorous check, but it can't see the dashboard.
- **Old-deployed ↔ new subtyping** — `didc check` (twice: once for the
  service methods, once via a shim for the `InitArgs` constructor argument —
  didc doesn't validate the constructor directly) confirms `arb_bot.did` is
  still safely callable by anything using the interface that's *actually
  live on mainnet right now*, tracked in `src/arb_bot/arb_bot.did.deployed`.
  This answers a different question from the grep-diff and the cargo test
  above: those two only confirm the three sources AGREE with each other,
  not that an EXISTING caller (an old cached dashboard, an external script)
  can still call the interface at all. A record used as a function argument
  can only safely gain `opt` fields, never required ones — Rust's
  `#[serde(default)]` does not help here, since it only protects the
  internal stable-memory state blob, a completely different mechanism from
  Candid's wire-format decoding of an inbound call.

  **`src/arb_bot/arb_bot.did.deployed` must be updated (copied from
  `arb_bot.did`) as part of every successful mainnet deploy.** It is not
  updated automatically — it's the deploy step's responsibility, and this
  check's value depends entirely on that file staying in sync with what's
  actually running. It does not track itself; nothing else in this repo
  will remind you.

All three must pass (exit 0). Use `scripts/check-candid.sh --no-cargo` to
skip the cargo test (faster, no build) — the grep-diff and didc checks still
run. No CI / GitHub Actions is involved — this is a purely local command.

## Stage-1 disposition inventory

Before any change to `lib.rs`'s public method set (new methods, reclassifications, or retirements), run the Stage-1 disposition inventory check alongside `check-candid.sh`:

```sh
scripts/check-stage1-disposition.sh
```

This script verifies that every public `#[update]`/`#[query]` method in the code has an explicit Stage-1 retirement disposition — one of five categories: fail-closed (compatibility stubs), read-only (local/query only), volume-config (configuration operations), volume-op (volume operation participants), or generic-recovery (recovery operations). The check fails if any method is unclassified or if a classification lists a method that no longer exists. This enforces PR #22's Stage-1 acceptance criterion: an unclassified mutator is not acceptable.
