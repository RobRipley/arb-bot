//! Adapter for the PartyDEX pool canister (ICP/ckUSDC and ICP/ckUSDT pools).
//!
//! On PartyDEX, ICP is ALWAYS `base` and the stable token is ALWAYS `quote` —
//! there is no token0/token1 resolution to do (unlike ICPSwap). `buy` = spend
//! quote (stable) to receive base (ICP); `sell` = spend base (ICP) to receive
//! quote (stable).
//!
//! Candid footguns (see scratchpad/partydex-interface.md): amounts/reserves/
//! fees are `nat` -> `candid::Nat`; the trade `fee` field and a few others are
//! signed `nat` -> `candid::Int`; `Tick` is `int32` -> `i32` (can be negative);
//! `fee_pips`/`slippage_bps` are `nat32` -> `u32`; every trade result's `err`
//! arm is the `ApiError` record, never a bare string; `get_user` returns a
//! plain `opt record { ... }` (no ok/err wrapper).
//!
//! Custody flow (per orchestrator decision #1): approve (done once up front
//! via `swaps::approve_infinite`, wired in `setup_approvals`) -> deposit
//! (token_in) -> quote_trade (re-quote at exec time) -> atomic_trade
//! (allow_partial=false, min_output=Some) -> get_user -> withdraw (token_out).
//! On any pre-trade failure after the deposit, the deposited token_in is
//! withdrawn back so funds return to the bot's main balance. Degraded/halted
//! ApiErrors are treated as soft `Err`s — this module never traps.

use candid::{CandidType, Deserialize, Int, Nat, Principal};

use crate::prices::nat_to_u64;
use crate::swaps::SwapError;

// ─── Candid types (verbatim field names/shapes from partydex-interface.md) ───
//
// NOTE: Candid record decoding matches by field name, not position, and a
// target struct may omit fields present on the wire (they're just dropped).
// So types below only carry the fields this adapter actually reads.

#[derive(CandidType, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    #[serde(rename = "buy")]
    Buy,
    #[serde(rename = "sell")]
    Sell,
}

#[derive(CandidType, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSide {
    #[serde(rename = "base")]
    Base,
    #[serde(rename = "quote")]
    Quote,
}

#[derive(CandidType, Deserialize, Debug, Clone, Copy)]
pub enum TimeInForce {
    #[serde(rename = "fok")]
    Fok,
    #[serde(rename = "gtc")]
    Gtc,
    #[serde(rename = "ioc")]
    Ioc,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
#[allow(dead_code)] // ErrorCategory variants are decode-only; not all are inspected today.
pub enum ErrorCategory {
    #[serde(rename = "admin")]
    Admin,
    #[serde(rename = "authorization")]
    Authorization,
    #[serde(rename = "external")]
    External,
    #[serde(rename = "other")]
    Other,
    #[serde(rename = "rate_limit")]
    RateLimit,
    #[serde(rename = "resource")]
    Resource,
    #[serde(rename = "state")]
    State,
    #[serde(rename = "validation")]
    Validation,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct ApiError {
    pub category: ErrorCategory,
    pub code: String,
    pub message: String,
    pub metadata: Option<Vec<(String, String)>>,
}

// Every ApiError — including the `degraded`/`halted` system-state cases — is
// surfaced as an ordinary `Err` throughout this module; nothing in here ever
// traps, so no special-casing by category is needed.
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {} ({})", self.category, self.message, self.code)
    }
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct BookOrderSpec {
    pub input_amount: Nat,
    pub limit_tick: i32,
    pub side: Side,
    pub time_in_force: TimeInForce,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct PoolSwapSpec {
    pub fee_pips: u32,
    pub input_amount: Nat,
    pub limit_tick: i32,
    pub side: Side,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub enum VenueType {
    #[serde(rename = "book")]
    Book,
    #[serde(rename = "pool")]
    Pool(u32),
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct VenueBreakdown {
    pub fee_amount: Nat,
    pub input_amount: Nat,
    pub output_amount: Nat,
    pub venue_id: VenueType,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct QuoteResult {
    pub book_orders: Vec<BookOrderSpec>,
    pub effective_tick: i32,
    pub input_amount: Nat,
    pub output_amount: Nat,
    pub pool_swaps: Vec<PoolSwapSpec>,
    pub reference_tick: i32,
    pub total_fees: Nat,
    pub venue_breakdown: Vec<VenueBreakdown>,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub enum QuoteTradeResult {
    #[serde(rename = "ok")]
    Ok(QuoteResult),
    #[serde(rename = "err")]
    Err(ApiError),
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct AtomicTradeArgs {
    pub allow_partial: bool,
    pub book_orders: Vec<BookOrderSpec>,
    pub min_output: Option<Nat>,
    pub pool_swaps: Vec<PoolSwapSpec>,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub enum OrderSettlement {
    #[serde(rename = "cancelled")]
    Cancelled,
    #[serde(rename = "filled")]
    Filled,
    #[serde(rename = "fok_rejected")]
    FokRejected,
    #[serde(rename = "partial")]
    Partial,
    #[serde(rename = "resting")]
    Resting,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct OrderResultOk {
    pub fee: Int,
    pub input_amount: Nat,
    pub order_id: u64,
    pub output_amount: Nat,
    pub remaining_input: Nat,
    pub settlement: OrderSettlement,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub enum ApiResult1 {
    #[serde(rename = "ok")]
    Ok(OrderResultOk),
    #[serde(rename = "err")]
    Err(ApiError),
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct OrderResultItem {
    pub index: u32,
    pub result: ApiResult1,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct SwapResultOk {
    pub fee: Int,
    pub input_amount: Nat,
    pub output_amount: Nat,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub enum ApiResult2 {
    #[serde(rename = "ok")]
    Ok(SwapResultOk),
    #[serde(rename = "err")]
    Err(ApiError),
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct SwapResultItem {
    pub index: u32,
    pub result: ApiResult2,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub enum SystemState {
    #[serde(rename = "degraded")]
    Degraded,
    #[serde(rename = "halted")]
    Halted,
    #[serde(rename = "normal")]
    Normal,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct SystemGuard {
    pub global_backpressure: bool,
    pub system_state: SystemState,
    pub user_calls_remaining: Int,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct PollVersions {
    pub available_base: Nat,
    pub available_quote: Nat,
    pub candle: Nat,
    pub guard: SystemGuard,
    pub orderbook: Nat,
    pub platform: Nat,
    pub user: Nat,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct AtomicTradeOk {
    pub order_results: Vec<OrderResultItem>,
    pub swap_results: Vec<SwapResultItem>,
    pub versions: PollVersions,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub enum AtomicTradeResult {
    #[serde(rename = "ok")]
    Ok(AtomicTradeOk),
    #[serde(rename = "err")]
    Err(ApiError),
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct DepositWithdrawOk {
    pub amount: Nat,
    pub block_index: Nat,
    pub versions: PollVersions,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub enum DepositResult {
    #[serde(rename = "ok")]
    Ok(DepositWithdrawOk),
    #[serde(rename = "err")]
    Err(ApiError),
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub enum WithdrawResult {
    #[serde(rename = "ok")]
    Ok(DepositWithdrawOk),
    #[serde(rename = "err")]
    Err(ApiError),
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct TokenPair {
    pub base: Nat,
    pub quote: Nat,
}

/// `get_user` returns `UserData = opt record { available: TokenPair; ... }`
/// directly (a plain Option, not an ok/err wrapper). We only decode the
/// `available` field — Candid subtyping drops the rest of the (much larger)
/// wire record.
#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct UserDataInner {
    pub available: TokenPair,
}

// ─── Raw calls ───

async fn call_quote_trade(
    pool: Principal,
    side: Side,
    input_amount: Nat,
    limit_tick: Option<i32>,
    slippage_bps: Option<u32>,
) -> Result<QuoteResult, String> {
    let result: Result<(QuoteTradeResult,), _> =
        ic_cdk::call(pool, "quote_trade", (side, input_amount, limit_tick, slippage_bps)).await;
    match result {
        Ok((QuoteTradeResult::Ok(qr),)) => Ok(qr),
        Ok((QuoteTradeResult::Err(e),)) => Err(format!("PartyDEX quote_trade error: {}", e)),
        Err((code, msg)) => Err(format!("PartyDEX quote_trade call failed ({:?}): {}", code, msg)),
    }
}

async fn call_deposit(pool: Principal, token: TokenSide, amount: Nat) -> Result<Nat, SwapError> {
    let result: Result<(DepositResult,), _> = ic_cdk::call(pool, "deposit", (token, amount)).await;
    match result {
        Ok((DepositResult::Ok(ok),)) => Ok(ok.amount),
        Ok((DepositResult::Err(e),)) => Err(SwapError::SwapFailed(format!("PartyDEX deposit error: {}", e))),
        Err((code, msg)) => Err(SwapError::SwapFailed(format!("PartyDEX deposit call failed ({:?}): {}", code, msg))),
    }
}

async fn call_withdraw(pool: Principal, token: TokenSide, amount: Nat) -> Result<Nat, SwapError> {
    let result: Result<(WithdrawResult,), _> = ic_cdk::call(pool, "withdraw", (token, amount)).await;
    match result {
        Ok((WithdrawResult::Ok(ok),)) => Ok(ok.amount),
        Ok((WithdrawResult::Err(e),)) => Err(SwapError::SwapFailed(format!("PartyDEX withdraw error: {}", e))),
        Err((code, msg)) => Err(SwapError::SwapFailed(format!("PartyDEX withdraw call failed ({:?}): {}", code, msg))),
    }
}

async fn call_get_user(pool: Principal) -> Result<Option<UserDataInner>, SwapError> {
    let result: Result<(Option<UserDataInner>,), _> = ic_cdk::call(pool, "get_user", ()).await;
    match result {
        Ok((user,)) => Ok(user),
        Err((code, msg)) => Err(SwapError::SwapFailed(format!("PartyDEX get_user call failed ({:?}): {}", code, msg))),
    }
}

async fn call_atomic_trade(pool: Principal, args: AtomicTradeArgs) -> Result<AtomicTradeOk, SwapError> {
    let result: Result<(AtomicTradeResult,), _> = ic_cdk::call(pool, "atomic_trade", (args,)).await;
    match result {
        Ok((AtomicTradeResult::Ok(ok),)) => Ok(ok),
        Ok((AtomicTradeResult::Err(e),)) => Err(SwapError::SwapFailed(format!("PartyDEX atomic_trade error: {}", e))),
        Err((code, msg)) => Err(SwapError::SwapFailed(format!("PartyDEX atomic_trade call failed ({:?}): {}", code, msg))),
    }
}

/// Best-effort refund: withdraw whatever is currently available on `token`
/// side and swallow (log-worthy, but not fatal) failures — the caller is
/// already on an error path and must not trap.
async fn refund_side(pool: Principal, token: TokenSide) {
    if let Ok(Some(user)) = call_get_user(pool).await {
        let avail = match token {
            TokenSide::Base => user.available.base,
            TokenSide::Quote => user.available.quote,
        };
        if avail > Nat::from(0u64) {
            let _ = call_withdraw(pool, token, avail).await;
        }
    }
}

// ─── Public adapter API ───

/// Price of 1 ICP in the pool's stable native units (sell 1 ICP, no slippage
/// bound — informational quote only). `fee_pips` is accepted for signature
/// symmetry with the other venue fns but unused here: `quote_trade` quotes
/// across the router's full depth (book + all pool tiers), not a single fee
/// tier — only `atomic_trade`'s `pool_swaps` (see `swap` below) can pin a tier.
pub async fn price_icp(pool: Principal, fee_pips: u32) -> Result<u64, String> {
    let _ = fee_pips;
    let qr = call_quote_trade(pool, Side::Sell, Nat::from(100_000_000u64), None, None).await?;
    Ok(nat_to_u64(&qr.output_amount))
}

/// Quote buying ICP by spending `amount` of the stable (quote) token.
pub async fn quote_stable_to_icp(pool: Principal, fee_pips: u32, amount: u64) -> Result<u64, String> {
    let _ = fee_pips;
    let qr = call_quote_trade(pool, Side::Buy, Nat::from(amount), None, None).await?;
    Ok(nat_to_u64(&qr.output_amount))
}

/// Quote selling `amount` ICP for the stable (quote) token.
pub async fn quote_icp_to_stable(pool: Principal, fee_pips: u32, amount: u64) -> Result<u64, String> {
    let _ = fee_pips;
    let qr = call_quote_trade(pool, Side::Sell, Nat::from(amount), None, None).await?;
    Ok(nat_to_u64(&qr.output_amount))
}

/// Execute a PartyDEX swap using the custody flow from orchestrator decision #1:
/// deposit(token_in) -> quote_trade (re-quote at exec time) -> atomic_trade
/// (allow_partial=false, min_output=Some) -> get_user -> withdraw(token_out).
///
/// `token_in_is_base` says which TokenSide the input token is (ICP=base is
/// always true when selling ICP; stable=quote is always true when buying
/// ICP). `token_in_ledger`/`token_out_ledger` are accepted for parity with
/// the ICPSwap-side call shape (and for clearer error messages) — the actual
/// deposit/withdraw calls address the pool's own base/quote sides, not a
/// ledger principal, since ICRC-2 allowances are set up once via
/// `swaps::approve_infinite` in `setup_approvals`, not per-call here.
pub async fn swap(
    pool: Principal,
    fee_pips: u32,
    side: Side,
    amount_in: u64,
    min_out: u64,
    token_in_ledger: Principal,
    token_out_ledger: Principal,
    token_in_is_base: bool,
) -> Result<u64, SwapError> {
    let token_in_side = if token_in_is_base { TokenSide::Base } else { TokenSide::Quote };
    let token_out_side = if token_in_is_base { TokenSide::Quote } else { TokenSide::Base };

    // 1. Deposit token_in into the pool's internal balance.
    call_deposit(pool, token_in_side, Nat::from(amount_in)).await.map_err(|e| {
        SwapError::SwapFailed(format!("deposit({:?}) from {} failed: {}", token_in_side, token_in_ledger, e))
    })?;

    // 2. Re-quote at execution time; bail out (refunding token_in) if the
    // fresh quote already can't clear min_out.
    let quote = match call_quote_trade(pool, side, Nat::from(amount_in), None, None).await {
        Ok(q) => q,
        Err(e) => {
            refund_side(pool, token_in_side).await;
            return Err(SwapError::QuoteFailed(e));
        }
    };
    if nat_to_u64(&quote.output_amount) < min_out {
        refund_side(pool, token_in_side).await;
        return Err(SwapError::QuoteFailed(format!(
            "re-quote {} < min_out {}", nat_to_u64(&quote.output_amount), min_out
        )));
    }

    // 3. Execute atomically, pinned to the configured fee tier. limit_tick is
    // set to the tick we just re-quoted at (effective_tick) so the atomic
    // trade's own price bound is consistent with the quote we validated
    // against — min_output is the primary slippage guard.
    let args = AtomicTradeArgs {
        allow_partial: false,
        book_orders: vec![],
        min_output: Some(Nat::from(min_out)),
        pool_swaps: vec![PoolSwapSpec {
            fee_pips,
            input_amount: Nat::from(amount_in),
            limit_tick: quote.effective_tick,
            side,
        }],
    };
    if let Err(e) = call_atomic_trade(pool, args).await {
        refund_side(pool, token_in_side).await;
        return Err(SwapError::SwapFailed(format!(
            "atomic_trade failed (token_out={}): {}", token_out_ledger, e
        )));
    }

    // 4. Sweep the output side back out to the bot's main account.
    let user = match call_get_user(pool).await {
        Ok(Some(u)) => u,
        Ok(None) => return Err(SwapError::SwapFailed("get_user returned no record after trade".to_string())),
        Err(e) => return Err(e),
    };
    let avail = match token_out_side {
        TokenSide::Base => user.available.base,
        TokenSide::Quote => user.available.quote,
    };
    let out_amount = call_withdraw(pool, token_out_side, avail).await?;
    Ok(nat_to_u64(&out_amount))
}
