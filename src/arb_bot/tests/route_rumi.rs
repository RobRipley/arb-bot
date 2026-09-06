use arb_bot::route_arb::{asset_pins, directed_edges, Asset, VenueKind};
use arb_bot::route_rumi::*;
use candid::{Nat, Principal};
use icrc_ledger_types::icrc1::account::Account;

fn intent() -> Intent {
    let edge = directed_edges().into_iter().find(|e| e.venue == VenueKind::Rumi3Pool && e.from == Asset::IcUsd && e.to == Asset::CkUsdc).unwrap();
    Intent { pool: edge.pool_principal, owner: Principal::self_authenticating(b"bot"), edge_id: edge.edge_id,
        request: SwapRequestV1 { intent_id: vec![17;32], i:0, j:2, dx:1_000_000, min_dy:9_900 },
        input_ledger: asset_pins()[0].ledger, output_ledger: asset_pins()[2].ledger, input_fee:10, output_fee:10 }
}
fn transfer(i: &Intent, leg:u8, amount:u128) -> SwapTransferV1 {
    SwapTransferV1 { ledger: if leg==1 {i.output_ledger} else {i.input_ledger},
        from: Account {owner: if leg==0 {i.owner} else {i.pool},subaccount:None},
        to: Account {owner: if leg==0 {i.pool} else {i.owner},subaccount:None},
        amount, fee:10, created_at_time:1234, memo:expected_memo(i,leg), block_index:Some(Nat::from(leg)), status:SwapTransferStatusV1::Confirmed }
}
fn receipt(i:&Intent) -> SwapReceiptV1 {
    SwapReceiptV1 {version:1,owner:i.owner,request:i.request.clone(),status:SwapReceiptStatusV1::Completed,
        input:Some(transfer(i,0,1_000_000)),output:Some(transfer(i,1,10_000)),refund:None,
        gross_output:Some(10_010),pool_fee:Some(25),error:None}
}
#[test]
fn authentic_full_fill_binds_zero_block_and_exact_net_fees() {
    let i=intent(); let r=receipt(&i);
    let wire=candid::encode_one(&r).unwrap();
    let decoded:SwapReceiptV1=candid::decode_one(&wire).unwrap();
    let s=bind_receipt(&i,&decoded,1235).unwrap().unwrap();
    assert_eq!(s.input_debit_native,1_000_010); assert_eq!(s.effective_input_native,1_000_000);
    assert_eq!(s.output_credit_native,10_000); assert_eq!(s.refund_credit_native,0);
    assert_eq!(s.evidence.len(),3); assert!(s.evidence[0].source_reference.ends_with(":0"));
    let persisted=serde_json::to_vec(&i).unwrap(); let restored:Intent=serde_json::from_slice(&persisted).unwrap();
    assert!(bind_receipt(&restored,&decoded,1236).unwrap().is_some());
}
#[test]
fn receipt_identity_and_source_transfer_mutations_cannot_settle() {
    let i=intent();
    for n in 0..12 {
        let mut r=receipt(&i);
        match n {
            0=>r.owner=Principal::anonymous(),1=>r.request.intent_id[0]^=1,2=>r.request.min_dy+=1,
            3=>r.input.as_mut().unwrap().from.owner=Principal::anonymous(),
            4=>r.output.as_mut().unwrap().to.subaccount=Some([1;32]),
            5=>r.output.as_mut().unwrap().ledger=i.input_ledger,
            6=>r.input.as_mut().unwrap().memo[0]^=1,
            7=>r.input.as_mut().unwrap().amount-=1,
            8=>r.output.as_mut().unwrap().fee+=1,
            9=>r.output.as_mut().unwrap().block_index=None,
            10=>r.output.as_mut().unwrap().status=SwapTransferStatusV1::Submitted,
            _=>r.gross_output=Some(100),
        }
        assert!(bind_receipt(&i,&r,1235).is_err(),"mutation {n} admitted");
    }
}
#[test]
fn delayed_unknown_and_missing_receipts_do_not_mean_settled() {
    let i=intent(); let mut r=receipt(&i);
    for phase in [SwapReceiptStatusV1::Prepared,SwapReceiptStatusV1::InputSubmitted,SwapReceiptStatusV1::OutputSubmitted,SwapReceiptStatusV1::RefundSubmitted,SwapReceiptStatusV1::Unresolved] {
        r.status=phase; assert!(bind_receipt(&i,&r,1235).unwrap().is_none());
    }
    r.status=SwapReceiptStatusV1::Completed; r.input=None;
    assert!(bind_receipt(&i,&r,1235).is_err());
}
#[test]
fn complete_refund_proves_two_fees_and_cannot_include_a_paid_output() {
    let i=intent(); let mut r=receipt(&i);
    r.status=SwapReceiptStatusV1::Refunded;
    r.output.as_mut().unwrap().status=SwapTransferStatusV1::Rejected;
    r.output.as_mut().unwrap().block_index=None;
    r.refund=Some(transfer(&i,2,999_990));
    let s=bind_receipt(&i,&r,1235).unwrap().unwrap();
    assert_eq!(s.effective_input_native,0); assert_eq!(s.output_credit_native,0);
    assert_eq!(s.input_debit_native-s.refund_credit_native,20);
    r.output.as_mut().unwrap().status=SwapTransferStatusV1::Confirmed;
    r.output.as_mut().unwrap().block_index=Some(Nat::from(19u64));
    assert!(bind_receipt(&i,&r,1235).is_err());
}
#[test]
fn definitive_failure_before_debit_is_zero_movement() {
    let i=intent(); let mut r=receipt(&i); r.status=SwapReceiptStatusV1::Failed;
    r.output=None; r.input=None;
    let s=bind_receipt(&i,&r,1235).unwrap().unwrap(); assert_eq!(s.input_debit_native,0);
    r.input=Some(transfer(&i,0,1_000_000));
    assert!(bind_receipt(&i,&r,1235).is_err());
}
