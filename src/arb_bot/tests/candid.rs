//! Candid drift guard: Rust ↔ arb_bot.did
//!
//! `arb_bot::generated_candid_interface()` (defined at the bottom of
//! `src/lib.rs` via `candid::export_service!`) produces a candid service
//! description straight from the `#[update]`/`#[query]` signatures the canister
//! actually exports. This test asserts that service is structurally equal to
//! the committed `arb_bot.did`. If someone adds, removes, renames, or retypes a
//! method / config field / snapshot field in Rust without updating the `.did`
//! (or vice versa), this test fails — instead of the drift surviving to mainnet
//! as a silent candid decode trap.
//!
//! `service_equal` compares structure (bidirectional subtyping), so field and
//! method ordering and type *names* are irrelevant — only the shape matters.
//!
//! Run via `scripts/check-candid.sh` (which also diffs the dashboard IDL that
//! this test cannot see), or directly with `cargo test -p arb_bot --test candid`.

use candid_parser::utils::{service_equal, CandidSource};

#[test]
fn candid_interface_matches_committed_did() {
    // Generated from the live Rust #[update]/#[query] signatures.
    let generated = arb_bot::generated_candid_interface();
    // The committed, hand-maintained interface that clients rely on.
    let committed = include_str!("../arb_bot.did");

    if let Err(e) = service_equal(
        CandidSource::Text(&generated),
        CandidSource::Text(committed),
    ) {
        panic!(
            "Rust <-> arb_bot.did candid drift detected:\n  {e}\n\n\
             The service generated from the Rust #[update]/#[query] signatures no \
             longer matches src/arb_bot/arb_bot.did.\n\
             Reconcile the two by hand (add/remove/retype the offending method or \
             field). To dump the currently-generated interface, run:\n\
             \n    cargo test -p arb_bot --test candid print_generated_candid -- --ignored --nocapture\n"
        );
    }
}

/// Convenience: prints the interface currently generated from the Rust
/// signatures. Ignored by default; run with `--ignored` to dump it when
/// reconciling drift against arb_bot.did.
#[test]
#[ignore]
fn print_generated_candid() {
    println!("{}", arb_bot::generated_candid_interface());
}
