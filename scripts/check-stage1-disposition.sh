#!/usr/bin/env bash
#
# check-stage1-disposition.sh — enforces PR #22 Section 10's Stage-1
# Candid-method disposition as a standing, automated check.
#
# Every public #[update]/#[query] method in lib.rs must appear in exactly
# one of the five disposition buckets below. A method present in the code
# but absent from every bucket list fails this check — per the spec's own
# acceptance criterion, an unclassified mutator is not acceptable.
#
# Usage: scripts/check-stage1-disposition.sh
# Exit status is non-zero if any current method is unclassified, or if a
# bucket lists a method that no longer exists in the code (a stale entry
# is itself a drift signal worth surfacing, not silently ignored).

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RUST_LIB="$ROOT/src/arb_bot/src/lib.rs"

if [[ ! -f "$RUST_LIB" ]]; then
  echo "FATAL: expected source not found: $RUST_LIB" >&2
  exit 2
fi

# Every current #[update]/#[query] fn name, one per line, sorted.
actual_methods() {
  awk '
    /^#\[(update|query)\]/ { want = 1; next }
    want && /^(async )?fn [a-z_0-9]+/ {
      match($0, /^(async )?fn [a-z_0-9]+/)
      name = substr($0, RSTART, RLENGTH)
      sub(/^(async )?fn /, "", name)
      print name
      want = 0
    }
  ' "$RUST_LIB" | sort -u
}

# ── Fail-closed compatibility stubs before any inter-canister call ──
FAIL_CLOSED='
get_prices
get_bot_health
setup_approvals
manual_arb_cycle
execute_strategy_a
execute_strategy_b
execute_strategy_c
execute_strategy_d
execute_strategy_f
execute_strategy_k
execute_strategy_l
execute_strategy_m
execute_strategy_n
execute_strategy_o
execute_strategy_p
execute_strategy_q
execute_strategy_r
execute_strategy_s
dry_run_arb_cycle
dry_run_strategy_b
dry_run_strategy_c
dry_run_strategy_d
dry_run_strategy_f
dry_run_strategy_k
dry_run_strategy_l
dry_run_strategy_m
dry_run_strategy_n
dry_run_strategy_o
dry_run_strategy_p
dry_run_strategy_q
dry_run_strategy_r
dry_run_strategy_t
rumi_quote
rumi_manual_swap
pool_deposit
pool_redeem
pool_quote_deposit
pool_quote_redeem
pool_exchange
pool_quote_exchange
set_config
set_rumi_amm_paused
set_slippage_bps
set_arb_interval_secs
set_icp_inventory_band
set_bob_inventory_band
set_strategy_t_ckusdc_band
set_strategy_t_ckusdt_band
set_strategy_t_dry_run
set_strategy_t_enabled
set_strategy_t_icusd_band
set_strategy_t_pools
set_strategy_t_thresholds
set_bob_execution_enabled
set_bob_params
set_bob_pools
backfill_trade_legs
'

# ── Preserved local/query compatibility, no fund-moving or retired-venue call ──
READ_ONLY='
is_admin
add_admin
remove_admin
get_config
get_route_arb_config_v1
get_route_arb_status_v1
get_route_wallet_balances_v1
get_route_observation_v1
get_route_observations_v1
get_best_route_candidates_v1
get_route_mutation_lock_v1
get_route_reservations_v1
get_held_positions_v1
get_current_route_execution_v1
get_terminal_route_executions_v1
get_trade_history
get_trade_legs
get_activity_log
get_errors
get_snapshots
get_summary
get_public_health
get_volume_stats
get_volume_trades
cycles_balance
http_request
pause
resume
clear_cycle_lock
'

# ── Preserved volume configuration, no direct balance mutation ──
VOLUME_CONFIG='
set_route_arb_config_v1
set_volume_config
set_volume_global
pause_volume
resume_volume
'

# ── Route-observation updates: query-only inter-canister calls, no mutation ──
ROUTE_OBSERVATION='
start_route_observation_v1
quote_route_observation_batch_v1
'

# ── Surviving volume operation (Stage-4 global lock participant) ──
VOLUME_OPERATION='
volume_swap
fund_volume_subaccount
withdraw_volume_subaccount
trigger_volume_cycle
trigger_volume_rebalance
'

# ── Surviving generic/recovery operation (Stage-4 global lock participant) ──
GENERIC_RECOVERY='
withdraw
recover_partydex_balance
'

# New executor paths remain separately authorized and globally serialized.
ROUTE_EXECUTION='
prepare_route_execution_v1
advance_route_execution_v1
reconcile_route_execution_v1
'

RUNTIME_CONFIG='
get_route_runtime_status_v1
set_route_runtime_authorized_v1
'

all_classified() {
  printf '%s\n%s\n%s\n%s\n%s\n%s\n%s\n%s\n' \
    "$FAIL_CLOSED" "$READ_ONLY" "$VOLUME_CONFIG" "$ROUTE_OBSERVATION" "$VOLUME_OPERATION" "$GENERIC_RECOVERY" "$ROUTE_EXECUTION" "$RUNTIME_CONFIG" \
    | grep -v '^[[:space:]]*$' | sort -u
}

ACTUAL="$(actual_methods)"
CLASSIFIED="$(all_classified)"

fail=0

echo "== Stage 1 disposition inventory =="

UNCLASSIFIED="$(comm -23 <(printf '%s\n' "$ACTUAL") <(printf '%s\n' "$CLASSIFIED"))"
if [[ -n "$UNCLASSIFIED" ]]; then
  fail=1
  echo "FAIL: method(s) in lib.rs with no Stage-1 disposition:" >&2
  printf '  %s\n' $UNCLASSIFIED >&2
fi

STALE="$(comm -13 <(printf '%s\n' "$ACTUAL") <(printf '%s\n' "$CLASSIFIED"))"
if [[ -n "$STALE" ]]; then
  fail=1
  echo "FAIL: method(s) classified here that no longer exist in lib.rs (stale entry — update this script):" >&2
  printf '  %s\n' $STALE >&2
fi

if [[ "$fail" -eq 0 ]]; then
  n=$(printf '%s\n' "$ACTUAL" | grep -c .)
  echo "PASS: all $n public methods have an exact Stage-1 disposition (fail-closed: $(printf '%s\n' "$FAIL_CLOSED" | grep -c .), read-only: $(printf '%s\n' "$READ_ONLY" | grep -c .), volume-config: $(printf '%s\n' "$VOLUME_CONFIG" | grep -c .), route-observation: $(printf '%s\n' "$ROUTE_OBSERVATION" | grep -c .), volume-op: $(printf '%s\n' "$VOLUME_OPERATION" | grep -c .), generic-recovery: $(printf '%s\n' "$GENERIC_RECOVERY" | grep -c .))."
fi

exit "$fail"
