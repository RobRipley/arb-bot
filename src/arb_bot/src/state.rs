use candid::{CandidType, Deserialize, Principal};
use serde::Serialize;
use std::borrow::Cow;
use std::cell::RefCell;

use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    storable::{Bound, Storable},
    DefaultMemoryImpl, StableCell, StableLog,
};

type Memory = VirtualMemory<DefaultMemoryImpl>;

fn default_principal() -> Principal {
    Principal::anonymous()
}

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
    /// Whether ICP is token0 in the ICPSwap pool (resolved from pool metadata at init)
    pub icpswap_icp_is_token0: bool,
    /// Additional admin principals (e.g. Internet Identity) that can call admin methods
    #[serde(default)]
    pub admins: Vec<Principal>,
    /// Strategy B: ICPSwap icUSD/ICP pool canister
    #[serde(default = "default_principal")]
    pub icpswap_icusd_pool: Principal,
    /// Strategy B: icUSD ledger canister
    #[serde(default = "default_principal")]
    pub icusd_ledger: Principal,
    /// Whether ICP is token0 in the ICPSwap icUSD/ICP pool
    #[serde(default)]
    pub icpswap_icusd_icp_is_token0: bool,
    /// Minimum net profit (6-decimal USD) required to execute a trade. 0 = disabled.
    #[serde(default)]
    pub min_profit_usd: i64,
    /// Strategy C: ICPSwap ckUSDT/ICP pool canister
    #[serde(default = "default_principal")]
    pub icpswap_ckusdt_pool: Principal,
    /// Strategy C: ckUSDT ledger canister
    #[serde(default = "default_principal")]
    pub ckusdt_ledger: Principal,
    /// Whether ICP is token0 in the ICPSwap ckUSDT/ICP pool
    #[serde(default)]
    pub icpswap_ckusdt_icp_is_token0: bool,
    /// ICPSwap 3USD/ICP pool canister
    #[serde(default = "default_principal")]
    pub icpswap_3usd_pool: Principal,
    /// Whether ICP is token0 in the ICPSwap 3USD/ICP pool
    #[serde(default)]
    pub icpswap_3usd_icp_is_token0: bool,
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
    pub ledger_fees_usd: i64,
    pub net_profit_usd: i64,
    pub spread_bps: u32,
}

#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub struct ErrorRecord {
    pub timestamp: u64,
    pub message: String,
}

#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub struct ActivityRecord {
    pub timestamp: u64,
    pub category: String,
    pub message: String,
}

/// Snapshot of all prices, balances, and spreads captured every arb cycle.
#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub struct CycleSnapshot {
    pub timestamp: u64,
    // Strategy A prices
    pub rumi_icp_price_3usd: u64,        // 3USD per 1 ICP (8 dec)
    pub rumi_icp_price_usd: u64,         // USD per 1 ICP (6 dec)
    pub icpswap_icp_price_ckusdc: u64,   // ckUSDC per 1 ICP (6 dec)
    pub virtual_price: u64,              // 3pool VP (18 dec)
    pub spread_a_bps: i32,               // Strategy A spread
    // Strategy B prices
    pub icpswap_icp_price_icusd: u64,    // icUSD per 1 ICP (8 dec), 0 if N/A
    pub spread_b_bps: i32,               // Strategy B spread, 0 if N/A
    // Balances (native decimals)
    pub balance_icp: u64,
    pub balance_3usd: u64,
    pub balance_ckusdc: u64,
    #[serde(default)]
    pub balance_ckusdt: u64,
    pub balance_icusd: u64,
    /// Strategy C: ckUSDT per 1 ICP (6 dec). 0 if N/A.
    #[serde(default)]
    pub icpswap_icp_price_ckusdt: u64,
    /// Strategy C spread, 0 if N/A
    #[serde(default)]
    pub spread_c_bps: i32,
    /// Strategy D spread (Rumi 3pool vs ICPSwap icUSD), 0 if N/A
    #[serde(default)]
    pub spread_d_bps: i32,
    /// Strategy F spread (ICPSwap icUSD/ICP vs ICPSwap ckUSDT/ICP), 0 if N/A
    #[serde(default)]
    pub spread_f_bps: i32,
    /// Strategy G spread (Rumi 3USD/ICP vs ICPSwap 3USD/ICP), 0 if N/A
    #[serde(default)]
    pub spread_g_bps: i32,
    /// Strategy H spread (ICPSwap 3USD/ICP vs ICPSwap icUSD/ICP), 0 if N/A
    #[serde(default)]
    pub spread_h_bps: i32,
    /// Strategy I spread (ICPSwap 3USD/ICP vs ICPSwap ckUSDC/ICP), 0 if N/A
    #[serde(default)]
    pub spread_i_bps: i32,
    /// Strategy J spread (ICPSwap 3USD/ICP vs ICPSwap ckUSDT/ICP), 0 if N/A
    #[serde(default)]
    pub spread_j_bps: i32,
    // Trade activity
    pub traded: bool,
    pub strategy_used: String,           // "", "A", "B", "C", or "D"
}

/// Identifies a specific liquidity pool for drain routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pool {
    RumiThreeUsd,
    IcpswapCkusdc,
    IcpswapIcusd,
    IcpswapCkusdt,
    IcpswapThreeUsd,
}

// ─── Volume bot types ───

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
    #[serde(default)]
    pub daily_cost_usd: i64,
}

impl Default for VolumePoolState {
    fn default() -> Self {
        Self {
            last_price: None,
            next_direction: VolumeDirection::BuyIcp,
            trade_count: 0,
            total_volume_usd: 0,
            total_cost_usd: 0,
            daily_cost_usd: 0,
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

/// Records the intended exit pool after a successful Leg 1, so the drain
/// can prefer it (and avoid draining back into the entry pool).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingExit {
    pub entry_pool: Pool,
    pub intended_exit_pool: Pool,
    pub timestamp: u64,
    /// ICP received by Leg1 — drain must not exceed this amount.
    #[serde(default)]
    pub icp_amount: u64,
}

#[derive(CandidType, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LegType {
    Leg1,
    Leg2,
    Drain,
}

#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub struct TradeLeg {
    pub timestamp: u64,
    pub leg_type: LegType,
    pub dex: String,             // "Rumi" or "ICPSwap"
    pub sold_token: String,      // "3USD", "ICP", "ckUSDC"
    pub sold_amount: u64,        // raw amount in token's native decimals
    pub bought_token: String,
    pub bought_amount: u64,
    pub sold_usd_value: i64,     // 6-decimal USD (0 for ICP legs)
    pub bought_usd_value: i64,   // 6-decimal USD (0 for ICP legs)
    pub fees_usd: i64,           // ledger fees in 6-decimal USD
}

/// Slimmed-down BotState — only small, bounded fields live in heap/cell.
/// Growing collections (trades, errors, activity, trade_legs, snapshots)
/// are stored in dedicated StableLogs, accessed via helper functions.
#[derive(Serialize, Deserialize, Clone)]
pub struct BotState {
    pub config: BotConfig,
    #[serde(default)]
    pub token_ordering_resolved: bool,
    #[serde(default)]
    pub icusd_token_ordering_resolved: bool,
    #[serde(default)]
    pub ckusdt_token_ordering_resolved: bool,
    #[serde(default)]
    pub icpswap_3usd_token_ordering_resolved: bool,
    #[serde(default)]
    pub pending_exit: Option<PendingExit>,
    #[serde(default)]
    pub volume: VolumeConfig,
    /// ICP amount stranded in the default account after a volume bot
    /// transfer-to-subaccount failure.  The arb drain must not touch this.
    #[serde(default)]
    pub volume_stranded_icp: u64,
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
                max_trade_size_usd: 100_000_000,
                paused: true,
                icpswap_icp_is_token0: true,
                admins: Vec::new(),
                icpswap_icusd_pool: Principal::anonymous(),
                icusd_ledger: Principal::anonymous(),
                icpswap_icusd_icp_is_token0: false,
                min_profit_usd: 0,
                icpswap_ckusdt_pool: Principal::anonymous(),
                ckusdt_ledger: Principal::anonymous(),
                icpswap_ckusdt_icp_is_token0: false,
                icpswap_3usd_pool: Principal::anonymous(),
                icpswap_3usd_icp_is_token0: false,
            },
            token_ordering_resolved: false,
            icusd_token_ordering_resolved: false,
            ckusdt_token_ordering_resolved: false,
            icpswap_3usd_token_ordering_resolved: false,
            pending_exit: None,
            volume: VolumeConfig::default(),
            volume_stranded_icp: 0,
        }
    }
}

/// Legacy (pre-stable-structures) state layout, used only for one-time
/// migration from raw-JSON stable memory into the new StableLogs.
#[derive(Deserialize)]
struct LegacyBotState {
    config: BotConfig,
    #[serde(default)]
    trades: Vec<TradeRecord>,
    #[serde(default)]
    errors: Vec<ErrorRecord>,
    #[serde(default)]
    activity_log: Vec<ActivityRecord>,
    #[serde(default)]
    token_ordering_resolved: bool,
    #[serde(default)]
    icusd_token_ordering_resolved: bool,
    #[serde(default)]
    ckusdt_token_ordering_resolved: bool,
    #[serde(default)]
    icpswap_3usd_token_ordering_resolved: bool,
    #[serde(default)]
    trade_legs: Vec<TradeLeg>,
    #[serde(default)]
    snapshots: Vec<CycleSnapshot>,
    #[serde(default)]
    pending_exit: Option<PendingExit>,
}

// ─── Storable impls (JSON encoding) ───

macro_rules! json_storable {
    ($t:ty) => {
        impl Storable for $t {
            const BOUND: Bound = Bound::Unbounded;
            fn to_bytes(&self) -> Cow<'_, [u8]> {
                Cow::Owned(serde_json::to_vec(self).expect("serialize"))
            }
            fn from_bytes(bytes: Cow<'_, [u8]>) -> Self {
                serde_json::from_slice(bytes.as_ref()).expect("deserialize")
            }
        }
    };
}

json_storable!(TradeRecord);
json_storable!(ErrorRecord);
json_storable!(ActivityRecord);
json_storable!(TradeLeg);
json_storable!(CycleSnapshot);
json_storable!(VolumeTradeLeg);

// ─── Stable memory layout ───
//
// MemoryId 0:       META_CELL (StableCell<Vec<u8>>) — JSON-encoded BotState
// MemoryId 1,2:     TRADES log (index + data)
// MemoryId 3,4:     ERRORS log
// MemoryId 5,6:     ACTIVITY log
// MemoryId 7,8:     TRADE_LEGS log
// MemoryId 9,10:    SNAPSHOTS log
// MemoryId 11,12:   VOLUME_TRADES log
//
// NEVER reuse or reorder these IDs — doing so corrupts existing data.

thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    static META_CELL: RefCell<StableCell<Vec<u8>, Memory>> = RefCell::new(
        StableCell::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0))),
            Vec::new(),
        ).expect("init META_CELL"),
    );

    static TRADES: RefCell<StableLog<TradeRecord, Memory, Memory>> = RefCell::new(
        StableLog::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(1))),
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(2))),
        ).expect("init TRADES"),
    );

    static ERRORS: RefCell<StableLog<ErrorRecord, Memory, Memory>> = RefCell::new(
        StableLog::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(3))),
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(4))),
        ).expect("init ERRORS"),
    );

    static ACTIVITY: RefCell<StableLog<ActivityRecord, Memory, Memory>> = RefCell::new(
        StableLog::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(5))),
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(6))),
        ).expect("init ACTIVITY"),
    );

    static TRADE_LEGS: RefCell<StableLog<TradeLeg, Memory, Memory>> = RefCell::new(
        StableLog::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(7))),
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(8))),
        ).expect("init TRADE_LEGS"),
    );

    static SNAPSHOTS: RefCell<StableLog<CycleSnapshot, Memory, Memory>> = RefCell::new(
        StableLog::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(9))),
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(10))),
        ).expect("init SNAPSHOTS"),
    );

    static VOLUME_TRADES: RefCell<StableLog<VolumeTradeLeg, Memory, Memory>> = RefCell::new(
        StableLog::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(11))),
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(12))),
        ).expect("init VOLUME_TRADES"),
    );

    // Heap cache mirroring META_CELL for fast reads.
    static STATE: RefCell<Option<BotState>> = RefCell::default();
}

// ─── Meta state access (write-through to META_CELL) ───

pub fn read_state<F, R>(f: F) -> R
where
    F: FnOnce(&BotState) -> R,
{
    STATE.with(|s| f(s.borrow().as_ref().expect("State not initialized")))
}

pub fn mutate_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut BotState) -> R,
{
    STATE.with(|s| {
        let mut guard = s.borrow_mut();
        let state = guard.as_mut().expect("State not initialized");
        let result = f(state);
        // Write-through: persist updated BotState into the stable cell.
        let bytes = serde_json::to_vec(state).expect("serialize BotState");
        META_CELL.with(|c| {
            let _ = c.borrow_mut().set(bytes);
        });
        result
    })
}

pub fn init_state(state: BotState) {
    let bytes = serde_json::to_vec(&state).expect("serialize BotState");
    META_CELL.with(|c| {
        let _ = c.borrow_mut().set(bytes);
    });
    STATE.with(|s| *s.borrow_mut() = Some(state));
}

// ─── Log helpers ───

pub fn append_trade(t: TradeRecord) {
    TRADES.with(|log| {
        let _ = log.borrow_mut().append(&t);
    });
}

pub fn trades_len() -> u64 {
    TRADES.with(|log| log.borrow().len())
}

pub fn get_trades_page(offset: u64, limit: u64) -> Vec<TradeRecord> {
    TRADES.with(|log| {
        let log = log.borrow();
        let total = log.len();
        let start = total.saturating_sub(offset + limit);
        let end = total.saturating_sub(offset);
        (start..end).filter_map(|i| log.get(i)).collect()
    })
}

pub fn append_error(e: ErrorRecord) {
    ERRORS.with(|log| {
        let _ = log.borrow_mut().append(&e);
    });
}

pub fn get_errors_page(offset: u64, limit: u64) -> Vec<ErrorRecord> {
    ERRORS.with(|log| {
        let log = log.borrow();
        let total = log.len();
        let start = total.saturating_sub(offset + limit);
        let end = total.saturating_sub(offset);
        (start..end).filter_map(|i| log.get(i)).collect()
    })
}

pub fn append_activity(a: ActivityRecord) {
    ACTIVITY.with(|log| {
        let _ = log.borrow_mut().append(&a);
    });
}

pub fn get_activity_page(offset: u64, limit: u64) -> Vec<ActivityRecord> {
    ACTIVITY.with(|log| {
        let log = log.borrow();
        let total = log.len();
        let start = total.saturating_sub(offset + limit);
        let end = total.saturating_sub(offset);
        (start..end).filter_map(|i| log.get(i)).collect()
    })
}

pub fn append_trade_leg(leg: TradeLeg) {
    TRADE_LEGS.with(|log| {
        let _ = log.borrow_mut().append(&leg);
    });
}

pub fn trade_legs_len() -> u64 {
    TRADE_LEGS.with(|log| log.borrow().len())
}

pub fn get_trade_legs_page(offset: u64, limit: u64) -> Vec<TradeLeg> {
    TRADE_LEGS.with(|log| {
        let log = log.borrow();
        let total = log.len();
        let start = total.saturating_sub(offset + limit);
        let end = total.saturating_sub(offset);
        (start..end).filter_map(|i| log.get(i)).collect()
    })
}

/// Fold over every trade leg (iterates the full stable log).
pub fn fold_trade_legs<T, F>(init: T, mut f: F) -> T
where
    F: FnMut(T, TradeLeg) -> T,
{
    TRADE_LEGS.with(|log| {
        let log = log.borrow();
        let mut acc = init;
        for i in 0..log.len() {
            if let Some(leg) = log.get(i) {
                acc = f(acc, leg);
            }
        }
        acc
    })
}

/// Scan trade legs from newest to oldest, mapping each through `f`.
/// Returns the first non-None result. Equivalent to `iter().rev().find_map(f)`.
pub fn find_map_last_trade_leg<T, F>(f: F) -> Option<T>
where
    F: Fn(TradeLeg) -> Option<T>,
{
    TRADE_LEGS.with(|log| {
        let log = log.borrow();
        let len = log.len();
        for i in (0..len).rev() {
            if let Some(leg) = log.get(i) {
                if let Some(out) = f(leg) {
                    return Some(out);
                }
            }
        }
        None
    })
}

/// Append an arbitrary batch of trade legs. Used by backfill admin method.
/// NOTE: With the move to append-only StableLog, backfill now APPENDS to
/// the end (previously prepended). Chronology of historical backfills is
/// not preserved — this is an admin-only tool and the caller was warned.
pub fn append_trade_legs_batch(legs: Vec<TradeLeg>) -> usize {
    let count = legs.len();
    TRADE_LEGS.with(|log| {
        let log = log.borrow_mut();
        for leg in legs {
            let _ = log.append(&leg);
        }
    });
    count
}

pub fn append_snapshot(s: CycleSnapshot) {
    SNAPSHOTS.with(|log| {
        let _ = log.borrow_mut().append(&s);
    });
}

pub fn snapshots_len() -> u64 {
    SNAPSHOTS.with(|log| log.borrow().len())
}

pub fn get_snapshots_page(offset: u64, limit: u64) -> Vec<CycleSnapshot> {
    SNAPSHOTS.with(|log| {
        let log = log.borrow();
        let total = log.len();
        let start = total.saturating_sub(offset + limit);
        let end = total.saturating_sub(offset);
        (start..end).filter_map(|i| log.get(i)).collect()
    })
}

pub fn append_volume_trade(leg: VolumeTradeLeg) {
    VOLUME_TRADES.with(|t| {
        let _ = t.borrow().append(&leg);
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
        (start..end).filter_map(|i| log.get(i)).collect()
    })
}

pub fn volume_trades_count() -> u64 {
    VOLUME_TRADES.with(|t| t.borrow().len())
}

// ─── log_activity (same signature as before) ───

pub fn log_activity(category: &str, message: &str) {
    append_activity(ActivityRecord {
        timestamp: ic_cdk::api::time(),
        category: category.to_string(),
        message: message.to_string(),
    });
}

pub fn log_error(message: String) {
    append_error(ErrorRecord {
        timestamp: ic_cdk::api::time(),
        message,
    });
}

// ─── Upgrade entry points ───

/// Called from `#[pre_upgrade]`.
///
/// With stable structures, every mutation to BotState is already
/// write-through to META_CELL, and every log entry is already in its
/// StableLog. There is nothing to serialize here — the whole point of
/// switching away from JSON-blob stable memory was to eliminate this
/// serialization step (and its instruction-limit trap risk).
pub fn save_to_stable_memory() {
    // No-op. Kept as a named entry point so lib.rs doesn't need to change.
}

/// Called from `#[post_upgrade]`.
///
/// On first upgrade from the legacy raw-JSON layout, this reads the old
/// BotState from the raw stable-memory blob and migrates its contents
/// into the new StableLogs + META_CELL. On subsequent upgrades it simply
/// loads BotState from META_CELL.
pub fn load_from_stable_memory() {
    let size = ic_cdk::api::stable::stable64_size();

    // Detect legacy raw-JSON layout.
    //
    // ic-stable-structures' MemoryManager writes the ASCII magic "MGR"
    // at the start of stable memory when it initializes. The legacy
    // layout wrote a little-endian u64 length at offset 0, which cannot
    // start with those three bytes. So: if we see "MGR", there's
    // nothing to migrate; otherwise, try to parse as legacy JSON.
    //
    // IMPORTANT: we must read raw stable memory BEFORE touching any
    // thread_local stable structure, because the first `.with()` call
    // triggers MemoryManager init — which overwrites offset 0 with the
    // "MGR" header and destroys the legacy blob.
    let legacy: Option<LegacyBotState> = if size == 0 {
        None
    } else {
        let mut magic = [0u8; 3];
        ic_cdk::api::stable::stable64_read(0, &mut magic);
        if &magic == b"MGR" {
            None
        } else {
            let mut len_bytes = [0u8; 8];
            ic_cdk::api::stable::stable64_read(0, &mut len_bytes);
            let len = u64::from_le_bytes(len_bytes) as usize;
            if len == 0 {
                None
            } else {
                let mut bytes = vec![0u8; len];
                ic_cdk::api::stable::stable64_read(8, &mut bytes);
                match serde_json::from_slice::<LegacyBotState>(&bytes) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        ic_cdk::println!(
                            "Migration: failed to parse legacy BotState: {}. Starting fresh.",
                            e
                        );
                        None
                    }
                }
            }
        }
    };

    if let Some(legacy) = legacy {
        // Rebuild the new slim BotState from the legacy meta fields.
        let new_state = BotState {
            config: legacy.config,
            token_ordering_resolved: legacy.token_ordering_resolved,
            icusd_token_ordering_resolved: legacy.icusd_token_ordering_resolved,
            ckusdt_token_ordering_resolved: legacy.ckusdt_token_ordering_resolved,
            icpswap_3usd_token_ordering_resolved: legacy.icpswap_3usd_token_ordering_resolved,
            pending_exit: legacy.pending_exit,
            volume: VolumeConfig::default(),
            volume_stranded_icp: 0,
        };

        // Touching any thread_local stable structure below triggers
        // MemoryManager init, which overwrites the legacy bytes with
        // its own "MGR" header. After this point, legacy raw data is
        // no longer readable, but we've already captured everything
        // we need in local variables above.
        let trade_count = legacy.trades.len();
        for t in legacy.trades {
            append_trade(t);
        }
        let error_count = legacy.errors.len();
        for e in legacy.errors {
            append_error(e);
        }
        let activity_count = legacy.activity_log.len();
        for a in legacy.activity_log {
            append_activity(a);
        }
        let leg_count = legacy.trade_legs.len();
        for l in legacy.trade_legs {
            append_trade_leg(l);
        }
        let snapshot_count = legacy.snapshots.len();
        for sn in legacy.snapshots {
            append_snapshot(sn);
        }

        init_state(new_state);

        // Record the migration in the activity log.
        log_activity(
            "admin",
            &format!(
                "Stable-memory migration complete: {} trades, {} errors, {} activity, {} legs, {} snapshots",
                trade_count, error_count, activity_count, leg_count, snapshot_count
            ),
        );
        return;
    }

    // Not a migration — either fresh install or already on new layout.
    let bytes = META_CELL.with(|c| c.borrow().get().clone());
    if bytes.is_empty() {
        STATE.with(|s| *s.borrow_mut() = Some(BotState::default()));
    } else {
        match serde_json::from_slice::<BotState>(&bytes) {
            Ok(state) => STATE.with(|s| *s.borrow_mut() = Some(state)),
            Err(e) => {
                ic_cdk::println!(
                    "Failed to deserialize BotState from META_CELL: {}. Using default.",
                    e
                );
                STATE.with(|s| *s.borrow_mut() = Some(BotState::default()));
            }
        }
    }
}
