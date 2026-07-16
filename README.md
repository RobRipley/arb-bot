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

It does two things:

- **Rust ↔ `.did`** — a `cargo test` (`src/arb_bot/tests/candid.rs`) compares
  the candid service generated from the live Rust signatures against the
  committed `arb_bot.did` using candid's own subtyping machinery
  (`service_equal`). Field/method ordering and type names don't matter — only
  structure. This is the rigorous check, but it can't see the dashboard.
- **Rust ↔ `.did` ↔ dashboard** — a fast grep-diff of the three highest-drift
  surfaces (`execute_strategy_*` / `dry_run_strategy_*` method sets, `BotConfig`
  fields, `CycleSnapshot` fields) across all three sources, covering the
  dashboard IDL the cargo test can't reach.

Both must pass (exit 0). Use `scripts/check-candid.sh --no-cargo` for the fast
grep-only pass (no build). No CI / GitHub Actions is involved — this is a
purely local command.
