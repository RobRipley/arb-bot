//! Source-bound Rumi receipt adapter. No observation or submission reply alone
//! advances a route. Only the pinned pool's retained, exact request receipt can.
use candid::{CandidType, Deserialize, Nat, Principal};
use icrc_ledger_types::icrc1::account::Account;
use serde::Serialize;
use sha2::{Digest, Sha256};
use crate::route_arb::{asset_pins, directed_edges, ReconciliationEvidenceV1, VenueKind};
use crate::route_runtime::{RuntimeRequest, RuntimeSettlement};

#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct SwapRequestV1 { pub intent_id: Vec<u8>, pub i: u8, pub j: u8, pub dx: u128, pub min_dy: u128 }
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub enum SwapReceiptStatusV1 { Prepared, InputSubmitted, OutputSubmitted, RefundSubmitted, Completed, Refunded, Failed, Unresolved }
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub enum SwapTransferStatusV1 { Submitted, Confirmed, Rejected, Unresolved, SkippedDust }
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct SwapTransferV1 {
    pub ledger: Principal, pub from: Account, pub to: Account, pub amount: u128, pub fee: u128,
    pub created_at_time: u64, pub memo: Vec<u8>, pub block_index: Option<Nat>, pub status: SwapTransferStatusV1,
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct SwapReceiptV1 {
    pub version: u16, pub owner: Principal, pub request: SwapRequestV1, pub status: SwapReceiptStatusV1,
    pub input: Option<SwapTransferV1>, pub output: Option<SwapTransferV1>, pub refund: Option<SwapTransferV1>,
    pub pool_fee: Option<u128>, pub gross_output: Option<u128>, pub error: Option<String>,
}
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Intent {
    pub pool: Principal, pub owner: Principal, pub edge_id: String, pub request: SwapRequestV1,
    pub input_ledger: Principal, pub output_ledger: Principal, pub input_fee: u64, pub output_fee: u64,
}

pub fn build_intent(r: &RuntimeRequest) -> Result<Intent, String> {
    let edge = directed_edges().into_iter().find(|e| e.edge_id == r.edge.edge_id)
        .ok_or("Rumi edge is not pinned")?;
    if edge.venue != VenueKind::Rumi3Pool || edge.pool_principal != r.edge.pool_principal
        || edge.from != r.edge.from || edge.to != r.edge.to || r.input_native == 0 || r.min_output_native == 0 {
        return Err("Rumi request does not match immutable edge or has zero input/floor".into());
    }
    // The immutable Rumi pool order is icUSD, ckUSDT, ckUSDC.
    let i = u8::try_from(edge.from.index()).map_err(|_| "Rumi index overflow")?;
    let j = u8::try_from(edge.to.index()).map_err(|_| "Rumi index overflow")?;
    if i > 2 || j > 2 || i == j { return Err("invalid Rumi direction".into()); }
    Ok(Intent { pool: edge.pool_principal, owner: r.owner, edge_id: edge.edge_id,
        request: SwapRequestV1 { intent_id: r.intent_id.to_vec(), i, j, dx: u128::from(r.input_native), min_dy: u128::from(r.min_output_native) },
        input_ledger: asset_pins()[edge.from.index()].ledger, output_ledger: asset_pins()[edge.to.index()].ledger,
        input_fee: r.input_fee_native, output_fee: r.output_fee_native })
}
fn validate_intent(i: &Intent) -> Result<(), String> {
    let edge = directed_edges().into_iter().find(|e| e.edge_id == i.edge_id).ok_or("Rumi intent edge missing")?;
    if edge.venue != VenueKind::Rumi3Pool || edge.pool_principal != i.pool || i.request.intent_id.len() != 32
        || i.request.i as usize != edge.from.index() || i.request.j as usize != edge.to.index()
        || asset_pins()[edge.from.index()].ledger != i.input_ledger || asset_pins()[edge.to.index()].ledger != i.output_ledger
        || i.request.dx == 0 || i.request.min_dy == 0 {
        return Err("persisted Rumi intent no longer matches immutable request pins".into());
    }
    Ok(())
}
pub async fn prepare(r: &RuntimeRequest) -> Result<Intent, String> {
    let intent = build_intent(r)?;
    if r.owner != ic_cdk::id() { return Err("Rumi source must be this canister".into()); }
    let allowed: Result<(bool,), _> = ic_cdk::call(intent.pool, "is_swap_receipt_client_v1", (intent.owner,)).await;
    if !matches!(allowed, Ok((true,))) { return Err("Rumi receipt client is not enabled or capability is unavailable".into()); }
    let result: Result<(Option<SwapReceiptV1>,), _> = ic_cdk::call(intent.pool, "get_swap_receipt_v1", (intent.request.intent_id.clone(),)).await;
    match result {
        Ok((None,)) => Ok(intent),
        Ok((Some(_),)) => Err("Rumi intent already exists; only recorded execution may reconcile it".into()),
        Err(error) => Err(format!("Rumi receipt capability unavailable: {error:?}")),
    }
}
/// Caller must persist LegSubmitted before invoking this function; it never retries.
pub async fn submit_once_outcome(intent: &Intent) -> crate::route_runtime::RuntimeSubmissionOutcome {
    use crate::route_runtime::RuntimeSubmissionOutcome as Outcome;
    if let Err(error) = validate_intent(intent) { return Outcome::RejectedBeforeDebit(error); }
    if intent.owner != ic_cdk::id() { return Outcome::RejectedBeforeDebit("Rumi source must be this canister".into()); }
    let response: Result<(Result<SwapReceiptV1, SwapReceiptErrorV1>,), _> = ic_cdk::call(intent.pool, "swap_with_receipt_v1", (intent.request.clone(),)).await;
    match response {
        Ok((Ok(_),)) => Outcome::Accepted,
        // These source-defined admission errors are returned before reserve/debit.
        Ok((Err(SwapReceiptErrorV1::InvalidIntentId | SwapReceiptErrorV1::InvalidRequest | SwapReceiptErrorV1::Unauthorized | SwapReceiptErrorV1::CapacityExceeded),)) =>
            Outcome::RejectedBeforeDebit("Rumi rejected before debit".into()),
        // A conflicting retained intent might already have transferred; never infer no debit.
        Ok((Err(SwapReceiptErrorV1::IntentConflict),)) => Outcome::Unknown("Rumi intent conflict requires reconciliation".into()),
        Err(e) => Outcome::Unknown(format!("Rumi submission outcome unknown: {e:?}")),
    }
}
#[derive(CandidType, Deserialize, Serialize, Clone, Debug)]
pub enum SwapReceiptErrorV1 { InvalidIntentId, InvalidRequest, IntentConflict, CapacityExceeded, Unauthorized }
pub async fn submit_once(intent: &Intent) -> Result<(), String> {
    use crate::route_runtime::RuntimeSubmissionOutcome as Outcome;
    match submit_once_outcome(intent).await { Outcome::Accepted => Ok(()), Outcome::RejectedBeforeDebit(e) | Outcome::Unknown(e) => Err(e) }
}
pub fn expected_memo(intent: &Intent, leg: u8) -> Vec<u8> {
    let mut hash = Sha256::new(); hash.update(b"rumi-3pool-swap-receipt-v1");
    hash.update([intent.owner.as_slice().len() as u8]); hash.update(intent.owner.as_slice());
    hash.update(&intent.request.intent_id); hash.update([leg]); hash.finalize().to_vec()
}
fn default_account(a: &Account, owner: Principal) -> bool { a.owner == owner && (a.subaccount.is_none() || a.subaccount == Some([0; 32])) }
fn validate_transfer(intent: &Intent, t: &SwapTransferV1, leg: u8, amount: u128, confirmed: bool) -> Result<(), String> {
    let (ledger, from, to, fee) = match leg {
        0 => (intent.input_ledger, intent.owner, intent.pool, intent.input_fee),
        1 => (intent.output_ledger, intent.pool, intent.owner, intent.output_fee),
        _ => (intent.input_ledger, intent.pool, intent.owner, intent.input_fee),
    };
    if t.ledger != ledger || !default_account(&t.from, from) || !default_account(&t.to, to)
        || t.amount != amount || t.fee != u128::from(fee) || t.memo != expected_memo(intent, leg)
        || (confirmed && (t.status != SwapTransferStatusV1::Confirmed || t.block_index.is_none()))
        || (!confirmed && (t.status != SwapTransferStatusV1::Rejected || t.block_index.is_some())) {
        return Err("Rumi receipt transfer does not bind expected accounts, amount, fee, memo and result".into());
    }
    if t.block_index.as_ref().is_some_and(|n| n.to_string().len() > 80) { return Err("Rumi ledger index exceeds evidence bound".into()); }
    Ok(())
}
fn evidence(intent: &Intent, t: &SwapTransferV1, kind: &str, now: u64) -> Result<ReconciliationEvidenceV1, String> {
    let index = t.block_index.as_ref().ok_or("confirmed transfer lacks ledger index")?;
    Ok(ReconciliationEvidenceV1 { evidence_kind: format!("rumi_receipt_{kind}"),
        source_reference: format!("{}:{}:{}", intent.pool, t.ledger, index),
        amount_native: u64::try_from(t.amount).map_err(|_| "Rumi amount exceeds runtime range")?, observed_at_ns: now })
}
fn receipt_evidence(intent: &Intent, receipt: &SwapReceiptV1, now: u64) -> Result<ReconciliationEvidenceV1, String> {
    let encoded = candid::encode_one(receipt).map_err(|e| format!("Rumi receipt encoding: {e}"))?;
    if encoded.len() > 8_000 { return Err("Rumi receipt exceeds durable evidence bound".into()); }
    let hex: String = encoded.iter().map(|b| format!("{b:02x}")).collect();
    Ok(ReconciliationEvidenceV1 { evidence_kind: "rumi_receipt_candid_v1".into(),
        source_reference: format!("{}:{hex}", intent.pool), amount_native: 0, observed_at_ns: now })
}
/// Pure binding verifier over a receipt obtained directly from the pinned pool.
/// Arbitrary user supplied receipts are never accepted by a public bot method.
pub fn bind_receipt(intent: &Intent, r: &SwapReceiptV1, now: u64) -> Result<Option<RuntimeSettlement>, String> {
    validate_intent(intent)?;
    if r.version != 1 || r.owner != intent.owner || r.request != intent.request { return Err("Rumi receipt request identity mismatch".into()); }
    let dx = intent.request.dx;
    let debit = dx.checked_add(u128::from(intent.input_fee)).ok_or("Rumi debit overflow")?;
    let as_u64 = |v| u64::try_from(v).map_err(|_| "Rumi settlement exceeds runtime range".to_string());
    match r.status {
        SwapReceiptStatusV1::Completed => {
            if r.refund.is_some() { return Err("completed Rumi receipt unexpectedly contains refund".into()); }
            let input = r.input.as_ref().ok_or("Rumi input receipt absent")?;
            let output = r.output.as_ref().ok_or("Rumi output receipt absent")?;
            let gross = r.gross_output.ok_or("Rumi gross output absent")?;
            let credit = gross.checked_sub(u128::from(intent.output_fee)).ok_or("Rumi output fee consumes output")?;
            if credit < intent.request.min_dy || r.pool_fee.is_none() { return Err("Rumi receipt violates net minimum or lacks pool fee".into()); }
            validate_transfer(intent, input, 0, dx, true)?;
            validate_transfer(intent, output, 1, credit, true)?;
            Ok(Some(RuntimeSettlement { input_debit_native: as_u64(debit)?, effective_input_native: as_u64(dx)?,
                output_credit_native: as_u64(credit)?, refund_credit_native: 0,
                evidence: vec![evidence(intent, input, "input", now)?, evidence(intent, output, "output", now)?, receipt_evidence(intent, r, now)?] }))
        }
        SwapReceiptStatusV1::Refunded => {
            let input = r.input.as_ref().ok_or("refunded receipt input absent")?;
            let output = r.output.as_ref().ok_or("refunded receipt failed output absent")?;
            let refund = r.refund.as_ref().ok_or("refunded receipt refund absent")?;
            let gross = r.gross_output.ok_or("refunded receipt gross output absent")?;
            let output_amount = gross.checked_sub(u128::from(intent.output_fee)).ok_or("refund output fee underflow")?;
            let returned = dx.checked_sub(u128::from(intent.input_fee)).ok_or("refund input fee underflow")?;
            validate_transfer(intent, input, 0, dx, true)?;
            validate_transfer(intent, output, 1, output_amount, false)?;
            validate_transfer(intent, refund, 2, returned, true)?;
            Ok(Some(RuntimeSettlement { input_debit_native: as_u64(debit)?, effective_input_native: 0,
                output_credit_native: 0, refund_credit_native: as_u64(returned)?,
                evidence: vec![evidence(intent, input, "input", now)?, evidence(intent, refund, "refund", now)?, receipt_evidence(intent, r, now)?] }))
        }
        SwapReceiptStatusV1::Failed => {
            if r.output.is_some() || r.refund.is_some() { return Ok(None); }
            if let Some(input) = &r.input { validate_transfer(intent, input, 0, dx, false)?; }
            Ok(Some(RuntimeSettlement { input_debit_native: 0, effective_input_native: 0, output_credit_native: 0,
                refund_credit_native: 0, evidence: vec![ReconciliationEvidenceV1 {
                    evidence_kind: "rumi_receipt_rejected_before_debit".into(), source_reference: intent.pool.to_string(),
                    amount_native: 0, observed_at_ns: now }, receipt_evidence(intent, r, now)?] }))
        }
        _ => Ok(None),
    }
}
pub async fn reconcile(intent: &Intent) -> Result<Option<RuntimeSettlement>, String> {
    validate_intent(intent)?;
    if intent.owner != ic_cdk::id() { return Err("Rumi source must be this canister".into()); }
    let response: Result<(Option<SwapReceiptV1>,), _> = ic_cdk::call(intent.pool, "get_swap_receipt_v1", (intent.request.intent_id.clone(),)).await;
    match response { Ok((Some(receipt),)) => bind_receipt(intent, &receipt, ic_cdk::api::time()), Ok((None,)) => Ok(None), Err(e) => Err(format!("Rumi receipt unavailable: {e:?}")) }
}
