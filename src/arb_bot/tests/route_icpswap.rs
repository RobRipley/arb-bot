use arb_bot::{
    route_arb::{self, VenueKind},
    route_icpswap::*,
};
use candid::{Nat, Principal};
fn fixture() -> (SwapRequest, Cutoff, Transaction) {
    let e = route_arb::directed_edges()
        .into_iter()
        .find(|e| e.venue == VenueKind::IcpSwap)
        .unwrap();
    let owner = Principal::self_authenticating([7; 32]);
    let r = pinned_request(&e.edge_id, e.from, owner, 10000, 9000, 10, 20).unwrap();
    let account = |owner| Account {
        owner,
        subaccount: None,
    };
    let token = |address| Token {
        address,
        standard: "ICRC2".into(),
    };
    let transfer = |token, from, to, amount, fee, index| Transfer {
        token,
        standard: "ICRC2".into(),
        from: account(from),
        to: account(to),
        amount: Nat::from(amount as u64),
        fee: Nat::from(fee as u64),
        memo: None,
        index: Nat::from(index as u64),
    };
    let tx = Transaction {
        id: 50u64.into(),
        timestamp: 200u64.into(),
        owner,
        canisterId: r.pool,
        action: Action::OneStepSwap(OneStep {
            deposit: Deposit {
                transfer: transfer(r.token_in, owner, r.pool, 10000, 10, 0),
                status: DepositStatus::Completed,
                err: None,
            },
            withdraw: Withdraw {
                transfer: transfer(r.token_out, r.pool, owner, 9500, 20, 0),
                status: WithdrawStatus::Completed,
                err: None,
            },
            swap: Swap {
                tokenIn: token(r.token_in),
                tokenOut: token(r.token_out),
                amountIn: 10000u64.into(),
                amountOut: 9500u64.into(),
                amountInFee: 10u64.into(),
                amountOutFee: 20u64.into(),
                status: SwapStatus::Completed,
                err: None,
            },
            status: OneStepStatus::Completed,
            err: None,
        }),
    };
    (r, capture_cutoff(&[], 100).unwrap(), tx)
}
fn pending(r: &SwapRequest, c: &Cutoff, t: &[Transaction]) {
    assert!(matches!(bind_receipt(r, c, t), ReceiptVerdict::Pending(_)));
}
#[test]
fn typed_receipt_roundtrip_and_full_fill_proof() {
    let (r, c, t) = fixture();
    let bytes = candid::encode_args((PoolResult::Ok(vec![(t.id.clone(), t)]),)).unwrap();
    let (reply,): (PoolResult<Vec<(Nat, Transaction)>>,) = decode_bounded(&bytes).unwrap();
    let PoolResult::Ok(v) = reply else { panic!() };
    let ReceiptVerdict::Settled(p) = bind_receipt(&r, &c, &[v[0].1.clone()]) else {
        panic!()
    };
    assert_eq!(
        (p.input_debit, p.effective_input, p.output_credit),
        (10010, 10000, 9480)
    );
    assert_eq!(p.input_block, Nat::from(0u64));
    assert_eq!(p.output_block, None);
    assert!(!p.receipt_candid.is_empty());
}
#[test]
fn reject_wrong_identity_direction_account_cutoff_and_missing() {
    let (r, c, t) = fixture();
    pending(&r, &c, &[]);
    let mut bad = t.clone();
    bad.owner = Principal::anonymous();
    pending(&r, &c, &[bad]);
    let mut bad = t.clone();
    bad.canisterId = Principal::anonymous();
    pending(&r, &c, &[bad]);
    let mut bad = t.clone();
    bad.timestamp = 99u64.into();
    pending(&r, &c, &[bad]);
    pending(
        &r,
        &capture_cutoff(&[t.clone()], 100).unwrap(),
        &[t.clone()],
    );
    let mut bad = t.clone();
    if let Action::OneStepSwap(s) = &mut bad.action {
        s.swap.tokenIn.address = r.token_out;
    }
    pending(&r, &c, &[bad]);
    let mut bad = t;
    if let Action::OneStepSwap(s) = &mut bad.action {
        s.withdraw.transfer.to.subaccount = Some(vec![0; 32]);
    }
    pending(&r, &c, &[bad]);
}
#[test]
fn completed_is_not_enough_for_partial_or_refund_or_output() {
    let (r, c, t) = fixture();
    let mut partial = t.clone();
    if let Action::OneStepSwap(s) = &mut partial.action {
        s.swap.amountIn = 9999u64.into();
    }
    pending(&r, &c, &[partial]);
    let mut bad = t.clone();
    if let Action::OneStepSwap(s) = &mut bad.action {
        s.withdraw.status = WithdrawStatus::CreditCompleted;
    }
    pending(&r, &c, &[bad]);
    let mut bad = t.clone();
    if let Action::OneStepSwap(s) = &mut bad.action {
        s.withdraw.transfer.amount = 0u64.into();
    }
    pending(&r, &c, &[bad]);
    let mut refund = t.clone();
    let Action::OneStepSwap(s) = &t.action else {
        panic!()
    };
    refund.id = 51u64.into();
    refund.action = Action::Refund(Refund {
        relatedIndex: t.id.clone(),
        transfer: s.deposit.transfer.clone(),
        status: WithdrawStatus::Completed,
        err: None,
    });
    pending(&r, &c, &[t, refund]);
}
#[test]
fn duplicate_and_oversized_results_fail_closed() {
    let (r, c, t) = fixture();
    pending(&r, &c, &[t.clone(), t.clone()]);
    let mut other = t.clone();
    other.id = 51u64.into();
    pending(&r, &c, &[t.clone(), other]);
    pending(&r, &c, &vec![t; MAX_RECEIPTS + 1]);
    assert!(decode_bounded::<(Nat,)>(&vec![0; MAX_RESPONSE_BYTES + 1]).is_err());
}
#[test]
fn candid_decodes_every_upstream_action_variant() {
    let (_, _, mut t) = fixture();
    // Concrete payload projects into Reserved, preserving the entire wire variant width.
    #[derive(candid::CandidType)]
    enum ConcreteAction {
        Deposit(u64),
        Withdraw(u64),
        Refund(Refund),
        AddLiquidity(u64),
        DecreaseLiquidity(u64),
        Claim(u64),
        Swap(u64),
        OneStepSwap(OneStep),
        TransferPosition(u64),
        AddLimitOrder(u64),
        RemoveLimitOrder(u64),
        ExecuteLimitOrder(u64),
    }
    let variants = [
        ConcreteAction::Deposit(1),
        ConcreteAction::Withdraw(1),
        ConcreteAction::AddLiquidity(1),
        ConcreteAction::DecreaseLiquidity(1),
        ConcreteAction::Claim(1),
        ConcreteAction::Swap(1),
        ConcreteAction::TransferPosition(1),
        ConcreteAction::AddLimitOrder(1),
        ConcreteAction::RemoveLimitOrder(1),
        ConcreteAction::ExecuteLimitOrder(1),
    ];
    for v in variants {
        let bytes = candid::encode_args((v,)).unwrap();
        let (_decoded,): (Action,) = decode_bounded(&bytes).unwrap();
    }
    let Action::OneStepSwap(s) = t.action.clone() else {
        panic!()
    };
    let f = Refund {
        relatedIndex: t.id.clone(),
        transfer: s.deposit.transfer.clone(),
        status: WithdrawStatus::Completed,
        err: None,
    };
    for v in [ConcreteAction::OneStepSwap(s), ConcreteAction::Refund(f)] {
        let bytes = candid::encode_args((v,)).unwrap();
        let (decoded,): (Action,) = decode_bounded(&bytes).unwrap();
        t.action = decoded;
    }
}

#[test]
fn rejects_fee_input_minimum_and_transfer_failure_mutations() {
    let (r, c, t) = fixture();
    for mutation in 0..7 {
        let mut bad = t.clone();
        let Action::OneStepSwap(s) = &mut bad.action else {
            panic!()
        };
        match mutation {
            0 => s.deposit.transfer.amount = 9999u64.into(),
            1 => s.deposit.transfer.fee = 11u64.into(),
            2 => s.swap.amountInFee = 11u64.into(),
            3 => s.swap.amountOutFee = 21u64.into(),
            4 => {
                s.swap.amountOut = 8999u64.into();
                s.withdraw.transfer.amount = 8999u64.into();
            }
            5 => s.deposit.status = DepositStatus::TransferCompleted,
            _ => s.withdraw.err = Some("ledger transfer rejected".into()),
        }
        pending(&r, &c, &[bad]);
    }
}

fn refund_fixture(full_refund: bool) -> (SwapRequest, Cutoff, Vec<Transaction>) {
    let (r, c, mut t) = fixture();
    let Action::OneStepSwap(s) = &mut t.action else {
        panic!()
    };
    let amount = if full_refund {
        s.status = OneStepStatus::Failed;
        s.err = Some("Refund completed after failure".into());
        s.swap.status = SwapStatus::Failed;
        s.swap.err = Some("Slippage check failed".into());
        s.swap.amountOut = r.min_gross_output.into();
        s.withdraw.status = WithdrawStatus::Failed;
        s.withdraw.transfer.amount = r.min_gross_output.into();
        r.input
    } else {
        s.swap.amountIn = 6000u64.into();
        r.input - 6000
    };
    let mut transfer = s.deposit.transfer.clone();
    transfer.from.owner = r.pool;
    transfer.to.owner = r.owner;
    transfer.amount = amount.into();
    transfer.memo = Some(51u64.to_be_bytes().to_vec());
    transfer.index = 123u64.into();
    let refund = Transaction {
        id: 51u64.into(),
        timestamp: 201u64.into(),
        owner: r.owner,
        canisterId: r.pool,
        action: Action::Refund(Refund {
            relatedIndex: t.id.clone(),
            transfer,
            status: WithdrawStatus::Completed,
            err: None,
        }),
    };
    (r, c, vec![t, refund])
}
#[test]
fn partial_fill_and_confirmed_refund_conserve_input_and_fees() {
    let (r, c, txs) = refund_fixture(false);
    // Decode the actual full wire shape before reconciliation.
    let bytes = candid::encode_args((PoolResult::Ok(
        txs.iter()
            .map(|t| (t.id.clone(), t.clone()))
            .collect::<Vec<_>>(),
    ),))
    .unwrap();
    let (reply,): (PoolResult<Vec<(Nat, Transaction)>>,) = decode_bounded(&bytes).unwrap();
    let PoolResult::Ok(txs) = reply else { panic!() };
    let txs = txs.into_iter().map(|(_, t)| t).collect::<Vec<_>>();
    let ReceiptVerdict::Settled(p) = bind_receipt(&r, &c, &txs) else {
        panic!()
    };
    assert_eq!(
        (
            p.input_debit,
            p.effective_input,
            p.output_credit,
            p.refund_credit
        ),
        (10010, 6000, 9480, 3990)
    );
    assert_eq!(
        p.input_debit - p.refund_credit,
        p.effective_input + 2 * r.input_fee
    );
    assert_eq!(p.refund_block, Some(123u64.into()));
    let proof: Vec<Transaction> = candid::decode_one(&p.receipt_candid).unwrap();
    assert_eq!(proof, txs);
}
#[test]
fn failed_swap_with_complete_input_refund_proves_only_fee_loss() {
    let (r, c, txs) = refund_fixture(true);
    let ReceiptVerdict::Settled(p) = bind_receipt(&r, &c, &txs) else {
        panic!()
    };
    assert_eq!(
        (p.effective_input, p.output_credit, p.refund_credit),
        (0, 0, 9990)
    );
    assert_eq!(p.input_debit - p.refund_credit, 2 * r.input_fee);
    let mut unconfirmed = txs.clone();
    let Action::Refund(f) = &mut unconfirmed[1].action else {
        panic!()
    };
    f.status = WithdrawStatus::CreditCompleted;
    pending(&r, &c, &unconfirmed);
}
#[test]
fn refund_binding_rejects_missing_extra_wrong_identity_memo_amount_fee_or_stage() {
    for full in [false, true] {
        let (r, c, txs) = refund_fixture(full);
        pending(&r, &c, &txs[..1]);
        let mut duplicate = txs.clone();
        duplicate.push(txs[1].clone());
        pending(&r, &c, &duplicate);
        for mutation in 0..11 {
            let mut bad = txs.clone();
            match mutation {
                0 => bad[1].owner = Principal::anonymous(),
                1 => bad[1].canisterId = Principal::anonymous(),
                2 => bad[1].timestamp = 99u64.into(),
                _ => {
                    let Action::Refund(f) = &mut bad[1].action else {
                        panic!()
                    };
                    match mutation {
                        3 => f.transfer.token = r.token_out,
                        4 => f.transfer.to.subaccount = Some(vec![0; 32]),
                        5 => f.transfer.amount = 1u64.into(),
                        6 => f.transfer.fee = 11u64.into(),
                        7 => f.transfer.memo = Some(50u64.to_be_bytes().to_vec()),
                        8 => f.status = WithdrawStatus::CreditCompleted,
                        9 => f.transfer.index = 0u64.into(),
                        _ => f.relatedIndex = 49u64.into(),
                    }
                }
            }
            pending(&r, &c, &bad);
        }
    }
}
