#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "== Six-asset route-arbitrage acceptance =="

tests=(
  route_registry route_graph route_accounting route_policy route_observation
  route_storage account_mutation_lock route_execution route_execution_detail route_rumi route_icpswap route_scheduler dashboard_route_ui
  stage1_retirement legacy_freeze state_decode candid
)
for test_name in "${tests[@]}"; do
  echo "== cargo test: ${test_name} =="
  RUSTFLAGS=-Awarnings cargo test -p arb_bot --test "$test_name"
done

echo "== cargo test: route_runtime library module =="
RUSTFLAGS=-Awarnings cargo test -p arb_bot --lib route_runtime

scripts/check-candid.sh
scripts/check-stage1-disposition.sh
scripts/check-stage1-zero-calls.sh
scripts/check-route-call-targets.sh

if rg -n '^[[:space:]]*(pub[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+drain_residual_(icp|bob)' src/arb_bot/src; then
  echo "FAIL: an automatic arbitrage drain function remains" >&2
  exit 1
fi
echo "PASS: automatic arbitrage drain function definitions are absent."

echo "== dashboard JavaScript syntax =="
dashboard_js="$(mktemp)"
trap 'rm -f "$dashboard_js"' EXIT
sed -n '/<script type="module">/,/<\/script>/p' src/arb_bot/src/dashboard.html \
  | sed '1d;$d' > "$dashboard_js"
node --check --input-type=module < "$dashboard_js"
echo "PASS: dashboard module JavaScript parses."
node scripts/test-dashboard-data-state.cjs
node scripts/test-dashboard-health.cjs
node scripts/test-dashboard-runtime.cjs
node scripts/test-dashboard-observation.cjs
node scripts/test-dashboard-ledger.cjs

echo "== release Wasm build =="
cargo clean -p arb_bot
RUSTFLAGS=-Awarnings cargo build --target wasm32-unknown-unknown --release -p arb_bot

wasm="$ROOT/target/wasm32-unknown-unknown/release/arb_bot.wasm"
if [[ ! -f "$wasm" ]]; then
  echo "FAIL: release Wasm not found at $wasm" >&2
  exit 1
fi

python3 - "$wasm" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
data = path.read_bytes()
if data[:8] != b"\0asm\x01\0\0\0":
    raise SystemExit("FAIL: artifact is not a Wasm v1 module")

def read_uleb(offset):
    value = 0
    shift = 0
    while True:
        if offset >= len(data):
            raise SystemExit("FAIL: truncated Wasm section length")
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7f) << shift
        if byte & 0x80 == 0:
            return value, offset
        shift += 7
        if shift > 63:
            raise SystemExit("FAIL: invalid Wasm section length")

offset = 8
code_payload = None
while offset < len(data):
    section_id = data[offset]
    offset += 1
    size, payload_start = read_uleb(offset)
    payload_end = payload_start + size
    if payload_end > len(data):
        raise SystemExit("FAIL: truncated Wasm section")
    if section_id == 10:
        code_payload = size
    offset = payload_end

if code_payload is None:
    raise SystemExit("FAIL: Wasm code section is missing")

network_limit = 12_582_912
print(f"Wasm total size: {len(data):,} bytes")
print(f"Wasm code-section payload: {code_payload:,} bytes")
print(f"Current network executable-code limit: {network_limit:,} bytes")
if code_payload > network_limit:
    raise SystemExit("FAIL: Wasm code section exceeds the current network limit")
print(f"PASS: code-section headroom: {network_limit - code_payload:,} bytes")
PY

git diff --check
echo "PASS: six-asset route-arbitrage source acceptance is green. No deployment was performed."
