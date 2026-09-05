#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIB="$ROOT/src/arb_bot/src/lib.rs"
ROUTER="$ROOT/src/arb_bot/src/route_arb.rs"

required_pins=(
  ryjl3-tyaaa-aaaaa-aaaba-cai t6bor-paaaa-aaaap-qrd5q-cai
  cngnf-vqaaa-aaaar-qag4q-cai xevnm-gaaaa-aaaar-qafnq-cai
  mxzaz-hqaaa-aaaar-qaada-cai ss2fx-dyaaa-aaaar-qacoq-cai
  fohh4-yyaaa-aaaap-qtkpa-cai ijlzs-2yaaa-aaaap-quaaq-cai
  nqxwe-hiaaa-aaaar-qb5yq-cai mu2zw-6iaaa-aaaar-qb56q-cai
  gxvvw-aiaaa-aaaar-qcada-cai ybilh-nqaaa-aaaag-qkhzq-cai
  mohjv-bqaaa-aaaag-qjyia-cai hkstf-6iaaa-aaaag-qkcoq-cai
  xjiq2-fiaaa-aaaan-q52ra-cai 6b2bo-kyaaa-aaaao-qpira-cai
)

for principal in "${required_pins[@]}"; do
  if ! grep -q "$principal" "$LIB" "$ROUTER"; then
    echo "FAIL: missing immutable call-target pin $principal" >&2
    exit 1
  fi
done

grep -q 'withdraw rejected: ledger is not in the immutable active/recovery registry' "$LIB"
grep -q 'PartyDEX recovery rejected: pool is not one of the two immutable retired recovery pins' "$LIB"
grep -q 'validate_volume_registry' "$LIB"
echo "PASS: active, volume, generic-withdrawal, and retired-recovery call targets are code-pinned."
