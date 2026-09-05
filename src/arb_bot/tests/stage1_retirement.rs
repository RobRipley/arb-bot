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

/// Extracts the body of `fn setup_timer() { ... }` (brace-matched) from
/// lib.rs, for structural assertions about what it does and does not do.
fn setup_timer_body(source: &str) -> String {
    let start = source.find("fn setup_timer() {").expect("find setup_timer");
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
    let body = setup_timer_body(&lib_rs_source());
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
