# Configurable icUSD price deployment and live readback

Status: deployed, configured at the user-authorized initial icUSD accounting value of `$0.97`, and resumed. The first eligible automatic observation and resulting two-leg execution completed and settled. This is a separate operational record for the merged combined release. The preceding release description is frozen in `2026-09-28-icusd-manual-price-release.md`.

## Candidate artifact

- Source PR #53 merged at `ff02c41e39bae1b54c0412da864c288a45e47c25`; reviewed tree `786e9e10dd5a61ded7996d51370e0d927e9985fb` matches the validated candidate.
- Candidate Wasm SHA-256: `0bcf9e99f99f5add07d76c2be716d90158e7ff4aa77d13d1d6ebfbc4973c4c52`.
- Artifact size: 4,514,170 bytes; executable code section: 3,235,584 bytes. Final build exited 0.
- The user authorized deployment/configuration/resumption with initial icUSD accounting price `970000` USD6 (`$0.97`).

## Fresh pre-upgrade quiet-window evidence

All reads used the merged candidate interface `src/arb_bot/arb_bot.did` against canister `ucjxv-nqaaa-aaaaj-qrsaq-cai`. On `2026-09-28T11:14:34Z`, the old module was still installed (`03ee4baad1a194cdb91ffccac26cbb2689940fda4d684dcac415085833d8a99d`) and the canister was Running with 2,344,631,455,497 cycles. Route execution `route-execution-9-35` was active on ckUSDT → ICP → icUSD; it was at leg 1 with an ICP reservation and a `RouteExecution` lock. The scheduler callback was in flight. Runtime authorization and execution were enabled, dry-run was off. No deployment/configuration action was attempted while that execution was active.

At `2026-09-28T11:16:29Z`, read-only detail showed `route-execution-9-35` Completed with no incident and both legs Settled:

- Leg 0: 20,000,000 native ckUSDT debited; 667,908,398 native ICP credited. Source-bound ICPSwap evidence: receipt 251,956, input block 1,109,821.
- Leg 1: 667,908,398 native ICP debited; 2,044,171,870 native icUSD credited. Source-bound ICPSwap evidence: receipt 12,375, input block 38,660,090.
- Realized route profit: 441,718 USD6. No held inventory was created.

The same read window showed current execution, mutation lock, scheduler callback, observation, held positions, and route reservations all empty. Runtime remained authorized/enabled with dry-run off. The 600-second profile cadence placed the next eligible observation at `2026-09-28T11:21:33.068Z`. The route profile and limits remained unchanged: four approved routes, three ICPSwap pools, $5/$20 ladder, $40 cap, two legs, 20 quote calls, concurrency two, and zero stable profit floors. The old interface returned no explicit `icusd_price_usd6` value (`[]`); the old installed module predates this field and remains the $1.00-compatible behavior. Legacy volume remained paused with all three pools disabled.

This clear state is a point-in-time preflight only. The authorized writer must disable live authorization and re-read current execution, lock, observation, and scheduler callback before upgrading. If any execution or lock appears, wait for normal settlement and a new cadence quiet window; do not change price/config generation during an active execution.

The sole writer paused live authorization at approximately `2026-09-28T11:16Z`. My independent named-interface read at `2026-09-28T11:18:37Z` confirmed `live_authorized=false`, with `enabled=true`, `dry_run=false`, no runtime error, and no scheduler callback in flight. Profile remained active and valid; next observation remained due at `11:21:33.068Z`. Current execution, mutation lock, observation, held positions, and reservations were all empty. The exact three approved ICPSwap pools and route limits were unchanged, the legacy volume manager remained paused with all three pools disabled, and the new price field was still absent on the old installed module. No unexpected execution or lock appeared after the authorization pause.

## Independent post-deployment readback

At `2026-09-28T11:22:17Z`, a fresh read-only status query confirmed the canister Running on candidate Wasm hash `0bcf9e99f99f5add07d76c2be716d90158e7ff4aa77d13d1d6ebfbc4973c4c52`, with cycle balance `2,321,736,387,336`. The current generated interface returned `icusd_price_usd6=970000`; runtime authorization and execution were enabled, dry-run was off, and no runtime error was present. The profile was active and valid with the approved four-route/two-leg scope, 600-second cadence, `$5`/`$20` ladder, `$40` cap, 20 quote cap, concurrency two, zero stable floors, and exactly the three approved ICPSwap pools. Legacy volume remained paused with all three pools disabled.

The first automatic observation, `route-observation-10-1790594502628248252`, had started at `2026-09-28T11:21:42.628Z`. It reported four routes, eight route-size work items, 16 required pool quotes, and was still in progress with no candidates evaluated at this read. Current execution, mutation lock, held positions, and reservations were empty. No execution or funds movement was evidenced yet. No manual observation or execution action was issued. The next eligible observation was `2026-09-28T11:31:42.628Z`.

At `2026-09-28T11:24:13Z`, the same observation had completed at `2026-09-28T11:23:21.817Z`: four routes, eight work items, 16 required quotes, eight candidates evaluated, and no incident. The best stable candidate was eligible and full-fill for ckUSDC → ICP → icUSD at the `$5` size; `net_profit_native=22343`, `net_profit_bps=44`, and the candidate recorded `accounting_icusd_price_usd6=970000`. The scheduler had automatically created `route-execution-10-36`, phase `LegPrepared` at leg 0, with its `RouteExecution` mutation lock. No terminal settlement evidence or source funds movement was present at this read. This proves the configured value informed the first eligible automatic observation and selection; it does not prove trade settlement or realized profit.

At `2026-09-28T11:26:06Z`, a read-only execution detail query showed `route-execution-10-36` Completed, both legs Settled, with no incident and realized profit `22,343` USD6 (about `$0.022343`):

- Leg 0, `icpswap-icp-ckusdc`: wallet debit `5,000,000` native ckUSDC; venue input `4,990,000` with `10,000` fee; ICP credit `166,809,951` e8s (`1.66809951 ICP`). Source-bound transfer evidence reported receipt `1,044,812` and input block `787,627`. `output_block` was `None` in the receipt source reference and is unavailable in this record.
- Leg 1, `icpswap-icp-icusd`: ICP debit `166,809,951`; icUSD credit `517,767,330` native (`5.17767330 icUSD` at eight decimals). Source-bound transfer evidence reported receipt `12,379` and input block `38,660,212`. `output_block` was `None` in the receipt source reference and is unavailable in this record.

This distinguishes the accounting-price observation and candidate selection from the later execution and settlement. The recorded `$0.97` is an accounting input; it is not evidence that icUSD traded at or maintained a market peg of `$0.97`. No manual scan, prepare, advance, retry, or drain action was issued.

## Deployment and final post-deployment readback

The sole writer reported pausing live authorization before stopping and upgrading the canister, verifying the candidate module while stopped, starting with authorization off, applying `set_icusd_price_usd6_v1(970000)`, reading back the configuration, and only then separately restoring live authorization. The independent reads below verify the resulting installed module, configuration, runtime, observation, and settlement; no deployment or configuration write was performed by this witness.

At `2026-09-28T11:26:58Z`, an independent read through the current generated interface confirmed the canister Running on module hash `0bcf9e99f99f5add07d76c2be716d90158e7ff4aa77d13d1d6ebfbc4973c4c52`, with cycle balance `2,362,353,569,458`. Runtime was `live_authorized=true`, `enabled=true`, `dry_run=false`, with no runtime error or callback in flight. The active profile read back `icusd_price_usd6=970000`, 600-second cadence, `$5`/`$20` size ladder, `$40` maximum principal, two legs, quote-call cap 20, concurrency two, and zero stable-profit floors. Exactly the three approved ICPSwap pools were enabled (`icpswap-icp-ckusdc`, `icpswap-icp-icusd`, `icpswap-icp-ckusdt`); all other route pools were disabled. The route status was valid and gave the next eligible observation time as `2026-09-28T11:31:42.628Z`.

The same final read showed current execution, mutation lock, scheduler callback, held positions, and route reservations all empty after settlement. Legacy volume remained paused and all three volume pools remained disabled; the legacy trade count was 7,795. Lifetime route summary reported 27 completed, 9 aborted, 0 held, and stable realized profit of 8,902,488 USD6. These are readbacks at the stated time, not a guarantee of future state. No claim is made here that the accounting input establishes a market peg.
