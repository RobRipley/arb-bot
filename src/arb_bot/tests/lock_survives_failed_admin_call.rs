//! A failed admin transfer must never leave the durable account-mutation
//! lock behind.
//!
//! Every volume cycle and route execution takes that lock first, so a lock
//! that outlives its operation halts the whole bot. Two things used to cause
//! exactly that:
//!
//! * `ic_cdk::trap` after the lock was committed. A trap discards everything
//!   the current callback did — including a `release_mutation_lock` made just
//!   before it — while the acquisition, made in an earlier message, stays.
//!   The lock was then held, unflagged, with nothing able to release it.
//! * Treating a certain no-op as an ambiguous outcome. When a ledger answers
//!   with an explicit refusal (`InsufficientFunds`, `BadFee`, …), or the call
//!   could not be sent at all (too few cycles to reserve for the reply, a full
//!   queue), nothing moved and there is nothing to reconcile, yet the lock was
//!   flagged reconciliation-required. The second case is the likely cause of
//!   the 2026-10-01 incident: the canister was at its freezing threshold.

const LIB: &str = include_str!("../src/lib.rs");
const SWAPS: &str = include_str!("../src/swaps.rs");

/// Every free function in lib.rs as (name, body).
fn functions(source: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut offset = 0;
    while let Some(at) = source[offset..].find("\nasync fn ").or_else(|| source[offset..].find("\nfn ")) {
        // Take whichever of the two forms comes first.
        let a = source[offset..].find("\nasync fn ");
        let b = source[offset..].find("\nfn ");
        let at = match (a, b) { (Some(a), Some(b)) => a.min(b), _ => at };
        let start = offset + at + 1;
        let end = source[start..].find("\n}\n").map(|n| start + n + 2).unwrap_or(source.len());
        let body = &source[start..end];
        let name = body.split("fn ").nth(1).and_then(|t| t.split(['(', '<']).next()).unwrap().trim();
        found.push((name, body));
        offset = end;
    }
    found
}

fn function<'a>(name: &str) -> &'a str {
    functions(LIB)
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(_, body)| body)
        .unwrap_or_else(|| panic!("lib.rs has no fn {name}"))
}

#[test]
fn nothing_traps_once_it_holds_the_lock() {
    let mut holders = Vec::new();
    for (name, body) in functions(LIB) {
        let Some(acquire) = body.find("acquire_mutation_lock") else { continue };
        // The acquire statement itself may trap: failing to take the lock
        // leaves nothing held.
        let statement_end = acquire + body[acquire..].find(";\n").expect("acquire statement");
        let held = &body[statement_end..];
        assert!(
            !held.contains("ic_cdk::trap("),
            "{name} traps after acquiring the account-mutation lock; a trap rolls back the release and wedges the bot"
        );
        holders.push(name);
    }
    for expected in [
        "withdraw", "volume_swap", "fund_volume_subaccount", "withdraw_volume_subaccount",
        "recover_partydex_balance", "trigger_volume_rebalance", "run_volume_cycle_if_unfrozen",
    ] {
        assert!(holders.contains(&expected), "scan missed lock holder {expected}: {holders:?}");
    }
    // The work done under the lock lives in these helpers; they must not trap either.
    for helper in ["withdraw_under_lock", "volume_swap_under_lock"] {
        assert!(!function(helper).contains("ic_cdk::trap("), "{helper} must report failure by returning Err");
    }
}

#[test]
fn withdraw_and_volume_swap_report_failure_with_a_reject_not_a_trap() {
    for name in ["withdraw", "volume_swap"] {
        let attribute = format!("#[update(manual_reply = true)]\nasync fn {name}(");
        assert!(LIB.contains(&attribute), "{name} must reply manually so it can reject without trapping");
        let body = function(name);
        assert!(body.contains(&format!("{name}_under_lock(")));
        assert!(body.contains("ic_cdk::api::call::reject(&message)"), "{name} must surface the error text to the caller");
        assert!(body.contains("ic_cdk::api::call::reply(())"), "{name} still replies with no values on success");
        // No declared return type: the exported candid signature stays `-> ()`.
        let signature = &body[..body.find('{').unwrap()];
        assert!(!signature.contains("->"), "{name} must keep its `-> ()` interface: {signature}");
    }
}

#[test]
fn withdraw_releases_the_lock_on_every_failure_that_moved_nothing() {
    let body = function("withdraw_under_lock");
    for failure in [
        "withdraw balance check failed",
        "withdraw rejected: amount would consume reserved inventory",
        "Transfer failed: ",
    ] {
        let at = body.find(failure).unwrap_or_else(|| panic!("missing failure path: {failure}"));
        let before = &body[..at];
        let release = before.rfind("release_mutation_lock(operation_id)").expect("release before reporting");
        let mark = before.rfind("mark_mutation_lock_reconciliation_required").unwrap_or(0);
        assert!(release > mark, "`{failure}` moved nothing and must release, not flag");
    }
    // The reservation check itself can fail; that path used to trap without
    // releasing at all.
    let spendable = body.find("state::spendable_native(").unwrap();
    let after = &body[spendable..];
    assert!(after[..after.find("if amount > available").unwrap()].contains("release_mutation_lock(operation_id)"));
    // A refusal and a call that was never sent are both certain no-ops.
    let no_op = body.find("is_certain_no_op()").expect("withdraw must recognise a certain no-op");
    assert!(no_op < body.find("Transfer failed: ").unwrap());
    // Only a transfer that was sent and whose outcome is unknown keeps the
    // lock for reconciliation.
    let ambiguous = body.find("Transfer call failed: ").unwrap();
    let mark = body.rfind("mark_mutation_lock_reconciliation_required").unwrap();
    assert!(no_op < mark && mark < ambiguous && body[mark..ambiguous].matches("release_mutation_lock").count() == 0);
}

#[test]
fn a_certain_no_op_releases_the_subaccount_transfer_locks() {
    for name in ["fund_volume_subaccount", "withdraw_volume_subaccount"] {
        let body = function(name);
        let no_op = body.find("is_certain_no_op()").unwrap_or_else(|| panic!("{name} must recognise a certain no-op"));
        let mark = body.find("mark_mutation_lock_reconciliation_required").expect("ambiguous failures still flag");
        assert!(no_op < mark, "{name}: the no-op arm comes first");
        assert!(
            body[no_op..mark].contains("release_mutation_lock(&operation_id)"),
            "{name}: a transfer that moved nothing must release the lock"
        );
        assert!(body.matches("log_activity").count() >= 2, "{name}: both failure kinds are recorded");
    }
}

#[test]
fn every_transfer_is_classified_by_the_one_helper() {
    for helper in ["pub async fn transfer_from_subaccount(", "pub async fn transfer_to_subaccount("] {
        let start = SWAPS.find(helper).unwrap();
        let body = &SWAPS[start..start + SWAPS[start..].find("\n}\n").unwrap()];
        assert!(body.contains("icrc1_transfer(token_ledger, args).await"), "{helper}");
        assert!(!body.contains("ic_cdk::call("), "{helper} must not classify transfer outcomes itself");
    }
    assert!(function("withdraw_under_lock").contains("swaps::icrc1_transfer(token_ledger, transfer_args).await"));
    assert_eq!(LIB.matches("\"icrc1_transfer\"").count(), 0, "lib.rs must not send raw transfers");

    let start = SWAPS.find("pub async fn icrc1_transfer(").unwrap();
    let body = &SWAPS[start..start + SWAPS[start..].find("\n}\n").unwrap()];
    let first_poll = body.find("futures::poll!(").expect("the first poll is what proves a call was never sent");
    let not_sent = body.find("SwapError::NotSent(").unwrap();
    let awaited = body.find("call.await").unwrap();
    assert!(first_poll < not_sent && not_sent < awaited);
    assert!(body.contains("transfer_result_from_ledger("));
}

#[test]
fn a_volume_swap_that_got_past_its_first_transfer_never_frees_the_lock_on_failure() {
    let body = function("volume_swap_under_lock");
    // Once the input has left the volume subaccount, a failed step cannot
    // prove where the value is: the venue may have taken it, and a refund is
    // then paid from route-owned inventory. Only full success releases.
    assert_eq!(body.matches("release_mutation_lock(").count(), 1, "the only release is the success tail");
    let release = body.find("release_mutation_lock(").unwrap();
    assert!(body[release..].contains("Ok(())") && !body[release..].contains("return Err"));
    assert_eq!(body.matches("volume_swap_step_failed(").count(), 4, "both later steps, both directions");
    let step_failed = function("volume_swap_step_failed");
    assert!(step_failed.contains("mark_mutation_lock_reconciliation_required") && !step_failed.contains("release_mutation_lock"));
    // Its first transfer is different: a certain no-op there moved nothing.
    let first = function("volume_swap_source_transfer_failed");
    let no_op = first.find("is_certain_no_op()").unwrap();
    assert!(first[no_op..].contains("release_mutation_lock(operation_id)"));
    // A swap whose output could not be returned must not report success.
    assert!(body.matches("lock held for reconciliation").count() >= 1);
    let tail = &body[body.rfind("if settlement_ambiguous").unwrap()..];
    assert!(tail.contains("return Err("), "an unreturned output is reported as a failure, not a success");
    // Dust that cannot cover the fees is refused before the lock is taken.
    let entry = function("volume_swap");
    assert!(entry.find("amount too small").unwrap() < entry.find("acquire_mutation_lock").unwrap());
}

#[test]
fn the_fund_lock_id_is_built_from_the_prefix_the_recovery_guard_checks() {
    assert!(function("fund_volume_subaccount").contains("state::VOLUME_FUND_OPERATION_PREFIX"));
    let literal_uses = LIB.matches("\"volume-fund-").count() + include_str!("../src/state.rs").matches("\"volume-fund-").count();
    assert_eq!(literal_uses, 1, "the prefix must be spelled once, in the constant");
}
