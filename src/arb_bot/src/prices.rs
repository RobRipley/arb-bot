use candid::{CandidType, Deserialize, Nat, Principal};
use serde::Serialize;

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
    pub rumi_icp_price_3usd_native: u64,
    pub virtual_price: u64,
    pub icpswap_icp_price_ckusdc_native: u64,
}

impl PriceData {
    pub fn rumi_price_usd_6dec(&self) -> u64 {
        let usd_e8s = self.rumi_icp_price_3usd_native as u128
            * self.virtual_price as u128
            / 100_000_000;
        (usd_e8s / 100) as u64
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
        Ok((IcpSwapResult::Err(e),)) => Err(format!("ICPSwap quote error: {}", e.message)),
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

pub fn nat_to_u64(n: &Nat) -> u64 {
    n.0.to_string().parse::<u64>().unwrap_or(0)
}
