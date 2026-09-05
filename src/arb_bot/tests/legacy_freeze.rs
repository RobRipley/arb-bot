//! Tests for the Stage-1 legacy pending_exit/pending_bob_exit freeze —
//! see docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md Section 11.

use arb_bot::state::{BotState, LegacyFreezeAsset, PendingBobExit, PendingExit, Pool, BobPool, legacy_route_freeze_reason};

#[test]
fn clear_state_freezes_nothing() {
    let state = BotState::default();
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Icp).is_none());
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Bob).is_none());
}

#[test]
fn pending_exit_freezes_icp_only() {
    let mut state = BotState::default();
    state.pending_exit = Some(PendingExit {
        entry_pool: Pool::IcpswapCkusdc,
        intended_exit_pool: Pool::IcpswapIcusd,
        timestamp: 0,
        icp_amount: 0, // zero amount must still freeze — presence alone matters
    });
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Icp).is_some());
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Bob).is_none());
}

#[test]
fn pending_bob_exit_freezes_bob_only() {
    let mut state = BotState::default();
    state.pending_bob_exit = Some(PendingBobExit {
        entry_pool: BobPool::BobIcp,
        bob_amount: 0, // zero amount must still freeze
    });
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Bob).is_some());
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Icp).is_none());
}

#[test]
fn both_pending_incidents_freeze_both_assets() {
    let mut state = BotState::default();
    state.pending_exit = Some(PendingExit {
        entry_pool: Pool::IcpswapCkusdc,
        intended_exit_pool: Pool::IcpswapIcusd,
        timestamp: 0,
        icp_amount: 500_000_000,
    });
    state.pending_bob_exit = Some(PendingBobExit { entry_pool: BobPool::IcusdBob, bob_amount: 100 });
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Icp).is_some());
    assert!(legacy_route_freeze_reason(&state, LegacyFreezeAsset::Bob).is_some());
}
