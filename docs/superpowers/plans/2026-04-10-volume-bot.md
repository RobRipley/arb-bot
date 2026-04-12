# Volume Bot Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an adaptive ping-pong volume generation bot to the existing arb bot canister, targeting the icUSD/ICP and 3USD/ICP pools on ICPSwap.

**Architecture:** New `volume.rs` module with its own timer, operating from ICRC-1 subaccount `[0,...,0,1]` for accounting separation. Tokens are transferred from the subaccount to the default account before each ICPSwap swap (since `depositFromAndSwap` doesn't support subaccounts), then output is transferred back. Rebalancing uses the existing Rumi AMM swap infrastructure.

**Tech Stack:** Rust, IC CDK, ic-stable-structures, ICPSwap (Uniswap v3-style), ICRC-1/ICRC-2 ledgers

**Spec:** `docs/superpowers/specs/2026-04-10-volume-bot-design.md`

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `src/arb_bot/src/volume.rs` | Create | Core volume bot logic: idle detection, ping-pong execution, rebalancing, randomization |
| `src/arb_bot/src/state.rs` | Modify | Add VolumeConfig, VolumeState, VolumePool, VolumeTradeLeg types; VOLUME_TRADES StableLog (MemoryIds 11-12) |
| `src/arb_bot/src/swaps.rs` | Modify | Add `approve_infinite_subaccount()` and subaccount-aware ICRC-1 transfer helpers |
| `src/arb_bot/src/arb.rs` | Modify | Expose `CYCLE_IN_PROGRESS` via public getter function |
| `src/arb_bot/src/lib.rs` | Modify | Add volume timer setup, admin endpoints, query endpoints, approval setup for volume subaccount |
| `src/arb_bot/src/dashboard.html` | Modify | Add Volume Bot tab with status, analytics, trade history, and admin controls |
| `src/arb_bot/arb_bot.did` | Modify | Add volume-related Candid type and method definitions |

---

### Task 1: State Types and Stable Memory

**Files:**
- Modify: `src/arb_bot/src/state.rs`

Add all volume-related types and the VOLUME_TRADES stable log.

- [ ] **Step 1: Add VolumePool enum and Direction type**

In `state.rs`, add after the existing `Pool` enum (~line 161):

```rust
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq)]
pub enum VolumePool {
    IcusdIcp,
    ThreeUsdIcp,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq)]
pub enum VolumeDirection {
    BuyIcp,
    SellIcp,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq)]
pub enum VolumeTradeType {
    PingPong,
    Rebalance,
}
```

- [ ] **Step 2: Add VolumePoolConfig and VolumeConfig structs**

```rust
#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct VolumePoolConfig {
    pub enabled: bool,
    pub idle_threshold_bps: u64,
    pub trade_size_usd: u64,       // 6-decimal USD
    pub trade_variance_pct: u64,
    pub daily_cost_cap_usd: u64,   // 6-decimal USD
}

impl Default for VolumePoolConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_threshold_bps: 50,
            trade_size_usd: 10_000_000,  // $10
            trade_variance_pct: 5,
            daily_cost_cap_usd: 5_000_000, // $5
        }
    }
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct VolumePoolState {
    pub last_price: Option<u64>,
    pub next_direction: VolumeDirection,
    pub trade_count: u64,
    pub total_volume_usd: u64,
    pub total_cost_usd: i64,
}

impl Default for VolumePoolState {
    fn default() -> Self {
        Self {
            last_price: None,
            next_direction: VolumeDirection::BuyIcp,
            trade_count: 0,
            total_volume_usd: 0,
            total_cost_usd: 0,
        }
    }
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct VolumeConfig {
    pub volume_paused: bool,
    pub interval_secs: u64,
    pub rebalance_drift_pct: u64,
    pub last_rebalance_ts: u64,
    pub daily_spend_reset_ts: u64,
    pub daily_spend_usd: i64,
    pub icusd_icp: VolumePoolConfig,
    pub three_usd_icp: VolumePoolConfig,
    pub icusd_icp_state: VolumePoolState,
    pub three_usd_icp_state: VolumePoolState,
}

impl Default for VolumeConfig {
    fn default() -> Self {
        Self {
            volume_paused: true,
            interval_secs: 1800,
            rebalance_drift_pct: 70,
            last_rebalance_ts: 0,
            daily_spend_reset_ts: 0,
            daily_spend_usd: 0,
            icusd_icp: VolumePoolConfig::default(),
            three_usd_icp: VolumePoolConfig::default(),
            icusd_icp_state: VolumePoolState::default(),
            three_usd_icp_state: VolumePoolState::default(),
        }
    }
}
```

- [ ] **Step 3: Add VolumeTradeLeg type**

```rust
#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct VolumeTradeLeg {
    pub timestamp: u64,
    pub pool: VolumePool,
    pub direction: VolumeDirection,
    pub trade_type: VolumeTradeType,
    pub token_in: Principal,
    pub token_out: Principal,
    pub amount_in: u64,
    pub amount_out: u64,
    pub cost_usd: i64,
    pub price_before: u64,
    pub price_after: u64,
}

json_storable!(VolumeTradeLeg);
```

- [ ] **Step 4: Add VolumeConfig to BotState**

Modify the `BotState` struct to include the volume config:

```rust
pub struct BotState {
    pub config: BotConfig,
    pub token_ordering_resolved: bool,
    pub icusd_token_ordering_resolved: bool,
    pub ckusdt_token_ordering_resolved: bool,
    pub icpswap_3usd_token_ordering_resolved: bool,
    pub pending_exit: Option<PendingExit>,
    #[serde(default)]
    pub volume: VolumeConfig,
}
```

The `#[serde(default)]` ensures backward-compatible deserialization from existing state that doesn't have this field.

- [ ] **Step 5: Add VOLUME_TRADES StableLog**

In the `thread_local!` block where other logs are defined (~line 296), add:

```rust
static VOLUME_TRADES: RefCell<StableLog<VolumeTradeLeg, VirtualMemory<DefaultMemoryImpl>, VirtualMemory<DefaultMemoryImpl>>> = RefCell::new(
    StableLog::init(
        MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(11))),
        MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(12))),
    ).expect("failed to init VOLUME_TRADES log")
);
```

- [ ] **Step 6: Add VOLUME_TRADES accessor functions**

Following the pattern of existing log accessors (e.g., `log_trade_leg`, `get_trade_legs_page`):

```rust
pub fn log_volume_trade(leg: VolumeTradeLeg) {
    VOLUME_TRADES.with(|t| {
        t.borrow().append(&leg).expect("failed to log volume trade");
    });
}

pub fn get_volume_trades_page(offset: u64, limit: u64) -> Vec<VolumeTradeLeg> {
    VOLUME_TRADES.with(|t| {
        let log = t.borrow();
        let total = log.len();
        if total == 0 || offset >= total {
            return vec![];
        }
        let end = total.saturating_sub(offset);
        let start = end.saturating_sub(limit);
        (start..end).rev().filter_map(|i| log.get(i)).collect()
    })
}

pub fn volume_trades_count() -> u64 {
    VOLUME_TRADES.with(|t| t.borrow().len())
}
```

- [ ] **Step 7: Add VolumeStats query type**

```rust
#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct VolumeStats {
    pub volume_paused: bool,
    pub interval_secs: u64,
    pub daily_spend_usd: i64,
    pub daily_cost_cap_usd_icusd: u64,
    pub daily_cost_cap_usd_3usd: u64,
    pub icusd_icp: VolumePoolStatus,
    pub three_usd_icp: VolumePoolStatus,
    pub total_trade_count: u64,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct VolumePoolStatus {
    pub config: VolumePoolConfig,
    pub state: VolumePoolState,
}
```

- [ ] **Step 8: Verify compilation**

Run: `cargo build --target wasm32-unknown-unknown --release -p arb_bot`
Expected: compiles successfully (new types are defined but not yet used externally)

- [ ] **Step 9: Commit**

```bash
git add src/arb_bot/src/state.rs
git commit -m "feat(volume): add volume bot state types and stable log"
```

---

### Task 2: Expose CYCLE_IN_PROGRESS and Add Subaccount Helpers

**Files:**
- Modify: `src/arb_bot/src/arb.rs`
- Modify: `src/arb_bot/src/swaps.rs`

- [ ] **Step 1: Expose CYCLE_IN_PROGRESS in arb.rs**

Add a public getter function in `arb.rs` after the `CYCLE_IN_PROGRESS` thread_local (~line 31):

```rust
pub fn is_cycle_in_progress() -> bool {
    CYCLE_IN_PROGRESS.with(|c| c.get())
}
```

- [ ] **Step 2: Add VOLUME_SUBACCOUNT constant to swaps.rs**

At the top of `swaps.rs`:

```rust
pub const VOLUME_SUBACCOUNT: [u8; 32] = {
    let mut sub = [0u8; 32];
    sub[31] = 1;
    sub
};
```

- [ ] **Step 3: Add approve_infinite_subaccount to swaps.rs**

```rust
pub async fn approve_infinite_subaccount(
    token_ledger: Principal,
    spender: Principal,
    subaccount: [u8; 32],
) -> Result<(), SwapError> {
    let approve_args = ApproveArgs {
        from_subaccount: Some(subaccount.to_vec()),
        spender: Account { owner: spender, subaccount: None },
        amount: Nat::from(340_282_366_920_938_463_463_374_607_431_768_211_455u128),
        expected_allowance: None,
        expires_at: None,
        fee: None,
        memo: None,
        created_at_time: None,
    };
    let result: Result<(ApproveResult,), _> = ic_cdk::call(
        token_ledger, "icrc2_approve", (approve_args,),
    ).await;
    match result {
        Ok((ApproveResult::Ok(_),)) => Ok(()),
        Ok((ApproveResult::Err(e),)) => Err(SwapError::SwapFailed(format!("Approve: {:?}", e))),
        Err((code, msg)) => Err(SwapError::SwapFailed(format!("Approve call ({:?}): {}", code, msg))),
    }
}
```

Note: check the existing `approve_infinite` for the exact `ApproveResult` type — follow the same pattern but with `from_subaccount: Some(...)`.

- [ ] **Step 4: Add ICRC-1 transfer helpers for subaccount operations**

These are used to move tokens between the volume subaccount and the default account (needed because ICPSwap's `depositFromAndSwap` pulls from the default subaccount):

```rust
/// Transfer tokens from the volume subaccount to the default account
pub async fn transfer_from_subaccount(
    token_ledger: Principal,
    amount: u64,
    from_subaccount: [u8; 32],
) -> Result<u64, SwapError> {
    let self_principal = ic_cdk::id();
    let args = TransferArg {
        from_subaccount: Some(from_subaccount.to_vec()),
        to: Account { owner: self_principal, subaccount: None },
        fee: None,
        created_at_time: None,
        memo: None,
        amount: Nat::from(amount),
    };
    let result: Result<(Result<Nat, TransferError>,), _> = ic_cdk::call(
        token_ledger, "icrc1_transfer", (args,),
    ).await;
    match result {
        Ok((Ok(block),)) => Ok(nat_to_u64(&block)),
        Ok((Err(e),)) => Err(SwapError::SwapFailed(format!("Transfer: {:?}", e))),
        Err((code, msg)) => Err(SwapError::SwapFailed(format!("Transfer call ({:?}): {}", code, msg))),
    }
}

/// Transfer tokens from the default account to the volume subaccount
pub async fn transfer_to_subaccount(
    token_ledger: Principal,
    amount: u64,
    to_subaccount: [u8; 32],
) -> Result<u64, SwapError> {
    let self_principal = ic_cdk::id();
    let args = TransferArg {
        from_subaccount: None,
        to: Account { owner: self_principal, subaccount: Some(to_subaccount.to_vec()) },
        fee: None,
        created_at_time: None,
        memo: None,
        amount: Nat::from(amount),
    };
    let result: Result<(Result<Nat, TransferError>,), _> = ic_cdk::call(
        token_ledger, "icrc1_transfer", (args,),
    ).await;
    match result {
        Ok((Ok(block),)) => Ok(nat_to_u64(&block)),
        Ok((Err(e),)) => Err(SwapError::SwapFailed(format!("Transfer: {:?}", e))),
        Err((code, msg)) => Err(SwapError::SwapFailed(format!("Transfer call ({:?}): {}", code, msg))),
    }
}

/// Query ICRC-1 balance for a subaccount
pub async fn icrc1_balance_of_subaccount(
    token_ledger: Principal,
    subaccount: [u8; 32],
) -> Result<u64, String> {
    let account = Account {
        owner: ic_cdk::id(),
        subaccount: Some(subaccount.to_vec()),
    };
    let result: Result<(Nat,), _> = ic_cdk::call(
        token_ledger, "icrc1_balance_of", (account,),
    ).await;
    match result {
        Ok((balance,)) => Ok(nat_to_u64(&balance)),
        Err((code, msg)) => Err(format!("Balance call ({:?}): {}", code, msg)),
    }
}
```

Note: check what types are already imported in `swaps.rs`. `TransferArg` and `TransferError` come from `icrc-ledger-types`. The `Account` type may already be imported for approvals. Add any missing imports.

- [ ] **Step 5: Verify compilation**

Run: `cargo build --target wasm32-unknown-unknown --release -p arb_bot`
Expected: compiles successfully

- [ ] **Step 6: Commit**

```bash
git add src/arb_bot/src/arb.rs src/arb_bot/src/swaps.rs
git commit -m "feat(volume): expose cycle guard, add subaccount transfer helpers"
```

---

### Task 3: Core Volume Bot Logic

**Files:**
- Create: `src/arb_bot/src/volume.rs`
- Modify: `src/arb_bot/src/lib.rs` (add `mod volume;`)

This is the main logic module. It handles idle detection, randomized trade sizing, ping-pong execution, and daily rebalancing.

- [ ] **Step 1: Create volume.rs with module structure and constants**

Create `src/arb_bot/src/volume.rs`:

```rust
use candid::Principal;
use crate::arb;
use crate::prices;
use crate::state::{self, VolumePool, VolumeDirection, VolumeTradeType, VolumeTradeLeg, VolumePoolConfig};
use crate::swaps::{self, VOLUME_SUBACCOUNT};

const ICUSD_FEE: u64 = 100_000;    // 0.001 icUSD (8 dec)
const ICP_FEE: u64 = 10_000;       // 0.0001 ICP (8 dec)
const THREE_USD_FEE: u64 = 100_000_000_000_000; // 3USD fee (18 dec) - verify actual value
const NANOS_PER_DAY: u64 = 86_400_000_000_000;
```

- [ ] **Step 2: Add randomized trade size function**

```rust
async fn randomized_trade_size(base_usd: u64, variance_pct: u64) -> u64 {
    if variance_pct == 0 {
        return base_usd;
    }
    // Get 32 bytes of randomness from the management canister
    let rand_bytes: Vec<u8> = match ic_cdk::api::management_canister::main::raw_rand().await {
        Ok((bytes,)) => bytes,
        Err(_) => return base_usd, // fallback to exact size on error
    };
    // Use first 4 bytes to generate a float in [-1.0, 1.0]
    let raw = u32::from_le_bytes([rand_bytes[0], rand_bytes[1], rand_bytes[2], rand_bytes[3]]);
    let factor = (raw as f64 / u32::MAX as f64) * 2.0 - 1.0; // [-1.0, 1.0]
    let variance = base_usd as f64 * variance_pct as f64 / 100.0 * factor;
    let result = (base_usd as f64 + variance).round() as u64;
    result.max(1) // never zero
}
```

- [ ] **Step 3: Add idle detection function**

```rust
fn is_pool_idle(current_price: u64, last_price: Option<u64>, threshold_bps: u64) -> bool {
    match last_price {
        None => true, // first check — treat as idle to start trading
        Some(prev) => {
            if prev == 0 { return true; }
            let diff = if current_price > prev {
                current_price - prev
            } else {
                prev - current_price
            };
            let movement_bps = diff * 10_000 / prev;
            movement_bps <= threshold_bps
        }
    }
}
```

- [ ] **Step 4: Add the per-pool trade execution function**

This is the core swap logic. It transfers tokens from the volume subaccount to the default account, executes the ICPSwap swap, then transfers the output back to the subaccount.

```rust
async fn execute_volume_trade(
    pool: VolumePool,
    direction: &VolumeDirection,
    trade_size_usd: u64,
    config: &state::BotConfig,
) -> Result<(u64, u64, u64, u64), String> {
    // Returns: (amount_in, amount_out, price_before, price_after)

    let (icpswap_pool, icp_is_token0, stable_ledger, stable_fee, stable_decimals) = match pool {
        VolumePool::IcusdIcp => (
            config.icpswap_icusd_pool,
            config.icpswap_icusd_icp_is_token0,
            config.icusd_ledger,
            ICUSD_FEE,
            8u32,
        ),
        VolumePool::ThreeUsdIcp => (
            config.icpswap_3usd_pool,
            config.icpswap_3usd_icp_is_token0,
            config.three_usd_ledger,
            THREE_USD_FEE,
            18u32,
        ),
    };

    // Fetch price before trade (1 ICP quote)
    let zero_for_one_quote = icp_is_token0; // ICP→stable direction
    let price_before = prices::fetch_icpswap_price(icpswap_pool, zero_for_one_quote)
        .await
        .map_err(|e| format!("Price fetch failed: {}", e))?;

    // Convert trade_size_usd (6-dec) to native token amount
    let (token_in, amount_in_native, zero_for_one, token_in_fee, token_out_fee) = match direction {
        VolumeDirection::BuyIcp => {
            // Selling stable for ICP
            let amount = match stable_decimals {
                8 => trade_size_usd * 100,     // 6-dec USD -> 8-dec icUSD
                18 => trade_size_usd * 1_000_000_000_000, // 6-dec USD -> 18-dec 3USD (approximate, ignoring VP)
                _ => trade_size_usd,           // 6-dec stays 6-dec
            };
            let zfo = !icp_is_token0; // stable→ICP = !icp_is_token0
            (stable_ledger, amount, zfo, stable_fee, ICP_FEE)
        },
        VolumeDirection::SellIcp => {
            // Selling ICP for stable
            // Convert USD to ICP amount using current price
            // price_before is in native stable decimals per 1 ICP (1e8)
            let icp_amount = if price_before > 0 {
                // trade_size_usd is 6-dec, price is native-dec per 1e8 ICP
                // For icUSD (8-dec): price_before = ~X * 1e8 per 1e8 ICP
                //   icp_amount = trade_size_usd * 1e8 * 1e2 / price_before (to get 8-dec aligned)
                // Simplify: just use 1e8 * trade_size_usd_in_native / price
                let trade_native = match stable_decimals {
                    8 => trade_size_usd * 100,
                    18 => trade_size_usd * 1_000_000_000_000,
                    _ => trade_size_usd,
                };
                (trade_native as u128 * 100_000_000u128 / price_before as u128) as u64
            } else {
                return Err("Zero price".to_string());
            };
            let zfo = icp_is_token0; // ICP→stable = icp_is_token0
            (config.icp_ledger, icp_amount, zfo, ICP_FEE, stable_fee)
        },
    };

    // Step 1: Transfer tokens from volume subaccount to default account
    swaps::transfer_from_subaccount(token_in, amount_in_native, VOLUME_SUBACCOUNT)
        .await
        .map_err(|e| format!("Transfer from subaccount failed: {:?}", e))?;

    // Step 2: Execute the swap on ICPSwap (from default account)
    let amount_out = swaps::icpswap_swap(
        icpswap_pool,
        amount_in_native - token_in_fee, // subtract fee for the transfer that just happened
        zero_for_one,
        0, // min_amount_out = 0 for volume trades (we accept any output)
        token_in_fee,
        token_out_fee,
    ).await.map_err(|e| format!("Swap failed: {:?}", e))?;

    // Step 3: Transfer output tokens back to volume subaccount
    let token_out = match direction {
        VolumeDirection::BuyIcp => config.icp_ledger,
        VolumeDirection::SellIcp => stable_ledger,
    };
    let out_fee = match direction {
        VolumeDirection::BuyIcp => ICP_FEE,
        VolumeDirection::SellIcp => stable_fee,
    };
    if amount_out > out_fee {
        swaps::transfer_to_subaccount(token_out, amount_out - out_fee, VOLUME_SUBACCOUNT)
            .await
            .map_err(|e| format!("Transfer to subaccount failed: {:?}", e))?;
    }

    // Fetch price after trade
    let price_after = prices::fetch_icpswap_price(icpswap_pool, zero_for_one_quote)
        .await
        .unwrap_or(price_before);

    Ok((amount_in_native, amount_out, price_before, price_after))
}
```

- [ ] **Step 5: Add the main volume cycle function**

```rust
pub async fn run_volume_cycle() {
    // Guard: skip if arb cycle is running
    if arb::is_cycle_in_progress() {
        return;
    }

    let (volume_config, bot_config) = state::read_state(|s| (s.volume.clone(), s.config.clone()));

    if volume_config.volume_paused {
        return;
    }

    let now = ic_cdk::api::time();

    // Check if daily spend needs resetting (24h elapsed)
    let should_reset_daily = now.saturating_sub(volume_config.daily_spend_reset_ts) >= NANOS_PER_DAY;
    if should_reset_daily {
        state::mutate_state(|s| {
            s.volume.daily_spend_usd = 0;
            s.volume.daily_spend_reset_ts = now;
        });
    }

    // Process each pool
    for pool in [VolumePool::IcusdIcp, VolumePool::ThreeUsdIcp] {
        let (pool_config, pool_state) = state::read_state(|s| {
            match &pool {
                VolumePool::IcusdIcp => (s.volume.icusd_icp.clone(), s.volume.icusd_icp_state.clone()),
                VolumePool::ThreeUsdIcp => (s.volume.three_usd_icp.clone(), s.volume.three_usd_icp_state.clone()),
            }
        });

        if !pool_config.enabled {
            continue;
        }

        // Check daily cap (use the pool's own cap)
        let current_daily = state::read_state(|s| s.volume.daily_spend_usd);
        if current_daily >= pool_config.daily_cost_cap_usd as i64 {
            continue;
        }

        // Fetch current price
        let (icpswap_pool, icp_is_token0) = state::read_state(|s| match &pool {
            VolumePool::IcusdIcp => (s.config.icpswap_icusd_pool, s.config.icpswap_icusd_icp_is_token0),
            VolumePool::ThreeUsdIcp => (s.config.icpswap_3usd_pool, s.config.icpswap_3usd_icp_is_token0),
        });
        let current_price = match prices::fetch_icpswap_price(icpswap_pool, icp_is_token0).await {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Idle detection
        if !is_pool_idle(current_price, pool_state.last_price, pool_config.idle_threshold_bps) {
            // Pool had organic activity — update price and skip
            state::mutate_state(|s| {
                match &pool {
                    VolumePool::IcusdIcp => s.volume.icusd_icp_state.last_price = Some(current_price),
                    VolumePool::ThreeUsdIcp => s.volume.three_usd_icp_state.last_price = Some(current_price),
                }
            });
            continue;
        }

        // Check balance of input token in subaccount
        let input_token = match &pool_state.next_direction {
            VolumeDirection::BuyIcp => match &pool {
                VolumePool::IcusdIcp => bot_config.icusd_ledger,
                VolumePool::ThreeUsdIcp => bot_config.three_usd_ledger,
            },
            VolumeDirection::SellIcp => bot_config.icp_ledger,
        };
        let balance = match swaps::icrc1_balance_of_subaccount(input_token, VOLUME_SUBACCOUNT).await {
            Ok(b) => b,
            Err(_) => continue,
        };

        // Compute randomized trade size
        let trade_size = randomized_trade_size(pool_config.trade_size_usd, pool_config.trade_variance_pct).await;

        // Convert trade_size to native to check if we have enough
        let min_native = match (&pool_state.next_direction, &pool) {
            (VolumeDirection::BuyIcp, VolumePool::IcusdIcp) => trade_size * 100, // 6→8 dec
            (VolumeDirection::BuyIcp, VolumePool::ThreeUsdIcp) => trade_size * 1_000_000_000_000, // 6→18 dec
            (VolumeDirection::SellIcp, _) => {
                if current_price > 0 {
                    // Approximate ICP needed
                    let stable_native = match &pool {
                        VolumePool::IcusdIcp => trade_size * 100,
                        VolumePool::ThreeUsdIcp => trade_size * 1_000_000_000_000,
                    };
                    (stable_native as u128 * 100_000_000u128 / current_price as u128) as u64
                } else {
                    continue;
                }
            }
        };

        if balance < min_native {
            // Not enough balance — skip this pool
            state::log_activity(format!(
                "Volume: skipping {:?} {:?} — insufficient balance ({} < {})",
                pool, pool_state.next_direction, balance, min_native
            ));
            continue;
        }

        // Execute the trade
        match execute_volume_trade(pool.clone(), &pool_state.next_direction, trade_size, &bot_config).await {
            Ok((amount_in, amount_out, price_before, price_after)) => {
                // Calculate cost in 6-dec USD
                let (in_usd, out_usd) = match (&pool_state.next_direction, &pool) {
                    (VolumeDirection::BuyIcp, VolumePool::IcusdIcp) => {
                        let in_6 = amount_in / 100; // 8-dec icUSD → 6-dec USD
                        let out_6 = (amount_out as u128 * price_before as u128 / 100_000_000u128 / 100) as u64;
                        (in_6, out_6)
                    },
                    _ => (trade_size, trade_size), // approximate for other combos — cost is small
                };
                let cost = in_usd as i64 - out_usd as i64;

                // Log the trade
                let leg = VolumeTradeLeg {
                    timestamp: ic_cdk::api::time(),
                    pool: pool.clone(),
                    direction: pool_state.next_direction.clone(),
                    trade_type: VolumeTradeType::PingPong,
                    token_in: input_token,
                    token_out: match &pool_state.next_direction {
                        VolumeDirection::BuyIcp => bot_config.icp_ledger,
                        VolumeDirection::SellIcp => match &pool {
                            VolumePool::IcusdIcp => bot_config.icusd_ledger,
                            VolumePool::ThreeUsdIcp => bot_config.three_usd_ledger,
                        },
                    },
                    amount_in,
                    amount_out,
                    cost_usd: cost,
                    price_before,
                    price_after,
                };
                state::log_volume_trade(leg);

                // Update state
                state::mutate_state(|s| {
                    let (ps, _) = match &pool {
                        VolumePool::IcusdIcp => (&mut s.volume.icusd_icp_state, &s.volume.icusd_icp),
                        VolumePool::ThreeUsdIcp => (&mut s.volume.three_usd_icp_state, &s.volume.three_usd_icp),
                    };
                    ps.last_price = Some(price_after);
                    ps.next_direction = match ps.next_direction {
                        VolumeDirection::BuyIcp => VolumeDirection::SellIcp,
                        VolumeDirection::SellIcp => VolumeDirection::BuyIcp,
                    };
                    ps.trade_count += 1;
                    ps.total_volume_usd += trade_size;
                    ps.total_cost_usd += cost;
                    s.volume.daily_spend_usd += cost;
                });

                state::log_activity(format!(
                    "Volume: {:?} {:?} on {:?} — in: {}, out: {}, cost: {} USD",
                    pool_state.next_direction, pool, pool, amount_in, amount_out, cost
                ));
            },
            Err(e) => {
                state::log_activity(format!("Volume: {:?} trade failed: {}", pool, e));
            }
        }
    }

    // Daily rebalance check
    let volume = state::read_state(|s| s.volume.clone());
    if now.saturating_sub(volume.last_rebalance_ts) >= NANOS_PER_DAY {
        run_rebalance(&bot_config).await;
    }
}
```

- [ ] **Step 6: Add the rebalance function**

```rust
async fn run_rebalance(config: &state::BotConfig) {
    let volume = state::read_state(|s| s.volume.clone());
    let drift_threshold = volume.rebalance_drift_pct;

    for pool in [VolumePool::IcusdIcp, VolumePool::ThreeUsdIcp] {
        let pool_config = match &pool {
            VolumePool::IcusdIcp => &volume.icusd_icp,
            VolumePool::ThreeUsdIcp => &volume.three_usd_icp,
        };
        if !pool_config.enabled {
            continue;
        }

        let (stable_ledger, stable_fee) = match &pool {
            VolumePool::IcusdIcp => (config.icusd_ledger, ICUSD_FEE),
            VolumePool::ThreeUsdIcp => (config.three_usd_ledger, THREE_USD_FEE),
        };

        // Get subaccount balances
        let icp_bal = swaps::icrc1_balance_of_subaccount(config.icp_ledger, VOLUME_SUBACCOUNT)
            .await.unwrap_or(0);
        let stable_bal = swaps::icrc1_balance_of_subaccount(stable_ledger, VOLUME_SUBACCOUNT)
            .await.unwrap_or(0);

        // Convert both to a common unit (6-dec USD) for ratio comparison
        // For simplicity, use the current pool price
        let (icpswap_pool, icp_is_token0) = match &pool {
            VolumePool::IcusdIcp => (config.icpswap_icusd_pool, config.icpswap_icusd_icp_is_token0),
            VolumePool::ThreeUsdIcp => (config.icpswap_3usd_pool, config.icpswap_3usd_icp_is_token0),
        };
        let price = match prices::fetch_icpswap_price(icpswap_pool, icp_is_token0).await {
            Ok(p) => p,
            Err(_) => continue,
        };

        if price == 0 { continue; }

        // Convert ICP balance to stable-equivalent for ratio calc
        let icp_as_stable = (icp_bal as u128 * price as u128 / 100_000_000u128) as u64;
        let total = icp_as_stable + stable_bal;
        if total == 0 { continue; }

        let icp_pct = icp_as_stable * 100 / total;

        if icp_pct > drift_threshold {
            // Too much ICP — sell some ICP for stable via Rumi AMM
            let excess_icp = icp_bal / 2; // sell half the ICP to rebalance
            if excess_icp > ICP_FEE * 2 {
                // Transfer ICP from subaccount to default
                match swaps::transfer_from_subaccount(config.icp_ledger, excess_icp, VOLUME_SUBACCOUNT).await {
                    Ok(_) => {},
                    Err(e) => {
                        state::log_activity(format!("Volume rebalance: ICP transfer failed: {:?}", e));
                        continue;
                    }
                }
                // Swap via Rumi AMM (ICP → 3USD)
                let rumi_pool_id = "fohh4-yyaaa-aaaap-qtkpa-cai_ryjl3-tyaaa-aaaaa-aaaba-cai";
                match swaps::rumi_swap(config.rumi_amm, rumi_pool_id, config.icp_ledger, excess_icp - ICP_FEE, 0).await {
                    Ok(three_usd_out) => {
                        // If pool is icUSD, we need to convert 3USD → icUSD via 3pool redeem
                        // If pool is 3USD, we can transfer directly back
                        match &pool {
                            VolumePool::ThreeUsdIcp => {
                                // Transfer 3USD back to subaccount
                                let _ = swaps::transfer_to_subaccount(config.three_usd_ledger, three_usd_out.saturating_sub(THREE_USD_FEE), VOLUME_SUBACCOUNT).await;
                            },
                            VolumePool::IcusdIcp => {
                                // Redeem 3USD LP for icUSD via 3pool, then transfer to subaccount
                                // Use pool_remove_one_coin(coin_index=0 for icUSD, lp_amount, min_out=0)
                                match swaps::pool_remove_one_coin(config.rumi_3pool, 0, three_usd_out, 0).await {
                                    Ok(icusd_out) => {
                                        let _ = swaps::transfer_to_subaccount(config.icusd_ledger, icusd_out.saturating_sub(ICUSD_FEE), VOLUME_SUBACCOUNT).await;
                                    },
                                    Err(e) => {
                                        state::log_activity(format!("Volume rebalance: 3pool redeem failed: {:?}", e));
                                    }
                                }
                            }
                        }
                        state::log_activity(format!("Volume rebalance: sold {} ICP via Rumi for {:?}", excess_icp, pool));
                    },
                    Err(e) => {
                        state::log_activity(format!("Volume rebalance: Rumi swap failed: {:?}", e));
                        // Transfer ICP back to subaccount to not lose it
                        let _ = swaps::transfer_to_subaccount(config.icp_ledger, excess_icp - ICP_FEE * 2, VOLUME_SUBACCOUNT).await;
                    }
                }
            }
        } else if (100 - icp_pct) > drift_threshold {
            // Too much stable — buy ICP via Rumi AMM
            let excess_stable = stable_bal / 2;
            let min_amount = match &pool {
                VolumePool::IcusdIcp => ICUSD_FEE * 3,
                VolumePool::ThreeUsdIcp => THREE_USD_FEE * 3,
            };
            if excess_stable > min_amount {
                // For icUSD pool: need to deposit icUSD into 3pool to get 3USD, then swap 3USD→ICP on Rumi
                // For 3USD pool: swap 3USD→ICP on Rumi directly
                match &pool {
                    VolumePool::ThreeUsdIcp => {
                        match swaps::transfer_from_subaccount(config.three_usd_ledger, excess_stable, VOLUME_SUBACCOUNT).await {
                            Ok(_) => {
                                let rumi_pool_id = "fohh4-yyaaa-aaaap-qtkpa-cai_ryjl3-tyaaa-aaaaa-aaaba-cai";
                                match swaps::rumi_swap(config.rumi_amm, rumi_pool_id, config.three_usd_ledger, excess_stable - THREE_USD_FEE, 0).await {
                                    Ok(icp_out) => {
                                        let _ = swaps::transfer_to_subaccount(config.icp_ledger, icp_out.saturating_sub(ICP_FEE), VOLUME_SUBACCOUNT).await;
                                        state::log_activity(format!("Volume rebalance: bought {} ICP with 3USD", icp_out));
                                    },
                                    Err(e) => {
                                        state::log_activity(format!("Volume rebalance: Rumi swap failed: {:?}", e));
                                    }
                                }
                            },
                            Err(e) => {
                                state::log_activity(format!("Volume rebalance: transfer failed: {:?}", e));
                            }
                        }
                    },
                    VolumePool::IcusdIcp => {
                        // icUSD → 3pool deposit → 3USD → Rumi → ICP
                        match swaps::transfer_from_subaccount(config.icusd_ledger, excess_stable, VOLUME_SUBACCOUNT).await {
                            Ok(_) => {
                                match swaps::pool_add_liquidity(config.rumi_3pool, 0, excess_stable - ICUSD_FEE, 0).await {
                                    Ok(lp_out) => {
                                        let rumi_pool_id = "fohh4-yyaaa-aaaap-qtkpa-cai_ryjl3-tyaaa-aaaaa-aaaba-cai";
                                        match swaps::rumi_swap(config.rumi_amm, rumi_pool_id, config.three_usd_ledger, lp_out, 0).await {
                                            Ok(icp_out) => {
                                                let _ = swaps::transfer_to_subaccount(config.icp_ledger, icp_out.saturating_sub(ICP_FEE), VOLUME_SUBACCOUNT).await;
                                                state::log_activity(format!("Volume rebalance: bought {} ICP with icUSD via 3pool+Rumi", icp_out));
                                            },
                                            Err(e) => state::log_activity(format!("Volume rebalance: Rumi swap failed: {:?}", e)),
                                        }
                                    },
                                    Err(e) => state::log_activity(format!("Volume rebalance: 3pool deposit failed: {:?}", e)),
                                }
                            },
                            Err(e) => state::log_activity(format!("Volume rebalance: transfer failed: {:?}", e)),
                        }
                    },
                }
            }
        }
    }

    // Update rebalance timestamp
    state::mutate_state(|s| {
        s.volume.last_rebalance_ts = ic_cdk::api::time();
    });
}
```

Note: `pool_remove_one_coin` and `pool_add_liquidity` may need to be made `pub` in `swaps.rs` if they aren't already. Check existing visibility. Also verify the exact function signatures — the rebalance function uses them with `(canister, coin_index, amount, min_out)` args.

- [ ] **Step 7: Add mod volume to lib.rs**

In `src/arb_bot/src/lib.rs`, add alongside the existing module declarations:

```rust
mod volume;
```

- [ ] **Step 8: Verify compilation**

Run: `cargo build --target wasm32-unknown-unknown --release -p arb_bot`
Expected: compiles. Fix any import issues, visibility problems, or type mismatches.

- [ ] **Step 9: Commit**

```bash
git add src/arb_bot/src/volume.rs src/arb_bot/src/lib.rs
git commit -m "feat(volume): add core volume bot logic with idle detection and rebalancing"
```

---

### Task 4: Timer Setup and Admin Endpoints

**Files:**
- Modify: `src/arb_bot/src/lib.rs`
- Modify: `src/arb_bot/src/volume.rs` (if needed for public re-exports)

- [ ] **Step 1: Add volume timer setup**

In `lib.rs`, add a new function after `setup_timer()`:

```rust
fn setup_volume_timer() {
    let interval = state::read_state(|s| s.volume.interval_secs);
    ic_cdk_timers::set_timer_interval(
        std::time::Duration::from_secs(interval),
        || ic_cdk::spawn(volume::run_volume_cycle()),
    );
}
```

Call `setup_volume_timer()` from both `init` and `post_upgrade`, after `setup_timer()`.

Note: when the user changes `interval_secs` via config, the timer interval won't update until the next upgrade. This is acceptable for v1. Document this in the admin UI or add a note. Alternatively, always use a short interval (e.g., 60s) and check elapsed time in `run_volume_cycle` itself — but that's more complex. Keep it simple for now.

- [ ] **Step 2: Add volume approval setup**

In `setup_approvals()` in `lib.rs`, add approvals for the volume subaccount. After the existing approval loop, add:

```rust
// Volume bot subaccount approvals
let volume_approvals = vec![
    ("Vol: icUSD→ICPSwap-icUSD", icusd_ledger, config.icpswap_icusd_pool),
    ("Vol: ICP→ICPSwap-icUSD", config.icp_ledger, config.icpswap_icusd_pool),
    ("Vol: 3USD→ICPSwap-3USD", config.three_usd_ledger, config.icpswap_3usd_pool),
    ("Vol: ICP→ICPSwap-3USD", config.icp_ledger, config.icpswap_3usd_pool),
    ("Vol: ICP→RumiAMM", config.icp_ledger, config.rumi_amm),
    ("Vol: 3USD→RumiAMM", config.three_usd_ledger, config.rumi_amm),
    ("Vol: icUSD→3pool", icusd_ledger, config.rumi_3pool),
];
for (label, token, spender) in volume_approvals {
    match swaps::approve_infinite_subaccount(token, spender, swaps::VOLUME_SUBACCOUNT).await {
        Ok(_) => results.push(format!("{}: OK", label)),
        Err(e) => results.push(format!("{}: FAILED {:?}", label, e)),
    }
}
```

- [ ] **Step 3: Add admin update endpoints**

In `lib.rs`, add these endpoints:

```rust
#[update]
async fn set_volume_config(pool: state::VolumePool, config: state::VolumePoolConfig) {
    require_admin();
    state::mutate_state(|s| {
        match pool {
            state::VolumePool::IcusdIcp => s.volume.icusd_icp = config,
            state::VolumePool::ThreeUsdIcp => s.volume.three_usd_icp = config,
        }
    });
}

#[update]
fn set_volume_global(interval_secs: u64, rebalance_drift_pct: u64) {
    require_admin();
    state::mutate_state(|s| {
        s.volume.interval_secs = interval_secs;
        s.volume.rebalance_drift_pct = rebalance_drift_pct;
    });
}

#[update]
fn pause_volume() {
    require_admin();
    state::mutate_state(|s| s.volume.volume_paused = true);
}

#[update]
fn resume_volume() {
    require_admin();
    state::mutate_state(|s| s.volume.volume_paused = false);
}

#[update]
async fn fund_volume_subaccount(token_ledger: Principal, amount: u64) {
    require_admin();
    swaps::transfer_to_subaccount(token_ledger, amount, swaps::VOLUME_SUBACCOUNT)
        .await
        .expect("Failed to fund volume subaccount");
}

#[update]
async fn withdraw_volume_subaccount(token_ledger: Principal, amount: u64) {
    require_admin();
    swaps::transfer_from_subaccount(token_ledger, amount, swaps::VOLUME_SUBACCOUNT)
        .await
        .expect("Failed to withdraw from volume subaccount");
}

#[update]
async fn trigger_volume_cycle() {
    require_admin();
    volume::run_volume_cycle().await;
}

#[update]
async fn trigger_volume_rebalance() {
    require_admin();
    let config = state::read_state(|s| s.config.clone());
    volume::run_rebalance(&config).await;
}
```

Note: `run_rebalance` needs to be made `pub` in `volume.rs` for the manual trigger.

- [ ] **Step 4: Add query endpoints**

```rust
#[query]
fn get_volume_stats() -> state::VolumeStats {
    state::read_state(|s| state::VolumeStats {
        volume_paused: s.volume.volume_paused,
        interval_secs: s.volume.interval_secs,
        daily_spend_usd: s.volume.daily_spend_usd,
        daily_cost_cap_usd_icusd: s.volume.icusd_icp.daily_cost_cap_usd,
        daily_cost_cap_usd_3usd: s.volume.three_usd_icp.daily_cost_cap_usd,
        icusd_icp: state::VolumePoolStatus {
            config: s.volume.icusd_icp.clone(),
            state: s.volume.icusd_icp_state.clone(),
        },
        three_usd_icp: state::VolumePoolStatus {
            config: s.volume.three_usd_icp.clone(),
            state: s.volume.three_usd_icp_state.clone(),
        },
        total_trade_count: s.volume.icusd_icp_state.trade_count + s.volume.three_usd_icp_state.trade_count,
    })
}

#[query]
fn get_volume_trades(offset: u64, limit: u64) -> Vec<state::VolumeTradeLeg> {
    state::get_volume_trades_page(offset, limit)
}
```

- [ ] **Step 5: Verify compilation**

Run: `cargo build --target wasm32-unknown-unknown --release -p arb_bot`

- [ ] **Step 6: Commit**

```bash
git add src/arb_bot/src/lib.rs src/arb_bot/src/volume.rs
git commit -m "feat(volume): add timer setup, admin endpoints, and query methods"
```

---

### Task 5: Candid Interface Update

**Files:**
- Modify: `src/arb_bot/arb_bot.did`

- [ ] **Step 1: Add volume types to .did file**

Add these type definitions to the Candid file:

```candid
type VolumePool = variant { IcusdIcp; ThreeUsdIcp };

type VolumeDirection = variant { BuyIcp; SellIcp };

type VolumeTradeType = variant { PingPong; Rebalance };

type VolumePoolConfig = record {
    enabled: bool;
    idle_threshold_bps: nat64;
    trade_size_usd: nat64;
    trade_variance_pct: nat64;
    daily_cost_cap_usd: nat64;
};

type VolumePoolState = record {
    last_price: opt nat64;
    next_direction: VolumeDirection;
    trade_count: nat64;
    total_volume_usd: nat64;
    total_cost_usd: int64;
};

type VolumePoolStatus = record {
    config: VolumePoolConfig;
    state: VolumePoolState;
};

type VolumeStats = record {
    volume_paused: bool;
    interval_secs: nat64;
    daily_spend_usd: int64;
    daily_cost_cap_usd_icusd: nat64;
    daily_cost_cap_usd_3usd: nat64;
    icusd_icp: VolumePoolStatus;
    three_usd_icp: VolumePoolStatus;
    total_trade_count: nat64;
};

type VolumeTradeLeg = record {
    timestamp: nat64;
    pool: VolumePool;
    direction: VolumeDirection;
    trade_type: VolumeTradeType;
    token_in: principal;
    token_out: principal;
    amount_in: nat64;
    amount_out: nat64;
    cost_usd: int64;
    price_before: nat64;
    price_after: nat64;
};
```

- [ ] **Step 2: Add volume service methods**

Add to the `service` block:

```candid
  // Volume bot
  set_volume_config: (VolumePool, VolumePoolConfig) -> ();
  set_volume_global: (nat64, nat64) -> ();
  pause_volume: () -> ();
  resume_volume: () -> ();
  fund_volume_subaccount: (principal, nat64) -> ();
  withdraw_volume_subaccount: (principal, nat64) -> ();
  trigger_volume_cycle: () -> ();
  trigger_volume_rebalance: () -> ();
  get_volume_stats: () -> (VolumeStats) query;
  get_volume_trades: (nat64, nat64) -> (vec VolumeTradeLeg) query;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo build --target wasm32-unknown-unknown --release -p arb_bot`

- [ ] **Step 4: Commit**

```bash
git add src/arb_bot/arb_bot.did
git commit -m "feat(volume): update Candid interface with volume bot types and methods"
```

---

### Task 6: Dashboard — Volume Bot Tab

**Files:**
- Modify: `src/arb_bot/src/dashboard.html`

This task adds the Volume Bot tab to the dashboard with status, analytics, trade history, and admin controls.

- [ ] **Step 1: Add Volume Bot nav item**

In the sidebar nav (~line 537), add after the Admin nav item:

```html
<a class="nav-item" data-view="volume" id="nav-volume" style="display:none">Volume Bot</a>
```

In the login success handler (where `nav-admin` is shown), also show `nav-volume`:

```javascript
document.getElementById('nav-volume').style.display = '';
```

- [ ] **Step 2: Add the Candid IDL entries for volume methods**

In the `idlFactory` definition in the `<script>` section, add the volume types and methods. Find the existing `idlFactory` and add:

The `VolumePool` variant type:
```javascript
const VolumePool = IDL.Variant({ IcusdIcp: IDL.Null, ThreeUsdIcp: IDL.Null });
const VolumeDirection = IDL.Variant({ BuyIcp: IDL.Null, SellIcp: IDL.Null });
const VolumeTradeType = IDL.Variant({ PingPong: IDL.Null, Rebalance: IDL.Null });
const VolumePoolConfig = IDL.Record({
    enabled: IDL.Bool,
    idle_threshold_bps: IDL.Nat64,
    trade_size_usd: IDL.Nat64,
    trade_variance_pct: IDL.Nat64,
    daily_cost_cap_usd: IDL.Nat64,
});
const VolumePoolState = IDL.Record({
    last_price: IDL.Opt(IDL.Nat64),
    next_direction: VolumeDirection,
    trade_count: IDL.Nat64,
    total_volume_usd: IDL.Nat64,
    total_cost_usd: IDL.Int64,
});
const VolumePoolStatus = IDL.Record({
    config: VolumePoolConfig,
    state: VolumePoolState,
});
const VolumeStats = IDL.Record({
    volume_paused: IDL.Bool,
    interval_secs: IDL.Nat64,
    daily_spend_usd: IDL.Int64,
    daily_cost_cap_usd_icusd: IDL.Nat64,
    daily_cost_cap_usd_3usd: IDL.Nat64,
    icusd_icp: VolumePoolStatus,
    three_usd_icp: VolumePoolStatus,
    total_trade_count: IDL.Nat64,
});
const VolumeTradeLeg = IDL.Record({
    timestamp: IDL.Nat64,
    pool: VolumePool,
    direction: VolumeDirection,
    trade_type: VolumeTradeType,
    token_in: IDL.Principal,
    token_out: IDL.Principal,
    amount_in: IDL.Nat64,
    amount_out: IDL.Nat64,
    cost_usd: IDL.Int64,
    price_before: IDL.Nat64,
    price_after: IDL.Nat64,
});
```

Add service methods:
```javascript
get_volume_stats: IDL.Func([], [VolumeStats], ['query']),
get_volume_trades: IDL.Func([IDL.Nat64, IDL.Nat64], [IDL.Vec(VolumeTradeLeg)], ['query']),
set_volume_config: IDL.Func([VolumePool, VolumePoolConfig], [], []),
set_volume_global: IDL.Func([IDL.Nat64, IDL.Nat64], [], []),
pause_volume: IDL.Func([], [], []),
resume_volume: IDL.Func([], [], []),
fund_volume_subaccount: IDL.Func([IDL.Principal, IDL.Nat64], [], []),
withdraw_volume_subaccount: IDL.Func([IDL.Principal, IDL.Nat64], [], []),
trigger_volume_cycle: IDL.Func([], [], []),
trigger_volume_rebalance: IDL.Func([], [], []),
```

- [ ] **Step 3: Add the Volume Bot view HTML**

Add a new section after the Admin view section. Follow the existing card/grid pattern from the admin view:

```html
<section class="view" id="view-volume">
  <h2 class="view-title">Volume Bot</h2>
  <div class="admin-grid">

    <!-- Status Card -->
    <div class="card">
      <h3>Status</h3>
      <div id="volume-status">Loading...</div>
    </div>

    <!-- Analytics Card -->
    <div class="card">
      <h3>Analytics</h3>
      <div id="volume-analytics">Loading...</div>
    </div>

    <!-- Controls Card -->
    <div class="card">
      <h3>Controls</h3>
      <div style="display:flex;gap:8px;flex-wrap:wrap;margin-bottom:12px">
        <button class="btn" onclick="doAction(this, async()=>await authenticatedActor.pause_volume(), 'Volume paused')">Pause</button>
        <button class="btn btn-primary" onclick="doAction(this, async()=>await authenticatedActor.resume_volume(), 'Volume resumed')">Resume</button>
        <button class="btn" onclick="doAction(this, async()=>await authenticatedActor.trigger_volume_cycle(), 'Volume cycle triggered')">Run Cycle</button>
        <button class="btn" onclick="doAction(this, async()=>await authenticatedActor.trigger_volume_rebalance(), 'Rebalance triggered')">Rebalance</button>
      </div>
    </div>

    <!-- icUSD/ICP Pool Config Card -->
    <div class="card">
      <h3>icUSD/ICP Pool</h3>
      <div class="form-group">
        <label><input type="checkbox" id="vol-icusd-enabled"> Enabled</label>
      </div>
      <div class="form-group">
        <label>Trade Size (USD)</label>
        <input type="number" id="vol-icusd-trade-size" step="0.01" value="10">
      </div>
      <div class="form-group">
        <label>Variance %</label>
        <input type="number" id="vol-icusd-variance" value="5">
      </div>
      <div class="form-group">
        <label>Idle Threshold (bps)</label>
        <input type="number" id="vol-icusd-idle" value="50">
      </div>
      <div class="form-group">
        <label>Daily Cost Cap (USD)</label>
        <input type="number" id="vol-icusd-cap" step="0.01" value="5">
      </div>
      <button class="btn btn-primary" onclick="saveVolumePoolConfig('IcusdIcp')">Save icUSD/ICP Config</button>
    </div>

    <!-- 3USD/ICP Pool Config Card -->
    <div class="card">
      <h3>3USD/ICP Pool</h3>
      <div class="form-group">
        <label><input type="checkbox" id="vol-3usd-enabled"> Enabled</label>
      </div>
      <div class="form-group">
        <label>Trade Size (USD)</label>
        <input type="number" id="vol-3usd-trade-size" step="0.01" value="10">
      </div>
      <div class="form-group">
        <label>Variance %</label>
        <input type="number" id="vol-3usd-variance" value="5">
      </div>
      <div class="form-group">
        <label>Idle Threshold (bps)</label>
        <input type="number" id="vol-3usd-idle" value="50">
      </div>
      <div class="form-group">
        <label>Daily Cost Cap (USD)</label>
        <input type="number" id="vol-3usd-cap" step="0.01" value="5">
      </div>
      <button class="btn btn-primary" onclick="saveVolumePoolConfig('ThreeUsdIcp')">Save 3USD/ICP Config</button>
    </div>

    <!-- Global Config Card -->
    <div class="card">
      <h3>Global Settings</h3>
      <div class="form-group">
        <label>Check Interval (seconds)</label>
        <input type="number" id="vol-interval" value="1800">
      </div>
      <div class="form-group">
        <label>Rebalance Drift %</label>
        <input type="number" id="vol-rebalance-drift" value="70">
      </div>
      <button class="btn btn-primary" onclick="saveVolumeGlobal()">Save Global Settings</button>
    </div>

    <!-- Fund/Withdraw Card -->
    <div class="card">
      <h3>Fund / Withdraw</h3>
      <div class="form-group">
        <label>Token</label>
        <select id="vol-fund-token">
          <option value="icp">ICP</option>
          <option value="icusd">icUSD</option>
          <option value="3usd">3USD</option>
        </select>
      </div>
      <div class="form-group">
        <label>Amount</label>
        <input type="number" id="vol-fund-amount" step="0.0001">
      </div>
      <div style="display:flex;gap:8px">
        <button class="btn btn-primary" onclick="fundVolume()">Fund</button>
        <button class="btn" onclick="withdrawVolume()">Withdraw</button>
      </div>
    </div>

    <!-- Recent Trades Card -->
    <div class="card" style="grid-column: 1 / -1">
      <h3>Recent Volume Trades</h3>
      <div class="table-wrap">
        <table class="data-table">
          <thead>
            <tr><th>Time</th><th>Pool</th><th>Direction</th><th>Type</th><th>In</th><th>Out</th><th>Cost (USD)</th></tr>
          </thead>
          <tbody id="volume-trades-body"></tbody>
        </table>
      </div>
    </div>

  </div>
</section>
```

- [ ] **Step 4: Add JavaScript functions for the Volume Bot tab**

Add these functions in the `<script>` section:

```javascript
async function loadVolumeData() {
    if (!anonymousActor) return;
    try {
        const [stats, trades] = await Promise.all([
            anonymousActor.get_volume_stats(),
            anonymousActor.get_volume_trades(0n, 20n),
        ]);

        // Status
        const statusDiv = document.getElementById('volume-status');
        statusDiv.innerHTML = `
            <div style="margin-bottom:8px">
                <strong>Status:</strong>
                <span class="badge ${stats.volume_paused ? 'badge-warn' : 'badge-ok'}">
                    ${stats.volume_paused ? 'Paused' : 'Active'}
                </span>
            </div>
            <div><strong>icUSD/ICP:</strong> ${stats.icusd_icp.config.enabled ? 'Enabled' : 'Disabled'}
                — Next: ${Object.keys(stats.icusd_icp.state.next_direction)[0]}
                — Trades: ${stats.icusd_icp.state.trade_count}</div>
            <div><strong>3USD/ICP:</strong> ${stats.three_usd_icp.config.enabled ? 'Enabled' : 'Disabled'}
                — Next: ${Object.keys(stats.three_usd_icp.state.next_direction)[0]}
                — Trades: ${stats.three_usd_icp.state.trade_count}</div>
            <div><strong>Interval:</strong> ${stats.interval_secs}s</div>
        `;

        // Analytics
        const analyticsDiv = document.getElementById('volume-analytics');
        const fmt6 = (n) => (Number(n) / 1_000_000).toFixed(2);
        analyticsDiv.innerHTML = `
            <div><strong>Daily Spend:</strong> $${fmt6(stats.daily_spend_usd)}</div>
            <div><strong>icUSD/ICP Volume:</strong> $${fmt6(stats.icusd_icp.state.total_volume_usd)} — Cost: $${fmt6(stats.icusd_icp.state.total_cost_usd)}</div>
            <div><strong>3USD/ICP Volume:</strong> $${fmt6(stats.three_usd_icp.state.total_volume_usd)} — Cost: $${fmt6(stats.three_usd_icp.state.total_cost_usd)}</div>
            <div><strong>Total Trades:</strong> ${stats.total_trade_count}</div>
        `;

        // Populate config forms with current values
        document.getElementById('vol-icusd-enabled').checked = stats.icusd_icp.config.enabled;
        document.getElementById('vol-icusd-trade-size').value = (Number(stats.icusd_icp.config.trade_size_usd) / 1_000_000).toFixed(2);
        document.getElementById('vol-icusd-variance').value = Number(stats.icusd_icp.config.trade_variance_pct);
        document.getElementById('vol-icusd-idle').value = Number(stats.icusd_icp.config.idle_threshold_bps);
        document.getElementById('vol-icusd-cap').value = (Number(stats.icusd_icp.config.daily_cost_cap_usd) / 1_000_000).toFixed(2);

        document.getElementById('vol-3usd-enabled').checked = stats.three_usd_icp.config.enabled;
        document.getElementById('vol-3usd-trade-size').value = (Number(stats.three_usd_icp.config.trade_size_usd) / 1_000_000).toFixed(2);
        document.getElementById('vol-3usd-variance').value = Number(stats.three_usd_icp.config.trade_variance_pct);
        document.getElementById('vol-3usd-idle').value = Number(stats.three_usd_icp.config.idle_threshold_bps);
        document.getElementById('vol-3usd-cap').value = (Number(stats.three_usd_icp.config.daily_cost_cap_usd) / 1_000_000).toFixed(2);

        document.getElementById('vol-interval').value = Number(stats.interval_secs);
        document.getElementById('vol-rebalance-drift').value = Number(stats.rebalance_drift_pct || 70);

        // Trades table
        const tbody = document.getElementById('volume-trades-body');
        tbody.innerHTML = trades.map(t => {
            const time = new Date(Number(t.timestamp) / 1_000_000).toLocaleString();
            const pool = Object.keys(t.pool)[0];
            const dir = Object.keys(t.direction)[0];
            const type_ = Object.keys(t.trade_type)[0];
            return `<tr>
                <td>${time}</td>
                <td>${pool}</td>
                <td>${dir}</td>
                <td>${type_}</td>
                <td>${Number(t.amount_in).toLocaleString()}</td>
                <td>${Number(t.amount_out).toLocaleString()}</td>
                <td>$${fmt6(t.cost_usd)}</td>
            </tr>`;
        }).join('');

    } catch(e) {
        console.error('Volume data load failed:', e);
    }
}

async function saveVolumePoolConfig(poolKey) {
    const prefix = poolKey === 'IcusdIcp' ? 'vol-icusd' : 'vol-3usd';
    const config = {
        enabled: document.getElementById(`${prefix}-enabled`).checked,
        idle_threshold_bps: BigInt(document.getElementById(`${prefix}-idle`).value),
        trade_size_usd: BigInt(Math.round(parseFloat(document.getElementById(`${prefix}-trade-size`).value) * 1_000_000)),
        trade_variance_pct: BigInt(document.getElementById(`${prefix}-variance`).value),
        daily_cost_cap_usd: BigInt(Math.round(parseFloat(document.getElementById(`${prefix}-cap`).value) * 1_000_000)),
    };
    const pool = poolKey === 'IcusdIcp' ? { IcusdIcp: null } : { ThreeUsdIcp: null };
    await doAction(event.target, async () => {
        await authenticatedActor.set_volume_config(pool, config);
    }, 'Config saved');
}

async function saveVolumeGlobal() {
    const interval = BigInt(document.getElementById('vol-interval').value);
    const drift = BigInt(document.getElementById('vol-rebalance-drift').value);
    await doAction(event.target, async () => {
        await authenticatedActor.set_volume_global(interval, drift);
    }, 'Global settings saved');
}

async function fundVolume() {
    const tokenSelect = document.getElementById('vol-fund-token').value;
    const amount = parseFloat(document.getElementById('vol-fund-amount').value);
    const config = await anonymousActor.get_config();
    let ledger, decimals;
    switch(tokenSelect) {
        case 'icp': ledger = config.icp_ledger; decimals = 8; break;
        case 'icusd': ledger = config.icusd_ledger; decimals = 8; break;
        case '3usd': ledger = config.three_usd_ledger; decimals = 18; break;
    }
    const amountNative = BigInt(Math.round(amount * Math.pow(10, decimals)));
    await doAction(event.target, async () => {
        await authenticatedActor.fund_volume_subaccount(ledger, amountNative);
    }, 'Funded');
}

async function withdrawVolume() {
    const tokenSelect = document.getElementById('vol-fund-token').value;
    const amount = parseFloat(document.getElementById('vol-fund-amount').value);
    const config = await anonymousActor.get_config();
    let ledger, decimals;
    switch(tokenSelect) {
        case 'icp': ledger = config.icp_ledger; decimals = 8; break;
        case 'icusd': ledger = config.icusd_ledger; decimals = 8; break;
        case '3usd': ledger = config.three_usd_ledger; decimals = 18; break;
    }
    const amountNative = BigInt(Math.round(amount * Math.pow(10, decimals)));
    await doAction(event.target, async () => {
        await authenticatedActor.withdraw_volume_subaccount(ledger, amountNative);
    }, 'Withdrawn');
}
```

- [ ] **Step 5: Hook loadVolumeData into the view switching logic**

Find the existing view-switching code (where clicking a nav item shows the corresponding view) and add a call to `loadVolumeData()` when the volume view becomes active. Also add it to the periodic refresh if one exists.

- [ ] **Step 6: Verify the dashboard renders correctly**

Build and deploy locally:
```bash
dfx build arb_bot
```

Or just verify the HTML is valid and the `include_str!` will work:
```bash
cargo build --target wasm32-unknown-unknown --release -p arb_bot
```

- [ ] **Step 7: Commit**

```bash
git add src/arb_bot/src/dashboard.html
git commit -m "feat(volume): add Volume Bot dashboard tab with controls and analytics"
```

---

### Task 7: Integration Testing and Deployment

**Files:**
- No new files — this is testing and verification

- [ ] **Step 1: Full build verification**

```bash
cargo build --target wasm32-unknown-unknown --release -p arb_bot
```

Expected: clean compilation, no warnings related to volume code.

- [ ] **Step 2: Deploy to mainnet**

```bash
dfx deploy arb_bot --network ic
```

Expected: successful deployment. The volume bot starts paused (`volume_paused: true`) so no automatic trading occurs.

- [ ] **Step 3: Run setup_approvals from the dashboard**

Navigate to the dashboard Admin tab and click "Setup Approvals". Verify the volume subaccount approvals succeed (check the response for "Vol: ..." entries with "OK" status).

- [ ] **Step 4: Fund the volume subaccount**

From the Volume Bot tab:
1. Fund with ~$50 worth of icUSD (e.g., 50 icUSD = 5,000,000,000 in 8-dec)
2. Fund with ~$50 worth of ICP (e.g., ~5 ICP at ~$10/ICP)

Verify balances show correctly on the dashboard.

- [ ] **Step 5: Test with manual trigger**

1. Configure icUSD/ICP pool: enabled=true, trade_size=$10, variance=5%, idle_threshold=50bps, daily_cap=$5
2. Click "Run Cycle" to trigger a manual volume cycle
3. Verify a trade appears in the Recent Volume Trades table
4. Verify the direction flips (check status card)
5. Click "Run Cycle" again — verify the opposite direction trade executes

- [ ] **Step 6: Enable and monitor**

1. Resume the volume bot (click "Resume")
2. Monitor for 1-2 hours to verify automatic cycle execution
3. Check activity log for volume entries
4. Verify daily spend tracking works

- [ ] **Step 7: Commit any fixes**

```bash
git add -A
git commit -m "fix(volume): address issues found during integration testing"
```
