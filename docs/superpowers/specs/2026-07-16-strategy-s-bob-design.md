# Strategy S — icUSD/BOB Triangular Arb + ICP Inventory Band (Design)

**Date:** 2026-07-16
**Status:** Approved by Robert in conversation; execution authorized without further gates.

## Context

Robert is creating a new **icUSD/BOB pool on ICPSwap** (the first BOB/stable pair anywhere).
BOB's only real market is ICPSwap **BOB/ICP** (`ybilh-nqaaa-aaaag-qkhzq-cai`, fee 3000 pips = 0.3%,
token0 = BOB `7pail-xaaaa-aaaas-aabmq-cai`, token1 = ICP — verified live 2026-07-16).
BOB ledger: fee `1_000_000` e8s (0.01 BOB), 8 decimals (verified live).
KongSwap is dead and is never a venue.

The bot keeps the new pool honestly priced by arbing it against the triangle
reference: fair icUSD-per-BOB = (ICP per BOB from BOB/ICP) x (USD per ICP from
the best stable/ICP quote). The icUSD/ICP pool (~$3.5K TVL) is a *reference*,
never an execution leg at size.

## Decisions (settled with Robert)

1. **2-leg execution with an ICP inventory band**, not 3-leg atomic.
   - Forward (BOB cheap in our pool): icUSD → BOB (our pool) → ICP (BOB/ICP). Terminal ICP joins inventory.
   - Reverse (BOB rich): ICP (from inventory) → BOB (BOB/ICP) → icUSD (our pool).
   - Directions feed each other; recycled ICP skips stable-leg fees.
2. **ICP band pinned in ICP units** (not USD): floor **2 ICP**, ceiling **20 ICP**, both admin-configurable e8s.
   Pure local arithmetic — no quote dependency, no manipulation surface. Replaces hardcoded `ICP_RESERVE = 1 ICP`.
   - Below floor before a reverse trade → prepend a top-up leg buying ICP from whichever stable pool gives the most ICP per USD.
   - Above ceiling → the existing drain skims the excess to the best-USD stable pool (drain floor becomes the ceiling when no `pending_exit`; stays the floor during `pending_exit` recovery, still capped to the leg-1 amount).
3. **USD pinning for accounting:** strategy S books ICP legs at the reference USD quote fetched at trade time
   (existing strategies keep `sold_usd_value = 0` for ICP legs — unchanged). Every S trade's `net_profit_usd`
   is complete at trade completion. BOB mid-route legs are also marked via the reference for visibility.
4. **Best-USD routing** for band-edge legs (top-up and skim): quote all configured stable/ICP pools
   (ICPSwap ckUSDC/ckUSDT/icUSD, Rumi AMM w/ virtual price), pick the best output. Robert explicitly
   does not care which stable. PartyDEX excluded from these legs in v1 (matches drain's existing exclusion).
5. **Dry-run first:** `bob_execution_enabled` defaults **false**. Dry-run evaluation + dashboard surfacing
   always run when the pools are configured; execution requires the flag.
6. **Defaults:** `bob_max_trade_size_usd` $50 (BOB/ICP: ~$265 moves price 1%), `bob_min_spread_bps` 150
   (two-to-three 0.3% fee legs + thin-pool slippage + reference uncertainty).
7. **Stranded-BOB recovery:** new `pending_bob_exit` state + BOB-aware drain at cycle top —
   any BOB balance above dust is residue (the bot never intentionally holds BOB between cycles);
   sell via the pool it did NOT come from (prefer intended exit), marked to reference USD.

## Risks accepted

- Self-dealing economics: bot repricing Robert's own pool largely transfers value between his positions,
  minus DEX fee leakage; net new profit requires external flow. Strategy's job is pool integrity.
- BOB/ICP (~$53K TVL) is the sole reference and is cheap to move; mitigations: quote at trade size,
  small clips, 150 bps floor. Deviation-persistence (2-cycle confirmation) deferred — noted as follow-up.
- icUSD/ckUSDC/ckUSDT inventory drift across trades — existing strategies B/M etc. recycle; acceptable.

## Deploy shape

Two stacked PRs: **PR1** ICP inventory band (standalone, safe), **PR2** strategy S on top.
No CI in this repo; verification = `cargo check`/wasm build + `scripts/check-candid.sh` + persona review.
Deploy checklist (for Robert, after merge): `cargo clean` if dashboard.html changed (include_str cache),
deploy, `setup_approvals`, set BOB pool principals once the icUSD/BOB pool exists, leave
`bob_execution_enabled` false until dry-run numbers look sane on the dashboard.
