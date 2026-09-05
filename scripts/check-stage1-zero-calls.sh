#!/usr/bin/env bash
#
# check-stage1-zero-calls.sh — proves every Stage-1 fail-closed method's
# CURRENT body makes zero inter-canister calls, structurally: it contains
# no ic_cdk::call, no venue-adapter reference, and (since every fail-closed
# body is a single trap/early-return with no await) no `.await`.
#
# This is a structural proxy for "zero inter-canister calls," not a live
# call-count assertion — this codebase has no ic_cdk::call mocking
# infrastructure. See the Stage-1 plan's Task 9 for why this is the
# honest, achievable version of the spec's call-counting requirement.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RUST_LIB="$ROOT/src/arb_bot/src/lib.rs"

FAIL_CLOSED_METHODS=(
  get_prices get_bot_health setup_approvals manual_arb_cycle
  execute_strategy_a execute_strategy_b execute_strategy_c execute_strategy_d
  execute_strategy_f execute_strategy_k execute_strategy_l execute_strategy_m
  execute_strategy_n execute_strategy_o execute_strategy_p execute_strategy_q
  execute_strategy_r execute_strategy_s
  dry_run_arb_cycle dry_run_strategy_b dry_run_strategy_c dry_run_strategy_d
  dry_run_strategy_f dry_run_strategy_k dry_run_strategy_l dry_run_strategy_m
  dry_run_strategy_n dry_run_strategy_o dry_run_strategy_p dry_run_strategy_q
  dry_run_strategy_r dry_run_strategy_t
  rumi_quote rumi_manual_swap pool_deposit pool_redeem
  pool_quote_deposit pool_quote_redeem pool_exchange pool_quote_exchange
  set_config set_rumi_amm_paused set_slippage_bps set_arb_interval_secs
  set_icp_inventory_band set_bob_inventory_band
  set_strategy_t_ckusdc_band set_strategy_t_ckusdt_band set_strategy_t_dry_run
  set_strategy_t_enabled set_strategy_t_icusd_band set_strategy_t_pools
  set_strategy_t_thresholds set_bob_execution_enabled set_bob_params set_bob_pools
  backfill_trade_legs
)

# Extracts the brace-matched body of `(async )?fn <name>(...) ... {` from lib.rs.
extract_body() { # $1=fn name
  awk -v fn="fn $1(" '
    index($0, fn) && !found { found = 1 }
    found {
      buf = buf $0 "\n"
      n = gsub(/\{/, "{", $0); depth += n
      n = gsub(/\}/, "}", $0); depth -= n
      if (depth <= 0 && buf ~ /\{/) { print buf; exit }
    }
  ' "$RUST_LIB"
}

fail=0
checked=0

echo "== Stage 1 zero-inter-canister-call structural check =="

for m in "${FAIL_CLOSED_METHODS[@]}"; do
  body="$(extract_body "$m")"
  if [[ -z "$body" ]]; then
    echo "FAIL: could not locate fn $m in $RUST_LIB" >&2
    fail=1
    continue
  fi
  checked=$((checked + 1))
  if echo "$body" | grep -qE '\bic_cdk::call\(|\.await\b|\bprices::|\bswaps::|\bpartydex::|run_specific_strategy\('; then
    echo "FAIL: $m's body still references a live call path:" >&2
    echo "$body" | grep -E '\bic_cdk::call\(|\.await\b|\bprices::|\bswaps::|\bpartydex::|run_specific_strategy\(' | sed 's/^/    /' >&2
    fail=1
  fi
done

if [[ "$fail" -eq 0 ]]; then
  echo "PASS: all $checked fail-closed methods' bodies are free of any live call path."
fi

exit "$fail"
