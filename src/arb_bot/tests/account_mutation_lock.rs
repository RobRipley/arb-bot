#[test]
fn every_surviving_shared_account_entrypoint_mentions_the_durable_lock() {
    let source = include_str!("../src/lib.rs");
    for name in [
        "withdraw", "recover_partydex_balance", "volume_swap",
        "fund_volume_subaccount", "withdraw_volume_subaccount",
        "run_volume_cycle_if_unfrozen", "trigger_volume_rebalance",
    ] {
        let needle = if name == "run_volume_cycle_if_unfrozen" {
            format!("async fn {name}")
        } else {
            format!("fn {name}")
        };
        let start = source.find(&needle).unwrap_or_else(|| panic!("missing {name}"));
        let tail = &source[start..];
        let end = tail.find("\n#[").unwrap_or(tail.len());
        let body = &tail[..end];
        assert!(body.contains("acquire_mutation_lock"), "{name} must acquire the durable lock");
    }
    let clear = source.split("fn clear_cycle_lock").nth(1).unwrap().split("\n#[").next().unwrap();
    assert!(!clear.contains("release_mutation_lock"));
}

#[test]
fn caller_supplied_targets_are_checked_against_code_pins_before_calls() {
    let source = include_str!("../src/lib.rs");
    assert!(source.contains("withdraw rejected: ledger is not in the immutable active/recovery registry"));
    assert!(source.contains("PartyDEX recovery rejected: pool is not one of the two immutable retired recovery pins"));
    assert!(source.contains("volume operation disabled: Rumi 3pool does not match immutable pin"));
}
