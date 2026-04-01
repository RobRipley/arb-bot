# Rumi Arbitrage Bot — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build and deploy a standalone ICP canister that arbitrages ICP price between Rumi AMM (3USD/ICP) and ICPSwap (ckUSDC/ICP).

**Architecture:** Single Rust canister using `ic-cdk-timers` for a 60-second polling loop. Holds 3USD + ckUSDC as working capital. Fetches prices from both DEXs, compares, and executes cross-DEX swaps when spread exceeds threshold. Serves an embedded HTML dashboard via `http_request`.

**Tech Stack:** Rust, ic-cdk, ic-cdk-timers, ic-cdk-macros, candid, icrc-ledger-types, serde/serde_json, dfx

---

## Phase 1: Project Scaffolding

### Step 1.1: Initialize dfx project

Create dfx.json and Cargo workspace at `/Users/robertripley/coding/rumi-arb-bot/`.

**File: `/Users/robertripley/coding/rumi-arb-bot/dfx.json`**
```json
{
  "canisters": {
    "arb_bot": {
      "type": "rust",
      "package": "arb_bot",
      "candid": "src/arb_bot/arb_bot.did"
    }
  },
  "defaults": {
    "build": {
      "packtool": ""
    }
  },
  "version": 1
}
```

**File: `/Users/robertripley/coding/rumi-arb-bot/Cargo.toml`**
```toml
[workspace]
members = ["src/arb_bot"]
```

**File: `/Users/robertripley/coding/rumi-arb-bot/src/arb_bot/Cargo.toml`**
```toml
[package]
name = "arb_bot"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]
path = "src/lib.rs"

[dependencies]
candid = "0.10"
ic-cdk = "0.13"
ic-cdk-macros = "0.9"
ic-cdk-timers = "0.7"
icrc-ledger-types = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
futures = "0.3"
num-traits = "0.2"
```

**File: `/Users/robertripley/coding/rumi-arb-bot/src/arb_bot/src/lib.rs`**
```rust
use candid::{CandidType, Deserialize};
use ic_cdk_macros::{init, post_upgrade, pre_upgrade, query, update};

mod state;

use state::BotConfig;

#[derive(CandidType, Deserialize)]
pub struct InitArgs {
    pub config: BotConfig,
}

#[init]
fn init(args: InitArgs) {
    state::init_state(state::BotState {
        config: args.config,
        ..Default::default()
    });
}

#[pre_upgrade]
fn pre_upgrade() {
    state::save_to_stable_memory();
}

#[post_upgrade]
fn post_upgrade() {
    state::load_from_stable_memory();
}

#[query]
fn get_config() -> BotConfig {
    state::read_state(|s| s.config.clone())
}
```

**File: `/Users/robertripley/coding/rumi-arb-bot/src/arb_bot/arb_bot.did`**
```candid
service : (record { config: record {
  owner: principal;
  rumi_amm: principal;
  rumi_3pool: principal;
  icpswap_pool: principal;
  icp_ledger: principal;
  ckusdc_ledger: principal;
  three_usd_ledger: principal;
  min_spread_bps: nat32;
  max_trade_size_usd: nat64;
  paused: bool;
}}) -> {
  get_config: () -> (record {
    owner: principal;
    rumi_amm: principal;
    rumi_3pool: principal;
    icpswap_pool: principal;
    icp_ledger: principal;
    ckusdc_ledger: principal;
    three_usd_ledger: principal;
    min_spread_bps: nat32;
    max_trade_size_usd: nat64;
    paused: bool;
  }) query;
}
```

**Commit:** `feat: scaffold arb bot dfx project`

### Step 1.2: Create state module

**File: `/Users/robertripley/coding/rumi-arb-bot/src/arb_bot/src/state.rs`**
```rust
use candid::{CandidType, Deserialize, Principal};
use serde::Serialize;
use std::cell::RefCell;

#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub struct BotConfig {
    pub owner: Principal,
    pub rumi_amm: Principal,
    pub rumi_3pool: Principal,
    pub icpswap_pool: Principal,
    pub icp_ledger: Principal,
    pub ckusdc_ledger: Principal,
    pub three_usd_ledger: Principal,
    pub min_spread_bps: u32,
    pub max_trade_size_usd: u64,
    pub paused: bool,
}

#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub enum Direction {
    RumiToIcpswap,
    IcpswapToRumi,
}

#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub enum Token {
    ThreeUSD,
    CkUSDC,
}

#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub struct TradeRecord {
    pub timestamp: u64,
    pub direction: Direction,
    pub icp_amount: u64,
    pub input_amount: u64,
    pub input_token: Token,
    pub output_amount: u64,
    pub output_token: Token,
    pub virtual_price: u64,
    pub ledger_fees_usd: i64,   // fixed-point, 6 decimals
    pub net_profit_usd: i64,    // fixed-point, 6 decimals
    pub spread_bps: u32,
}

#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub struct ErrorRecord {
    pub timestamp: u64,
    pub message: String,
}

#[derive(Serialize, Deserialize)]
pub struct BotState {
    pub config: BotConfig,
    pub trades: Vec<TradeRecord>,
    pub errors: Vec<ErrorRecord>,
}

impl Default for BotState {
    fn default() -> Self {
        Self {
            config: BotConfig {
                owner: Principal::anonymous(),
                rumi_amm: Principal::anonymous(),
                rumi_3pool: Principal::anonymous(),
                icpswap_pool: Principal::anonymous(),
                icp_ledger: Principal::anonymous(),
                ckusdc_ledger: Principal::anonymous(),
                three_usd_ledger: Principal::anonymous(),
                min_spread_bps: 50,
                max_trade_size_usd: 100_000_000, // $100 in 6-decimal fixed-point
                paused: true,
            },
            trades: Vec::new(),
            errors: Vec::new(),
        }
    }
}

thread_local! {
    static STATE: RefCell<Option<BotState>> = RefCell::default();
}

pub fn mutate_state<F, R>(f: F) -> R
where F: FnOnce(&mut BotState) -> R {
    STATE.with(|s| f(s.borrow_mut().as_mut().expect("State not initialized")))
}

pub fn read_state<F, R>(f: F) -> R
where F: FnOnce(&BotState) -> R {
    STATE.with(|s| f(s.borrow().as_ref().expect("State not initialized")))
}

pub fn init_state(state: BotState) {
    STATE.with(|s| *s.borrow_mut() = Some(state));
}

pub fn save_to_stable_memory() {
    STATE.with(|s| {
        let state = s.borrow();
        let state = state.as_ref().expect("State not initialized");
        let bytes = serde_json::to_vec(state).expect("Failed to serialize state");
        let len = bytes.len() as u64;
        let pages_needed = (len + 8 + 65535) / 65536;
        let current_pages = ic_cdk::api::stable::stable64_size();
        if pages_needed > current_pages {
            ic_cdk::api::stable::stable64_grow(pages_needed - current_pages)
                .expect("Failed to grow stable memory");
        }
        ic_cdk::api::stable::stable64_write(0, &len.to_le_bytes());
        ic_cdk::api::stable::stable64_write(8, &bytes);
    });
}

pub fn load_from_stable_memory() {
    let size = ic_cdk::api::stable::stable64_size();
    if size == 0 {
        init_state(BotState::default());
        return;
    }
    let mut len_bytes = [0u8; 8];
    ic_cdk::api::stable::stable64_read(0, &mut len_bytes);
    let len = u64::from_le_bytes(len_bytes) as usize;
    if len == 0 {
        init_state(BotState::default());
        return;
    }
    let mut bytes = vec![0u8; len];
    ic_cdk::api::stable::stable64_read(8, &mut bytes);
    let state: BotState = serde_json::from_slice(&bytes).expect("Failed to deserialize state");
    init_state(state);
}
```

**Verify:** `cd /Users/robertripley/coding/rumi-arb-bot && cargo check`

**Commit:** `feat: add state module with config, trade records, stable memory persistence`

---

## Phase 2: Price Fetching

### Step 2.1: Create prices module with Rumi AMM quote

Queries the Rumi AMM `get_quote` method to get ICP price in 3USD.

**File: `/Users/robertripley/coding/rumi-arb-bot/src/arb_bot/src/prices.rs`**
```rust
use candid::{CandidType, Deserialize, Nat, Principal};

// ─── Rumi AMM Types ───

#[derive(CandidType, Deserialize, Debug)]
pub enum AmmError {
    PoolNotFound,
    InsufficientLiquidity,
    SlippageExceeded,
    ZeroAmount,
    TransferFailed(String),
    Unauthorized,
    PoolPaused,
    MathOverflow,
}

#[derive(CandidType, Deserialize, Debug)]
pub enum AmmResult<T> {
    #[serde(rename = "Ok")]
    Ok(T),
    #[serde(rename = "Err")]
    Err(AmmError),
}

// ─── Rumi 3Pool Types ───

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct PoolStatus {
    pub balances: Vec<Nat>,
    pub lp_total_supply: Nat,
    pub current_a: Nat,
    pub virtual_price: Nat,
    pub swap_fee_bps: Nat,
    pub admin_fee_bps: Nat,
    pub tokens: Vec<Principal>,
}

#[derive(CandidType, Deserialize, Debug)]
pub enum ThreePoolError {
    InsufficientOutput { expected_min: Nat, actual: Nat },
    InsufficientLiquidity,
    InvalidCoinIndex,
    ZeroAmount,
    PoolEmpty,
    SlippageExceeded,
    TransferFailed { token: String, reason: String },
    Unauthorized,
    MathOverflow,
    InvariantNotConverged,
    PoolPaused,
}

#[derive(CandidType, Deserialize, Debug)]
pub enum ThreePoolResult<T> {
    #[serde(rename = "Ok")]
    Ok(T),
    #[serde(rename = "Err")]
    Err(ThreePoolError),
}

// ─── ICPSwap Types ───

#[derive(CandidType, Deserialize, Debug)]
pub enum IcpSwapResult {
    #[serde(rename = "ok")]
    Ok(Nat),
    #[serde(rename = "err")]
    Err(IcpSwapError),
}

#[derive(CandidType, Deserialize, Debug)]
pub struct IcpSwapError {
    pub message: String,
}

// ─── Price Data ───

pub struct PriceData {
    /// ICP price in 3USD (e8s-scale, i.e. how many 3USD-native-units per 1 ICP)
    pub rumi_icp_price_3usd_native: u64,
    /// 3pool virtual price (e8s-scale, represents value of 1 3USD LP token)
    pub virtual_price: u64,
    /// ICP price in ckUSDC (6-decimal native units per 1 ICP)
    pub icpswap_icp_price_ckusdc_native: u64,
}

impl PriceData {
    /// Rumi ICP price normalized to USD (6-decimal fixed-point).
    /// rumi_price_3usd * virtual_price / 1e8 (since virtual_price is e8s-scale)
    /// Then convert from 3USD-native to 6-decimal USD.
    ///
    /// Assuming 3USD has 8 decimals (like icUSD):
    /// price_3usd_native is "3USD-e8s per ICP-e8s" but really it's the output
    /// of get_quote for 1 ICP worth of input. We need to think about this carefully.
    ///
    /// Actually, get_quote returns amount_out for a given amount_in.
    /// So we query: get_quote(pool_id, icp_principal, 1_0000_0000) -> 3USD output in native units.
    /// If 3USD is 8 decimals, this gives us e8s of 3USD per 1 ICP.
    /// To convert to USD: multiply by virtual_price / 1e8.
    /// Result is still in e8s scale. To get 6-decimal USD: divide by 100.
    pub fn rumi_price_usd_6dec(&self) -> u64 {
        // rumi_icp_price_3usd_native is 3USD-e8s per 1 ICP
        // virtual_price is e8s-scale (1e8 = $1 par value, >1e8 means >$1)
        // USD value = 3usd_amount * virtual_price / 1e8
        // Convert e8s to 6-dec: divide by 100
        let usd_e8s = self.rumi_icp_price_3usd_native as u128
            * self.virtual_price as u128
            / 100_000_000;
        (usd_e8s / 100) as u64 // e8s -> 6-decimal
    }

    /// ICPSwap price is already in ckUSDC (6-decimal ≈ USD)
    pub fn icpswap_price_usd_6dec(&self) -> u64 {
        self.icpswap_icp_price_ckusdc_native
    }

    /// Spread in basis points. Positive = Rumi cheaper, Negative = ICPSwap cheaper.
    pub fn spread_bps(&self) -> i32 {
        let rumi = self.rumi_price_usd_6dec() as i64;
        let icpswap = self.icpswap_price_usd_6dec() as i64;
        if rumi == 0 || icpswap == 0 {
            return 0;
        }
        let diff = icpswap - rumi;
        let min_price = rumi.min(icpswap);
        (diff * 10_000 / min_price) as i32
    }
}

/// Fetch ICP price in 3USD from Rumi AMM.
/// Queries get_quote for 1 ICP -> 3USD output.
pub async fn fetch_rumi_price(
    rumi_amm: Principal,
    pool_id: &str,
    icp_ledger: Principal,
) -> Result<u64, String> {
    let amount_in = Nat::from(100_000_000u64); // 1 ICP in e8s

    let result: Result<(AmmResult<Nat>,), _> = ic_cdk::call(
        rumi_amm,
        "get_quote",
        (pool_id.to_string(), icp_ledger, amount_in),
    ).await;

    match result {
        Ok((AmmResult::Ok(amount_out),)) => Ok(nat_to_u64(&amount_out)),
        Ok((AmmResult::Err(e),)) => Err(format!("Rumi AMM quote error: {:?}", e)),
        Err((code, msg)) => Err(format!("Rumi AMM call failed ({:?}): {}", code, msg)),
    }
}

/// Fetch 3pool virtual price from get_pool_status.
pub async fn fetch_virtual_price(rumi_3pool: Principal) -> Result<u64, String> {
    let result: Result<(PoolStatus,), _> = ic_cdk::call(
        rumi_3pool,
        "get_pool_status",
        (),
    ).await;

    match result {
        Ok((status,)) => Ok(nat_to_u64(&status.virtual_price)),
        Err((code, msg)) => Err(format!("3pool status call failed ({:?}): {}", code, msg)),
    }
}

/// Fetch ICP price in ckUSDC from ICPSwap pool.
/// Uses the quote query: how much ckUSDC for 1 ICP?
pub async fn fetch_icpswap_price(
    icpswap_pool: Principal,
    zero_for_one: bool,
) -> Result<u64, String> {
    #[derive(CandidType)]
    struct SwapArgs {
        #[serde(rename = "amountIn")]
        amount_in: String,
        #[serde(rename = "zeroForOne")]
        zero_for_one: bool,
        #[serde(rename = "amountOutMinimum")]
        amount_out_minimum: String,
    }

    let args = SwapArgs {
        amount_in: "100000000".to_string(), // 1 ICP in e8s
        zero_for_one,
        amount_out_minimum: "0".to_string(),
    };

    let result: Result<(IcpSwapResult,), _> = ic_cdk::call(
        icpswap_pool,
        "quote",
        (args,),
    ).await;

    match result {
        Ok((IcpSwapResult::Ok(amount),)) => Ok(nat_to_u64(&amount)),
        Ok((IcpSwapResult::Err(e),)) => Err(format!("ICPSwap quote error: {}", e.message)),
        Err((code, msg)) => Err(format!("ICPSwap call failed ({:?}): {}", code, msg)),
    }
}

/// Fetch all prices in parallel.
pub async fn fetch_all_prices(
    rumi_amm: Principal,
    pool_id: &str,
    icp_ledger: Principal,
    rumi_3pool: Principal,
    icpswap_pool: Principal,
    icpswap_zero_for_one: bool,
) -> Result<PriceData, String> {
    let rumi_fut = fetch_rumi_price(rumi_amm, pool_id, icp_ledger);
    let vp_fut = fetch_virtual_price(rumi_3pool);
    let icpswap_fut = fetch_icpswap_price(icpswap_pool, icpswap_zero_for_one);

    let (rumi_result, vp_result, icpswap_result) =
        futures::future::join3(rumi_fut, vp_fut, icpswap_fut).await;

    Ok(PriceData {
        rumi_icp_price_3usd_native: rumi_result?,
        virtual_price: vp_result?,
        icpswap_icp_price_ckusdc_native: icpswap_result?,
    })
}

pub fn nat_to_u64(n: &Nat) -> u64 {
    n.0.to_string().parse::<u64>().unwrap_or(0)
}
```

Update `lib.rs` to add `mod prices;`.

**Verify:** `cargo check`

**Commit:** `feat: add prices module with Rumi AMM, 3pool, and ICPSwap quote fetching`

---

## Phase 3: Swap Execution

### Step 3.1: Create swaps module for Rumi AMM

**File: `/Users/robertripley/coding/rumi-arb-bot/src/arb_bot/src/swaps.rs`**
```rust
use candid::{CandidType, Deserialize, Nat, Principal};
use icrc_ledger_types::icrc1::account::Account;
use icrc_ledger_types::icrc2::approve::{ApproveArgs, ApproveError};

use crate::prices::{self, AmmResult, nat_to_u64};

#[derive(Debug)]
pub enum SwapError {
    QuoteFailed(String),
    SwapFailed(String),
    ApproveFailed(String),
}

impl std::fmt::Display for SwapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SwapError::QuoteFailed(msg) => write!(f, "Quote failed: {}", msg),
            SwapError::SwapFailed(msg) => write!(f, "Swap failed: {}", msg),
            SwapError::ApproveFailed(msg) => write!(f, "Approve failed: {}", msg),
        }
    }
}

// ─── ICPSwap Types ───

#[derive(CandidType)]
struct IcpSwapDepositAndSwapArgs {
    #[serde(rename = "amountIn")]
    amount_in: String,
    #[serde(rename = "zeroForOne")]
    zero_for_one: bool,
    #[serde(rename = "amountOutMinimum")]
    amount_out_minimum: String,
    #[serde(rename = "tokenInFee")]
    token_in_fee: Nat,
    #[serde(rename = "tokenOutFee")]
    token_out_fee: Nat,
}

#[derive(CandidType, Deserialize, Debug)]
struct SwapResult {
    amount_out: Nat,
    fee: Nat,
}

// ─── Rumi AMM Swap ───

/// Swap on Rumi AMM. Approval must already be in place.
/// Returns (amount_out, fee) in native units of the output token.
pub async fn rumi_swap(
    rumi_amm: Principal,
    pool_id: &str,
    token_in: Principal,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<u64, SwapError> {
    let result: Result<(AmmResult<SwapResult>,), _> = ic_cdk::call(
        rumi_amm,
        "swap",
        (
            pool_id.to_string(),
            token_in,
            Nat::from(amount_in),
            Nat::from(min_amount_out),
        ),
    ).await;

    match result {
        Ok((AmmResult::Ok(r),)) => Ok(nat_to_u64(&r.amount_out)),
        Ok((AmmResult::Err(e),)) => Err(SwapError::SwapFailed(format!("{:?}", e))),
        Err((code, msg)) => Err(SwapError::SwapFailed(format!("{:?}: {}", code, msg))),
    }
}

// ─── ICPSwap Swap ───

/// Swap on ICPSwap using depositFromAndSwap. Approval must already be in place.
/// Returns amount_out in native units of the output token.
pub async fn icpswap_swap(
    icpswap_pool: Principal,
    amount_in: u64,
    zero_for_one: bool,
    min_amount_out: u64,
    token_in_fee: u64,
    token_out_fee: u64,
) -> Result<u64, SwapError> {
    let args = IcpSwapDepositAndSwapArgs {
        amount_in: amount_in.to_string(),
        zero_for_one,
        amount_out_minimum: min_amount_out.to_string(),
        token_in_fee: Nat::from(token_in_fee),
        token_out_fee: Nat::from(token_out_fee),
    };

    let result: Result<(prices::IcpSwapResult,), _> = ic_cdk::call(
        icpswap_pool,
        "depositFromAndSwap",
        (args,),
    ).await;

    match result {
        Ok((prices::IcpSwapResult::Ok(amount),)) => Ok(nat_to_u64(&amount)),
        Ok((prices::IcpSwapResult::Err(e),)) => Err(SwapError::SwapFailed(format!("ICPSwap: {}", e.message))),
        Err((code, msg)) => Err(SwapError::SwapFailed(format!("ICPSwap call ({:?}): {}", code, msg))),
    }
}

// ─── ICRC-2 Approvals ───

/// Set a one-time infinite approval for a token spender.
pub async fn approve_infinite(
    token_ledger: Principal,
    spender: Principal,
) -> Result<(), SwapError> {
    let approve_args = ApproveArgs {
        from_subaccount: None,
        spender: Account { owner: spender, subaccount: None },
        amount: Nat::from(340_282_366_920_938_463_463_374_607_431_768_211_455u128), // u128::MAX
        expected_allowance: None,
        expires_at: None, // never expires
        fee: None,
        memo: None,
        created_at_time: None,
    };

    let result: Result<(Result<Nat, ApproveError>,), _> =
        ic_cdk::call(token_ledger, "icrc2_approve", (approve_args,)).await;

    match result {
        Ok((Ok(_),)) => Ok(()),
        Ok((Err(e),)) => Err(SwapError::ApproveFailed(format!("{:?}", e))),
        Err((code, msg)) => Err(SwapError::ApproveFailed(format!("{:?}: {}", code, msg))),
    }
}
```

Update `lib.rs` to add `mod swaps;`.

**Verify:** `cargo check`

**Commit:** `feat: add swaps module with Rumi AMM, ICPSwap swap execution, and ICRC-2 approvals`

---

## Phase 4: Core Arbitrage Loop

### Step 4.1: Create the arb loop module

This is the main orchestration: drain residual ICP, fetch prices, check spread, calculate trade size, execute, log.

**File: `/Users/robertripley/coding/rumi-arb-bot/src/arb_bot/src/arb.rs`**
```rust
use candid::{Nat, Principal};

use crate::prices::{self, PriceData, nat_to_u64};
use crate::state::{self, Direction, ErrorRecord, Token, TradeRecord};
use crate::swaps;

/// Known token decimal configs
const ICP_FEE: u64 = 10_000;        // 0.0001 ICP
const CKUSDC_FEE: u64 = 10_000;     // 0.01 ckUSDC (6 decimals, so 10_000 = 0.01)
const THREE_USD_FEE: u64 = 10_000;  // TBD — placeholder, update when known

/// The pool_id string used on the Rumi AMM for the 3USD/ICP pool.
/// This will need to match whatever the AMM uses. Populated from config or hardcoded.
const RUMI_POOL_ID: &str = "3usd_icp"; // TBD — confirm with Rumi AMM

/// ICPSwap token ordering: whether ICP is token0 (zeroForOne=true means ICP→ckUSDC).
/// This must be determined once from the pool's metadata. For now, stored as a constant.
/// TBD — query pool metadata on init and store in state.
const ICPSWAP_ICP_IS_TOKEN0: bool = true; // TBD — verify

/// Main arb loop, called every 60 seconds by the timer.
pub async fn run_arb_cycle() {
    let config = state::read_state(|s| s.config.clone());

    if config.paused {
        return;
    }

    // Step 0: Drain residual ICP
    if let Err(e) = drain_residual_icp(&config).await {
        log_error(&format!("Drain residual ICP failed: {}", e));
        // Continue anyway — don't block the cycle
    }

    // Step 1-2: Fetch & compare prices
    let prices = match prices::fetch_all_prices(
        config.rumi_amm,
        RUMI_POOL_ID,
        config.icp_ledger,
        config.rumi_3pool,
        config.icpswap_pool,
        ICPSWAP_ICP_IS_TOKEN0,
    ).await {
        Ok(p) => p,
        Err(e) => {
            log_error(&format!("Price fetch failed: {}", e));
            return;
        }
    };

    // Step 3: Check spread
    let spread = prices.spread_bps();
    let abs_spread = spread.unsigned_abs();
    if abs_spread < config.min_spread_bps {
        return; // Spread too small, skip
    }

    // Step 4-5: Calculate trade size and execute
    if spread > 0 {
        // Positive spread = Rumi is cheaper, buy on Rumi, sell on ICPSwap
        execute_rumi_to_icpswap(&config, &prices, abs_spread).await;
    } else {
        // Negative spread = ICPSwap is cheaper, buy on ICPSwap, sell on Rumi
        execute_icpswap_to_rumi(&config, &prices, abs_spread).await;
    }
}

/// Buy ICP on Rumi (3USD → ICP), sell on ICPSwap (ICP → ckUSDC)
async fn execute_rumi_to_icpswap(config: &state::BotConfig, prices: &PriceData, spread_bps: u32) {
    // Calculate how much 3USD to spend
    // For now: use a fixed fraction of available balance, capped by max_trade_size
    let three_usd_balance = match fetch_balance(config.three_usd_ledger).await {
        Ok(b) => b,
        Err(e) => { log_error(&format!("Failed to get 3USD balance: {}", e)); return; }
    };

    if three_usd_balance < 1_000_000 { // minimum ~0.01 3USD
        log_error("Insufficient 3USD balance for arb");
        return;
    }

    // Cap trade size: max_trade_size_usd is in 6-decimal USD
    // Convert to 3USD native: max_usd / virtual_price * 1e8 (if 3USD is 8 decimals)
    let max_three_usd = if prices.virtual_price > 0 {
        (config.max_trade_size_usd as u128 * 100_000_000 * 100 / prices.virtual_price as u128) as u64
    } else {
        three_usd_balance
    };
    let trade_amount_3usd = three_usd_balance.min(max_three_usd);

    // Step A: Swap 3USD → ICP on Rumi
    let icp_out = match swaps::rumi_swap(
        config.rumi_amm,
        RUMI_POOL_ID,
        config.three_usd_ledger,
        trade_amount_3usd,
        0, // min_amount_out — we'll set a proper value from the quote
    ).await {
        Ok(amount) => amount,
        Err(e) => { log_error(&format!("Rumi swap 3USD→ICP failed: {}", e)); return; }
    };

    // Step B: Swap ICP → ckUSDC on ICPSwap
    let ckusdc_out = match swaps::icpswap_swap(
        config.icpswap_pool,
        icp_out,
        ICPSWAP_ICP_IS_TOKEN0,
        0, // min_amount_out — should calculate from quote with slippage
        ICP_FEE,
        CKUSDC_FEE,
    ).await {
        Ok(amount) => amount,
        Err(e) => {
            log_error(&format!("ICPSwap swap ICP→ckUSDC failed (holding {} ICP): {}", icp_out, e));
            return;
        }
    };

    // Step 6: Log trade
    // Input: trade_amount_3usd of 3USD
    // Output: ckusdc_out of ckUSDC
    // Input USD value (6-dec): trade_amount_3usd * virtual_price / 1e8 / 100
    let input_usd_6dec = (trade_amount_3usd as u128 * prices.virtual_price as u128 / 100_000_000 / 100) as i64;
    let output_usd_6dec = ckusdc_out as i64; // ckUSDC is already 6-decimal USD
    // Ledger fees: 1 ICP transfer fee + 1 ckUSDC transfer fee (from the swaps)
    // ICP fee in USD-6dec: 10_000 e8s * price / 1e8 ≈ negligible
    // ckUSDC fee in USD-6dec: 10_000 (= $0.01)
    let ledger_fees_usd = 10_000i64; // ~$0.01 ckUSDC fee dominates

    let net_profit = output_usd_6dec - input_usd_6dec - ledger_fees_usd;

    state::mutate_state(|s| {
        s.trades.push(TradeRecord {
            timestamp: ic_cdk::api::time(),
            direction: Direction::RumiToIcpswap,
            icp_amount: icp_out,
            input_amount: trade_amount_3usd,
            input_token: Token::ThreeUSD,
            output_amount: ckusdc_out,
            output_token: Token::CkUSDC,
            virtual_price: prices.virtual_price,
            ledger_fees_usd: ledger_fees_usd,
            net_profit_usd: net_profit,
            spread_bps: spread_bps,
        });
    });
}

/// Buy ICP on ICPSwap (ckUSDC → ICP), sell on Rumi (ICP → 3USD)
async fn execute_icpswap_to_rumi(config: &state::BotConfig, prices: &PriceData, spread_bps: u32) {
    let ckusdc_balance = match fetch_balance(config.ckusdc_ledger).await {
        Ok(b) => b,
        Err(e) => { log_error(&format!("Failed to get ckUSDC balance: {}", e)); return; }
    };

    if ckusdc_balance < 10_000 { // minimum $0.01
        log_error("Insufficient ckUSDC balance for arb");
        return;
    }

    // Cap trade size: max_trade_size_usd is in 6-decimal USD, ckUSDC is 6-decimal
    let trade_amount_ckusdc = ckusdc_balance.min(config.max_trade_size_usd);

    // Step A: Swap ckUSDC → ICP on ICPSwap
    let icp_out = match swaps::icpswap_swap(
        config.icpswap_pool,
        trade_amount_ckusdc,
        !ICPSWAP_ICP_IS_TOKEN0, // reverse direction: ckUSDC → ICP
        0,
        CKUSDC_FEE,
        ICP_FEE,
    ).await {
        Ok(amount) => amount,
        Err(e) => { log_error(&format!("ICPSwap swap ckUSDC→ICP failed: {}", e)); return; }
    };

    // Step B: Swap ICP → 3USD on Rumi
    let three_usd_out = match swaps::rumi_swap(
        config.rumi_amm,
        RUMI_POOL_ID,
        config.icp_ledger,
        icp_out,
        0,
    ).await {
        Ok(amount) => amount,
        Err(e) => {
            log_error(&format!("Rumi swap ICP→3USD failed (holding {} ICP): {}", icp_out, e));
            return;
        }
    };

    // Log trade
    let input_usd_6dec = trade_amount_ckusdc as i64; // ckUSDC is 6-dec USD
    let output_usd_6dec = (three_usd_out as u128 * prices.virtual_price as u128 / 100_000_000 / 100) as i64;
    let ledger_fees_usd = 10_000i64;
    let net_profit = output_usd_6dec - input_usd_6dec - ledger_fees_usd;

    state::mutate_state(|s| {
        s.trades.push(TradeRecord {
            timestamp: ic_cdk::api::time(),
            direction: Direction::IcpswapToRumi,
            icp_amount: icp_out,
            input_amount: trade_amount_ckusdc,
            input_token: Token::CkUSDC,
            output_amount: three_usd_out,
            output_token: Token::ThreeUSD,
            virtual_price: prices.virtual_price,
            ledger_fees_usd: ledger_fees_usd,
            net_profit_usd: net_profit,
            spread_bps: spread_bps,
        });
    });
}

/// Check if the bot holds residual ICP and try to sell it.
async fn drain_residual_icp(config: &state::BotConfig) -> Result<(), String> {
    let icp_balance = fetch_balance(config.icp_ledger).await?;

    // Only drain if we have more than dust (> 0.001 ICP)
    if icp_balance <= 100_000 {
        return Ok(());
    }

    // Get quotes from both DEXs to find better price
    let rumi_quote = prices::fetch_rumi_price(
        config.rumi_amm, RUMI_POOL_ID, config.icp_ledger
    );
    let icpswap_quote = prices::fetch_icpswap_price(
        config.icpswap_pool, ICPSWAP_ICP_IS_TOKEN0
    );
    let vp = prices::fetch_virtual_price(config.rumi_3pool);

    let (rumi_res, icpswap_res, vp_res) =
        futures::future::join3(rumi_quote, icpswap_quote, vp).await;

    // Prefer the DEX with the better price; fall back to whichever works
    let rumi_usd = rumi_res.ok().and_then(|r| {
        vp_res.as_ref().ok().map(|vp| (r as u128 * *vp as u128 / 100_000_000 / 100) as u64)
    });
    let icpswap_usd = icpswap_res.ok();

    match (rumi_usd, icpswap_usd) {
        (Some(r), Some(i)) if r >= i => {
            // Sell on Rumi (ICP → 3USD)
            let _ = swaps::rumi_swap(config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, icp_balance, 0).await;
        }
        (_, Some(_)) => {
            // Sell on ICPSwap (ICP → ckUSDC)
            let _ = swaps::icpswap_swap(config.icpswap_pool, icp_balance, ICPSWAP_ICP_IS_TOKEN0, 0, ICP_FEE, CKUSDC_FEE).await;
        }
        (Some(_), None) => {
            let _ = swaps::rumi_swap(config.rumi_amm, RUMI_POOL_ID, config.icp_ledger, icp_balance, 0).await;
        }
        (None, None) => {
            return Err("Both DEX quotes failed during ICP drain".to_string());
        }
    }

    Ok(())
}

/// Query ICRC-1 balance for this canister.
async fn fetch_balance(ledger: Principal) -> Result<u64, String> {
    let account = icrc_ledger_types::icrc1::account::Account {
        owner: ic_cdk::api::id(),
        subaccount: None,
    };

    let result: Result<(Nat,), _> = ic_cdk::call(ledger, "icrc1_balance_of", (account,)).await;

    match result {
        Ok((balance,)) => Ok(nat_to_u64(&balance)),
        Err((code, msg)) => Err(format!("Balance query failed ({:?}): {}", code, msg)),
    }
}

fn log_error(msg: &str) {
    state::mutate_state(|s| {
        s.errors.push(ErrorRecord {
            timestamp: ic_cdk::api::time(),
            message: msg.to_string(),
        });
        // Keep error log bounded
        if s.errors.len() > 1000 {
            s.errors.drain(0..500);
        }
    });
}
```

Update `lib.rs` to add `mod arb;` and wire up the timer:

```rust
// Add to lib.rs:
mod arb;

fn setup_timer() {
    ic_cdk_timers::set_timer_interval(
        std::time::Duration::from_secs(60),
        || ic_cdk::spawn(arb::run_arb_cycle()),
    );
}
```

Call `setup_timer()` from both `init` and `post_upgrade`.

**Verify:** `cargo check`

**Commit:** `feat: add core arb loop with price comparison, trade execution, and residual ICP drain`

---

## Phase 5: Admin Methods & Query Endpoints

### Step 5.1: Add admin and query methods to lib.rs

Update `/Users/robertripley/coding/rumi-arb-bot/src/arb_bot/src/lib.rs` to include:

```rust
use candid::{CandidType, Deserialize, Nat, Principal};
use ic_cdk_macros::{init, post_upgrade, pre_upgrade, query, update};

mod state;
mod prices;
mod swaps;
mod arb;

use state::{BotConfig, TradeRecord, ErrorRecord};

#[derive(CandidType, Deserialize)]
pub struct InitArgs {
    pub config: BotConfig,
}

#[init]
fn init(args: InitArgs) {
    state::init_state(state::BotState {
        config: args.config,
        ..Default::default()
    });
    setup_timer();
}

#[pre_upgrade]
fn pre_upgrade() {
    state::save_to_stable_memory();
}

#[post_upgrade]
fn post_upgrade() {
    state::load_from_stable_memory();
    setup_timer();
}

fn setup_timer() {
    ic_cdk_timers::set_timer_interval(
        std::time::Duration::from_secs(60),
        || ic_cdk::spawn(arb::run_arb_cycle()),
    );
}

fn require_owner() {
    let caller = ic_cdk::api::caller();
    let owner = state::read_state(|s| s.config.owner);
    if caller != owner {
        ic_cdk::trap("Unauthorized: only owner can call this");
    }
}

// ─── Query Methods ───

#[query]
fn get_config() -> BotConfig {
    state::read_state(|s| s.config.clone())
}

#[query]
fn get_trade_history(offset: u64, limit: u64) -> Vec<TradeRecord> {
    state::read_state(|s| {
        let len = s.trades.len();
        let start = (len as u64).saturating_sub(offset + limit) as usize;
        let end = (len as u64).saturating_sub(offset) as usize;
        s.trades[start..end].to_vec()
    })
}

#[derive(CandidType)]
pub struct TradeSummary {
    pub total_trades: u64,
    pub total_net_profit_usd: i64,
    pub total_ledger_fees_usd: i64,
    pub avg_profit_per_trade_usd: i64,
    pub rumi_to_icpswap_count: u64,
    pub rumi_to_icpswap_profit: i64,
    pub icpswap_to_rumi_count: u64,
    pub icpswap_to_rumi_profit: i64,
}

#[query]
fn get_summary() -> TradeSummary {
    state::read_state(|s| {
        let mut summary = TradeSummary {
            total_trades: s.trades.len() as u64,
            total_net_profit_usd: 0,
            total_ledger_fees_usd: 0,
            avg_profit_per_trade_usd: 0,
            rumi_to_icpswap_count: 0,
            rumi_to_icpswap_profit: 0,
            icpswap_to_rumi_count: 0,
            icpswap_to_rumi_profit: 0,
        };
        for trade in &s.trades {
            summary.total_net_profit_usd += trade.net_profit_usd;
            summary.total_ledger_fees_usd += trade.ledger_fees_usd;
            match trade.direction {
                state::Direction::RumiToIcpswap => {
                    summary.rumi_to_icpswap_count += 1;
                    summary.rumi_to_icpswap_profit += trade.net_profit_usd;
                }
                state::Direction::IcpswapToRumi => {
                    summary.icpswap_to_rumi_count += 1;
                    summary.icpswap_to_rumi_profit += trade.net_profit_usd;
                }
            }
        }
        if summary.total_trades > 0 {
            summary.avg_profit_per_trade_usd = summary.total_net_profit_usd / summary.total_trades as i64;
        }
        summary
    })
}

#[query]
fn get_errors(offset: u64, limit: u64) -> Vec<ErrorRecord> {
    state::read_state(|s| {
        let len = s.errors.len();
        let start = (len as u64).saturating_sub(offset + limit) as usize;
        let end = (len as u64).saturating_sub(offset) as usize;
        s.errors[start..end].to_vec()
    })
}

// ─── Admin Methods ───

#[update]
fn set_config(config: BotConfig) {
    require_owner();
    state::mutate_state(|s| s.config = config);
}

#[update]
fn pause() {
    require_owner();
    state::mutate_state(|s| s.config.paused = true);
}

#[update]
fn resume() {
    require_owner();
    state::mutate_state(|s| s.config.paused = false);
}

#[update]
async fn setup_approvals() {
    require_owner();
    let config = state::read_state(|s| s.config.clone());

    // 3USD → Rumi AMM
    let r1 = swaps::approve_infinite(config.three_usd_ledger, config.rumi_amm).await;
    // ICP → Rumi AMM
    let r2 = swaps::approve_infinite(config.icp_ledger, config.rumi_amm).await;
    // ICP → ICPSwap Pool
    let r3 = swaps::approve_infinite(config.icp_ledger, config.icpswap_pool).await;
    // ckUSDC → ICPSwap Pool
    let r4 = swaps::approve_infinite(config.ckusdc_ledger, config.icpswap_pool).await;

    let mut errors = Vec::new();
    if let Err(e) = r1 { errors.push(format!("3USD→RumiAMM: {}", e)); }
    if let Err(e) = r2 { errors.push(format!("ICP→RumiAMM: {}", e)); }
    if let Err(e) = r3 { errors.push(format!("ICP→ICPSwap: {}", e)); }
    if let Err(e) = r4 { errors.push(format!("ckUSDC→ICPSwap: {}", e)); }

    if !errors.is_empty() {
        ic_cdk::trap(&format!("Some approvals failed: {}", errors.join("; ")));
    }
}

#[update]
async fn withdraw(token_ledger: Principal, to: Principal, amount: u64) {
    require_owner();

    let transfer_args = icrc_ledger_types::icrc1::transfer::TransferArg {
        from_subaccount: None,
        to: icrc_ledger_types::icrc1::account::Account { owner: to, subaccount: None },
        amount: Nat::from(amount),
        fee: None,
        memo: None,
        created_at_time: None,
    };

    let result: Result<(Result<Nat, icrc_ledger_types::icrc1::transfer::TransferError>,), _> =
        ic_cdk::call(token_ledger, "icrc1_transfer", (transfer_args,)).await;

    match result {
        Ok((Ok(_),)) => {}
        Ok((Err(e),)) => ic_cdk::trap(&format!("Transfer failed: {:?}", e)),
        Err((code, msg)) => ic_cdk::trap(&format!("Transfer call failed: {:?} {}", code, msg)),
    }
}

/// Manually trigger one arb cycle (for testing).
#[update]
async fn manual_arb_cycle() {
    require_owner();
    arb::run_arb_cycle().await;
}
```

**Verify:** `cargo check`

**Commit:** `feat: add admin methods (config, pause, approvals, withdraw) and query endpoints (history, summary, errors)`

---

## Phase 6: Dashboard

### Step 6.1: Create the embedded HTML dashboard

**File: `/Users/robertripley/coding/rumi-arb-bot/src/arb_bot/src/dashboard.html`**

This will be a single HTML file with inline CSS and JS that uses `@dfinity/agent` from a CDN to call the canister. The file will be included via `include_str!` in the Rust code.

The dashboard should include:
- Header with bot status (paused/running) and canister ID
- Wallet balances panel (3USD, ckUSDC, ICP)
- Current prices panel (Rumi price, ICPSwap price, spread)
- Summary stats (total P&L, trade count, avg profit, fees)
- Trade history table (paginated, most recent first)
- Error log (collapsible)
- Admin controls: pause/resume, manual cycle trigger, withdraw form
- Internet Identity login button

The HTML will use agent-js from CDN (`https://unpkg.com/@dfinity/agent` and `@dfinity/auth-client`).

**This is a large file — implement as a functional but simple dashboard. Styling should be minimal and clean.**

### Step 6.2: Add http_request handler to lib.rs

```rust
// Add to lib.rs:

#[derive(CandidType, Deserialize)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(CandidType)]
pub struct HttpResponse {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

const DASHBOARD_HTML: &str = include_str!("dashboard.html");

#[query]
fn http_request(req: HttpRequest) -> HttpResponse {
    HttpResponse {
        status_code: 200,
        headers: vec![
            ("Content-Type".to_string(), "text/html; charset=utf-8".to_string()),
            ("Cache-Control".to_string(), "no-cache".to_string()),
        ],
        body: DASHBOARD_HTML.as_bytes().to_vec(),
    }
}
```

**Verify:** `cargo check`

**Commit:** `feat: add embedded HTML dashboard served via http_request`

---

## Phase 7: Candid Interface & Deployment

### Step 7.1: Write the complete .did file

Update `/Users/robertripley/coding/rumi-arb-bot/src/arb_bot/arb_bot.did` with all methods.

### Step 7.2: Build and test locally

```bash
cd /Users/robertripley/coding/rumi-arb-bot
dfx start --background
dfx deploy arb_bot --argument '(record { config = record {
  owner = principal "YOUR-PRINCIPAL";
  rumi_amm = principal "TBD";
  rumi_3pool = principal "TBD";
  icpswap_pool = principal "TBD";
  icp_ledger = principal "ryjl3-tyaaa-aaaaa-aaaba-cai";
  ckusdc_ledger = principal "xevnm-gaaaa-aaaar-qafnq-cai";
  three_usd_ledger = principal "TBD";
  min_spread_bps = 50 : nat32;
  max_trade_size_usd = 100_000_000 : nat64;
  paused = true : bool;
}})'
```

**Verify:** Canister deploys and dashboard loads at `http://localhost:4943/?canisterId=<id>`

**Commit:** `feat: complete Candid interface, ready for deployment`

### Step 7.3: Deploy to mainnet

```bash
dfx deploy --network ic arb_bot --argument '(...)'
```

Then call `setup_approvals` and fund the canister with 3USD and ckUSDC.

**Commit:** `chore: deploy to mainnet`

---

## Open Items (TBD before mainnet)

These need to be resolved during implementation:

1. **3USD ledger principal** — get from Rumi 3pool config
2. **3USD transfer fee** — query from ledger
3. **3USD decimal count** — confirm 8 decimals (like icUSD)
4. **Rumi AMM pool_id string** — confirm the exact pool identifier for 3USD/ICP
5. **ICPSwap pool canister ID** — query from factory `4mmnk-kiaaa-aaaag-qbllq-cai`
6. **ICPSwap token ordering** — query pool `metadata()` to determine if ICP is token0 or token1 (sets `zeroForOne` direction)
7. **Slippage protection** — the plan currently passes `min_amount_out = 0` for simplicity. Before mainnet, calculate proper minimums from quotes with a slippage buffer (e.g., 1-2%)
8. **Trade size optimization** — currently uses full available balance capped by max. Could be smarter about sizing to not move the price too far past parity

---

## Task Summary

| # | Task | Files |
|---|---|---|
| 1.1 | Scaffold dfx project | `dfx.json`, `Cargo.toml`, `src/arb_bot/Cargo.toml`, `src/arb_bot/src/lib.rs`, `.did` |
| 1.2 | State module | `src/arb_bot/src/state.rs` |
| 2.1 | Prices module | `src/arb_bot/src/prices.rs` |
| 3.1 | Swaps module | `src/arb_bot/src/swaps.rs` |
| 4.1 | Arb loop | `src/arb_bot/src/arb.rs`, update `lib.rs` |
| 5.1 | Admin & query endpoints | update `lib.rs` |
| 6.1 | Dashboard HTML | `src/arb_bot/src/dashboard.html` |
| 6.2 | HTTP handler | update `lib.rs` |
| 7.1 | Complete .did file | `src/arb_bot/arb_bot.did` |
| 7.2 | Local deploy & test | — |
| 7.3 | Mainnet deploy | — |
