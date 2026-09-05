//! Structural regression tests for PR #22 Stage 1 retirement — see
//! docs/superpowers/specs/2026-09-04-six-asset-route-arbitrage-policy-design.md.
//! These are grep-based structural checks, not live-canister tests,
//! matching this codebase's existing testing conventions (no
//! ic_cdk::call mocking infrastructure exists).

use std::fs;

fn lib_rs_source() -> String {
    fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("read lib.rs")
}

fn arb_rs_source() -> String {
    fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/arb.rs"))
        .expect("read arb.rs")
}

/// Extracts a named function body from lib.rs using brace matching.
fn function_body(source: &str, signature: &str) -> String {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("find {signature}"));
    let body_start = start + source[start..].find('{').unwrap();
    let bytes = source.as_bytes();
    let mut depth = 0i32;
    let mut end = body_start;
    for (i, &b) in bytes[body_start..].iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = body_start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    source[body_start..end].to_string()
}

#[test]
fn setup_timer_never_registers_the_automatic_arb_cycle() {
    let body = function_body(&lib_rs_source(), "fn setup_timer() {");
    assert!(
        !body.contains("set_timer_interval"),
        "setup_timer() must never register a periodic timer under Stage-1 retirement; found set_timer_interval in its body"
    );
    assert!(
        !body.contains("run_arb_cycle"),
        "setup_timer() must never reference run_arb_cycle under Stage-1 retirement"
    );
}

#[test]
fn automatic_volume_timer_cannot_bypass_legacy_asset_freezes() {
    let source = lib_rs_source();
    let timer = function_body(&source, "fn setup_volume_timer() {");
    assert!(
        timer.contains("run_volume_cycle_if_unfrozen"),
        "the automatic volume timer must use the same legacy-freeze gate as the manual trigger"
    );
    assert!(
        !timer.contains("volume::run_volume_cycle"),
        "the automatic volume timer must not call run_volume_cycle directly"
    );

    let guarded_runner = function_body(&source, "async fn run_volume_cycle_if_unfrozen()");
    assert!(guarded_runner.contains("LegacyFreezeAsset::Icp"));
    assert!(guarded_runner.contains("LegacyFreezeAsset::Bob"));
    assert!(guarded_runner.contains("volume::run_volume_cycle().await"));
}

#[test]
fn drain_residual_functions_are_deleted() {
    let source = arb_rs_source();
    assert!(
        !source.contains("fn drain_residual_icp"),
        "drain_residual_icp must be deleted under Stage-1 retirement, not merely unreachable"
    );
    assert!(
        !source.contains("fn drain_residual_bob"),
        "drain_residual_bob must be deleted under Stage-1 retirement, not merely unreachable"
    );
}
