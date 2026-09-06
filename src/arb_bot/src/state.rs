use candid::{CandidType, Deserialize, Principal};
use serde::Serialize;
use std::borrow::Cow;
use std::cell::RefCell;

use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    storable::{Bound, Storable},
    DefaultMemoryImpl, StableBTreeMap, StableCell, StableLog,
};

type Memory = VirtualMemory<DefaultMemoryImpl>;

fn default_principal() -> Principal {
    Principal::anonymous()
}

fn default_slippage_bps() -> u64 {
    50
}

fn default_arb_interval_secs() -> u64 {
    600
}

/// PartyDEX ICP/ckUSDC pool (500-pip / 0.05% fee tier is where liquidity is concentrated).
fn default_partydex_ckusdc_pool() -> Principal {
    Principal::from_text("xjiq2-fiaaa-aaaan-q52ra-cai").expect("valid principal")
}

/// PartyDEX ICP/ckUSDT pool.
fn default_partydex_ckusdt_pool() -> Principal {
    Principal::from_text("6b2bo-kyaaa-aaaao-qpira-cai").expect("valid principal")
}

fn default_partydex_fee_pips() -> u32 {
    500
}

/// ICP inventory band floor (e8s) — 2 ICP.
fn default_icp_inventory_floor() -> u64 {
    200_000_000
}

/// ICP inventory band ceiling (e8s) — 20 ICP.
fn default_icp_inventory_ceiling() -> u64 {
    2_000_000_000
}

/// BOB inventory band floor (e8s, 8 decimals) — 10 BOB.
fn default_bob_inventory_floor() -> u64 {
    1_000_000_000
}

/// BOB inventory band ceiling (e8s, 8 decimals) — 40 BOB.
fn default_bob_inventory_ceiling() -> u64 {
    4_000_000_000
}

/// BOB ledger — mainnet-verified principal (fee 1_000_000 e8s, 8 decimals).
fn default_bob_ledger() -> Principal {
    Principal::from_text("7pail-xaaaa-aaaas-aabmq-cai").expect("valid principal")
}

fn default_bob_ledger_fee() -> u64 {
    1_000_000
}

/// ICPSwap BOB/ICP pool — the sole BOB reference market (fee 3000 pips =
/// 0.3%, token0 = BOB — verified live 2026-07-16).
fn default_icpswap_bob_icp_pool() -> Principal {
    Principal::from_text("ybilh-nqaaa-aaaag-qkhzq-cai").expect("valid principal")
}

fn default_bob_max_trade_size_usd() -> u64 {
    50_000_000
}

fn default_bob_min_spread_bps() -> u64 {
    150
}

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

fn default_true() -> bool {
    true
}

#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub struct BotConfig {
    pub owner: Principal,
    pub rumi_amm: Principal,
    pub rumi_3pool: Principal,
    /// Kill switch for the Rumi AMM (3USD/ICP) venue. When true, Strategies
    /// A/C/D/Q/R (every strategy that trades against `rumi_amm`) are skipped
    /// entirely in both the auto cycle and force-execute — no network calls
    /// are made, so no cycles are burned checking a venue known to be
    /// illiquid. Toggle via `set_rumi_amm_paused`. Strategies B/F/K/L/M/N/O/P
    /// never touch `rumi_amm` and are unaffected.
    #[serde(default)]
    pub rumi_amm_paused: bool,
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
    /// Leg 1 and Leg 2 slippage tolerance in basis points. Runtime-tunable via
    /// `set_slippage_bps`. Widening this reduces Leg 2 failure rate (and the
    /// downstream drain losses) at the cost of accepting worse fills. Default 50.
    #[serde(default = "default_slippage_bps")]
    pub slippage_bps: u64,
    /// Interval between arb cycles in seconds. Runtime-tunable via
    /// `set_arb_interval_secs`. Higher values reduce cycle burn at the cost
    /// of slower reaction to arbitrage opportunities. Default 600.
    #[serde(default = "default_arb_interval_secs")]
    pub arb_interval_secs: u64,
    /// PartyDEX ICP/ckUSDC pool canister (used by Strategies K/L/M/Q in PR2b).
    #[serde(default = "default_partydex_ckusdc_pool")]
    pub partydex_ckusdc_pool: Principal,
    /// PartyDEX ICP/ckUSDT pool canister (used by Strategies N/O/P/R in PR2b).
    #[serde(default = "default_partydex_ckusdt_pool")]
    pub partydex_ckusdt_pool: Principal,
    /// Fee tier (pips) pool_swaps are pinned to on the PartyDEX ckUSDC pool. Default 500 (0.05%).
    #[serde(default = "default_partydex_fee_pips")]
    pub partydex_ckusdc_fee_pips: u32,
    /// Fee tier (pips) pool_swaps are pinned to on the PartyDEX ckUSDT pool. Default 500 (0.05%).
    #[serde(default = "default_partydex_fee_pips")]
    pub partydex_ckusdt_fee_pips: u32,
    /// ICP inventory band (e8s). Floor: minimum working balance the drain
    /// always leaves (fee buffer + strategy-S top-up trigger). Ceiling: the
    /// drain skims any balance above this to the best stable pool.
    #[serde(default = "default_icp_inventory_floor")]
    pub icp_inventory_floor_e8s: u64,
    #[serde(default = "default_icp_inventory_ceiling")]
    pub icp_inventory_ceiling_e8s: u64,
    /// BOB inventory band (e8s, 8 decimals). Mirrors `icp_inventory_floor_e8s`
    /// / `icp_inventory_ceiling_e8s` in shape only — stored/settable/displayed;
    /// not wired into any trading or drain logic.
    #[serde(default = "default_bob_inventory_floor")]
    pub bob_inventory_floor_e8s: u64,
    #[serde(default = "default_bob_inventory_ceiling")]
    pub bob_inventory_ceiling_e8s: u64,
    /// Strategy S: BOB ledger canister (mainnet-verified).
    #[serde(default = "default_bob_ledger")]
    pub bob_ledger: Principal,
    /// BOB ledger transfer fee (native units, 8 decimals).
    #[serde(default = "default_bob_ledger_fee")]
    pub bob_ledger_fee: u64,
    /// Strategy S: ICPSwap BOB/ICP pool canister — BOB's sole reference market.
    #[serde(default = "default_icpswap_bob_icp_pool")]
    pub icpswap_bob_icp_pool: Principal,
    /// Strategy S: ICPSwap icUSD/BOB pool canister. Anonymous until the pool
    /// is created — this is Strategy S's master gate (inert while anonymous).
    #[serde(default = "default_principal")]
    pub icpswap_icusd_bob_pool: Principal,
    /// Whether ICP is token0 in the ICPSwap BOB/ICP pool (resolved once).
    #[serde(default)]
    pub icpswap_bob_icp_icp_is_token0: bool,
    /// Whether icUSD is token0 in the ICPSwap icUSD/BOB pool (resolved once).
    #[serde(default)]
    pub icpswap_icusd_bob_icusd_is_token0: bool,
    /// Strategy S: max trade size per leg (6-dec USD). Default $50 — BOB/ICP
    /// moves ~1% per $265 of volume, so this keeps clips small relative to depth.
    #[serde(default = "default_bob_max_trade_size_usd")]
    pub bob_max_trade_size_usd: u64,
    /// Strategy S: minimum pool-vs-reference deviation (bps) required to trade.
    /// Default 150 — covers two-to-three 0.3% fee legs plus thin-pool slippage
    /// and reference uncertainty.
    #[serde(default = "default_bob_min_spread_bps")]
    pub bob_min_spread_bps: u64,
    /// Strategy S execution kill switch. Dry-run evaluation + dashboard
    /// surfacing always run once both BOB pools are configured; live
    /// execution additionally requires this to be true. Defaults false
    /// (dry-run-first, per design decision #5).
    #[serde(default)]
    pub bob_execution_enabled: bool,

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
}

/// Candid-boundary counterpart to `BotConfig`, used ONLY as the argument
/// type for `set_config` and inside `InitArgs`.
///
/// Candid subtyping is stricter than Rust's `#[serde(default)]`: the
/// latter protects `BotState`'s internal JSON-in-stable-memory
/// serialization (upgrades correctly fill in missing fields from an old
/// on-disk blob), but it does nothing for the WIRE format of an inbound
/// call. A record type used as a function ARGUMENT can only safely gain
/// `opt` fields — a caller built against the pre-Strategy-T interface
/// sends bytes with no `strategy_t_*` fields at all, and Candid's decoder
/// (unlike `#[serde(default)]`) rejects an incoming record that's missing
/// a field the target Rust type declares as required. Confirmed via
/// `didc check` against the pre-Strategy-T `.did`: `set_config`'s old
/// signature was not a safe subtype of the new one until these 14 fields
/// became `opt` at the boundary.
///
/// `BotConfig` itself (used for `get_config`'s RETURN type, and internally
/// everywhere else) is unaffected and stays fully required — a function's
/// RETURN type gaining fields is always a safe, backward-compatible
/// change, since an old caller simply doesn't read fields it doesn't know
/// about.
///
/// Every field other than the 14 `strategy_t_*` ones mirrors `BotConfig`
/// exactly (same name, same type, same required-ness) — an old caller's
/// payload for those is decoded and applied exactly as before.
///
/// The `#[serde(default = ...)]` attributes below are copied verbatim from
/// `BotConfig` for exactly that reason (identical decode behavior for the
/// non-`strategy_t_*` fields) — NOT because this struct participates in
/// the stable-memory upgrade path. It doesn't: `BotConfigInput` derives no
/// `Serialize` and is never persisted; it exists only as a transient
/// Candid-decode target for one inbound call, then it's consumed by
/// `into_full_config` and dropped.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct BotConfigInput {
    pub owner: Principal,
    pub rumi_amm: Principal,
    pub rumi_3pool: Principal,
    #[serde(default)]
    pub rumi_amm_paused: bool,
    pub icpswap_pool: Principal,
    pub icp_ledger: Principal,
    pub ckusdc_ledger: Principal,
    pub three_usd_ledger: Principal,
    pub min_spread_bps: u32,
    pub max_trade_size_usd: u64,
    pub paused: bool,
    pub icpswap_icp_is_token0: bool,
    #[serde(default)]
    pub admins: Vec<Principal>,
    #[serde(default = "default_principal")]
    pub icpswap_icusd_pool: Principal,
    #[serde(default = "default_principal")]
    pub icusd_ledger: Principal,
    #[serde(default)]
    pub icpswap_icusd_icp_is_token0: bool,
    #[serde(default)]
    pub min_profit_usd: i64,
    #[serde(default = "default_principal")]
    pub icpswap_ckusdt_pool: Principal,
    #[serde(default = "default_principal")]
    pub ckusdt_ledger: Principal,
    #[serde(default)]
    pub icpswap_ckusdt_icp_is_token0: bool,
    #[serde(default = "default_principal")]
    pub icpswap_3usd_pool: Principal,
    #[serde(default)]
    pub icpswap_3usd_icp_is_token0: bool,
    #[serde(default = "default_slippage_bps")]
    pub slippage_bps: u64,
    #[serde(default = "default_arb_interval_secs")]
    pub arb_interval_secs: u64,
    #[serde(default = "default_partydex_ckusdc_pool")]
    pub partydex_ckusdc_pool: Principal,
    #[serde(default = "default_partydex_ckusdt_pool")]
    pub partydex_ckusdt_pool: Principal,
    #[serde(default = "default_partydex_fee_pips")]
    pub partydex_ckusdc_fee_pips: u32,
    #[serde(default = "default_partydex_fee_pips")]
    pub partydex_ckusdt_fee_pips: u32,
    #[serde(default = "default_icp_inventory_floor")]
    pub icp_inventory_floor_e8s: u64,
    #[serde(default = "default_icp_inventory_ceiling")]
    pub icp_inventory_ceiling_e8s: u64,
    #[serde(default = "default_bob_inventory_floor")]
    pub bob_inventory_floor_e8s: u64,
    #[serde(default = "default_bob_inventory_ceiling")]
    pub bob_inventory_ceiling_e8s: u64,
    #[serde(default = "default_bob_ledger")]
    pub bob_ledger: Principal,
    #[serde(default = "default_bob_ledger_fee")]
    pub bob_ledger_fee: u64,
    #[serde(default = "default_icpswap_bob_icp_pool")]
    pub icpswap_bob_icp_pool: Principal,
    #[serde(default = "default_principal")]
    pub icpswap_icusd_bob_pool: Principal,
    #[serde(default)]
    pub icpswap_bob_icp_icp_is_token0: bool,
    #[serde(default)]
    pub icpswap_icusd_bob_icusd_is_token0: bool,
    #[serde(default = "default_bob_max_trade_size_usd")]
    pub bob_max_trade_size_usd: u64,
    #[serde(default = "default_bob_min_spread_bps")]
    pub bob_min_spread_bps: u64,
    #[serde(default)]
    pub bob_execution_enabled: bool,

    // ─── Strategy T: opt at the boundary — see the struct doc comment.
    // `None` means "caller didn't know about this field"; `into_full_config`
    // falls back to the caller-supplied `current` config for those, never
    // to the hardcoded inert default, so an old-style `set_config` call
    // can never silently reset Strategy T settings an admin already made
    // via the dedicated `set_strategy_t_*` setters. ───
    #[serde(default)]
    pub strategy_t_icusd_ckusdc_pool: Option<Principal>,
    #[serde(default)]
    pub strategy_t_icusd_ckusdt_pool: Option<Principal>,
    #[serde(default)]
    pub strategy_t_ckusdt_ckusdc_pool: Option<Principal>,
    #[serde(default)]
    pub strategy_t_enabled: Option<bool>,
    #[serde(default)]
    pub strategy_t_dry_run: Option<bool>,
    #[serde(default)]
    pub strategy_t_min_profit_usd: Option<i64>,
    #[serde(default)]
    pub strategy_t_min_profit_bps: Option<u32>,
    #[serde(default)]
    pub strategy_t_max_trade_size_usd: Option<u64>,
    #[serde(default)]
    pub strategy_t_icusd_floor: Option<u64>,
    #[serde(default)]
    pub strategy_t_icusd_ceiling: Option<u64>,
    #[serde(default)]
    pub strategy_t_ckusdt_floor: Option<u64>,
    #[serde(default)]
    pub strategy_t_ckusdt_ceiling: Option<u64>,
    #[serde(default)]
    pub strategy_t_ckusdc_floor: Option<u64>,
    #[serde(default)]
    pub strategy_t_ckusdc_ceiling: Option<u64>,
}

impl BotConfigInput {
    /// Builds a full `BotConfig`, falling back to `current`'s value for
    /// any Strategy T field this input omitted (`None`). `current` is the
    /// canister's existing config for `set_config` (preserve whatever's
    /// already there), or a fresh `BotState::default().config` for `init`
    /// (a genuinely new canister has no prior Strategy T state to
    /// preserve, so `None` correctly resolves to the same inert defaults
    /// `BotState::default()` already establishes).
    pub fn into_full_config(self, current: &BotConfig) -> BotConfig {
        BotConfig {
            owner: self.owner,
            rumi_amm: self.rumi_amm,
            rumi_3pool: self.rumi_3pool,
            rumi_amm_paused: self.rumi_amm_paused,
            icpswap_pool: self.icpswap_pool,
            icp_ledger: self.icp_ledger,
            ckusdc_ledger: self.ckusdc_ledger,
            three_usd_ledger: self.three_usd_ledger,
            min_spread_bps: self.min_spread_bps,
            max_trade_size_usd: self.max_trade_size_usd,
            paused: self.paused,
            icpswap_icp_is_token0: self.icpswap_icp_is_token0,
            admins: self.admins,
            icpswap_icusd_pool: self.icpswap_icusd_pool,
            icusd_ledger: self.icusd_ledger,
            icpswap_icusd_icp_is_token0: self.icpswap_icusd_icp_is_token0,
            min_profit_usd: self.min_profit_usd,
            icpswap_ckusdt_pool: self.icpswap_ckusdt_pool,
            ckusdt_ledger: self.ckusdt_ledger,
            icpswap_ckusdt_icp_is_token0: self.icpswap_ckusdt_icp_is_token0,
            icpswap_3usd_pool: self.icpswap_3usd_pool,
            icpswap_3usd_icp_is_token0: self.icpswap_3usd_icp_is_token0,
            slippage_bps: self.slippage_bps,
            arb_interval_secs: self.arb_interval_secs,
            partydex_ckusdc_pool: self.partydex_ckusdc_pool,
            partydex_ckusdt_pool: self.partydex_ckusdt_pool,
            partydex_ckusdc_fee_pips: self.partydex_ckusdc_fee_pips,
            partydex_ckusdt_fee_pips: self.partydex_ckusdt_fee_pips,
            icp_inventory_floor_e8s: self.icp_inventory_floor_e8s,
            icp_inventory_ceiling_e8s: self.icp_inventory_ceiling_e8s,
            bob_inventory_floor_e8s: self.bob_inventory_floor_e8s,
            bob_inventory_ceiling_e8s: self.bob_inventory_ceiling_e8s,
            bob_ledger: self.bob_ledger,
            bob_ledger_fee: self.bob_ledger_fee,
            icpswap_bob_icp_pool: self.icpswap_bob_icp_pool,
            icpswap_icusd_bob_pool: self.icpswap_icusd_bob_pool,
            icpswap_bob_icp_icp_is_token0: self.icpswap_bob_icp_icp_is_token0,
            icpswap_icusd_bob_icusd_is_token0: self.icpswap_icusd_bob_icusd_is_token0,
            bob_max_trade_size_usd: self.bob_max_trade_size_usd,
            bob_min_spread_bps: self.bob_min_spread_bps,
            bob_execution_enabled: self.bob_execution_enabled,
            strategy_t_icusd_ckusdc_pool: self.strategy_t_icusd_ckusdc_pool.unwrap_or(current.strategy_t_icusd_ckusdc_pool),
            strategy_t_icusd_ckusdt_pool: self.strategy_t_icusd_ckusdt_pool.unwrap_or(current.strategy_t_icusd_ckusdt_pool),
            strategy_t_ckusdt_ckusdc_pool: self.strategy_t_ckusdt_ckusdc_pool.unwrap_or(current.strategy_t_ckusdt_ckusdc_pool),
            strategy_t_enabled: self.strategy_t_enabled.unwrap_or(current.strategy_t_enabled),
            strategy_t_dry_run: self.strategy_t_dry_run.unwrap_or(current.strategy_t_dry_run),
            strategy_t_min_profit_usd: self.strategy_t_min_profit_usd.unwrap_or(current.strategy_t_min_profit_usd),
            strategy_t_min_profit_bps: self.strategy_t_min_profit_bps.unwrap_or(current.strategy_t_min_profit_bps),
            strategy_t_max_trade_size_usd: self.strategy_t_max_trade_size_usd.unwrap_or(current.strategy_t_max_trade_size_usd),
            strategy_t_icusd_floor: self.strategy_t_icusd_floor.unwrap_or(current.strategy_t_icusd_floor),
            strategy_t_icusd_ceiling: self.strategy_t_icusd_ceiling.unwrap_or(current.strategy_t_icusd_ceiling),
            strategy_t_ckusdt_floor: self.strategy_t_ckusdt_floor.unwrap_or(current.strategy_t_ckusdt_floor),
            strategy_t_ckusdt_ceiling: self.strategy_t_ckusdt_ceiling.unwrap_or(current.strategy_t_ckusdt_ceiling),
            strategy_t_ckusdc_floor: self.strategy_t_ckusdc_floor.unwrap_or(current.strategy_t_ckusdc_floor),
            strategy_t_ckusdc_ceiling: self.strategy_t_ckusdc_ceiling.unwrap_or(current.strategy_t_ckusdc_ceiling),
        }
    }
}

/// Which DEX venue an arb leg trades against. Internal to arb targets — not
/// part of BotConfig/CycleSnapshot, so it is not represented in arb_bot.did.
#[derive(CandidType, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Venue {
    Icpswap,
    PartyDex,
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
    /// Strategy K spread (PartyDEX ckUSDC vs ICPSwap ckUSDC/ICP), 0 if N/A. Populated in PR2b.
    #[serde(default)]
    pub spread_k_bps: i32,
    /// Strategy L spread (PartyDEX ckUSDC vs ICPSwap ckUSDT/ICP), 0 if N/A. Populated in PR2b.
    #[serde(default)]
    pub spread_l_bps: i32,
    /// Strategy M spread (PartyDEX ckUSDC vs ICPSwap icUSD/ICP), 0 if N/A. Populated in PR2b.
    #[serde(default)]
    pub spread_m_bps: i32,
    /// Strategy N spread (PartyDEX ckUSDT vs ICPSwap ckUSDC/ICP), 0 if N/A. Populated in PR2b.
    #[serde(default)]
    pub spread_n_bps: i32,
    /// Strategy O spread (PartyDEX ckUSDT vs ICPSwap ckUSDT/ICP), 0 if N/A. Populated in PR2b.
    #[serde(default)]
    pub spread_o_bps: i32,
    /// Strategy P spread (PartyDEX ckUSDT vs ICPSwap icUSD/ICP), 0 if N/A. Populated in PR2b.
    #[serde(default)]
    pub spread_p_bps: i32,
    /// Strategy Q spread (Rumi 3pool vs PartyDEX ckUSDC), 0 if N/A. Populated in PR2b.
    #[serde(default)]
    pub spread_q_bps: i32,
    /// Strategy R spread (Rumi 3pool vs PartyDEX ckUSDT), 0 if N/A. Populated in PR2b.
    #[serde(default)]
    pub spread_r_bps: i32,
    /// PartyDEX ckUSDC per 1 ICP (6 dec USD), 0 if N/A.
    #[serde(default)]
    pub partydex_icp_price_ckusdc: u64,
    /// PartyDEX ckUSDT per 1 ICP (6 dec USD), 0 if N/A.
    #[serde(default)]
    pub partydex_icp_price_ckusdt: u64,
    /// Strategy S: icUSD out per 1 BOB on the icUSD/BOB pool (8 dec), 0 if N/A.
    #[serde(default)]
    pub bob_pool_price_icusd_per_bob: u64,
    /// Strategy S: reference icUSD per 1 BOB — (ICP/BOB) × (USD/ICP) (8 dec), 0 if N/A.
    #[serde(default)]
    pub bob_ref_price_icusd_per_bob: u64,
    /// Strategy S spread (pool vs reference), 0 if N/A.
    #[serde(default)]
    pub spread_s_bps: i64,
    /// BOB balance (8 dec), 0 while Strategy S is inert.
    #[serde(default)]
    pub balance_bob: u64,
    // Trade activity
    pub traded: bool,
    pub strategy_used: String,           // "", "A", "B", "C", or "D"
}

/// Identifies a specific liquidity pool for drain routing.
#[derive(CandidType, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pool {
    RumiThreeUsd,
    IcpswapCkusdc,
    IcpswapIcusd,
    IcpswapCkusdt,
    IcpswapThreeUsd,
    /// PartyDEX ICP/ckUSDC pool. Label-only in PR2a — NOT wired into drain
    /// candidates (PartyDEX legs always settle ICP back to the main balance,
    /// so existing ICPSwap/Rumi drain already covers recovery).
    PartyDexIcpCkusdc,
    /// PartyDEX ICP/ckUSDT pool. Label-only in PR2a — see PartyDexIcpCkusdc.
    PartyDexIcpCkusdt,
}

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

// ─── Volume bot types ───

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq)]
pub enum VolumePool {
    IcusdIcp,
    ThreeUsdIcp,
    /// icUSD/BOB ICPSwap pool. Ships inert (enabled=false, pool anonymous)
    /// until an admin enables it and sets the pool — see `VolumePoolConfig::default()`.
    IcusdBob,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq)]
pub enum VolumeDirection {
    BuyIcp,
    SellIcp,
    /// icUSD/BOB pool: spend icUSD to buy BOB.
    BuyBob,
    /// icUSD/BOB pool: sell BOB for icUSD.
    SellBob,
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

/// Default state for the icUSD/BOB pool — same shape as `VolumePoolState::default()`
/// but starts on `BuyBob` (the icUSD/BOB analogue of `BuyIcp`) rather than `BuyIcp`,
/// since that pool's ping-pong never touches `BuyIcp`/`SellIcp`.
fn default_icusd_bob_state() -> VolumePoolState {
    VolumePoolState {
        next_direction: VolumeDirection::BuyBob,
        ..VolumePoolState::default()
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
    /// icUSD/BOB pool config. Ships inert — `VolumePoolConfig::default()` has
    /// `enabled: false`, and the pool principal on `BotConfig` defaults anonymous.
    #[serde(default)]
    pub icusd_bob: VolumePoolConfig,
    #[serde(default = "default_icusd_bob_state")]
    pub icusd_bob_state: VolumePoolState,
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
            icusd_bob: VolumePoolConfig::default(),
            icusd_bob_state: default_icusd_bob_state(),
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
    #[serde(default)]
    pub daily_cost_cap_usd_icusd_bob: u64,
    #[serde(default)]
    pub icusd_bob: VolumePoolStatus,
    pub total_trade_count: u64,
}

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, Default)]
pub struct VolumePoolStatus {
    pub config: VolumePoolConfig,
    pub state: VolumePoolState,
}

/// Live health snapshot for a single volume pool. `skip_reason` is the first
/// gate that would prevent the pool from trading in the next cycle, or None if
/// it would proceed. Populated by `get_bot_health` — mirrors the gate order in
/// `volume::run_volume_cycle`.
#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct PoolHealth {
    pub pool: VolumePool,
    pub enabled: bool,
    pub trade_size_usd: u64,
    pub daily_cost_usd: i64,
    pub daily_cost_cap_usd: u64,
    pub last_price: Option<u64>,
    pub current_price: Option<u64>,
    pub next_direction: VolumeDirection,
    pub input_balance: Option<u64>,
    pub min_required_native: Option<u64>,
    pub skip_reason: Option<String>,
}

/// Admin diagnostic: single call revealing every gate that could block the
/// arb drain or volume cycle. Returned by `get_bot_health`.
#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct BotHealthReport {
    pub arb_cycle_in_progress: bool,
    /// When the in-progress arb cycle acquired the lock (ns since epoch).
    /// None when idle. Lets the UI flag a cycle wedged far past the interval.
    #[serde(default)]
    pub arb_cycle_started_at_ns: Option<u64>,
    pub volume_cycle_in_progress: bool,
    pub volume_paused: bool,
    pub arb_paused: bool,
    pub volume_stranded_icp: u64,
    /// BOB stranded on the volume subaccount (8 dec) — see `volume_stranded_bob` state.
    #[serde(default)]
    pub volume_stranded_bob: u64,
    pub pending_exit: Option<PendingExit>,
    /// Strategy S: BOB acquired by a leg 1 whose leg 2 hasn't completed.
    #[serde(default)]
    pub pending_bob_exit: Option<PendingBobExit>,
    /// BOB balance (8 dec). 0 if the ledger query failed.
    #[serde(default)]
    pub balance_bob: u64,
    pub slippage_bps: u64,
    pub pools: Vec<PoolHealth>,
}

/// Anonymous-safe subset of `BotHealthReport`: bare stuck-state flags only —
/// no balances, no principals, no config. Returned by `get_public_health`.
#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct PublicHealth {
    pub has_pending_exit: bool,
    pub has_pending_bob_exit: bool,
    /// True if either stranded pot (ICP or BOB) is non-zero.
    pub has_stranded_volume_funds: bool,
    pub arb_paused: bool,
    pub volume_paused: bool,
}

/// Records the intended exit pool after a successful Leg 1, so the drain
/// can prefer it (and avoid draining back into the entry pool).
#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
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
    /// Strategy S: ICP inventory top-up bought from the best stable pool
    /// ahead of a reverse-direction (ICP→BOB→icUSD) trade. Appended after
    /// Drain — candid-append-safe, old logs decode unchanged.
    TopUp,
}

/// Which of the two Strategy S pools a stranded BOB balance entered through.
/// Internal to BotState (not part of any candid method signature yet).
#[derive(CandidType, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BobPool {
    /// ICPSwap icUSD/BOB pool.
    IcusdBob,
    /// ICPSwap BOB/ICP pool.
    BobIcp,
}

/// Records the pool Strategy S acquired BOB through after a successful
/// leg 1. Stage-1 retirement: this field's presence now triggers
/// `legacy_route_freeze_reason`'s freeze on BOB spending only (not ICP)
/// until an operator resolves the incident manually (see Section 11).
#[derive(CandidType, Clone, Debug, Serialize, Deserialize)]
pub struct PendingBobExit {
    pub entry_pool: BobPool,
    /// BOB received by leg 1 (8 dec).
    pub bob_amount: u64,
}

/// Which asset a Stage-1 legacy-incident freeze check is being asked
/// about. Covers exactly the two assets `pending_exit`/`pending_bob_exit`
/// can encumber.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyFreezeAsset {
    Icp,
    Bob,
}

/// Returns `Some(reason)` if `asset` is frozen by an unresolved legacy
/// `pending_exit`/`pending_bob_exit` incident, `None` if it's clear to
/// spend. Per the spec (Section 11): presence alone freezes the asset —
/// a zero or unknown amount is NOT treated as "no exposure." This checks
/// presence only; it does not attempt to prove the referenced funds are
/// in a structurally disjoint account (that proof, and the full durable
/// reservation ledger for an arbitrary future held position, is Stage 4
/// scope — there is no live execution yet to create one).
pub fn legacy_route_freeze_reason(state: &BotState, asset: LegacyFreezeAsset) -> Option<String> {
    match asset {
        LegacyFreezeAsset::Icp if state.pending_exit.is_some() => Some(
            "asset frozen: unresolved legacy pending_exit incident (Stage-1 retirement freeze, see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md Section 11)".to_string(),
        ),
        LegacyFreezeAsset::Bob
            if state.pending_bob_exit.is_some() || legacy_bob_freeze_marker() => Some(
            "asset frozen: unresolved legacy pending_bob_exit incident (durable whole-asset freeze; Stage-1 retirement freeze, see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md Section 11)".to_string(),
        ),
        _ => None,
    }
}

fn legacy_bob_freeze_marker() -> bool {
    LEGACY_BOB_ASSET_FROZEN.with(|cell| *cell.borrow().get())
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
    /// Versioned six-asset route policy. Serde default keeps upgrades from
    /// every pre-router schema inert and dry-run-only.
    #[serde(default)]
    pub route_arb: crate::route_arb::RouteArbConfigV1,
    #[serde(default)]
    pub route_arb_config_generation: u64,
    #[serde(default)]
    pub route_observation: Option<crate::route_arb::ObservationAccumulatorV1>,
    #[serde(default)]
    pub token_ordering_resolved: bool,
    #[serde(default)]
    pub icusd_token_ordering_resolved: bool,
    #[serde(default)]
    pub ckusdt_token_ordering_resolved: bool,
    #[serde(default)]
    pub icpswap_3usd_token_ordering_resolved: bool,
    /// Strategy S: BOB/ICP pool token-ordering resolved once (mirrors the
    /// `*_token_ordering_resolved` pattern above).
    #[serde(default)]
    pub bob_icp_ordering_resolved: bool,
    /// Strategy S: icUSD/BOB pool token-ordering resolved once.
    #[serde(default)]
    pub icusd_bob_ordering_resolved: bool,
    #[serde(default)]
    pub pending_exit: Option<PendingExit>,
    /// Strategy S: BOB acquired by leg 1 whose leg 2 has not completed.
    /// Stage-1 retirement: presence of this field now freezes BOB spending
    /// only (not ICP) via `legacy_route_freeze_reason` until an operator
    /// resolves the incident manually.
    #[serde(default)]
    pub pending_bob_exit: Option<PendingBobExit>,
    #[serde(default)]
    pub volume: VolumeConfig,
    /// ICP amount stranded in the default account after a volume bot
    /// transfer-to-subaccount failure.  The arb drain must not touch this.
    #[serde(default)]
    pub volume_stranded_icp: u64,
    /// BOB amount stranded in the default account after a volume bot
    /// icUSD/BOB transfer-to-subaccount failure (BuyBob leg only — SellBob
    /// receives icUSD, which never mixed with this balance). Mirrors
    /// `volume_stranded_icp`; the automatic *arb drain* that once had to
    /// specifically avoid sweeping this was deleted under Stage-1
    /// retirement, so that particular invariant (arb-drain-must-not-touch-
    /// this-balance) is now moot. This does NOT mean the balance is
    /// untouched: `run_volume_cycle` (still live via `trigger_volume_cycle`)
    /// reads, transfers, and zeroes this field as part of its stranded-BOB
    /// sweep.
    #[serde(default)]
    pub volume_stranded_bob: u64,
}

impl Default for BotState {
    fn default() -> Self {
        Self {
            config: BotConfig {
                owner: Principal::anonymous(),
                rumi_amm: Principal::anonymous(),
                rumi_3pool: Principal::anonymous(),
                rumi_amm_paused: false,
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
                slippage_bps: 50,
                arb_interval_secs: 600,
                partydex_ckusdc_pool: default_partydex_ckusdc_pool(),
                partydex_ckusdt_pool: default_partydex_ckusdt_pool(),
                partydex_ckusdc_fee_pips: default_partydex_fee_pips(),
                partydex_ckusdt_fee_pips: default_partydex_fee_pips(),
                icp_inventory_floor_e8s: default_icp_inventory_floor(),
                icp_inventory_ceiling_e8s: default_icp_inventory_ceiling(),
                bob_inventory_floor_e8s: default_bob_inventory_floor(),
                bob_inventory_ceiling_e8s: default_bob_inventory_ceiling(),
                bob_ledger: default_bob_ledger(),
                bob_ledger_fee: default_bob_ledger_fee(),
                icpswap_bob_icp_pool: default_icpswap_bob_icp_pool(),
                icpswap_icusd_bob_pool: Principal::anonymous(),
                icpswap_bob_icp_icp_is_token0: false,
                icpswap_icusd_bob_icusd_is_token0: false,
                bob_max_trade_size_usd: default_bob_max_trade_size_usd(),
                bob_min_spread_bps: default_bob_min_spread_bps(),
                bob_execution_enabled: false,
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
            },
            route_arb: crate::route_arb::RouteArbConfigV1::default(),
            route_arb_config_generation: 0,
            route_observation: None,
            token_ordering_resolved: false,
            icusd_token_ordering_resolved: false,
            ckusdt_token_ordering_resolved: false,
            icpswap_3usd_token_ordering_resolved: false,
            bob_icp_ordering_resolved: false,
            icusd_bob_ordering_resolved: false,
            pending_exit: None,
            pending_bob_exit: None,
            volume: VolumeConfig::default(),
            volume_stranded_icp: 0,
            volume_stranded_bob: 0,
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
    #[serde(default)]
    pending_bob_exit: Option<PendingBobExit>,
}

#[doc(hidden)]
pub fn legacy_pending_bob_survives_decode_for_test(bytes: &[u8]) -> bool {
    serde_json::from_slice::<LegacyBotState>(bytes)
        .ok()
        .and_then(|state| state.pending_bob_exit)
        .is_some()
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
json_storable!(crate::route_arb::ObservationAccumulatorV1);
json_storable!(crate::route_arb::MutationLockSlotV1);
json_storable!(crate::route_arb::OwnershipReservationV1);
json_storable!(crate::route_arb::HeldPositionV1);
json_storable!(crate::route_arb::ExecutionSlotV1);
json_storable!(crate::route_arb::ExecutionRecordV1);
json_storable!(crate::route_arb::RouteExecutionDetailV1);

// ─── Stable memory layout ───
//
// MemoryId 0:       META_CELL (StableCell<Vec<u8>>) — JSON-encoded BotState
// MemoryId 1,2:     TRADES log (index + data)
// MemoryId 3,4:     ERRORS log
// MemoryId 5,6:     ACTIVITY log
// MemoryId 7,8:     TRADE_LEGS log
// MemoryId 9,10:    SNAPSHOTS log
// MemoryId 11,12:   VOLUME_TRADES log
// MemoryId 13,14:   ROUTE_OBSERVATIONS log
// MemoryId 15:      ROUTE_MUTATION_LOCK cell
// MemoryId 16,17:   OWNERSHIP_RESERVATIONS event log
// MemoryId 18,19:   HELD_POSITIONS log
// MemoryId 20:      CURRENT_ROUTE_EXECUTION cell
// MemoryId 21,22:   TERMINAL_ROUTE_EXECUTIONS log
// MemoryId 23:      OWNERSHIP_RESERVATION_INDEX (bounded current-state map)
// MemoryId 24:      OWNERSHIP_RESERVATION_MIGRATED marker
// MemoryId 25:      LEGACY_BOB_ASSET_FROZEN marker
// MemoryId 27:      ROUTE_EXECUTION_DETAILS (bounded detail index)
//
// NEVER reuse or reorder these IDs — doing so corrupts existing data.

thread_local! {
    // MemoryId 26 is the versioned durable executor, independent of heap snapshots.
    static ROUTE_RUNTIME: RefCell<StableCell<Vec<u8>, Memory>> = RefCell::new(
        StableCell::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(26))), Vec::new())
            .expect("initialize route runtime")
    );
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

    static ROUTE_OBSERVATIONS: RefCell<StableLog<crate::route_arb::ObservationAccumulatorV1, Memory, Memory>> = RefCell::new(
        StableLog::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(13))),
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(14))),
        ).expect("init ROUTE_OBSERVATIONS"),
    );

    static ROUTE_MUTATION_LOCK: RefCell<StableCell<crate::route_arb::MutationLockSlotV1, Memory>> = RefCell::new(
        StableCell::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(15))),
            crate::route_arb::MutationLockSlotV1::default(),
        ).expect("init ROUTE_MUTATION_LOCK"),
    );

    static OWNERSHIP_RESERVATIONS: RefCell<StableLog<crate::route_arb::OwnershipReservationV1, Memory, Memory>> = RefCell::new(
        StableLog::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(16))),
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(17))),
        ).expect("init OWNERSHIP_RESERVATIONS"),
    );

    static HELD_POSITIONS: RefCell<StableLog<crate::route_arb::HeldPositionV1, Memory, Memory>> = RefCell::new(
        StableLog::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(18))),
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(19))),
        ).expect("init HELD_POSITIONS"),
    );

    static CURRENT_ROUTE_EXECUTION: RefCell<StableCell<crate::route_arb::ExecutionSlotV1, Memory>> = RefCell::new(
        StableCell::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(20))),
            crate::route_arb::ExecutionSlotV1::default(),
        ).expect("init CURRENT_ROUTE_EXECUTION"),
    );

    static TERMINAL_ROUTE_EXECUTIONS: RefCell<StableLog<crate::route_arb::ExecutionRecordV1, Memory, Memory>> = RefCell::new(
        StableLog::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(21))),
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(22))),
        ).expect("init TERMINAL_ROUTE_EXECUTIONS"),
    );

    // Current-state index for ownership reservations. Unlike the historical
    // event log previously used here, replacing a reservation updates the
    // existing key in place and releasing one removes it. This keeps durable
    // growth bounded by the 256-open-reservation ceiling.
    static OWNERSHIP_RESERVATION_INDEX: RefCell<StableBTreeMap<String, crate::route_arb::OwnershipReservationV1, Memory>> = RefCell::new(
        StableBTreeMap::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(23))),
        ),
    );

    // Migration marker for the old append-only reservation event log. The old
    // log remains read-only for compatibility, while all future writes use
    // OWNERSHIP_RESERVATION_INDEX.
    static OWNERSHIP_RESERVATION_MIGRATED: RefCell<StableCell<bool, Memory>> = RefCell::new(
        StableCell::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(24))),
            false,
        ).expect("init OWNERSHIP_RESERVATION_MIGRATED"),
    );

    // BOB is a retired legacy asset and is not represented by the active
    // six-asset enum. Keep its whole-asset incident freeze in its own durable
    // cell so decoding/clearing the legacy pending field cannot release it.
    static LEGACY_BOB_ASSET_FROZEN: RefCell<StableCell<bool, Memory>> = RefCell::new(
        StableCell::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(25))),
            false,
        ).expect("init LEGACY_BOB_ASSET_FROZEN"),
    );

    // Per-leg route execution detail is additive to the pre-detail layout.
    static ROUTE_EXECUTION_DETAILS: RefCell<StableBTreeMap<String, crate::route_arb::RouteExecutionDetailV1, Memory>> =
        RefCell::new(StableBTreeMap::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(27))),
        ));

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

pub fn append_route_observation(
    observation: crate::route_arb::ObservationAccumulatorV1,
) -> Result<(), String> {
    if !observation.scan_complete || observation.completed_at_ns.is_none() {
        return Err("only completed observations may enter terminal history".to_string());
    }
    ROUTE_OBSERVATIONS.with(|log| {
        let log = log.borrow_mut();
        if log.len() >= 10_000 {
            return Err("route observation history reached its 10,000-record cap".to_string());
        }
        log.append(&observation)
            .map(|_| ())
            .map_err(|error| format!("failed to append route observation: {error:?}"))
    })
}

pub fn get_route_observations_page(
    offset: u64,
    limit: u64,
) -> Result<Vec<crate::route_arb::ObservationAccumulatorV1>, String> {
    if limit == 0 || limit > u64::from(crate::route_arb::HARD_MAX_PAGE_SIZE) {
        return Err("limit must be between 1 and 100".to_string());
    }
    ROUTE_OBSERVATIONS.with(|log| {
        let log = log.borrow();
        let total = log.len();
        if offset >= total {
            return Ok(Vec::new());
        }
        let end = total.saturating_sub(offset);
        let start = end.saturating_sub(limit);
        Ok((start..end).filter_map(|index| log.get(index)).collect())
    })
}

fn validate_durable_text(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 {
        return Err(format!("{label} must contain 1..=256 bytes"));
    }
    Ok(())
}

pub fn get_mutation_lock() -> Option<crate::route_arb::MutationLockV1> {
    ROUTE_MUTATION_LOCK.with(|cell| cell.borrow().get().lock.clone())
}

pub fn acquire_mutation_lock(
    operation_id: &str,
    owner: crate::route_arb::MutationOwnerV1,
    acquired_at_ns: u64,
) -> Result<crate::route_arb::MutationLockV1, String> {
    validate_durable_text("operation_id", operation_id)?;
    ROUTE_MUTATION_LOCK.with(|cell| {
        let mut cell = cell.borrow_mut();
        if let Some(existing) = &cell.get().lock {
            return Err(format!(
                "account mutation lock held by {} ({:?})",
                existing.operation_id, existing.owner
            ));
        }
        let lock = crate::route_arb::MutationLockV1 {
            operation_id: operation_id.to_string(), owner, acquired_at_ns,
            reconciliation_required: false,
        };
        cell.set(crate::route_arb::MutationLockSlotV1 { lock: Some(lock.clone()) })
            .map_err(|error| format!("failed to persist mutation lock: {error:?}"))?;
        Ok(lock)
    })
}

pub fn mark_mutation_lock_reconciliation_required(operation_id: &str) -> Result<(), String> {
    ROUTE_MUTATION_LOCK.with(|cell| {
        let mut cell = cell.borrow_mut();
        let mut slot = cell.get().clone();
        let lock = slot.lock.as_mut().ok_or("account mutation lock is not held")?;
        if lock.operation_id != operation_id {
            return Err("only the lock owner may retain reconciliation ownership".to_string());
        }
        lock.reconciliation_required = true;
        cell.set(slot).map_err(|error| format!("failed to persist mutation lock: {error:?}"))?;
        Ok(())
    })
}

pub fn release_mutation_lock(operation_id: &str) -> Result<(), String> {
    ROUTE_MUTATION_LOCK.with(|cell| {
        let mut cell = cell.borrow_mut();
        let slot = cell.get();
        let lock = slot.lock.as_ref().ok_or("account mutation lock is not held")?;
        if lock.operation_id != operation_id {
            return Err("only the lock owner may release the account mutation lock".to_string());
        }
        if lock.reconciliation_required {
            return Err("reconciliation-required ownership cannot be administratively released".to_string());
        }
        cell.set(crate::route_arb::MutationLockSlotV1::default())
            .map_err(|error| format!("failed to persist mutation lock release: {error:?}"))?;
        Ok(())
    })
}

#[doc(hidden)]
pub fn release_mutation_lock_for_test() {
    ROUTE_MUTATION_LOCK.with(|cell| {
        let _ = cell.borrow_mut().set(crate::route_arb::MutationLockSlotV1::default());
    });
}

fn migrate_legacy_reservation_log_once() {
    let migrated = OWNERSHIP_RESERVATION_MIGRATED.with(|cell| *cell.borrow().get());
    if migrated {
        return;
    }

    // Preserve the latest row for each ID from the old append-only log, but
    // never overwrite a row already present in the indexed store. The latter
    // makes a retry after an interrupted migration safe: a reservation
    // written by the new code cannot be rolled back to an older log value.
    let legacy_rows: std::collections::BTreeMap<_, _> = OWNERSHIP_RESERVATIONS.with(|log| {
        let log = log.borrow();
        (0..log.len())
            .filter_map(|index| log.get(index))
            .map(|row| (row.reservation_id.clone(), row))
            .collect()
    });
    OWNERSHIP_RESERVATION_INDEX.with(|index| {
        let mut index = index.borrow_mut();
        for (reservation_id, row) in legacy_rows {
            if index.get(&reservation_id).is_none() {
                index.insert(reservation_id, row);
            }
        }
    });
    let _ = OWNERSHIP_RESERVATION_MIGRATED.with(|cell| cell.borrow_mut().set(true));
}

fn latest_reservations() -> std::collections::BTreeMap<String, crate::route_arb::OwnershipReservationV1> {
    migrate_legacy_reservation_log_once();
    OWNERSHIP_RESERVATION_INDEX.with(|index| {
        index
            .borrow()
            .iter()
            .map(|entry| entry)
            .collect()
    })
}

pub fn put_ownership_reservation(
    reservation: crate::route_arb::OwnershipReservationV1,
) -> Result<(), String> {
    validate_durable_text("reservation_id", &reservation.reservation_id)?;
    validate_durable_text("operation_id", &reservation.operation_id)?;
    let current = latest_reservations();
    if reservation.active
        && !current.contains_key(&reservation.reservation_id)
        && current.values().filter(|row| row.active).count() >= 256
    {
        return Err("open ownership-reservation cap reached".to_string());
    }
    OWNERSHIP_RESERVATION_INDEX.with(|index| {
        let mut index = index.borrow_mut();
        if reservation.active {
            index.insert(reservation.reservation_id.clone(), reservation);
        } else {
            index.remove(&reservation.reservation_id);
        }
        Ok(())
    })
}

pub fn get_ownership_reservations_page(
    offset: u64,
    limit: u64,
) -> Result<Vec<crate::route_arb::OwnershipReservationV1>, String> {
    if limit == 0 || limit > 100 {
        return Err("limit must be between 1 and 100".to_string());
    }
    let rows: Vec<_> = latest_reservations().into_values().filter(|row| row.active).collect();
    let start = usize::try_from(offset).unwrap_or(usize::MAX).min(rows.len());
    let end = start.saturating_add(limit as usize).min(rows.len());
    Ok(rows[start..end].to_vec())
}

pub fn reservation_totals_for_asset(asset: crate::route_arb::Asset) -> crate::route_arb::AssetReservationStatusV1 {
    let mut result = crate::route_arb::AssetReservationStatusV1 { asset: Some(asset), ..Default::default() };
    for reservation in latest_reservations().into_values().filter(|row| row.active && row.asset == asset) {
        result.whole_asset_frozen |= reservation.whole_asset_freeze;
        let target = match reservation.kind {
            crate::route_arb::ReservationKindV1::HeldPosition => &mut result.held,
            crate::route_arb::ReservationKindV1::ActiveRoute => &mut result.active_route,
            crate::route_arb::ReservationKindV1::NonRoute | crate::route_arb::ReservationKindV1::LegacyFreeze => &mut result.non_route,
        };
        *target = target.saturating_add(reservation.amount_native);
    }
    result
}

pub fn spendable_native(asset: crate::route_arb::Asset, ledger_balance: u64) -> Result<u64, String> {
    let totals = reservation_totals_for_asset(asset);
    if totals.whole_asset_frozen {
        return Err(format!("{:?} is frozen by an unresolved ownership reservation", asset));
    }
    ledger_balance
        .checked_sub(totals.held)
        .and_then(|value| value.checked_sub(totals.active_route))
        .and_then(|value| value.checked_sub(totals.non_route))
        .ok_or_else(|| format!("{:?} reservations exceed ledger balance", asset))
}

fn ensure_legacy_route_reservations() {
    let pending_icp = STATE.with(|state| {
        state.borrow().as_ref().is_some_and(|state| state.pending_exit.is_some())
    });
    let pending_bob = STATE.with(|state| {
        state.borrow().as_ref().is_some_and(|state| state.pending_bob_exit.is_some())
    });
    if pending_bob {
        // BOB is intentionally outside the active six-asset enum, so its
        // migration is represented by a durable whole-asset marker rather
        // than by a route reservation row.
        let _ = LEGACY_BOB_ASSET_FROZEN.with(|cell| cell.borrow_mut().set(true));
    }
    if pending_icp
        && !latest_reservations().contains_key("legacy-pending-exit-icp-freeze")
    {
        let _ = put_ownership_reservation(crate::route_arb::OwnershipReservationV1 {
            reservation_id: "legacy-pending-exit-icp-freeze".to_string(),
            asset: crate::route_arb::Asset::Icp,
            amount_native: 0,
            whole_asset_freeze: true,
            kind: crate::route_arb::ReservationKindV1::LegacyFreeze,
            owner: crate::route_arb::MutationOwnerV1::LegacyMigration,
            operation_id: "legacy-pending-exit".to_string(),
            reconciled_at_ns: 0,
            active: true,
        });
    }
}

pub fn put_held_position(position: crate::route_arb::HeldPositionV1) -> Result<(), String> {
    validate_durable_text("position_id", &position.position_id)?;
    validate_durable_text("originating_execution_id", &position.originating_execution_id)?;
    validate_durable_text("originating_route_id", &position.originating_route_id)?;
    if position.reason.len() > 512 || position.lots.is_empty() || position.lots.len() > 6 {
        return Err("held position must have 1..=6 lots and a <=512-byte reason".to_string());
    }
    let mut existing = Vec::new();
    for offset in [0,100,200] { existing.extend(get_held_positions_page(offset,100)?); }
    if let Some(row) = existing.iter().find(|row| row.position_id == position.position_id) {
        if row.originating_execution_id == position.originating_execution_id
            && row.originating_route_id == position.originating_route_id
            && row.basis == position.basis && row.lots == position.lots {
            return Ok(()); // Idempotent retry after reservations and held log persisted.
        }
        return Err("held position id already exists with different inventory".to_string());
    }
    if HELD_POSITIONS.with(|log| log.borrow().len()) >= 256 {
        return Err("held-position cap reached".to_string());
    }
    for lot in &position.lots {
        if lot.reserved_native != lot.native_amount {
            return Err("held lot must reserve its exact native amount".to_string());
        }
        put_ownership_reservation(crate::route_arb::OwnershipReservationV1 {
            reservation_id: format!("held:{}:{:?}", position.position_id, lot.asset),
            asset: lot.asset,
            amount_native: lot.reserved_native,
            whole_asset_freeze: false,
            kind: crate::route_arb::ReservationKindV1::HeldPosition,
            owner: crate::route_arb::MutationOwnerV1::RouteExecution,
            operation_id: position.originating_execution_id.clone(),
            reconciled_at_ns: position.last_reconciled_timestamp_ns,
            active: true,
        })?;
    }
    HELD_POSITIONS.with(|log| {
        log.borrow_mut().append(&position)
            .map(|_| ())
            .map_err(|error| format!("failed to persist held position: {error:?}"))
    })
}

pub fn get_held_positions_page(offset: u64, limit: u64) -> Result<Vec<crate::route_arb::HeldPositionV1>, String> {
    if limit == 0 || limit > 100 {
        return Err("limit must be between 1 and 100".to_string());
    }
    HELD_POSITIONS.with(|log| {
        let log = log.borrow();
        let total = log.len();
        if offset >= total { return Ok(Vec::new()); }
        let end = total.saturating_sub(offset);
        let start = end.saturating_sub(limit);
        Ok((start..end).filter_map(|index| log.get(index)).collect())
    })
}

fn validate_execution_record(record: &crate::route_arb::ExecutionRecordV1) -> Result<(), String> {
    for (label, value) in [
        ("execution_id", record.execution_id.as_str()),
        ("route_id", record.route_id.as_str()),
    ] {
        validate_durable_text(label, value)?;
    }
    if record.evidence.len() > 64 {
        return Err("execution evidence exceeds 64-item cap".to_string());
    }
    for evidence in &record.evidence {
        validate_durable_text("evidence_kind", &evidence.evidence_kind)?;
        if evidence.source_reference.len() > 16_384 { return Err("source evidence exceeds 16KiB".into()); }
    }
    let encoded = serde_json::to_vec(record).map_err(|error| format!("execution encoding failed: {error}"))?;
    if encoded.len() > 65_536 {
        return Err("execution record exceeds 65,536-byte cap".to_string());
    }
    Ok(())
}

pub fn validate_route_execution_detail(
    detail: &crate::route_arb::RouteExecutionDetailV1,
) -> Result<(), String> {
    validate_execution_record(&detail.record)?;
    if detail.legs.is_empty() || detail.legs.len() > 6 {
        return Err("route execution detail must contain 1..=6 legs".into());
    }
    if detail.asset_path.len() != detail.legs.len() + 1 {
        return Err("route execution detail asset path does not match leg count".into());
    }
    for (expected, leg) in detail.legs.iter().enumerate() {
        if usize::from(leg.leg_index) != expected {
            return Err("route execution detail leg indices must be ascending from zero".into());
        }
        for (label, value) in [
            ("edge_id", leg.edge_id.as_str()),
            ("pool_id", leg.pool_id.as_str()),
        ] {
            validate_durable_text(label, value)?;
        }
        if let Some(incident) = &leg.incident {
            if incident.len() > 16_384 {
                return Err("route execution incident exceeds 16KiB".into());
            }
        }
        for evidence in &leg.evidence {
            validate_durable_text("evidence_kind", &evidence.evidence_kind)?;
            if evidence.source_reference.len() > 16_384 {
                return Err("source evidence exceeds 16KiB".into());
            }
        }
    }
    let evidence_count = detail.record.evidence.len()
        + detail.legs.iter().map(|leg| leg.evidence.len()).sum::<usize>();
    if evidence_count > 64 {
        return Err("route execution detail evidence exceeds 64-item cap".into());
    }
    let encoded = serde_json::to_vec(detail)
        .map_err(|error| format!("route execution detail encoding failed: {error}"))?;
    if encoded.len() > 65_536 {
        return Err("route execution detail exceeds 65,536-byte cap".into());
    }
    Ok(())
}

pub fn put_route_execution_detail(
    detail: crate::route_arb::RouteExecutionDetailV1,
) -> Result<(), String> {
    validate_route_execution_detail(&detail)?;
    ROUTE_EXECUTION_DETAILS.with(|map| {
        let mut map = map.borrow_mut();
        if let Some(previous) = map.get(&detail.record.execution_id) {
            if previous.record.phase.is_terminal() {
                if previous == detail {
                    return Ok(());
                }
                return Err("terminal route execution detail changed on retry".into());
            }
        }
        map.insert(detail.record.execution_id.clone(), detail);
        Ok(())
    })
}

pub fn get_route_execution_detail(
    execution_id: &str,
) -> Result<Option<crate::route_arb::RouteExecutionDetailV1>, String> {
    validate_durable_text("execution_id", execution_id)?;
    ROUTE_EXECUTION_DETAILS.with(|map| Ok(map.borrow().get(&execution_id.to_string())))
}

pub fn find_route_execution_record(
    execution_id: &str,
) -> Result<Option<crate::route_arb::ExecutionRecordV1>, String> {
    validate_durable_text("execution_id", execution_id)?;
    if let Some(record) = get_current_route_execution() {
        if record.execution_id == execution_id {
            return Ok(Some(record));
        }
    }
    TERMINAL_ROUTE_EXECUTIONS.with(|log| {
        let log = log.borrow();
        for index in (0..log.len()).rev() {
            if let Some(record) = log.get(index) {
                if record.execution_id == execution_id {
                    return Ok(Some(record));
                }
            }
        }
        Ok(None)
    })
}

pub fn get_current_route_execution() -> Option<crate::route_arb::ExecutionRecordV1> {
    CURRENT_ROUTE_EXECUTION.with(|cell| cell.borrow().get().execution.clone())
}

pub fn put_current_route_execution(record: crate::route_arb::ExecutionRecordV1) -> Result<(), String> {
    validate_execution_record(&record)?;
    CURRENT_ROUTE_EXECUTION.with(|cell| {
        cell.borrow_mut().set(crate::route_arb::ExecutionSlotV1 { execution: Some(record) })
            .map(|_| ())
            .map_err(|error| format!("failed to persist route execution: {error:?}"))
    })
}

pub fn complete_current_route_execution(record: crate::route_arb::ExecutionRecordV1) -> Result<(), String> {
    validate_execution_record(&record)?;
    if !record.phase.is_terminal() {
        return Err("only terminal route executions may enter history".to_string());
    }
    TERMINAL_ROUTE_EXECUTIONS.with(|log| {
        let mut log = log.borrow_mut();
        if let Some(previous) = log.len().checked_sub(1).and_then(|i|log.get(i)) {
            if previous.execution_id == record.execution_id {
                if previous.phase != record.phase { return Err("terminal phase changed on retry".into()); }
                return Ok(());
            }
        }
        if log.len() >= 10_000 {
            return Err("terminal execution history reached its 10,000-record cap".to_string());
        }
        log.append(&record)
            .map_err(|error| format!("failed to append terminal execution: {error:?}"))?;
        Ok(())
    })?;
    CURRENT_ROUTE_EXECUTION.with(|cell| {
        cell.borrow_mut().set(crate::route_arb::ExecutionSlotV1::default())
            .map(|_| ())
            .map_err(|error| format!("failed to clear current execution: {error:?}"))
    })
}

pub fn get_terminal_route_executions_page(
    offset: u64,
    limit: u64,
) -> Result<Vec<crate::route_arb::ExecutionRecordV1>, String> {
    if limit == 0 || limit > 100 {
        return Err("limit must be between 1 and 100".to_string());
    }
    TERMINAL_ROUTE_EXECUTIONS.with(|log| {
        let log = log.borrow();
        let total = log.len();
        if offset >= total { return Ok(Vec::new()); }
        let end = total.saturating_sub(offset);
        let start = end.saturating_sub(limit);
        Ok((start..end).filter_map(|index| log.get(index)).collect())
    })
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
        let legacy_bob_frozen = legacy.pending_bob_exit.is_some();
        let new_state = BotState {
            config: legacy.config,
            route_arb: crate::route_arb::RouteArbConfigV1::default(),
            route_arb_config_generation: 0,
            route_observation: None,
            token_ordering_resolved: legacy.token_ordering_resolved,
            icusd_token_ordering_resolved: legacy.icusd_token_ordering_resolved,
            ckusdt_token_ordering_resolved: legacy.ckusdt_token_ordering_resolved,
            icpswap_3usd_token_ordering_resolved: legacy.icpswap_3usd_token_ordering_resolved,
            bob_icp_ordering_resolved: false,
            icusd_bob_ordering_resolved: false,
            pending_exit: legacy.pending_exit,
            pending_bob_exit: legacy.pending_bob_exit,
            volume: VolumeConfig::default(),
            volume_stranded_icp: 0,
            volume_stranded_bob: 0,
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
        if legacy_bob_frozen {
            let _ = LEGACY_BOB_ASSET_FROZEN.with(|cell| cell.borrow_mut().set(true));
        }
        ensure_legacy_route_reservations();

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
    ensure_legacy_route_reservations();
}

/// The runtime writes complete typed state before any outbound mutation.
pub(crate) fn runtime_bytes() -> Vec<u8> {
    ROUTE_RUNTIME.with(|cell| cell.borrow().get().clone())
}
pub(crate) fn set_runtime_bytes(bytes: Vec<u8>) -> Result<(), String> {
    if bytes.len() > 262_144 { return Err("runtime stable capacity exceeded".into()); }
    ROUTE_RUNTIME.with(|cell| cell.borrow_mut().set(bytes)
        .map(|_| ()).map_err(|e| format!("runtime persistence failed: {e:?}")))
}
pub(crate) fn admit_route_capacity(config: &crate::route_arb::RouteArbConfigV1) -> Result<(), String> {
    if HELD_POSITIONS.with(|log| log.borrow().len()) >= u64::from(config.max_open_held_positions) {
        return Err("held-position capacity exhausted before submission".into());
    }
    if TERMINAL_ROUTE_EXECUTIONS.with(|log| log.borrow().len()) >= u64::from(config.max_terminal_execution_records) {
        return Err("terminal history capacity exhausted before submission".into());
    }
    // Reserve headroom for all six assets before a route starts.
    if latest_reservations().len() + 6 > 256 {
        return Err("reservation capacity exhausted before submission".into());
    }
    Ok(())
}
/// Only the source-bound reconciler calls this after all ambiguity is resolved.
pub(crate) fn release_reconciled_route_lock(operation_id: &str) -> Result<(), String> {
    ROUTE_MUTATION_LOCK.with(|cell| {
        let mut cell = cell.borrow_mut();
        let lock = cell.get().lock.as_ref().ok_or("mutation lock missing")?;
        if lock.operation_id != operation_id || lock.owner != crate::route_arb::MutationOwnerV1::RouteExecution {
            return Err("route does not own mutation lock".into());
        }
        cell.set(crate::route_arb::MutationLockSlotV1::default())
            .map(|_| ()).map_err(|e| format!("release reconciled route lock: {e:?}"))
    })
}

#[cfg(test)]
pub(crate) fn reopen_runtime_cell_for_test() {
    ROUTE_RUNTIME.with(|cell| {
        *cell.borrow_mut() = StableCell::init(MEMORY_MANAGER.with(|m|m.borrow().get(MemoryId::new(26))),Vec::new()).unwrap();
    });
}
