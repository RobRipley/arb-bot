use candid::{CandidType, Deserialize, Nat, Principal};
use serde::Serialize;

// ─── Rumi AMM Types ───

#[derive(CandidType, Deserialize, Debug)]
pub enum AmmError {
    InsufficientOutput { actual: Nat, expected_min: Nat },
    PoolPaused,
    PoolCreationClosed,
    PoolNotFound,
    ZeroAmount,
    DisproportionateLiquidity,
    FeeBpsOutOfRange,
    InvalidToken,
    InsufficientLpShares { available: Nat, required: Nat },
    MathOverflow,
    TransferFailed { token: String, reason: String },
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
pub struct TokenInfo {
    pub decimals: u8,
    pub precision_mul: u64,
    pub ledger_id: Principal,
    pub symbol: String,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct PoolStatus {
    pub balances: Vec<Nat>,
    pub lp_total_supply: Nat,
    pub current_a: u64,
    pub virtual_price: Nat,
    pub swap_fee_bps: u64,
    pub admin_fee_bps: u64,
    pub tokens: Vec<TokenInfo>,
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
pub enum IcpSwapError {
    CommonError,
    InsufficientFunds,
    InternalError(String),
    UnsupportedToken(String),
}

// ─── Price Data ───

pub struct PriceData {
    pub rumi_icp_price_3usd_native: u64,
    pub virtual_price: u64,
    pub icpswap_icp_price_ckusdc_native: u64,
}

impl PriceData {
    /// Convert Rumi's 3USD-denominated ICP price to USD with 6 decimals.
    /// rumi_icp_price_3usd_native is in 8 decimals (3USD per 1 ICP).
    /// virtual_price is in 18 decimals (≈1.057e18 means 1 3USD = $1.057).
    /// Result: (3usd_amount * vp) / 1e18 gives USD in 8 decimals, then / 100 for 6 decimals.
    pub fn rumi_price_usd_6dec(&self) -> u64 {
        let usd_e8s = self.rumi_icp_price_3usd_native as u128
            * self.virtual_price as u128
            / 1_000_000_000_000_000_000; // divide by 1e18 (VP precision)
        (usd_e8s / 100) as u64 // 8-dec → 6-dec
    }

    pub fn icpswap_price_usd_6dec(&self) -> u64 {
        self.icpswap_icp_price_ckusdc_native
    }

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

pub async fn fetch_rumi_price(
    rumi_amm: Principal,
    pool_id: &str,
    icp_ledger: Principal,
) -> Result<u64, String> {
    let amount_in = Nat::from(100_000_000u64);
    let result: Result<(AmmResult<Nat>,), _> = ic_cdk::call(
        rumi_amm, "get_quote", (pool_id.to_string(), icp_ledger, amount_in),
    ).await;
    match result {
        Ok((AmmResult::Ok(amount_out),)) => Ok(nat_to_u64(&amount_out)),
        Ok((AmmResult::Err(e),)) => Err(format!("Rumi AMM quote error: {:?}", e)),
        Err((code, msg)) => Err(format!("Rumi AMM call failed ({:?}): {}", code, msg)),
    }
}

pub async fn fetch_virtual_price(rumi_3pool: Principal) -> Result<u64, String> {
    let result: Result<(PoolStatus,), _> = ic_cdk::call(rumi_3pool, "get_pool_status", ()).await;
    match result {
        Ok((status,)) => Ok(nat_to_u64(&status.virtual_price)),
        Err((code, msg)) => Err(format!("3pool status call failed ({:?}): {}", code, msg)),
    }
}

pub async fn fetch_icpswap_price(
    icpswap_pool: Principal,
    zero_for_one: bool,
) -> Result<u64, String> {
    #[derive(CandidType, Serialize)]
    struct SwapArgs {
        #[serde(rename = "amountIn")]
        amount_in: String,
        #[serde(rename = "zeroForOne")]
        zero_for_one: bool,
        #[serde(rename = "amountOutMinimum")]
        amount_out_minimum: String,
    }
    let args = SwapArgs {
        amount_in: "100000000".to_string(),
        zero_for_one,
        amount_out_minimum: "0".to_string(),
    };
    let result: Result<(IcpSwapResult,), _> = ic_cdk::call(icpswap_pool, "quote", (args,)).await;
    match result {
        Ok((IcpSwapResult::Ok(amount),)) => Ok(nat_to_u64(&amount)),
        Ok((IcpSwapResult::Err(e),)) => Err(format!("ICPSwap quote error: {:?}", e)),
        Err((code, msg)) => Err(format!("ICPSwap call failed ({:?}): {}", code, msg)),
    }
}

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

/// Like fetch_rumi_price but with a custom input amount instead of hardcoded 1 ICP.
/// token_in is the ledger of the token being sold (e.g. 3USD ledger or ICP ledger).
pub async fn fetch_rumi_quote_for_amount(
    rumi_amm: Principal,
    pool_id: &str,
    token_in: Principal,
    amount: u64,
) -> Result<u64, String> {
    let amount_in = Nat::from(amount);
    let result: Result<(AmmResult<Nat>,), _> = ic_cdk::call(
        rumi_amm, "get_quote", (pool_id.to_string(), token_in, amount_in),
    ).await;
    match result {
        Ok((AmmResult::Ok(amount_out),)) => Ok(nat_to_u64(&amount_out)),
        Ok((AmmResult::Err(e),)) => Err(format!("Rumi AMM quote error: {:?}", e)),
        Err((code, msg)) => Err(format!("Rumi AMM call failed ({:?}): {}", code, msg)),
    }
}

/// Like fetch_icpswap_price but with a custom input amount.
pub async fn fetch_icpswap_quote_for_amount(
    icpswap_pool: Principal,
    amount: u64,
    zero_for_one: bool,
) -> Result<u64, String> {
    #[derive(CandidType, Serialize)]
    struct SwapArgs {
        #[serde(rename = "amountIn")]
        amount_in: String,
        #[serde(rename = "zeroForOne")]
        zero_for_one: bool,
        #[serde(rename = "amountOutMinimum")]
        amount_out_minimum: String,
    }
    let args = SwapArgs {
        amount_in: amount.to_string(),
        zero_for_one,
        amount_out_minimum: "0".to_string(),
    };
    let result: Result<(IcpSwapResult,), _> = ic_cdk::call(icpswap_pool, "quote", (args,)).await;
    match result {
        Ok((IcpSwapResult::Ok(out),)) => Ok(nat_to_u64(&out)),
        Ok((IcpSwapResult::Err(e),)) => Err(format!("ICPSwap quote error: {:?}", e)),
        Err((code, msg)) => Err(format!("ICPSwap call failed ({:?}): {}", code, msg)),
    }
}

// ─── Strategy B Price Data ───

pub struct StrategyBPriceData {
    /// icUSD per 1 ICP (8 decimals) from the icUSD/ICP pool
    pub icusd_icp_price_native: u64,
    /// ckUSDC per 1 ICP (6 decimals) from the ckUSDC/ICP reference pool
    pub ckusdc_icp_price_native: u64,
}

impl StrategyBPriceData {
    /// icUSD/ICP price in 6-dec USD. icUSD ≈ $1.00 (8 decimals), so /100 → 6 dec.
    pub fn icusd_price_usd_6dec(&self) -> u64 {
        self.icusd_icp_price_native / 100
    }

    /// ckUSDC/ICP price is already in 6-dec USD.
    pub fn ckusdc_price_usd_6dec(&self) -> u64 {
        self.ckusdc_icp_price_native
    }

    /// Spread in bps: positive = ICP cheaper on icUSD pool (buy icUSD→ICP, sell ICP→ckUSDC)
    pub fn spread_bps(&self) -> i32 {
        let icusd = self.icusd_price_usd_6dec() as i64;
        let ckusdc = self.ckusdc_price_usd_6dec() as i64;
        if icusd == 0 || ckusdc == 0 {
            return 0;
        }
        let diff = ckusdc - icusd;
        let min_price = icusd.min(ckusdc);
        (diff * 10_000 / min_price) as i32
    }
}

pub async fn fetch_strategy_b_prices(
    icpswap_icusd_pool: Principal,
    icpswap_icusd_zero_for_one: bool,
    icpswap_ref_pool: Principal,
    icpswap_ref_zero_for_one: bool,
) -> Result<StrategyBPriceData, String> {
    let icusd_fut = fetch_icpswap_price(icpswap_icusd_pool, icpswap_icusd_zero_for_one);
    let ref_fut = fetch_icpswap_price(icpswap_ref_pool, icpswap_ref_zero_for_one);
    let (icusd_res, ref_res) = futures::future::join(icusd_fut, ref_fut).await;
    Ok(StrategyBPriceData {
        icusd_icp_price_native: icusd_res?,
        ckusdc_icp_price_native: ref_res?,
    })
}

pub fn nat_to_u64(n: &Nat) -> u64 {
    n.0.to_string().parse::<u64>().unwrap_or(0)
}

// ─── ICPSwap Pool Metadata ───

#[derive(CandidType, Deserialize, Debug, Clone)]
pub struct IcpSwapToken {
    pub address: String,
    pub standard: String,
}

#[derive(CandidType, Deserialize, Debug, Clone)]
#[allow(non_snake_case)]
pub struct PoolMetadata {
    pub token0: IcpSwapToken,
    pub token1: IcpSwapToken,
    pub fee: Nat,
    pub key: String,
    pub liquidity: Nat,
    pub sqrtPriceX96: Nat,
    pub tick: candid::Int,
}

#[derive(CandidType, Deserialize, Debug)]
pub enum IcpSwapMetadataResult {
    #[serde(rename = "ok")]
    Ok(PoolMetadata),
    #[serde(rename = "err")]
    Err(IcpSwapError),
}

/// Query ICPSwap pool metadata to determine if ICP is token0 or token1
pub async fn fetch_icpswap_token_ordering(
    icpswap_pool: Principal,
    icp_ledger: Principal,
) -> Result<bool, String> {
    let result: Result<(IcpSwapMetadataResult,), _> =
        ic_cdk::call(icpswap_pool, "metadata", ()).await;
    match result {
        Ok((IcpSwapMetadataResult::Ok(meta),)) => {
            let icp_text = icp_ledger.to_text();
            if meta.token0.address == icp_text {
                Ok(true) // ICP is token0
            } else if meta.token1.address == icp_text {
                Ok(false) // ICP is token1
            } else {
                Err(format!(
                    "ICP ledger {} not found in pool tokens: token0={}, token1={}",
                    icp_text, meta.token0.address, meta.token1.address
                ))
            }
        }
        Ok((IcpSwapMetadataResult::Err(e),)) => Err(format!("ICPSwap metadata error: {:?}", e)),
        Err((code, msg)) => Err(format!("ICPSwap metadata call failed ({:?}): {}", code, msg)),
    }
}
