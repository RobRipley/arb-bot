# Rumi Arbitrage Bot — Design Document

**Date:** 2026-04-01
**Status:** Draft

## Overview

A standalone Rust canister deployed on the Internet Computer that arbitrages the ICP price between the Rumi AMM (3USD/ICP constant product pool) and ICPSwap (ckUSDC/ICP pool). The bot profits from price discrepancies while keeping the Rumi pool price aligned with the broader market.

This is a personal project, separate from the Rumi Protocol repository.

## Architecture

Single canister that:
- Runs an arb loop on a 60-second timer
- Holds 3USD and ckUSDC as working capital (no ICP inventory needed)
- Executes cross-DEX arbitrage when spread exceeds a configurable threshold
- Tracks per-trade profitability and fee costs
- Serves an admin dashboard via HTTP

## Token & Canister Dependencies

| Entity | Principal | Notes |
|---|---|---|
| ICP Ledger | `ryjl3-tyaaa-aaaaa-aaaba-cai` | Transfer fee: 0.0001 ICP |
| ckUSDC Ledger | `xevnm-gaaaa-aaaar-qafnq-cai` | Transfer fee: 0.01 ckUSDC |
| 3USD Ledger | TBD | Transfer fee: TBD |
| Rumi AMM | TBD | 3USD/ICP constant product pool |
| Rumi 3pool | TBD | Virtual price source for 3USD → USD conversion |
| ICPSwap Factory | `4mmnk-kiaaa-aaaag-qbllq-cai` | Used once to look up pool canister |
| ICPSwap ckUSDC/ICP Pool | TBD (queried from factory) | Swap execution target |

## Core Loop

Every 60 seconds:

### 0. Drain Residual ICP

If the bot is holding ICP from a previously failed sell-side swap, attempt to sell it first (on whichever DEX offers the better price). Only proceed to price checks once ICP balance is zero (or the drain attempt also fails, in which case log and continue).

### 1. Fetch Prices (parallel queries)

- **Rumi AMM**: query ICP price in 3USD
- **Rumi 3pool**: query virtual price (3USD → USD conversion factor, ~1.05-1.06)
- **ICPSwap pool**: `quote` query for ICP price in ckUSDC

### 2. Normalize & Compare

- Rumi ICP price in USD = `rumi_icp_price_3usd × virtual_price`
- ICPSwap ICP price in USD = `icpswap_icp_price_ckusdc` (ckUSDC ≈ $1)
- Spread (bps) = `|rumi_price - icpswap_price| / min(rumi_price, icpswap_price) × 10000`

### 3. Check Threshold

If spread < `min_spread_bps`, skip. The threshold must cover:
- Rumi AMM swap fee
- ICPSwap pool fee (0.3%)
- Ledger transfer fees (~$0.02-0.04 per cycle depending on direction)
- Desired minimum profit margin

### 4. Calculate Trade Size

- Start from the full spread and work backward to find the amount of ICP that brings prices to parity
- Cap at `max_trade_size` config parameter
- Cap at available balance of the input stablecoin

### 5. Execute Arbitrage

**Case A: Rumi is cheaper (buy on Rumi, sell on ICPSwap)**

1. Call Rumi AMM: swap 3USD → ICP (ICRC-2 approval already in place)
2. Call ICPSwap pool: `depositFromAndSwap` ICP → ckUSDC (ICRC-2 approval already in place)

**Case B: ICPSwap is cheaper (buy on ICPSwap, sell on Rumi)**

1. Call ICPSwap pool: `depositFromAndSwap` ckUSDC → ICP (ICRC-2 approval already in place)
2. Call Rumi AMM: swap ICP → 3USD (ICRC-2 approval already in place)

### 6. Log Trade Result

Record the trade with full profitability data (see Profitability Tracking below).

## ICRC-2 Approval Strategy

On initialization (or via admin call), the canister sets one-time, non-expiring approvals:

| Token | Approved Spender | Purpose |
|---|---|---|
| 3USD | Rumi AMM | Buy ICP on Rumi |
| ICP | Rumi AMM | Sell ICP on Rumi |
| ICP | ICPSwap Pool | Sell ICP on ICPSwap |
| ckUSDC | ICPSwap Pool | Buy ICP on ICPSwap |

Approvals use a very large `amount` (e.g., `2^128`) and `expires_at: None`. If the ICPSwap pool canister ID ever changes, an admin call re-approves.

## Profitability Tracking

### Per-Trade Record

```
TradeRecord {
    timestamp: u64,
    direction: Direction,         // RumiToIcpswap or IcpswapToRumi
    icp_amount: u64,              // ICP transacted (e8s)
    input_amount: u64,            // stablecoin spent (native units)
    input_token: Token,           // ThreeUSD or CkUSDC
    output_amount: u64,           // stablecoin received (native units)
    output_token: Token,          // CkUSDC or ThreeUSD
    virtual_price: u64,           // 3USD virtual price at time of trade (for USD normalization)
    ledger_fees_usd: i64,         // sum of ICRC transfer fees in USD (fixed-point, 6 decimals)
    net_profit_usd: i64,          // output_usd - input_usd - ledger_fees (fixed-point, 6 decimals)
    spread_bps: u32,              // spread at time of execution
}
```

DEX swap fees (Rumi AMM fee, ICPSwap 0.3%) are implicit in the swap output — they reduce your output amount and therefore show up in `net_profit_usd`. They cannot be measured separately from slippage, so we don't pretend to isolate them.

### USD Normalization

- ckUSDC: 1:1 USD
- 3USD: multiply by virtual price at time of trade

### Storage

`Vec<TradeRecord>` persisted in stable memory. At expected volumes (< 100 trades/day), years of history fits easily.

### Summary Queries

- `get_trade_history(offset, limit)` — paginated trade log
- `get_summary()` — returns:
  - Total trades count
  - Total net profit (USD)
  - Total fees paid (USD)
  - Average profit per trade
  - Profit breakdown by direction
  - Lifetime P&L

## Dashboard

The canister serves an admin dashboard via `http_request` at `https://<canister-id>.icp0.io`.

### Features

- **Wallet view**: current balances of 3USD, ckUSDC, and any residual ICP
- **Live prices**: current Rumi price, ICPSwap price, spread
- **Trade history**: sortable table of all trades with profit/loss
- **Summary stats**: lifetime P&L, total fees, trade count, avg profit
- **Admin actions** (authenticated via Internet Identity):
  - Manual swap execution (for testing)
  - Withdraw profits to your wallet
  - Update config (min spread, max trade size)
  - Re-run approvals
  - Pause/resume the bot

### Implementation

- Static HTML/CSS/JS embedded in the canister via `include_str!` or `include_bytes!`
- Frontend uses agent-js to call canister query/update methods
- Internet Identity for authentication; canister checks caller principal against owner
- Simple, functional UI — no framework overhead

## Configuration

Stored in canister state, modifiable via authenticated admin calls:

```
BotConfig {
    owner: Principal,                    // your principal — sole admin
    rumi_amm: Principal,
    rumi_3pool: Principal,
    icpswap_pool: Principal,
    icp_ledger: Principal,
    ckusdc_ledger: Principal,
    three_usd_ledger: Principal,
    min_spread_bps: u32,                 // minimum spread to execute (e.g., 50 = 0.5%)
    max_trade_size_usd: u64,             // cap per arb in USD terms
    paused: bool,                        // emergency stop
}
```

## Error Handling

- If the buy-side swap succeeds but the sell-side fails: log the error, the bot now holds ICP temporarily. The next cycle's Step 0 (Drain Residual ICP) will attempt to sell it on whichever DEX offers the better price.
- If a quote query fails: skip the cycle, try again in 60s.
- All errors logged to a queryable error log in canister state.

## What's Explicitly Out of Scope

- Multi-hop routing or multi-pool arb
- Automatic rebalancing between 3USD and ckUSDC
- Integration with DEXs other than ICPSwap
- Price history charts or candlestick data
- Notifications or alerts
