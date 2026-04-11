# Volume Bot — Adaptive Ping-Pong with Idle Detection

**Date:** 2026-04-10
**Status:** Draft

## Goal

Bootstrap trading volume in the icUSD/ICP and 3USD/ICP pools on ICPSwap to improve discoverability and attract organic traders/LPs. The bot owner is the dominant LP in both pools (~$2k liquidity each), so swap fees are largely recaptured, making the effective cost of volume generation negligible.

## Approach

An **adaptive ping-pong** strategy integrated into the existing arb bot canister. The volume bot alternates buy/sell trades on target pools whenever the pool has been idle (no meaningful price movement). It operates from a dedicated ICRC-1 subaccount for accounting separation, runs on its own timer independent of the arb bot, and includes a daily rebalance mechanism to prevent token drift.

The existing arb bot continues to operate independently. If a volume bot trade creates a cross-pool spread, the arb bot may catch it and generate additional volume — a welcome side effect but not a dependency.

## Architecture

### Module Structure

New file: `src/arb_bot/src/volume.rs`

The volume bot is a self-contained module within the existing canister. It reuses:
- `swaps::icpswap_swap()` for pool trades (with subaccount support)
- `swaps::rumi_swap()` for daily rebalancing
- `prices.rs` quote logic for idle detection
- The existing ICRC-2 approval infrastructure

### Subaccount

The volume bot operates from ICRC-1 subaccount `[0, 0, ..., 0, 1]` (32 bytes, last byte = 1). This provides:
- Separate token balances from the arb bot
- Independent P&L tracking
- Clean funding/withdrawal via admin controls

Note: the canister principal is still visible on-chain for both bots. Subaccounts provide accounting separation, not trader anonymity.

### Implementation Notes

- **Subaccount plumbing:** The existing `icpswap_swap()` and ICRC-2 approval functions hardcode `from_subaccount: None`. These need an optional subaccount parameter added. Verify that ICPSwap's `depositFromAndSwap` Candid interface accepts a subaccount field.
- **`CYCLE_IN_PROGRESS` visibility:** Currently a thread-local in `arb.rs`. The volume module needs read access — expose via a public function or move to a shared location.
- **3USD decimal handling:** 3USD is an 18-decimal LP token, while `trade_size_usd` is 6-decimal USD. Size conversions between these representations require care during implementation.

### Timer

An independent `ic_cdk_timers::set_interval` timer, configurable interval (default 1800 seconds / 30 minutes). The timer is set up during `post_upgrade` and `init`, alongside the existing arb timer.

## Trade Execution Flow

Each volume timer tick:

1. **Guard checks** — skip if:
   - Volume bot is paused (`volume_paused` flag)
   - Arb cycle is in progress (`CYCLE_IN_PROGRESS` flag)
   - Daily spend cap has been reached

2. **Per-pool loop** (icUSD/ICP, then 3USD/ICP):
   - Skip if this pool is disabled
   - Fetch current pool price via existing quote logic (1 ICP probe)
   - Compare to last observed price:
     - If price moved more than `idle_threshold_bps` since last check, update stored price and **skip** (organic activity occurred)
     - If price is within threshold, pool is idle — proceed to trade
   - Determine next direction from alternating flag (buy ICP or sell ICP)
   - Generate randomized trade size:
     - `actual_size = base_size + (base_size * variance_pct / 100 * random_factor)`
     - `random_factor` is in range `[-1.0, 1.0]`, derived from `raw_rand()` (management canister)
     - Example: base $10, variance 5% -> trades between $9.50 and $10.50
   - Check subaccount has sufficient balance of the input token; skip if dry
   - Execute swap via `icpswap_swap` with subaccount parameter
   - Log the trade to `VOLUME_TRADES` stable log
   - Update: daily spend accumulator, direction flag, last observed price

3. **Daily rebalance** (checked each tick):
   - If hours since last rebalance >= 24 AND token ratio exceeds `rebalance_drift_pct`:
     - For icUSD/ICP: if ratio is e.g. 80% ICP / 20% icUSD, swap excess ICP -> icUSD via Rumi AMM
     - For 3USD/ICP: swap excess side via Rumi AMM
     - Log as rebalance event
     - Update `last_rebalance_ts`
   - The daily spend accumulator also resets when 24 hours have elapsed since last reset

## Configuration

### VolumeConfig (per pool)

| Field | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | false | Per-pool on/off toggle |
| `idle_threshold_bps` | u64 | 50 | Price movement threshold to consider pool "active" (0.5%) |
| `trade_size_usd` | u64 | 10_000_000 | Base trade size in 6-decimal USD |
| `trade_variance_pct` | u64 | 5 | Randomization radius as percentage of trade size |
| `daily_cost_cap_usd` | u64 | 5_000_000 | Max daily net cost (input value - output value) before halting |

### Global Volume Settings

| Field | Type | Default | Description |
|---|---|---|---|
| `volume_paused` | bool | true | Master pause for all volume activity |
| `interval_secs` | u64 | 1800 | Seconds between idle checks (global, single timer) |
| `rebalance_drift_pct` | u64 | 70 | Token ratio threshold to trigger rebalance (e.g., 70 = rebalance when one side > 70%) |
| `last_rebalance_ts` | u64 | 0 | Timestamp of last rebalance (nanoseconds) |
| `daily_spend_reset_ts` | u64 | 0 | Timestamp of last daily spend reset |
| `daily_spend_usd` | u64 | 0 | Accumulated spend today |

### Per-Pool Runtime State

| Field | Type | Description |
|---|---|---|
| `last_price` | Option<u64> | Last observed ICP price in this pool (6-dec USD) |
| `next_direction` | Direction | Next trade direction: BuyIcp or SellIcp |
| `trade_count` | u64 | All-time trade count |
| `total_volume_usd` | u64 | All-time volume generated (sum of trade input values) |
| `total_cost_usd` | i64 | All-time net cost (sum of input - output across all trades) |

## Admin Endpoints

### Canister Methods

```
// Configuration
set_volume_config(pool: VolumePool, config: VolumePoolConfig) -> ()
get_volume_config() -> VolumeConfigResponse
pause_volume() -> ()
resume_volume() -> ()

// Funding
fund_volume_subaccount(token: Principal, amount: Nat) -> Result<(), String>
withdraw_volume_subaccount(token: Principal, amount: Nat) -> Result<(), String>

// Manual triggers
trigger_volume_cycle() -> VolumeResult
trigger_volume_rebalance() -> RebalanceResult

// Queries
get_volume_stats() -> VolumeStats
get_volume_trades(offset: u64, limit: u64) -> Vec<VolumeTradeLeg>
```

All update methods are owner/admin-gated, consistent with existing access control.

### VolumePool Enum

```
enum VolumePool {
    IcusdIcp,
    ThreeUsdIcp,
}
```

## Storage

### Stable Memory

- **VolumeConfig + VolumeState**: stored within the existing `BotState` JSON in `META_CELL` (stable memory ID 0). This is a small addition to the existing config struct.
- **VOLUME_TRADES**: new `StableLog<VolumeTradeLeg>` using MemoryIds 11-12, following the same pattern as the existing `TRADE_LEGS` log.

### VolumeTradeLeg Record

```
struct VolumeTradeLeg {
    timestamp: u64,
    pool: VolumePool,
    direction: Direction,      // BuyIcp or SellIcp
    trade_type: VolumeTradeType, // PingPong or Rebalance
    token_in: Principal,
    token_out: Principal,
    amount_in: u64,
    amount_out: u64,
    cost_usd: i64,            // input_value - output_value (negative = profit)
    price_before: u64,        // pool price before trade (6-dec USD)
    price_after: u64,         // pool price after trade (6-dec USD)
}
```

## Dashboard

### New "Volume Bot" Tab

Added to the existing sidebar navigation alongside Overview, Charts, Trades, Admin.

#### Status Card
- Global: Active / Paused indicator
- Per pool: enabled/disabled, last trade time, next direction, idle/active status
- Subaccount balances: icUSD, ICP, 3USD (fetched from ledger queries)

#### Analytics Card
- Volume generated: today, 7-day, all-time (USD)
- Net cost: today, 7-day, all-time (USD) — framed as "spend" not "loss"
- Trade count: today, all-time
- Average cost per trade

#### Recent Trades Table
- Columns: Time, Pool, Direction, Amount In, Amount Out, Cost, Type (ping-pong / rebalance)
- Paginated, sourced from `get_volume_trades()`

#### Admin Controls (authenticated only)
- Per-pool enable/disable toggles
- Config form: interval, idle threshold, trade size, variance %, daily cap
- Fund / withdraw inputs (token selector + amount)
- Manual trigger buttons: "Run Volume Cycle" and "Run Rebalance"
- Rebalance drift % setting

## Collision Avoidance

The volume bot checks `CYCLE_IN_PROGRESS` (existing thread-local flag) before executing any trade. If the arb bot is mid-cycle, the volume bot skips this tick entirely and tries again at the next interval.

The arb bot does not need any changes — it is unaware of the volume bot. If a volume trade creates a spread, the arb bot discovers it organically in its next cycle.

## Token Fees

| Token | Ledger Fee | Decimals |
|---|---|---|
| icUSD | 0.001 icUSD | 8 |
| ICP | 0.0001 ICP | 8 |
| 3USD | inherited from pool | 18 (LP token) |
| ICPSwap swap fee | 0.3% | — |

## Cost Estimates

With $10 base trade size on a pool where the operator is the dominant LP:
- ICPSwap fee (0.3%): ~$0.03 per trade, mostly recaptured as LP fees
- Ledger fees: < $0.001 per trade
- Slippage: near-zero net over a round trip (buy pushes price up, sell pushes it back)
- Estimated net cost: ~$0.01-0.05 per trade
- At 2 trades/hour (one per pool), 48 trades/day: ~$0.50-2.50/day

Well within the $1-5/day budget target.

## Subaccount Approval Setup

The volume bot subaccount needs ICRC-2 approvals to ICPSwap pools, similar to the existing `setup_approvals()`. On first enable (or canister init/upgrade), approvals are granted from the volume subaccount to:
- ICPSwap icUSD/ICP pool
- ICPSwap 3USD/ICP pool

For rebalancing via Rumi AMM, approvals to the Rumi AMM canister are also needed from the subaccount.

## Migration

Adding `VolumeConfig` and `VolumeState` to `BotState` requires a backwards-compatible deserialization change. Since these are new optional fields, they should deserialize as `None`/defaults when loading existing state. The volume bot starts paused (`volume_paused: true`) so no action occurs until the admin explicitly enables it and funds the subaccount.
