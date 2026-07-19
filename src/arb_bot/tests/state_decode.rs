//! Upgrade-decode guard: state persisted by an older build (missing newer
//! config fields) must deserialize with the documented serde defaults.
//! This is the actual mechanism protecting production state across upgrades
//! (state is a serde_json blob in stable memory), so we prove it directly:
//! serialize a current BotState, strip the new fields to simulate an old
//! blob, and assert the defaults come back.

use arb_bot::state::{BotState, CycleSnapshot};
use candid::Principal;

#[test]
fn old_state_without_band_fields_decodes_with_defaults() {
    let mut v = serde_json::to_value(BotState::default()).expect("serialize");
    let cfg = v
        .get_mut("config")
        .and_then(|c| c.as_object_mut())
        .expect("config object");
    assert!(cfg.remove("icp_inventory_floor_e8s").is_some());
    assert!(cfg.remove("icp_inventory_ceiling_e8s").is_some());

    let decoded: BotState = serde_json::from_value(v).expect("decode old-shape state");
    assert_eq!(decoded.config.icp_inventory_floor_e8s, 200_000_000, "floor default = 2 ICP");
    assert_eq!(decoded.config.icp_inventory_ceiling_e8s, 2_000_000_000, "ceiling default = 20 ICP");
}

/// Same guard for the BOB inventory band: a blob saved before these fields
/// existed must decode with the documented 10 BOB / 40 BOB defaults.
#[test]
fn old_state_without_bob_band_fields_decodes_with_defaults() {
    let mut v = serde_json::to_value(BotState::default()).expect("serialize");
    let cfg = v
        .get_mut("config")
        .and_then(|c| c.as_object_mut())
        .expect("config object");
    assert!(cfg.remove("bob_inventory_floor_e8s").is_some());
    assert!(cfg.remove("bob_inventory_ceiling_e8s").is_some());

    let decoded: BotState = serde_json::from_value(v).expect("decode old-shape state");
    assert_eq!(decoded.config.bob_inventory_floor_e8s, 1_000_000_000, "floor default = 10 BOB");
    assert_eq!(decoded.config.bob_inventory_ceiling_e8s, 4_000_000_000, "ceiling default = 40 BOB");
}

/// Same guard for the nine Strategy S BotConfig fields: a pre-S blob must
/// decode with the documented defaults — mainnet-verified principals for the
/// BOB ledger and BOB/ICP pool, anonymous for the not-yet-existing icUSD/BOB
/// pool (the strategy's master gate), and dry-run-first execution disabled.
#[test]
fn old_state_without_bob_fields_decodes_with_defaults() {
    let mut v = serde_json::to_value(BotState::default()).expect("serialize");
    let cfg = v
        .get_mut("config")
        .and_then(|c| c.as_object_mut())
        .expect("config object");
    for field in [
        "bob_ledger",
        "bob_ledger_fee",
        "icpswap_bob_icp_pool",
        "icpswap_icusd_bob_pool",
        "icpswap_bob_icp_icp_is_token0",
        "icpswap_icusd_bob_icusd_is_token0",
        "bob_max_trade_size_usd",
        "bob_min_spread_bps",
        "bob_execution_enabled",
    ] {
        assert!(cfg.remove(field).is_some(), "field {field} present before strip");
    }

    let decoded: BotState = serde_json::from_value(v).expect("decode pre-BOB state");
    assert_eq!(
        decoded.config.bob_ledger,
        Principal::from_text("7pail-xaaaa-aaaas-aabmq-cai").unwrap(),
        "bob_ledger default = mainnet BOB ledger"
    );
    assert_eq!(decoded.config.bob_ledger_fee, 1_000_000, "bob fee default = 0.01 BOB");
    assert_eq!(
        decoded.config.icpswap_bob_icp_pool,
        Principal::from_text("ybilh-nqaaa-aaaag-qkhzq-cai").unwrap(),
        "bob/icp pool default = mainnet ICPSwap BOB/ICP"
    );
    assert_eq!(
        decoded.config.icpswap_icusd_bob_pool,
        Principal::anonymous(),
        "icusd/bob pool default = anonymous (master gate)"
    );
    assert!(!decoded.config.icpswap_bob_icp_icp_is_token0);
    assert!(!decoded.config.icpswap_icusd_bob_icusd_is_token0);
    assert_eq!(decoded.config.bob_max_trade_size_usd, 50_000_000, "max trade default = $50");
    assert_eq!(decoded.config.bob_min_spread_bps, 150);
    assert!(!decoded.config.bob_execution_enabled, "dry-run-first: execution off by default");
}

/// Same guard for CycleSnapshot, which is stored in its own StableLog
/// (serde_json via the `json_storable!` Storable impl) rather than inside
/// the BotState blob. Old snapshots — appended before Strategy S added its
/// four fields — must still decode, with those fields defaulting to 0.
#[test]
fn old_snapshot_without_strategy_s_fields_decodes_with_defaults() {
    let snapshot = CycleSnapshot {
        timestamp: 1,
        rumi_icp_price_3usd: 2,
        rumi_icp_price_usd: 3,
        icpswap_icp_price_ckusdc: 4,
        virtual_price: 5,
        spread_a_bps: 6,
        icpswap_icp_price_icusd: 7,
        spread_b_bps: 8,
        balance_icp: 9,
        balance_3usd: 10,
        balance_ckusdc: 11,
        balance_ckusdt: 12,
        balance_icusd: 13,
        icpswap_icp_price_ckusdt: 14,
        spread_c_bps: 15,
        spread_d_bps: 16,
        spread_f_bps: 17,
        spread_k_bps: 18,
        spread_l_bps: 19,
        spread_m_bps: 20,
        spread_n_bps: 21,
        spread_o_bps: 22,
        spread_p_bps: 23,
        spread_q_bps: 24,
        spread_r_bps: 25,
        partydex_icp_price_ckusdc: 26,
        partydex_icp_price_ckusdt: 27,
        bob_pool_price_icusd_per_bob: 999,
        bob_ref_price_icusd_per_bob: 999,
        spread_s_bps: 999,
        balance_bob: 999,
        traded: true,
        strategy_used: "A".to_string(),
    };

    let mut v = serde_json::to_value(&snapshot).expect("serialize");
    let obj = v.as_object_mut().expect("snapshot object");
    assert!(obj.remove("bob_pool_price_icusd_per_bob").is_some());
    assert!(obj.remove("bob_ref_price_icusd_per_bob").is_some());
    assert!(obj.remove("spread_s_bps").is_some());
    assert!(obj.remove("balance_bob").is_some());

    let decoded: CycleSnapshot = serde_json::from_value(v).expect("decode old-shape snapshot");
    assert_eq!(decoded.bob_pool_price_icusd_per_bob, 0);
    assert_eq!(decoded.bob_ref_price_icusd_per_bob, 0);
    assert_eq!(decoded.spread_s_bps, 0);
    assert_eq!(decoded.balance_bob, 0);
    // Sanity: an untouched pre-existing field still round-trips.
    assert_eq!(decoded.timestamp, 1);
    assert_eq!(decoded.strategy_used, "A");
}

/// Same guard for the volume bot's icUSD/BOB fields: a blob persisted before
/// the icUSD/BOB pool was added to `VolumeConfig` must still decode, with
/// `icusd_bob` defaulting inert (`enabled: false`) and `icusd_bob_state`
/// defaulting to `BuyBob` (not `VolumePoolState::default()`'s `BuyIcp` —
/// this pool's ping-pong never touches `BuyIcp`/`SellIcp`).
#[test]
fn old_state_without_icusd_bob_volume_fields_decodes_with_defaults() {
    use arb_bot::state::VolumeDirection;

    let mut v = serde_json::to_value(BotState::default()).expect("serialize");
    let volume = v
        .get_mut("volume")
        .and_then(|c| c.as_object_mut())
        .expect("volume object");
    assert!(volume.remove("icusd_bob").is_some());
    assert!(volume.remove("icusd_bob_state").is_some());

    let decoded: BotState = serde_json::from_value(v).expect("decode pre-icUSD/BOB volume state");
    assert!(!decoded.volume.icusd_bob.enabled, "icusd_bob ships inert");
    assert_eq!(decoded.volume.icusd_bob_state.next_direction, VolumeDirection::BuyBob);
    assert_eq!(decoded.volume.icusd_bob_state.trade_count, 0);
}

/// Same guard for `volume_stranded_bob` (top-level BotState field, added
/// alongside the icUSD/BOB hardening pass): a blob persisted before it
/// existed must still decode, with the balance defaulting to 0 (nothing
/// stranded) so `drain_residual_bob` doesn't withhold BOB from a fresh
/// upgrade that never had this field.
#[test]
fn old_state_without_volume_stranded_bob_decodes_with_default() {
    let mut v = serde_json::to_value(BotState::default()).expect("serialize");
    let obj = v.as_object_mut().expect("state object");
    assert!(obj.remove("volume_stranded_bob").is_some());

    let decoded: BotState = serde_json::from_value(v).expect("decode pre-volume_stranded_bob state");
    assert_eq!(decoded.volume_stranded_bob, 0);
}
