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
    assert!(source.contains("ICPSwap ICP/ckUSDC\", config.icpswap_icp_is_token0, true, state.token_ordering_resolved"));
    assert!(source.contains("ICPSwap ICP/icUSD\", config.icpswap_icusd_icp_is_token0, true, state.icusd_token_ordering_resolved"));
    assert!(source.contains("ICPSwap ICP/ckUSDT\", config.icpswap_ckusdt_icp_is_token0, false, state.ckusdt_token_ordering_resolved"));
    assert!(source.contains("ICPSwap ICP/3USD\", config.icpswap_3usd_icp_is_token0, false, state.icpswap_3usd_token_ordering_resolved"));
    assert!(source.contains("ICPSwap BOB/ICP\", config.icpswap_bob_icp_icp_is_token0, false, state.bob_icp_ordering_resolved"));
    assert!(source.contains("!state.icusd_bob_ordering_resolved || config.icpswap_icusd_bob_icusd_is_token0"));
}

#[test]
fn volume_reservation_failure_retains_reconciliation_ownership() {
    let source = include_str!("../src/lib.rs");
    let start = source.find("async fn run_volume_cycle_if_unfrozen").unwrap();
    let body = &source[start..source[start..].find("\nfn require_admin").map(|n| start + n).unwrap()];
    assert!(body.contains("if let Err(error) = sync_volume_owned_reservations()"));
    assert!(body.contains("mark_mutation_lock_reconciliation_required(&operation_id)"));
    let failure = body.find("if let Err(error) = sync_volume_owned_reservations()").unwrap();
    let release = body.find("release_mutation_lock(&operation_id)").unwrap();
    assert!(failure < release, "reservation must persist before the global lock can release");

    let rebalance_start = source.find("async fn trigger_volume_rebalance").unwrap();
    let rebalance = &source[rebalance_start..source[rebalance_start..].find("\n// ─── Volume Bot Queries").map(|n| rebalance_start + n).unwrap()];
    assert!(rebalance.contains("if let Err(error) = sync_volume_owned_reservations()"));
    assert!(rebalance.contains("mark_mutation_lock_reconciliation_required(&operation_id)"));
}
