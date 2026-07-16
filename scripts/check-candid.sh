#!/usr/bin/env bash
#
# check-candid.sh — local guard against Candid interface drift.
#
# The canister's Candid interface is kept in sync BY HAND across three sources:
#   1. Rust      — src/arb_bot/src/lib.rs (#[update]/#[query]) + src/arb_bot/src/state.rs (structs)
#   2. .did      — src/arb_bot/arb_bot.did
#   3. dashboard — src/arb_bot/src/dashboard.html (the `IDL.*` / `I.Service` blocks)
#
# A mismatch produces a SILENT candid decode trap on mainnet that nothing
# catches at build time. This script guards the two highest-drift surfaces —
# the per-strategy execute/dry-run method sets, the BotConfig fields, and the
# CycleSnapshot fields — by extracting each from all three sources and diffing.
#
# It also runs the Rust<->.did equality test (tests/candid.rs), which uses
# candid's own type machinery for full rigor on the Rust/.did pair (something a
# grep-diff can't do), but which cannot see the hand-written dashboard IDL.
#
# Usage:
#   scripts/check-candid.sh              # grep-diff the 3 sources + run cargo test
#   scripts/check-candid.sh --no-cargo   # grep-diff only (fast, no build)
#
# Exit status is non-zero if any drift is found. No network or CI required.

set -uo pipefail

# Resolve repo root from this script's location so it runs from anywhere.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

RUST_LIB="$ROOT/src/arb_bot/src/lib.rs"
RUST_STATE="$ROOT/src/arb_bot/src/state.rs"
DID="$ROOT/src/arb_bot/arb_bot.did"
DASH="$ROOT/src/arb_bot/src/dashboard.html"

for f in "$RUST_LIB" "$RUST_STATE" "$DID" "$DASH"; do
  if [[ ! -f "$f" ]]; then
    echo "FATAL: expected source not found: $f" >&2
    exit 2
  fi
done

RUN_CARGO=1
[[ "${1:-}" == "--no-cargo" ]] && RUN_CARGO=0

fail=0

# ── extractors ────────────────────────────────────────────────────────────
# Each prints a sorted, de-duplicated, newline-separated list.

# Trailing letters of "<prefix>_<letter>" method identifiers in a file.
strategy_letters() { # $1=prefix  $2=file
  grep -oE "$1"'_[a-z]\b' "$2" | sed -E 's/.*_//' | sort -u
}

# Same, but reading text from stdin — used to scope the dashboard scan to its
# candid `I.Service({...})` declaration and ignore actor call-sites elsewhere
# in the file (which mention the same method names but don't drive decoding).
strategy_letters_stdin() { # $1=prefix
  grep -oE "$1"'_[a-z]\b' | sed -E 's/.*_//' | sort -u
}

# The dashboard's canister `return I.Service({ ... });` block (the first one —
# the bot service; a second I.Service defines the ICRC-1 ledger and is skipped).
dash_service_block() { # $1=file
  awk '
    /return I\.Service\(\{/ { inb = 1 }
    inb                     { print }
    inb && /\}\);/          { inb = 0; exit }
  ' "$1"
}

# `pub <name>:` field names inside a Rust `pub struct <Name> {` block.
rust_struct_fields() { # $1=struct name  $2=file
  awk -v s="pub struct $1 {" '
    index($0, s) { inb = 1; next }
    inb && /^}/  { inb = 0 }
    inb' "$2" \
  | grep -oE '^[[:space:]]*pub [a-z_][a-z0-9_]*:' \
  | sed -E 's/^[[:space:]]*pub //; s/:.*$//' | sort -u
}

# `<name>:` field names inside a candid `type <Name> = record { ... };` block.
did_record_fields() { # $1=type name  $2=file
  awk -v s="type $1 = record {" '
    index($0, s) { inb = 1; next }
    inb && /^};/ { inb = 0 }
    inb' "$2" \
  | grep -oE '^[[:space:]]*[a-z_][a-z0-9_]*:' \
  | sed -E 's/[[:space:]]//g; s/:$//' | sort -u
}

# Keys of a dashboard `const <Name> = IDL.Record({ ... });` literal
# (single- or multi-line). These records are flat (no nested IDL.Record), so
# every `key:` is a field name.
dash_record_fields() { # $1=const name  $2=file
  awk -v s="const $1 = IDL.Record({" '
    index($0, s) { inb = 1 }
    inb          { print }
    inb && /}\);/ { inb = 0 }
  ' "$2" \
  | grep -oE '[a-z_][a-z0-9_]*[[:space:]]*:' \
  | sed -E 's/[[:space:]]//g; s/:$//' \
  | grep -vxE 'IDL|const' | sort -u
}

# ── comparison ────────────────────────────────────────────────────────────
# compare3 <label> <rust-list> <did-list> <dash-list>
compare3() {
  local label="$1" rust="$2" did="$3" dash="$4"
  if [[ "$rust" == "$did" && "$did" == "$dash" ]]; then
    local n; n=$(printf '%s\n' "$rust" | grep -c . || true)
    printf '  ok   %-28s (%s entries, all 3 sources agree)\n' "$label" "$n"
    return 0
  fi
  fail=1
  printf '  DRIFT %-28s — sources disagree:\n' "$label"
  # Show the union with a per-source presence marker (R=rust .did=D H=dashboard).
  local union
  union=$(printf '%s\n%s\n%s\n' "$rust" "$did" "$dash" | grep . | sort -u)
  printf '        %-24s  rust  .did  dash\n' "entry"
  while IFS= read -r item; do
    [[ -z "$item" ]] && continue
    local r d h
    grep -qxF "$item" <<<"$rust" && r=" R " || r=" . "
    grep -qxF "$item" <<<"$did"  && d=" D " || d=" . "
    grep -qxF "$item" <<<"$dash" && h=" H " || h=" . "
    printf '        %-24s  %s   %s  %s\n' "$item" "$r" "$d" "$h"
  done <<<"$union"
}

echo "== Candid 3-way drift check (Rust / .did / dashboard) =="

DASH_SERVICE="$(dash_service_block "$DASH")"

compare3 "execute_strategy_* letters" \
  "$(strategy_letters execute_strategy "$RUST_LIB")" \
  "$(strategy_letters execute_strategy "$DID")" \
  "$(printf '%s\n' "$DASH_SERVICE" | strategy_letters_stdin execute_strategy)"

compare3 "dry_run_strategy_* letters" \
  "$(strategy_letters dry_run_strategy "$RUST_LIB")" \
  "$(strategy_letters dry_run_strategy "$DID")" \
  "$(printf '%s\n' "$DASH_SERVICE" | strategy_letters_stdin dry_run_strategy)"

compare3 "BotConfig fields" \
  "$(rust_struct_fields BotConfig "$RUST_STATE")" \
  "$(did_record_fields BotConfig "$DID")" \
  "$(dash_record_fields BotConfig "$DASH")"

compare3 "CycleSnapshot fields" \
  "$(rust_struct_fields CycleSnapshot "$RUST_STATE")" \
  "$(did_record_fields CycleSnapshot "$DID")" \
  "$(dash_record_fields CycleSnapshot "$DASH")"

echo
if [[ "$fail" -ne 0 ]]; then
  echo "FAIL: dashboard/Rust/.did drift detected above. Reconcile the three sources by hand." >&2
else
  echo "PASS: strategy method sets, BotConfig, and CycleSnapshot agree across all 3 sources."
fi

# ── Rust <-> .did equality test (full candid rigor) ───────────────────────
if [[ "$RUN_CARGO" -eq 1 ]]; then
  echo
  echo "== cargo test: Rust <-> arb_bot.did structural equality =="
  if cargo test -p arb_bot --test candid --manifest-path "$ROOT/Cargo.toml"; then
    echo "PASS: generated candid matches arb_bot.did."
  else
    echo "FAIL: Rust <-> arb_bot.did drift (see cargo output above)." >&2
    fail=1
  fi
else
  echo
  echo "(skipped cargo test — run without --no-cargo for full Rust<->.did rigor)"
fi

exit "$fail"
