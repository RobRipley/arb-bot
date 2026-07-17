//! Upgrade-decode guard: state persisted by an older build (missing newer
//! config fields) must deserialize with the documented serde defaults.
//! This is the actual mechanism protecting production state across upgrades
//! (state is a serde_json blob in stable memory), so we prove it directly:
//! serialize a current BotState, strip the new fields to simulate an old
//! blob, and assert the defaults come back.

use arb_bot::state::BotState;

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
