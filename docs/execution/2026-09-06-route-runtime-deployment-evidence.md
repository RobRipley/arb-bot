# Route runtime deployment evidence

Date: 2026-09-06 (America/Los_Angeles)

The merged route-runtime source at commit `12510213f8ce314f0fee4abf923831cbdc8960f1` (merged by PR #27 as `537a820b0f7f8b7cda6c9fcade7a9a9f04ecc95c`) was upgraded to canister `ucjxv-nqaaa-aaaaj-qrsaq-cai` by the authorized controller workflow. No route authorization, configuration mutation, approval, funding, transfer, swap, or trade was performed.

The exact prepared artifact was:

- `target/wasm32-unknown-unknown/release/arb_bot.wasm`
- size: `3,854,950` bytes
- SHA-256: `70599abaf9ef346f9e19da0cb14e0fcf0991fd21212ecd3363f38de4eaad4b78`

The immediate controller readback reported `Status: Running` and module hash `0x70599abaf9ef346f9e19da0cb14e0fcf0991fd21212ecd3363f38de4eaad4b78`, matching the prepared artifact.

Read-only post-upgrade queries returned:

- route runtime: `compiled_support=true`, `live_authorized=false`, `enabled=true`, `dry_run=true`, `last_error=[]`;
- route status: `config_valid=true`, `execution_compiled_in=true`, `live_execution_authorized=false`, `route_count=246`;
- current route execution: `[]`;
- route mutation lock: `[]`;
- volume pools: all `enabled=false`, daily spend `0`; the existing global `volume_paused=false` value was unchanged;
- existing config: `strategy_t_enabled=false`, `strategy_t_dry_run=true`, `bob_execution_enabled=false`, `rumi_amm_paused=true`, and `paused=false`.

The deployed `http_request("/")` response body was retrieved read-only from the canister and matched the local embedded `src/arb_bot/src/dashboard.html` byte-for-byte: 326,377 bytes, SHA-256 `b717af6f54dd4a1fa84edd4266b36746c7452ca138b25c81fb713c8c82b50e45`. The public gateway hostnames returned gateway-mismatch responses, so the direct Candid `http_request` readback is the verified UI comparison.

These readbacks establish artifact identity, installation, startup, disabled live authorization, disabled volume pools, empty route execution state, empty mutation-lock state, and dashboard parity. They do not authorize later route activation or live trading.
