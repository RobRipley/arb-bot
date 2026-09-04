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
# Finally it runs an old-deployed -> new subtyping check via `didc check`
# against src/arb_bot/arb_bot.did.deployed. This is a DIFFERENT kind of
# safety than the checks above: field/method PRESENCE agreeing across the
# three sources says nothing about whether an EXISTING caller (an old
# dashboard, an external script, `dfx canister call`) can still safely call
# this interface. Candid subtyping requires new fields on an ARGUMENT type
# to be `opt`; `#[serde(default)]` only protects the internal stable-memory
# JSON blob, not the wire format of an inbound call — the two are unrelated
# mechanisms and neither substitutes for the other. See BotConfigInput's
# doc comment in state.rs for the concrete incident this check exists to
# prevent (found by independent review, 2026-09-04: a `set_config` /
# `InitArgs` signature that could no longer accept a payload from any
# caller still using the pre-Strategy-T interface).
#
# arb_bot.did.deployed must be updated (copied from arb_bot.did) as part of
# EVERY successful mainnet deploy — it is the source of truth for "what's
# actually live," which is the correct baseline for this check. It is not
# updated automatically by this script.
#
# CAVEAT (confirmed empirically, 2026-09-04): `didc check <new>.did
# <old>.did` on the two FULL service files does NOT validate the service
# CONSTRUCTOR's own argument type (`service : (InitArgs) -> {...}`) — only
# ordinary method arguments/returns. A regression that reverts InitArgs to
# the old, incompatible shape passes `didc check` on the full files with
# no error. To get real coverage on InitArgs, this script also extracts
# InitArgs (and whatever type it references) from each .did and wraps it
# as an ordinary method on a throwaway shim service, then runs `didc
# check` on THAT pair too — didc's method-argument subtyping check does
# catch it there. See the init-args-shim section below.
#
# RESIDUAL GAP: the shim validates the SHAPE of the type currently named
# `InitArgs` in each file — it does not confirm the `service : (X) -> {`
# constructor still actually binds to a type named `InitArgs` (e.g. it
# would not catch the constructor argument being swapped for some other
# type entirely, name and all). That narrower case is already covered
# from the Rust side by the `cargo test` above (`service_equal` compares
# Class/init args between the generated interface and the committed
# `.did` directly), so this script does not duplicate it here.
#
# Usage:
#   scripts/check-candid.sh              # grep-diff the 3 sources + run cargo test + subtyping check
#   scripts/check-candid.sh --no-cargo   # grep-diff + subtyping check only (fast, no build)
#
# Exit status is non-zero if any drift, or any unsafe subtyping change, is
# found. No network or CI required (didc runs entirely locally).

set -uo pipefail

# Resolve repo root from this script's location so it runs from anywhere.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

RUST_LIB="$ROOT/src/arb_bot/src/lib.rs"
RUST_STATE="$ROOT/src/arb_bot/src/state.rs"
DID="$ROOT/src/arb_bot/arb_bot.did"
DASH="$ROOT/src/arb_bot/src/dashboard.html"
DID_DEPLOYED="$ROOT/src/arb_bot/arb_bot.did.deployed"

for f in "$RUST_LIB" "$RUST_STATE" "$DID" "$DASH" "$DID_DEPLOYED"; do
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

# Names of fields declared `Option<...>` inside a Rust `pub struct <Name> {`
# block — the argument-boundary-type equivalent of an `opt` candid field.
rust_struct_opt_fields() { # $1=struct name  $2=file
  awk -v s="pub struct $1 {" '
    index($0, s) { inb = 1; next }
    inb && /^}/  { inb = 0 }
    inb' "$2" \
  | grep -oE '^[[:space:]]*pub [a-z_][a-z0-9_]*: Option<' \
  | sed -E 's/^[[:space:]]*pub //; s/: Option<$//' | sort -u
}

# Names of fields declared `opt ...` inside a candid
# `type <Name> = record { ... };` block.
did_record_opt_fields() { # $1=type name  $2=file
  awk -v s="type $1 = record {" '
    index($0, s) { inb = 1; next }
    inb && /^};/ { inb = 0 }
    inb' "$2" \
  | grep -oE '^[[:space:]]*[a-z_][a-z0-9_]*: opt ' \
  | sed -E 's/[[:space:]]*//; s/: opt .*$//' | sort -u
}

# Names of fields wrapped `IDL.Opt(...)` inside a dashboard
# `const <Name> = IDL.Record({ ... });` literal.
dash_record_opt_fields() { # $1=const name  $2=file
  awk -v s="const $1 = IDL.Record({" '
    index($0, s) { inb = 1 }
    inb          { print }
    inb && /}\);/ { inb = 0 }
  ' "$2" \
  | grep -oE '[a-z_][a-z0-9_]*[[:space:]]*:[[:space:]]*IDL\.Opt\(' \
  | sed -E 's/[[:space:]]*:.*$//' | sort -u
}

# Contents of the dashboard's `const STRATEGY_T_OPT_FIELDS = [...]` literal
# — the SECOND, independently hand-maintained list of the same field names,
# used at config-save time to wrap values for the wire. Must stay identical
# to dash_record_opt_fields(BotConfigInput, ...) or the dashboard's own
# save flow can silently mis-encode.
dash_opt_fields_array() { # $1=file
  awk '
    /const STRATEGY_T_OPT_FIELDS = \[/ { inb = 1 }
    inb                                { print }
    inb && /\];/                       { inb = 0; exit }
  ' "$1" \
  | grep -oE "'[a-z_][a-z0-9_]*'" \
  | tr -d "'" | sort -u
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

compare3 "BotConfigInput fields" \
  "$(rust_struct_fields BotConfigInput "$RUST_STATE")" \
  "$(did_record_fields BotConfigInput "$DID")" \
  "$(dash_record_fields BotConfigInput "$DASH")"

compare3 "BotConfigInput opt-ness" \
  "$(rust_struct_opt_fields BotConfigInput "$RUST_STATE")" \
  "$(did_record_opt_fields BotConfigInput "$DID")" \
  "$(dash_record_opt_fields BotConfigInput "$DASH")"

# Dashboard-internal: STRATEGY_T_OPT_FIELDS (used at config-save time to
# wire-wrap values) is a second, independent list of the same field names
# hand-maintained in the same file — not one of the 3 cross-source pairs,
# but a real drift risk on its own (see the finding this guards against:
# a silent JS encode failure in the dashboard's config-save flow).
DASH_IDL_OPT="$(dash_record_opt_fields BotConfigInput "$DASH")"
DASH_ARRAY_OPT="$(dash_opt_fields_array "$DASH")"
if [[ "$DASH_IDL_OPT" == "$DASH_ARRAY_OPT" ]]; then
  n=$(printf '%s\n' "$DASH_IDL_OPT" | grep -c . || true)
  printf '  ok   %-28s (%s entries, IDL def and array agree)\n' "STRATEGY_T_OPT_FIELDS" "$n"
else
  fail=1
  printf '  DRIFT %-28s — BotConfigInput IDL opt-fields and the hand-written\n' "STRATEGY_T_OPT_FIELDS"
  echo "        STRATEGY_T_OPT_FIELDS array disagree — the dashboard's own"
  echo "        config-save flow will mis-wrap or miss a field:"
  diff <(printf '%s\n' "$DASH_IDL_OPT") <(printf '%s\n' "$DASH_ARRAY_OPT") | sed 's/^/        /'
fi

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

# ── Old-deployed -> new subtyping check (didc) ────────────────────────────
# Presence/absence agreement across the 3 sources (above) is a different
# question from "can an existing caller still call this interface" — see
# the header comment for why #[serde(default)] doesn't answer the latter.
echo
echo "== didc check: arb_bot.did.deployed -> arb_bot.did is a safe subtype =="
if ! command -v didc >/dev/null 2>&1; then
  echo "FAIL: didc not found on PATH. Install: cargo install didc" >&2
  echo "      (or: https://github.com/dfinity/candid/releases)" >&2
  fail=1
elif didc check "$DID" "$DID_DEPLOYED"; then
  echo "PASS: arb_bot.did is backward-compatible with the currently-deployed interface."
else
  echo "FAIL: arb_bot.did is NOT a safe subtype of arb_bot.did.deployed (see didc output above)." >&2
  echo "      A caller using the currently-deployed interface could fail to call this canister." >&2
  echo "      Fix: for an ARGUMENT type, only add opt fields, never required ones — see the" >&2
  echo "      BotConfigInput pattern in state.rs. For a RETURN type, adding required fields is safe." >&2
  fail=1
fi

# ── InitArgs constructor-argument subtyping check (didc, via a shim) ─────
# See the CAVEAT in the header: `didc check` on the two full service files
# does not validate the constructor's own argument type. Extract InitArgs
# (plus whatever type it references) from each .did and re-check it as an
# ordinary method argument on a throwaway shim service instead.

# Prints a `type <name> = record {...};` block verbatim.
did_type_block() { # $1=type name  $2=file
  awk -v s="type $1 = record {" '
    index($0, s) { inb = 1 }
    inb          { print }
    inb && /^};/ { inb = 0; exit }
  ' "$2"
}

# InitArgs plus every custom (capitalized) type it references, wrapped as
# an ordinary method on a throwaway service so didc's method-argument
# subtyping check (which DOES work) applies to it.
init_args_shim() { # $1=file
  local init_block dep
  init_block="$(did_type_block InitArgs "$1")"
  if [[ -z "$init_block" ]]; then
    echo "FATAL: no 'type InitArgs = record { ... };' block found in $1" >&2
    return 1
  fi
  dep="$(printf '%s\n' "$init_block" | grep -oE ':[[:space:]]*[A-Z][A-Za-z0-9_]*' | tr -d ' :' | sort -u)"
  local d
  for d in $dep; do
    did_type_block "$d" "$1"
  done
  printf '%s\n' "$init_block"
  echo "service : { __init_shim: (InitArgs) -> (); };"
}

echo
echo "== didc check: InitArgs constructor argument is a safe subtype =="
if ! command -v didc >/dev/null 2>&1; then
  echo "FAIL: didc not found on PATH (see above)." >&2
  fail=1
else
  NEW_SHIM="$(mktemp)"
  OLD_SHIM="$(mktemp)"
  trap 'rm -f "$NEW_SHIM" "$OLD_SHIM"' EXIT
  if ! init_args_shim "$DID" > "$NEW_SHIM"; then
    fail=1
  elif ! init_args_shim "$DID_DEPLOYED" > "$OLD_SHIM"; then
    fail=1
  elif didc check "$NEW_SHIM" "$OLD_SHIM"; then
    echo "PASS: InitArgs is backward-compatible with the currently-deployed interface."
  else
    echo "FAIL: InitArgs is NOT a safe subtype of the deployed InitArgs (see didc output above)." >&2
    echo "      A fresh 'dfx canister install' from an old caller's InitArgs payload would fail." >&2
    fail=1
  fi
fi

exit "$fail"
