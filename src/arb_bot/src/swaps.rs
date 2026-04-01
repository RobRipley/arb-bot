use candid::{CandidType, Deserialize, Nat, Principal};
use serde::Serialize;
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

#[derive(CandidType, Serialize)]
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

pub async fn rumi_swap(
    rumi_amm: Principal,
    pool_id: &str,
    token_in: Principal,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<u64, SwapError> {
    let result: Result<(AmmResult<SwapResult>,), _> = ic_cdk::call(
        rumi_amm, "swap",
        (pool_id.to_string(), token_in, Nat::from(amount_in), Nat::from(min_amount_out)),
    ).await;
    match result {
        Ok((AmmResult::Ok(r),)) => Ok(nat_to_u64(&r.amount_out)),
        Ok((AmmResult::Err(e),)) => Err(SwapError::SwapFailed(format!("{:?}", e))),
        Err((code, msg)) => Err(SwapError::SwapFailed(format!("{:?}: {}", code, msg))),
    }
}

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
        icpswap_pool, "depositFromAndSwap", (args,),
    ).await;
    match result {
        Ok((prices::IcpSwapResult::Ok(amount),)) => Ok(nat_to_u64(&amount)),
        Ok((prices::IcpSwapResult::Err(e),)) => Err(SwapError::SwapFailed(format!("ICPSwap: {}", e.message))),
        Err((code, msg)) => Err(SwapError::SwapFailed(format!("ICPSwap call ({:?}): {}", code, msg))),
    }
}

pub async fn approve_infinite(
    token_ledger: Principal,
    spender: Principal,
) -> Result<(), SwapError> {
    let approve_args = ApproveArgs {
        from_subaccount: None,
        spender: Account { owner: spender, subaccount: None },
        amount: Nat::from(340_282_366_920_938_463_463_374_607_431_768_211_455u128),
        expected_allowance: None,
        expires_at: None,
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
