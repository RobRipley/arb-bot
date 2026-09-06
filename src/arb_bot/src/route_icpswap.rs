//! ICPSwap one-step receipt adapter, source semantics pinned to 94eeb92.
//! Caller MUST hold the global bot account lock from snapshot through settlement,
//! persist Intent before submit, and never repeat submit after any outcome.
//! Source: SwapPool.mo depositFromAndSwap/_withdraw and transaction/lib.mo.
//! Completed output receipts attest transfer success; linked refunds account
//! exactly for unused input. Failed swaps settle only after a full input refund.
//! This pool
//! version discards its output block index. Never manufacture a ledger reference.
#![allow(non_snake_case)]
use crate::route_arb::{self, Asset, ReconciliationEvidenceV1, VenueKind};
use crate::route_runtime::{RuntimeRequest, RuntimeSettlement};
use candid::{CandidType, Deserialize, Int, Nat, Principal, Reserved};
use num_traits::ToPrimitive;
use serde::Serialize;

pub const MAX_RECEIPTS: usize = 256;
pub const MAX_RESPONSE_BYTES: usize = 512 * 1024;
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub owner: Principal,
    pub subaccount: Option<Vec<u8>>,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub address: Principal,
    pub standard: String,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub token: Principal,
    pub standard: String,
    pub from: Account,
    pub to: Account,
    pub amount: Nat,
    pub fee: Nat,
    pub memo: Option<Vec<u8>>,
    pub index: Nat,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub enum DepositStatus {
    Created,
    TransferCompleted,
    Completed,
    Failed,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub enum WithdrawStatus {
    Created,
    CreditCompleted,
    Completed,
    Failed,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub enum SwapStatus {
    Created,
    Completed,
    Failed,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub enum OneStepStatus {
    Created,
    DepositTransferCompleted,
    DepositCreditCompleted,
    PreSwapCompleted,
    SwapCompleted,
    WithdrawCreditCompleted,
    Completed,
    Failed,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Deposit {
    pub transfer: Transfer,
    pub status: DepositStatus,
    pub err: Option<String>,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Withdraw {
    pub transfer: Transfer,
    pub status: WithdrawStatus,
    pub err: Option<String>,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Swap {
    pub tokenIn: Token,
    pub tokenOut: Token,
    pub amountIn: Nat,
    pub amountOut: Nat,
    pub amountInFee: Nat,
    pub amountOutFee: Nat,
    pub status: SwapStatus,
    pub err: Option<String>,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct OneStep {
    pub deposit: Deposit,
    pub withdraw: Withdraw,
    pub swap: Swap,
    pub status: OneStepStatus,
    pub err: Option<String>,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Refund {
    pub relatedIndex: Nat,
    pub transfer: Transfer,
    pub status: WithdrawStatus,
    pub err: Option<String>,
}
// Include ALL upstream variants: Candid variant subtyping rejects a narrow enum.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Deposit(Reserved),
    Withdraw(Reserved),
    Refund(Refund),
    AddLiquidity(Reserved),
    DecreaseLiquidity(Reserved),
    Claim(Reserved),
    Swap(Reserved),
    OneStepSwap(OneStep),
    TransferPosition(Reserved),
    AddLimitOrder(Reserved),
    RemoveLimitOrder(Reserved),
    ExecuteLimitOrder(Reserved),
}
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub id: Nat,
    pub timestamp: Int,
    pub owner: Principal,
    pub canisterId: Principal,
    pub action: Action,
}
#[derive(CandidType, Deserialize, Clone, Debug)]
pub enum PoolError {
    CommonError,
    InternalError(String),
    UnsupportedToken(String),
    InsufficientFunds,
}
#[derive(CandidType, Deserialize, Clone, Debug)]
pub enum PoolResult<T> {
    #[serde(rename = "ok")]
    Ok(T),
    #[serde(rename = "err")]
    Err(PoolError),
}
#[derive(CandidType, Deserialize)]
pub struct SwapRecord {
    pub txInfo: Transaction,
}
#[derive(CandidType, Deserialize)]
pub struct RecordState {
    pub records: Vec<SwapRecord>,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct SwapArgs {
    pub zeroForOne: bool,
    pub tokenInFee: Nat,
    pub tokenOutFee: Nat,
    pub amountIn: String,
    pub amountOutMinimum: String,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct SwapRequest {
    pub edge_id: String,
    pub pool: Principal,
    pub owner: Principal,
    pub token_in: Principal,
    pub token_out: Principal,
    pub input: u64,
    pub min_gross_output: u64,
    pub input_fee: u64,
    pub output_fee: u64,
    pub args: SwapArgs,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct Cutoff {
    pub seen_ids: Vec<Nat>,
    pub submitted_after_ns: u64,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub struct Intent {
    pub request: SwapRequest,
    pub cutoff: Cutoff,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ReceiptProof {
    pub pool: Principal,
    pub receipt_id: Nat,
    pub input_block: Nat,
    pub output_block: Option<Nat>,
    pub effective_input: u64,
    pub input_debit: u64,
    pub output_credit: u64,
    pub refund_credit: u64,
    pub refund_block: Option<Nat>,
    pub receipt_candid: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiptVerdict {
    Settled(ReceiptProof),
    Pending(String),
}

pub fn pinned_request(
    edge_id: &str,
    token0: Asset,
    owner: Principal,
    input: u64,
    min_gross_output: u64,
    input_fee: u64,
    output_fee: u64,
) -> Result<SwapRequest, String> {
    let edge = route_arb::directed_edges()
        .into_iter()
        .find(|e| e.edge_id == edge_id && e.venue == VenueKind::IcpSwap)
        .ok_or("unregistered ICPSwap edge")?;
    if (token0 != edge.from && token0 != edge.to)
        || owner == Principal::anonymous()
        || input <= input_fee
        || min_gross_output <= output_fee
    {
        return Err("invalid request direction, owner, amount or fee".into());
    }
    input.checked_add(input_fee).ok_or("input debit overflow")?;
    let pins = route_arb::asset_pins();
    Ok(SwapRequest {
        edge_id: edge_id.into(),
        pool: edge.pool_principal,
        owner,
        token_in: pins[edge.from.index()].ledger,
        token_out: pins[edge.to.index()].ledger,
        input,
        min_gross_output,
        input_fee,
        output_fee,
        args: SwapArgs {
            zeroForOne: token0 == edge.from,
            tokenInFee: input_fee.into(),
            tokenOutFee: output_fee.into(),
            amountIn: input.to_string(),
            amountOutMinimum: min_gross_output.to_string(),
        },
    })
}
pub fn capture_cutoff(txs: &[Transaction], submitted_after_ns: u64) -> Result<Cutoff, String> {
    if txs.len() > MAX_RECEIPTS {
        return Err("receipt snapshot cap exceeded".into());
    }
    Ok(Cutoff {
        seen_ids: txs.iter().map(|t| t.id.clone()).collect(),
        submitted_after_ns,
    })
}
pub fn decode_bounded<'a, T: candid::utils::ArgumentDecoder<'a>>(
    bytes: &'a [u8],
) -> Result<T, String> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("receipt response byte cap exceeded".into());
    }
    let mut config = candid::de::DecoderConfig::new();
    config
        .set_decoding_quota(2_000_000)
        .set_skipping_quota(2_000_000);
    candid::utils::decode_args_with_config(bytes, &config)
        .map_err(|e| format!("receipt decode: {e}"))
}
async fn raw(pool: Principal, method: &str, args: Vec<u8>) -> Result<Vec<u8>, String> {
    ic_cdk::api::call::call_raw(pool, method, args, 0)
        .await
        .map_err(|e| format!("external call {method}: {e:?}"))
}
/// Two bounded responses; an overlarge non-paginated upstream cache fails closed.
pub async fn read_snapshot(pool: Principal, owner: Principal) -> Result<Vec<Transaction>, String> {
    let bytes = raw(
        pool,
        "getTransactionsByOwner",
        candid::encode_args((owner,)).map_err(|e| e.to_string())?,
    )
    .await?;
    let (result,): (PoolResult<Vec<(Nat, Transaction)>>,) = decode_bounded(&bytes)?;
    let pairs = match result {
        PoolResult::Ok(v) => v,
        PoolResult::Err(e) => return Err(format!("receipt economic/read refusal: {e:?}")),
    };
    if pairs.len() > MAX_RECEIPTS {
        return Err("owner receipt cap exceeded".into());
    }
    let mut txs = Vec::new();
    for (id, t) in pairs {
        if id != t.id {
            return Err("receipt map ID mismatch".into());
        }
        txs.push(t);
    }
    let bytes = raw(
        pool,
        "getSwapRecordState",
        candid::encode_args(()).map_err(|e| e.to_string())?,
    )
    .await?;
    let (result,): (PoolResult<RecordState>,) = decode_bounded(&bytes)?;
    let records = match result {
        PoolResult::Ok(v) => v.records,
        PoolResult::Err(e) => return Err(format!("receipt cache refusal: {e:?}")),
    };
    if records.len() > MAX_RECEIPTS {
        return Err("pool receipt cap exceeded".into());
    }
    for record in records {
        let t = record.txInfo;
        if t.owner != owner {
            continue;
        }
        if let Some(old) = txs.iter_mut().find(|o| o.id == t.id) {
            if *old != t {
                return Err("conflicting versions of receipt; poll again".into());
            }
        } else {
            txs.push(t);
        }
    }
    if txs.len() > MAX_RECEIPTS {
        return Err("combined receipt cap exceeded".into());
    }
    Ok(txs)
}
pub fn bind_receipt(r: &SwapRequest, cutoff: &Cutoff, txs: &[Transaction]) -> ReceiptVerdict {
    let pending = |s: &str| ReceiptVerdict::Pending(s.into());
    if txs.len() > MAX_RECEIPTS || cutoff.seen_ids.len() > MAX_RECEIPTS {
        return pending("receipt cap exceeded");
    }
    let candidates: Vec<_> = txs
        .iter()
        .filter(|t| {
            t.owner == r.owner
                && t.canisterId == r.pool
                && t.timestamp >= Int::from(cutoff.submitted_after_ns)
                && !cutoff.seen_ids.contains(&t.id)
                && matches!(&t.action, Action::OneStepSwap(_))
        })
        .collect();
    // Any additional fresh one-step operation violates the exclusive-account premise.
    if candidates.len() != 1 {
        return pending("missing or ambiguous fresh one-step receipt");
    }
    let t = candidates[0];
    let Action::OneStepSwap(s) = &t.action else {
        unreachable!()
    };
    let default = |owner| Account {
        owner,
        subaccount: None,
    };
    let d = &s.deposit.transfer;
    let w = &s.withdraw.transfer;
    if d.memo.is_some()
        || w.memo.is_some()
        || d.from != default(r.owner)
        || d.to != default(r.pool)
        || w.from != default(r.pool)
        || w.to != default(r.owner)
        || d.token != r.token_in
        || w.token != r.token_out
        || s.swap.tokenIn.address != r.token_in
        || s.swap.tokenOut.address != r.token_out
        || d.standard != s.swap.tokenIn.standard
        || w.standard != s.swap.tokenOut.standard
        || !matches!(d.standard.as_str(), "ICRC1" | "ICRC2")
        || !matches!(w.standard.as_str(), "ICRC1" | "ICRC2")
    {
        return pending("receipt account, ledger or direction mismatch");
    }
    if d.amount != Nat::from(r.input)
        || d.fee != Nat::from(r.input_fee)
        || w.fee != Nat::from(r.output_fee)
        || s.swap.amountInFee != Nat::from(r.input_fee)
        || s.swap.amountOutFee != Nat::from(r.output_fee)
    {
        return pending("receipt input or fee mismatch");
    }
    if s.deposit.status != DepositStatus::Completed || s.deposit.err.is_some() {
        return pending("input transfer incomplete or failed");
    }
    let refunds: Vec<_> = txs
        .iter()
        .filter_map(|x| match &x.action {
            Action::Refund(f) if f.relatedIndex == t.id => Some((x, f)),
            _ => None,
        })
        .collect();
    let completed_swap = s.status == OneStepStatus::Completed
        && s.swap.status == SwapStatus::Completed
        && s.withdraw.status == WithdrawStatus::Completed
        && s.err.is_none()
        && s.swap.err.is_none()
        && s.withdraw.err.is_none();
    // Failed swaps retain their original requested amounts, not actual economic
    // amounts. Only a fully linked input refund plus no completed output proves
    // zero effective input. Never interpret those unchanged amounts as a fill.
    let fully_refunded_failure = s.status == OneStepStatus::Failed
        && s.swap.status == SwapStatus::Failed
        && s.withdraw.status == WithdrawStatus::Failed
        && s.swap.amountIn == Nat::from(r.input)
        && s.swap.amountOut == Nat::from(r.min_gross_output)
        && w.amount == Nat::from(r.min_gross_output);
    let (effective, output_credit) = if completed_swap {
        let Some(effective) = s.swap.amountIn.0.to_u64() else {
            return pending("effective input overflow");
        };
        let Some(gross) = s.swap.amountOut.0.to_u64() else {
            return pending("output overflow");
        };
        if effective == 0
            || effective > r.input
            || w.amount != s.swap.amountOut
            || gross < r.min_gross_output
            || gross <= r.output_fee
        {
            return pending("output transfer/minimum or effective input mismatch");
        }
        (effective, gross - r.output_fee)
    } else if fully_refunded_failure {
        (0, 0)
    } else {
        return pending("swap/output status is not authoritatively terminal");
    };
    let refund_gross = r.input - effective;
    let (refund_credit, refund_block, refund_tx) = if refund_gross == 0 {
        if !refunds.is_empty() {
            return pending("unexpected linked refund for full fill");
        }
        (0, None, None)
    } else {
        if refunds.len() != 1 {
            return pending("missing or ambiguous linked refund");
        }
        let (rt, f) = refunds[0];
        // transaction/lib.mo startRefund stores its own ID as memo; the ledger
        // transfer uses the parent ID instead. Receipt memo is NOT ledger memo.
        let Some(refund_id) = rt.id.0.to_u64() else {
            return pending("refund ID exceeds supported memo width");
        };
        if rt.owner != r.owner
            || rt.canisterId != r.pool
            || rt.id <= t.id
            || rt.timestamp < t.timestamp
            || rt.timestamp < Int::from(cutoff.submitted_after_ns)
            || cutoff.seen_ids.contains(&rt.id)
            || f.status != WithdrawStatus::Completed
            || f.err.is_some()
            || f.transfer.token != r.token_in
            || f.transfer.standard != d.standard
            || f.transfer.from != default(r.pool)
            || f.transfer.to != default(r.owner)
            || f.transfer.index == d.index
            || f.transfer.amount != Nat::from(refund_gross)
            || f.transfer.fee != Nat::from(r.input_fee)
            || f.transfer.memo != Some(refund_id.to_be_bytes().to_vec())
            || refund_gross <= r.input_fee
        {
            return pending("linked refund identity, transfer, fee or conservation mismatch");
        }
        (
            refund_gross - r.input_fee,
            Some(f.transfer.index.clone()),
            Some(rt),
        )
    };
    // Durable evidence includes BOTH source receipts. A completed refund confirms
    // its real ledger index (including zero); output index remains unavailable.
    let mut proof_receipts = vec![t.clone()];
    if let Some(rt) = refund_tx {
        proof_receipts.push(rt.clone());
    }
    let Ok(receipt_candid) = candid::encode_one(proof_receipts) else {
        return pending("cannot encode durable receipt");
    };
    let Some(input_debit) = r.input.checked_add(r.input_fee) else {
        return pending("input debit overflow");
    };
    ReceiptVerdict::Settled(ReceiptProof {
        pool: r.pool,
        receipt_id: t.id.clone(),
        input_block: d.index.clone(),
        output_block: None,
        effective_input: effective,
        input_debit,
        output_credit,
        refund_credit,
        refund_block,
        receipt_candid,
    })
}
pub async fn prepare(r: &RuntimeRequest) -> Result<Intent, String> {
    if r.owner != ic_cdk::id() {
        return Err("receipt owner must be the executing bot".into());
    }
    let edge = route_arb::directed_edges()
        .into_iter()
        .find(|e| e.edge_id == r.edge.edge_id && e.venue == VenueKind::IcpSwap)
        .ok_or("unregistered edge")?;
    if edge.pool_principal != r.edge.pool_principal
        || edge.from != r.edge.from
        || edge.to != r.edge.to
    {
        return Err("runtime edge pin mismatch".into());
    }
    let bytes = raw(
        edge.pool_principal,
        "metadata",
        candid::encode_args(()).map_err(|e| e.to_string())?,
    )
    .await?;
    let (metadata,): (PoolResult<crate::prices::PoolMetadata>,) = decode_bounded(&bytes)?;
    let metadata = match metadata {
        PoolResult::Ok(m) => m,
        PoolResult::Err(e) => return Err(format!("metadata refusal: {e:?}")),
    };
    let t0 = Principal::from_text(&metadata.token0.address).map_err(|e| e.to_string())?;
    let t1 = Principal::from_text(&metadata.token1.address).map_err(|e| e.to_string())?;
    let token0 = route_arb::asset_for_ledger(t0).ok_or("unknown pool token0")?;
    let token1 = route_arb::asset_for_ledger(t1).ok_or("unknown pool token1")?;
    if !((token0 == edge.from && token1 == edge.to) || (token1 == edge.from && token0 == edge.to)) {
        return Err("pool token pair mismatch".into());
    }
    let request = pinned_request(
        &edge.edge_id,
        token0,
        r.owner,
        r.input_native,
        r.min_output_native
            .checked_add(r.output_fee_native)
            .ok_or("minimum overflow")?,
        r.input_fee_native,
        r.output_fee_native,
    )?;
    let txs = read_snapshot(request.pool, r.owner).await?;
    // Timestamp is only a conservative exclusion boundary, never receipt identity.
    // Cross-subnet clock skew can leave Pending; never widen this boundary.
    let cutoff = capture_cutoff(&txs, ic_cdk::api::time())?;
    Ok(Intent { request, cutoff })
}
/// Amount-only Ok means accepted for later reconciliation, NEVER settlement.
pub async fn submit_once(intent: &Intent) -> Result<(), String> {
    let bytes = raw(
        intent.request.pool,
        "depositFromAndSwap",
        candid::encode_args((&intent.request.args,)).map_err(|e| e.to_string())?,
    )
    .await?;
    let (result,): (PoolResult<Nat>,) = decode_bounded(&bytes)?;
    match result {
        PoolResult::Ok(_) => Ok(()),
        PoolResult::Err(e) => Err(format!(
            "pool refusal; reconcile deposits/refunds, never replay: {e:?}"
        )),
    }
}
pub async fn reconcile(intent: &Intent) -> Result<Option<RuntimeSettlement>, String> {
    let txs = read_snapshot(intent.request.pool, intent.request.owner).await?;
    match bind_receipt(&intent.request, &intent.cutoff, &txs) {
        ReceiptVerdict::Pending(_) => Ok(None),
        ReceiptVerdict::Settled(p) => {
            let encoded = p
                .receipt_candid
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            Ok(Some(RuntimeSettlement {
                input_debit_native: p.input_debit,
                effective_input_native: p.effective_input,
                output_credit_native: p.output_credit,
                refund_credit_native: p.refund_credit,
                evidence: vec![ReconciliationEvidenceV1 {
                    evidence_kind: "icpswap_source_bound_terminal_transfers_v1".into(),
                    source_reference: format!(
                        "pool={};receipt={};input_block={};output_block=absent;refund_block={:?};source=94eeb92;receipt_candid_hex={encoded}",
                        p.pool, p.receipt_id, p.input_block, p.refund_block
                    ),
                    amount_native: p.output_credit,
                    observed_at_ns: ic_cdk::api::time(),
                }],
            }))
        }
    }
}
