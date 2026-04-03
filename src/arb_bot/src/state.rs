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
    /// Whether ICP is token0 in the ICPSwap pool (resolved from pool metadata at init)
    pub icpswap_icp_is_token0: bool,
    /// Additional admin principals (e.g. Internet Identity) that can call admin methods
    #[serde(default)]
    pub admins: Vec<Principal>,
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

#[derive(Serialize, Deserialize)]
pub struct BotState {
    pub config: BotConfig,
    pub trades: Vec<TradeRecord>,
    pub errors: Vec<ErrorRecord>,
    #[serde(default)]
    pub activity_log: Vec<ActivityRecord>,
    #[serde(default)]
    pub token_ordering_resolved: bool,
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
            },
            trades: Vec::new(),
            errors: Vec::new(),
            activity_log: Vec::new(),
            token_ordering_resolved: false,
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

pub fn log_activity(category: &str, message: &str) {
    mutate_state(|s| {
        s.activity_log.push(ActivityRecord {
            timestamp: ic_cdk::api::time(),
            category: category.to_string(),
            message: message.to_string(),
        });
    });
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
