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
use icrc_ledger_types::icrc::generic_value::ICRC3Value;
use num_traits::ToPrimitive;
use serde::Serialize;
use sha2::{Digest, Sha224};

pub const MAX_RECEIPTS: usize = 256;
pub const MAX_RESPONSE_BYTES: usize = 512 * 1024;
// icUSD caps ICRC-3 page responses at 100 blocks even when a larger length is
// requested. Requesting the cap from the actual tail, rather than a larger
// page from an earlier offset, keeps recent submitted transfers visible.
const MAX_ICRC3_BLOCKS: u64 = 100;
const MAX_ICP_INDEX_TRANSACTIONS: u64 = 256;
const ICP_LEDGER: &str = "ryjl3-tyaaa-aaaaa-aaaba-cai";
const ICP_INDEX: &str = "qhbym-qaaaa-aaaaa-aaafq-cai";
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

/// ICRC-3 is the durable source of truth for transfers on the wrapped-token
/// ledgers.  The pool cache is useful, but it is not retention-guaranteed.
#[derive(CandidType, Deserialize)]
struct Icrc3GetBlocksArgs {
    start: Nat,
    length: Nat,
}
#[derive(CandidType, Deserialize)]
struct Icrc3BlockWithId {
    id: Nat,
    block: ICRC3Value,
}
#[derive(CandidType, Deserialize)]
struct Icrc3GetBlocksResult {
    log_length: Nat,
    blocks: Vec<Icrc3BlockWithId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LedgerTransfer {
    block: Nat,
    timestamp_ns: u64,
    from: Account,
    to: Account,
    spender: Option<Account>,
    amount: u64,
    fee: Option<u64>,
    memo: Vec<u8>,
}

// ICP's legacy ledger does not export ICRC-3.  Its maintained index does;
// this is the current public `get_account_transactions` wire shape.
#[derive(CandidType, Deserialize, Serialize)]
struct IcpIndexAccount {
    owner: Principal,
    subaccount: Option<Vec<u8>>,
}
#[derive(CandidType, Deserialize, Serialize)]
struct IcpIndexRequest {
    account: IcpIndexAccount,
    start: Option<u64>,
    max_results: Nat,
}
#[derive(CandidType, Deserialize)]
struct IcpTokens {
    e8s: u64,
}
#[derive(CandidType, Deserialize)]
struct IcpTransfer {
    to: String,
    fee: IcpTokens,
    from: String,
    amount: IcpTokens,
    spender: Option<String>,
}
#[derive(CandidType, Deserialize)]
enum IcpOperation {
    Burn(Reserved),
    Mint(Reserved),
    Transfer(IcpTransfer),
    Approve(Reserved),
}
#[derive(CandidType, Deserialize)]
struct IcpTimestamp {
    timestamp_nanos: u64,
}
#[derive(CandidType, Deserialize)]
struct IcpTransaction {
    memo: u64,
    icrc1_memo: Option<Vec<u8>>,
    operation: IcpOperation,
    timestamp: Option<IcpTimestamp>,
    created_at_time: Option<u64>,
}
#[derive(CandidType, Deserialize)]
struct IcpTransactionWithId {
    id: u64,
    transaction: IcpTransaction,
}
#[derive(CandidType, Deserialize)]
struct IcpTransactions {
    balance: u64,
    transactions: Vec<IcpTransactionWithId>,
    oldest_tx_id: Option<u64>,
}
#[derive(CandidType, Deserialize)]
enum IcpTransactionsResult {
    Ok(IcpTransactions),
    Err(IcpIndexError),
}
#[derive(CandidType, Deserialize)]
struct IcpIndexError {
    message: String,
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

fn nat_u64(v: &Nat) -> Option<u64> {
    v.0.to_u64()
}

fn map_field<'a>(map: &'a std::collections::BTreeMap<String, ICRC3Value>, name: &str) -> Option<&'a ICRC3Value> {
    map.get(name)
}

fn value_nat(v: &ICRC3Value) -> Option<u64> {
    match v {
        ICRC3Value::Nat(n) => nat_u64(n),
        _ => None,
    }
}

fn value_blob(v: &ICRC3Value) -> Option<Vec<u8>> {
    match v {
        ICRC3Value::Blob(b) => Some(b.to_vec()),
        _ => None,
    }
}

fn value_account(v: &ICRC3Value) -> Option<Account> {
    let ICRC3Value::Array(parts) = v else { return None };
    if parts.is_empty() || parts.len() > 2 {
        return None;
    }
    let owner = Principal::try_from_slice(&value_blob(parts.first()?)?).ok()?;
    let subaccount = match parts.get(1) {
        None => None,
        Some(v) => {
            let bytes: [u8; 32] = value_blob(v)?.try_into().ok()?;
            Some(bytes.to_vec())
        }
    };
    Some(Account { owner, subaccount })
}

fn decode_icrc3_transfer(block: &Icrc3BlockWithId) -> Option<LedgerTransfer> {
    let ICRC3Value::Map(root) = &block.block else { return None };
    let timestamp_ns = value_nat(map_field(root, "ts")?)?;
    let ICRC3Value::Map(tx) = map_field(root, "tx")? else { return None };
    if !matches!(map_field(tx, "op")?, ICRC3Value::Text(op) if op == "xfer") {
        return None;
    }
    Some(LedgerTransfer {
        block: block.id.clone(),
        timestamp_ns,
        from: value_account(map_field(tx, "from")?)?,
        to: value_account(map_field(tx, "to")?)?,
        spender: map_field(tx, "spender").and_then(value_account),
        amount: value_nat(map_field(tx, "amt")?)?,
        fee: map_field(tx, "fee").and_then(value_nat),
        memo: map_field(tx, "memo").and_then(value_blob).unwrap_or_default(),
    })
}

async fn read_icrc3_tail(ledger: Principal) -> Result<Vec<LedgerTransfer>, String> {
    let empty = candid::encode_args((vec![Icrc3GetBlocksArgs {
        start: Nat::from(0u8),
        length: Nat::from(0u8),
    }],))
    .map_err(|e| e.to_string())?;
    let bytes = raw(ledger, "icrc3_get_blocks", empty).await?;
    let (head,): (Icrc3GetBlocksResult,) = decode_bounded(&bytes)?;
    let Some(log_length) = nat_u64(&head.log_length) else {
        return Err("ICRC-3 log length exceeds supported range".into());
    };
    let start = log_length.saturating_sub(MAX_ICRC3_BLOCKS);
    let args = candid::encode_args((vec![Icrc3GetBlocksArgs {
        start: Nat::from(start),
        length: Nat::from(MAX_ICRC3_BLOCKS),
    }],))
    .map_err(|e| e.to_string())?;
    let bytes = raw(ledger, "icrc3_get_blocks", args).await?;
    let (tail,): (Icrc3GetBlocksResult,) = decode_bounded(&bytes)?;
    if tail.blocks.len() > MAX_ICRC3_BLOCKS as usize {
        return Err("ICRC-3 ledger tail exceeded cap".into());
    }
    Ok(tail.blocks.iter().filter_map(decode_icrc3_transfer).collect())
}

fn icp_account_identifier(owner: Principal) -> Vec<u8> {
    let mut preimage = b"\x0Aaccount-id".to_vec();
    preimage.extend_from_slice(owner.as_slice());
    preimage.extend_from_slice(&[0u8; 32]);
    let hash = Sha224::digest(&preimage);
    let mut crc = !0u32;
    for byte in hash {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    let mut account = (!crc).to_be_bytes().to_vec();
    account.extend_from_slice(&hash);
    account
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

async fn read_icp_account_transactions(owner: Principal) -> Result<Vec<LedgerTransfer>, String> {
    let index = Principal::from_text(ICP_INDEX).expect("pinned ICP index principal");
    let request = IcpIndexRequest {
        account: IcpIndexAccount { owner, subaccount: None },
        start: None,
        max_results: Nat::from(MAX_ICP_INDEX_TRANSACTIONS),
    };
    let bytes = raw(
        index,
        "get_account_transactions",
        candid::encode_args((request,)).map_err(|e| e.to_string())?,
    )
    .await?;
    let (result,): (IcpTransactionsResult,) = decode_bounded(&bytes)?;
    let txs = match result {
        IcpTransactionsResult::Ok(txs) => txs,
        IcpTransactionsResult::Err(e) => return Err(format!("ICP index refusal: {}", e.message)),
    };
    if txs.transactions.len() > MAX_ICP_INDEX_TRANSACTIONS as usize {
        return Err("ICP index account response exceeded cap".into());
    }
    let account = hex_encode(&icp_account_identifier(owner));
    let mut result = Vec::new();
    for row in txs.transactions {
        let IcpOperation::Transfer(transfer) = row.transaction.operation else { continue };
        let Some(timestamp_ns) = row.transaction.timestamp.map(|v| v.timestamp_nanos) else { continue };
        let Some(memo) = row.transaction.icrc1_memo else { continue };
        // The index has already selected this account, but check its identifier
        // explicitly so an outgoing bot transfer can never pass as a payout.
        if transfer.from != account && transfer.to != account {
            return Err("ICP index returned a transaction outside requested account".into());
        }
        result.push(LedgerTransfer {
            block: Nat::from(row.id),
            timestamp_ns,
            from: Account { owner: Principal::anonymous(), subaccount: Some(transfer.from.into_bytes()) },
            to: Account { owner: Principal::anonymous(), subaccount: Some(transfer.to.into_bytes()) },
            spender: transfer.spender.map(|v| Account { owner: Principal::anonymous(), subaccount: Some(v.into_bytes()) }),
            amount: transfer.amount.e8s,
            fee: Some(transfer.fee.e8s),
            memo,
        });
    }
    Ok(result)
}

async fn ledger_transfers(ledger: Principal, owner: Principal) -> Result<Vec<LedgerTransfer>, String> {
    if ledger == Principal::from_text(ICP_LEDGER).expect("pinned ICP ledger principal") {
        read_icp_account_transactions(owner).await
    } else {
        read_icrc3_tail(ledger).await
    }
}

fn is_default(a: &Account, owner: Principal) -> bool {
    a.owner == owner && a.subaccount.is_none()
}

fn icp_side_is(a: &Account, owner: Principal) -> bool {
    a.owner == Principal::anonymous()
        && a.subaccount.as_ref().is_some_and(|v| *v == hex_encode(&icp_account_identifier(owner)).into_bytes())
}

fn side_is(a: &Account, ledger: Principal, owner: Principal) -> bool {
    if ledger == Principal::from_text(ICP_LEDGER).expect("pinned ICP ledger principal") {
        icp_side_is(a, owner)
    } else {
        is_default(a, owner)
    }
}

/// Proves a one-step swap from two immutable ledger transfers when the pool's
/// ephemeral receipt cache has already discarded the completed operation.
async fn bind_ledger_receipt(intent: &Intent) -> Result<Option<ReceiptProof>, String> {
    let r = &intent.request;
    let input = ledger_transfers(r.token_in, r.owner).await?;
    let inputs: Vec<_> = input
        .iter()
        .filter(|t| {
            t.timestamp_ns >= intent.cutoff.submitted_after_ns
                && side_is(&t.from, r.token_in, r.owner)
                && side_is(&t.to, r.token_in, r.pool)
                && t.spender.as_ref().is_some_and(|s| side_is(s, r.token_in, r.pool))
                && t.amount == r.input
                && t.fee == Some(r.input_fee)
                && t.memo.len() == 8
        })
        .collect();
    if inputs.len() != 1 {
        return Err(format!(
            "ledger reconciliation found {} matching input transfers; expected exactly one",
            inputs.len()
        ));
    }
    let input = inputs[0];
    let output = ledger_transfers(r.token_out, r.owner).await?;
    let outputs: Vec<_> = output
        .iter()
        .filter(|t| {
            t.timestamp_ns >= input.timestamp_ns
                && side_is(&t.from, r.token_out, r.pool)
                && side_is(&t.to, r.token_out, r.owner)
                && t.spender.is_none()
                && t.fee == Some(r.output_fee)
                && t.memo == input.memo
                && t.amount >= r.min_gross_output.saturating_sub(r.output_fee)
        })
        .collect();
    if outputs.len() != 1 {
        return Err(format!(
            "ledger reconciliation found {} matching output transfers; expected exactly one",
            outputs.len()
        ));
    }
    let output = outputs[0];
    let Some(input_debit) = r.input.checked_add(r.input_fee) else {
        return Err("input debit overflow".into());
    };
    let receipt_candid = candid::encode_one((input.block.clone(), output.block.clone(), input.memo.clone()))
        .map_err(|e| format!("cannot encode ledger receipt: {e}"))?;
    Ok(Some(ReceiptProof {
        pool: r.pool,
        receipt_id: Nat::from(u64::from_be_bytes(input.memo.clone().try_into().map_err(|_| "non-eight-byte receipt memo")?)),
        input_block: input.block.clone(),
        output_block: Some(output.block.clone()),
        effective_input: r.input,
        input_debit,
        output_credit: output.amount,
        refund_credit: 0,
        refund_block: None,
        receipt_candid,
    }))
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
fn runtime_settlement(p: ReceiptProof, evidence_kind: &str) -> RuntimeSettlement {
    let encoded = p
        .receipt_candid
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    RuntimeSettlement {
        input_debit_native: p.input_debit,
        effective_input_native: p.effective_input,
        output_credit_native: p.output_credit,
        refund_credit_native: p.refund_credit,
        evidence: vec![ReconciliationEvidenceV1 {
            evidence_kind: evidence_kind.into(),
            source_reference: format!(
                "pool={};receipt={};input_block={};output_block={:?};refund_block={:?};receipt_candid_hex={encoded}",
                p.pool, p.receipt_id, p.input_block, p.output_block, p.refund_block
            ),
            amount_native: p.output_credit,
            observed_at_ns: ic_cdk::api::time(),
        }],
    }
}
pub async fn reconcile(intent: &Intent) -> Result<Option<RuntimeSettlement>, String> {
    let txs = read_snapshot(intent.request.pool, intent.request.owner).await?;
    match bind_receipt(&intent.request, &intent.cutoff, &txs) {
        ReceiptVerdict::Settled(p) => Ok(Some(runtime_settlement(
            p,
            "icpswap_source_bound_terminal_transfers_v1",
        ))),
        ReceiptVerdict::Pending(_) => bind_ledger_receipt(intent)
            .await
            .map(|proof| proof.map(|p| runtime_settlement(p, "icpswap_ledger_bound_terminal_transfers_v1"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icp_default_account_identifier_matches_the_live_bot_account() {
        let owner = Principal::from_text("ucjxv-nqaaa-aaaaj-qrsaq-cai").unwrap();
        assert_eq!(
            icp_account_identifier(owner),
            vec![
                0x4a, 0x3e, 0x49, 0x4f, 0xec, 0xd3, 0x1e, 0xc0, 0x20, 0x9a, 0x3d, 0xe7,
                0x0f, 0x49, 0x21, 0xa8, 0x56, 0xd7, 0x15, 0x45, 0x09, 0xf4, 0x53, 0x6f,
                0x4d, 0x6a, 0x9c, 0x68, 0x0a, 0x96, 0x39, 0x2f,
            ]
        );
        let pool = Principal::from_text("nqxwe-hiaaa-aaaar-qb5yq-cai").unwrap();
        assert_eq!(
            hex_encode(&icp_account_identifier(pool)),
            "18b8fa8253916c2f3306e0f608e27c23899021045fb8be232a54dcc062ac732b"
        );
    }



    #[test]
    fn ledger_side_checks_do_not_confuse_an_outgoing_icp_transfer_for_a_payout() {
        let owner = Principal::self_authenticating([9; 32]);
        let pool = Principal::self_authenticating([7; 32]);
        let bot_account = Account { owner: Principal::anonymous(), subaccount: Some(hex_encode(&icp_account_identifier(owner)).into_bytes()) };
        let pool_account = Account { owner: Principal::anonymous(), subaccount: Some(hex_encode(&icp_account_identifier(pool)).into_bytes()) };
        let ledger = Principal::from_text(ICP_LEDGER).unwrap();
        assert!(side_is(&bot_account, ledger, owner));
        assert!(!side_is(&pool_account, ledger, owner));
    }

}
