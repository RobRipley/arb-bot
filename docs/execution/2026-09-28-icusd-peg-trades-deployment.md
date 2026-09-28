# icUSD peg-trades deployment and activation evidence

Date: 2026-09-28 (UTC)

Status: the reviewed release was upgraded, restarted, and activated under the explicitly authorized four-route profile. One automatic scan selected and completed a two-leg trade with source-bound settlement evidence. The canister remains running with only the approved route profile enabled; no volume pools were enabled and no ICP drain or held-inventory sale was performed.

## Release and preflight

- Canister: `ucjxv-nqaaa-aaaaj-qrsaq-cai`.
- Merged source: `98ae3f26d438851ba69e74c7605cc5dd63b09bc8` (PR #50).
- Candidate Wasm: `/tmp/arb-bot-red-target/wasm32-unknown-unknown/release/arb_bot.wasm`; 4,470,235 bytes; SHA-256 `03ee4baad1a194cdb91ffccac26cbb2689940fda4d684dcac415085833d8a99d`; executable code section 3,218,103 bytes.
- Operator identity `rumi_identity` resolved to controller principal `fd7h3-mgmok-dmojz-awmxl-k7eqn-37mcv-jjkxp-parnt-ehngl-l2z3m-kae`. The controller set was unchanged.
- Before lifecycle change, authenticated status reported the canister stopped with old module hash `429f5378bb31e3baf0b42cb9b28defd5c65c7814f6aa009336cfca0a7babaac2`.
- A recoverable stopped-state snapshot was taken at `2026-09-28T09:03:54.701501Z`, canister version `4583125`, snapshot ID `00000000000000000000000001308c810101`. Its module matched the old live hash. Stable-memory SHA-256 was `dbf313e4c0dc827772136ea5fe03c985a35bab41e4316b64230726da068b33cd`.
- The snapshot was read offline using the repository's `ic-stable-structures 0.6.9` layout and JSON codecs. The one-off decoder checked the MemoryManager/SCL/B-tree headers and read a RAM copy; the downloaded stable-memory file remained unchanged. The decoded JSON SHA-256 was `97a2dabe8564d803875b1b128d118440e145665062c0556d7eb91653d621ecb3`; decoder source SHA-256 was `37c8a2b10089c07c283e10792d29b1665b4398ccaa94c5157388986e6284983b`.
- Pre-upgrade source-mapped values: current execution null in both execution projection and runtime slot; mutation-lock slot null; route reservations empty; held-position logs empty; runtime authorization false; route execution disabled; volume globally paused and all three volume pools disabled. The peg-profile field was absent in the old state. No canister start was used to obtain these values.

## Upgrade and activation

The stopped canister was upgraded in `upgrade` mode using the verified Wasm; the install command exited 0. The canister remained stopped immediately afterward, and authenticated status reported module hash `03ee4baad1a194cdb91ffccac26cbb2689940fda4d684dcac415085833d8a99d`. The candidate was then started. No reinstall, controller change, cycle top-up, volume mutation, or unrelated config change was made.

After start, readbacks again showed no current execution or lock, `live_authorized=false`, `enabled=false`, and the old stored `dry_run=false`; volume remained paused with all pools disabled. The dedicated `set_icusd_peg_trades_profile_v1(true)` action then selected the fixed profile and returned safe-off values: `enabled=false`, `dry_run=true`, while leaving authorization false. The current Candid interface used for all new-method calls and reads was `src/arb_bot/arb_bot.did`; `src/arb_bot/arb_bot.did.deployed` is only the pre-upgrade compatibility baseline.

The profile readback established:

- Exactly four two-leg routes: icUSD → ICP → ckUSDC; ckUSDC → ICP → icUSD; icUSD → ICP → ckUSDT; ckUSDT → ICP → icUSD.
- Only the three required ICPSwap pools enabled: icUSD/ICP, ICP/ckUSDC, and ICP/ckUSDT. The Rumi 3pool and all other pools were disabled.
- Stable accounting at $1; stable absolute and basis-point profit floors both zero, with ledger fees and the route-wide non-loss floor still applied.
- Stable size ladder `$5, $20`; maximum stable principal `$40`; maximum two legs; 20 pool-quote calls per observation and concurrency two.
- Profile active; observation cadence 600 seconds; automatic scanning gated by live authorization, enabled execution, and dry-run off.
- No automatic residual drain or held-ICP sale.

The full config was then changed using a file generated from the exact profile readback; a local equality check confirmed only the top-level `enabled` and `dry_run` gates changed. Readback showed `enabled=true`, `dry_run=false`, and `live_authorized=false`, with all profile economics, route/pool scope, and budgets unchanged. The separate `set_route_runtime_authorized_v1(true)` call was the final live gate and returned `live_authorized=true`, `enabled=true`, `dry_run=false`, and no runtime error. Volume remained globally paused and every volume pool remained disabled.

## Automatic scan and settled execution

No manual scan, route preparation, execution advance, or retry call was made. After the final authorization update, the automatic scheduler created observation `route-observation-9-1790587226139777593` at canister time `2026-09-28T09:20:26.139777Z`. Its readback showed `route_count=4`, eight work items (four routes at two sizes), 16 required quotes, sufficient budget, and no incident. The generic status field `route_count=56` is the base route enumerator; the profile observation's actual work universe was four routes. The observation was consumed when the scheduler prepared the winning route.

The scheduler selected and completed execution `route-execution-9-24`:

- Route: `icpswap-icp-ckusdc:CkUsdc>Icp|icpswap-icp-icusd:Icp>IcUsd`.
- Leg 0, ICPSwap ICP/ckUSDC: actual ckUSDC debit `20,000,000` native units (`$20.00`, including the `10,000` ledger fee); venue input `19,990,000`. Actual ICP credit was `665,215,002` native units after fees, meeting the persisted leg floor. Source-bound terminal-transfer evidence cited receipt `1,044,500` and input block `787,575`.
- Leg 1, ICPSwap ICP/icUSD: actual ICP debit `665,215,002` native units; actual icUSD credit `2,058,359,195` native units after fees. Ledger-bound terminal-transfer evidence cited receipt `12,308`, input block `38,658,589`, and output block `37,372`.
- Both legs read back `Settled`; the terminal execution phase was `Completed`. The execution detail and runtime status both reported realized profit `583,591` in 6-decimal USD units (approximately `$0.583591`), with terminal icUSD proceeds of `$20.58359195` at 8 decimals.
- After completion, current execution and mutation lock were null, held positions and route reservations were empty, and the runtime scheduler had no in-flight callback. No drain or follow-on held-inventory action occurred.

The final authenticated status reported the canister running with the candidate module hash and a balance of `2,430,689,013,600` cycles. No cycles were topped up. Runtime readback remained authorized/enabled/dry-run off; the peg profile was active with 600-second cadence. Volume remained paused, all volume pools remained disabled, and their historical trade counters were retained. No second scan was forced; the next eligible observation time was the first observation start plus 600 seconds.

The terminal execution detail was captured read-only; its Candid text SHA-256 was `cab2846f9df397d9a9faf1426055b4d766172801bab61910b6b46dedfd3c5d46`. Raw snapshot state, decoder output, and receipt payloads remain in temporary local storage; this committed record contains only bounded operational values and hashes.
