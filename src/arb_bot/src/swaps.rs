use candid::{CandidType, Deserialize, Nat, Principal};
use serde::Serialize;
use icrc_ledger_types::icrc1::account::Account;
use icrc_ledger_types::icrc1::transfer::{TransferArg, TransferError};
use icrc_ledger_types::icrc2::approve::{ApproveArgs, ApproveError};

use crate::prices::{self, nat_to_u64};

pub const VOLUME_SUBACCOUNT: [u8; 32] = {
    let mut sub = [0u8; 32];
    sub[31] = 1;
    sub
};

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
struct SwapOutput {
    amount_out: Nat,
    fee: Nat,
}

#[derive(CandidType, Deserialize, Debug)]
enum RumiSwapResult {
    Ok(SwapOutput),
    Err(prices::AmmError),
}

pub async fn rumi_swap(
    rumi_amm: Principal,
    pool_id: &str,
    token_in: Principal,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<u64, SwapError> {
    let result: Result<(RumiSwapResult,), _> = ic_cdk::call(
        rumi_amm, "swap",
        (pool_id.to_string(), token_in, Nat::from(amount_in), Nat::from(min_amount_out)),
    ).await;
    match result {
        Ok((RumiSwapResult::Ok(r),)) => Ok(nat_to_u64(&r.amount_out)),
        Ok((RumiSwapResult::Err(e),)) => Err(SwapError::SwapFailed(format!("{:?}", e))),
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
        Ok((prices::IcpSwapResult::Err(e),)) => Err(SwapError::SwapFailed(format!("ICPSwap: {:?}", e))),
        Err((code, msg)) => Err(SwapError::SwapFailed(format!("ICPSwap call ({:?}): {}", code, msg))),
    }
}

// ─── 3pool Operations ───

pub async fn pool_add_liquidity(
    rumi_3pool: Principal,
    amounts: Vec<Nat>,
    min_lp: u64,
) -> Result<u64, String> {
    let result: Result<(prices::ThreePoolResult<Nat>,), _> =
        ic_cdk::call(rumi_3pool, "add_liquidity", (amounts, Nat::from(min_lp))).await;
    match result {
        Ok((prices::ThreePoolResult::Ok(lp_minted),)) => Ok(nat_to_u64(&lp_minted)),
        Ok((prices::ThreePoolResult::Err(e),)) => Err(format!("3pool add_liquidity error: {:?}", e)),
        Err((code, msg)) => Err(format!("3pool add_liquidity call failed ({:?}): {}", code, msg)),
    }
}

pub async fn pool_remove_one_coin(
    rumi_3pool: Principal,
    lp_amount: u64,
    coin_index: u8,
    min_out: u64,
) -> Result<u64, String> {
    let result: Result<(prices::ThreePoolResult<Nat>,), _> =
        ic_cdk::call(rumi_3pool, "remove_one_coin", (Nat::from(lp_amount), coin_index, Nat::from(min_out))).await;
    match result {
        Ok((prices::ThreePoolResult::Ok(amount_out),)) => Ok(nat_to_u64(&amount_out)),
        Ok((prices::ThreePoolResult::Err(e),)) => Err(format!("3pool remove_one_coin error: {:?}", e)),
        Err((code, msg)) => Err(format!("3pool remove_one_coin call failed ({:?}): {}", code, msg)),
    }
}

pub async fn pool_calc_deposit(
    rumi_3pool: Principal,
    amounts: Vec<Nat>,
) -> Result<u64, String> {
    let result: Result<(prices::ThreePoolResult<Nat>,), _> =
        ic_cdk::call(rumi_3pool, "calc_add_liquidity_query", (amounts, Nat::from(0u64))).await;
    match result {
        Ok((prices::ThreePoolResult::Ok(lp_out),)) => Ok(nat_to_u64(&lp_out)),
        Ok((prices::ThreePoolResult::Err(e),)) => Err(format!("3pool calc error: {:?}", e)),
        Err((code, msg)) => Err(format!("3pool calc call failed ({:?}): {}", code, msg)),
    }
}

pub async fn pool_calc_redeem(
    rumi_3pool: Principal,
    lp_amount: u64,
    coin_index: u8,
) -> Result<u64, String> {
    let result: Result<(prices::ThreePoolResult<Nat>,), _> =
        ic_cdk::call(rumi_3pool, "calc_remove_one_coin_query", (Nat::from(lp_amount), coin_index)).await;
    match result {
        Ok((prices::ThreePoolResult::Ok(amount_out),)) => Ok(nat_to_u64(&amount_out)),
        Ok((prices::ThreePoolResult::Err(e),)) => Err(format!("3pool calc error: {:?}", e)),
        Err((code, msg)) => Err(format!("3pool calc call failed ({:?}): {}", code, msg)),
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

pub async fn approve_infinite_subaccount(
    token_ledger: Principal,
    spender: Principal,
    subaccount: [u8; 32],
) -> Result<(), SwapError> {
    let approve_args = ApproveArgs {
        from_subaccount: Some(subaccount),
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

/// Transfer tokens from the volume subaccount to the default account
pub async fn transfer_from_subaccount(
    token_ledger: Principal,
    amount: u64,
    from_subaccount: [u8; 32],
) -> Result<u64, SwapError> {
    let self_principal = ic_cdk::id();
    let args = TransferArg {
        from_subaccount: Some(from_subaccount),
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
        to: Account { owner: self_principal, subaccount: Some(to_subaccount) },
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
        subaccount: Some(subaccount),
    };
    let result: Result<(Nat,), _> = ic_cdk::call(
        token_ledger, "icrc1_balance_of", (account,),
    ).await;
    match result {
        Ok((balance,)) => Ok(nat_to_u64(&balance)),
        Err((code, msg)) => Err(format!("Balance call ({:?}): {}", code, msg)),
    }
}
