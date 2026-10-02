# Volume-lock recovery and adaptive route scheduler: deployment and live readback

Date: 2026-10-02 (UTC). Canister `ucjxv-nqaaa-aaaaj-qrsaq-cai`. Three upgrades, all by `rumi_identity`, all `--mode upgrade` with the canister stopped first.

Status: three upgrades deployed and verified live. Deploys 1 and 2 merged as PR #56; deploy 3 is the follow-up described at the end.

## Incident: volume bot unpaused but not trading

`volume_paused=false` and `icusd_icp` enabled since about 2026-10-01 04:55 UTC, with no volume trades. Two independent causes:

1. **Stuck durable account-mutation lock.** `get_route_mutation_lock_v1` returned owner `VolumeOperation`, operation `volume-fund-1790830642186099775`, acquired 2026-10-01 04:57:22 UTC, `reconciliation_required=true`. It was left by an admin `fund_volume_subaccount` call whose ledger transfer returned an error. `release_mutation_lock` refuses reconciliation-required locks and no other endpoint could release a volume-owned one, so every volume cycle and every route execution (which take the same lock) was blocked. Ledger evidence that the failed call moved nothing: the ICP index shows no volume-subaccount transaction after the successful 4 ICP fund at 04:57:11, and the icUSD ledger has no transaction between 03:20 and 07:54 that day.
2. **Cycles at the freezing threshold.** Balance 1.271T against a threshold of about 1.234T (idle burn × 90 days). Update calls were rejected with `IC0207` and the scheduler's last tick was about 09:30 UTC. The operator topped the canister up to about 3.27T.

## Deploy 1: `release_failed_volume_fund_lock`

- Wasm SHA-256 `fc7326ff90695a92e6433e3b45b9dd9aaea328a99deb627abe2cc60d323e2abd`, 4,519,650 bytes. Replaced module `af8f406271d6bbd668e9d2687002337acb1fe896f8345c160778353026295b78`.
- New admin endpoint `release_failed_volume_fund_lock(operation_id)`. It releases only a lock that matches the given id exactly, is owned by `VolumeOperation`, has the `volume-fund-` prefix, and is flagged reconciliation-required. That class is safe to release whatever the transfer's real outcome, because the transfer only moves value between the canister's default account and its own volume subaccount, and route code reads only the default account (`route_arb.rs`, `Account { owner, subaccount: None }`). Withdraw, cycle, rebalance and route locks stay unreleasable.
- Pre-upgrade state at 15:06 UTC: no current execution, no scheduler callback in flight, no reservations, no held positions. Stopped, upgraded, module hash verified while stopped, started 15:08 UTC.
- `release_failed_volume_fund_lock("volume-fund-1790830642186099775")` returned `Ok`; the lock read back `null`; the release is in the activity log under `admin`.
- Volume readback: the 15:27 cycle skipped as "pool not idle" (reference price 2.461 → 3.336 icUSD/ICP since the last trade) and refreshed the reference. The 15:47 cycle traded: `icusd_icp` trade count 5,593 → 5,594, total 7,795 → 7,796. A further trade followed at about 16:07 (7,797).
- No independent review completed before this deploy (the reviewing session was lost to an app restart). The route-code and operation-id claims above were checked by hand against the source.

## Cycle burn diagnosis

Measured by sampling `cycles_balance` about once a second (queries are not charged):

| Source | Per event | Per day |
|---|---|---|
| Fixed 10 s scheduler timer, idle | ~19.5M | ~165B |
| Route observation every 600 s (6 balance reads + 16 quotes) | ~0.44B | ~63B |
| Storage | – | ~13.7B |
| Volume cycles (estimate, not isolated) | – | ~10B |

Each timer firing in `ic-cdk-timers` 0.7.1 is three message executions (global timer, trap-isolating self-call, reply), each billed a flat fee, so an idle tick costs nearly the same as a busy one. Each inter-canister call reserves 42.09B until it returns, consistent with a 5M flat fee plus one cycle per instruction.

Storage: `Memory Size` (500.9 MB) includes a canister snapshot of 239.17 MiB taken 2026-09-28 09:03:54 UTC, which is why the reported idle burn rose from 7.27B to about 13.7B/day. The snapshot was left in place.

## Deploy 2: adaptive route scheduler

- Wasm SHA-256 `1a943f2e30c7483884c7921a50efb6247dba9efdcb604627e0a0ba480b6dedda`, 4,525,389 bytes, clean-built after the dashboard change.
- The scheduler no longer polls every 10 s. After each tick it computes how long until a tick could do anything (`route_scheduler::next_tick_delay_ns`, built on `next_action_for_profile`) and arms a one-shot timer for that delay. While there is work (an execution, an unfinished scan, a route to select) the cadence is still 10 s, measured from when the tick started. A repeating 300 s watchdog is the safety net, since a one-shot is lost if its callback traps. Eleven admin endpoints that can create scheduler work wake it through a drop guard, and a tick that dies mid-flight re-arms at the fast cadence.
- `RuntimeStatus` gains `scheduler_next_tick_due_ns : opt nat64`. The dashboard treats an old heartbeat as healthy only while a tick is scheduled, none is stuck in flight, and one completed within the last watchdog period.
- Independent review before deploy: verdict "ship with changes", with no state found in which the new scheduler sleeps through work the old one would have done. Its four findings (dashboard masking of a stuck or trapping scheduler, active cadence drifting off the 10 s grid, lost re-arm after a trapped tick, stale wake record suppressing wake hooks) and its test gaps were fixed before the build.
- Quiet window at 16:12:28 UTC: no execution, no lock, no scheduler callback in flight (waited for the 16:10 observation batch to finish). Stopped, upgraded, hash verified while stopped, started about 16:13 UTC.
- Readback: first tick 16:13:28 (10 s after start); state intact (`icusd_price_usd6=985000`, `quote_max_age_ns` 30 s, profile active, volume unpaused, 7,797 trades, no lock, no execution). Live dashboard byte-identical to `src/arb_bot/src/dashboard.html` (568,458 bytes); cockpit showed "Scanning" with "Heartbeat 2.1m ago · next check in 2.6m".

One full cycle, 16:14:50–16:24:41 UTC:

| Time | Event | Cost |
|---|---|---|
| 16:18:09 | watchdog tick; arms wake for 16:20:20 | 20.2M |
| 16:20:20 | wake; observation starts exactly 600 s after the previous start | 22.8M |
| 16:20:30–16:21:39 | quote batch | ~0.33B |
| 16:23:09 | watchdog tick; next eligibility over 300 s away, left to the watchdog | 20.4M |

Four timer firings where the fixed interval made about sixty. Net burn 560,993,969 cycles over 593 s, **81.8B/day**, against roughly 245–320B/day before. That is an idle baseline with no trades in the window. Over the following 66 minutes, with two volume trades and two route executions, the measured rate was about 137B/day.

## Deploy 3: failed admin transfers no longer leak the lock

An independent review of deploy 1, run after the fact, judged the recovery endpoint safe but pointed out that it treated a symptom. Three admin paths could still leave the lock behind, and two of them had no recovery at all:

- `withdraw` and `volume_swap` released the lock and then called `ic_cdk::trap` in the same reply callback. A trap discards that callback's state changes, the release included, while the acquisition from the earlier message stays. The lock was then held, unflagged, with nothing able to clear it. Both are reachable from the dashboard.
- `fund_volume_subaccount` and `withdraw_volume_subaccount` flagged the lock on any transfer error, including a certain no-op.

The 2026-10-01 incident itself was most likely a call that was never sent: the canister was at its freezing threshold, with about 37B cycles of headroom against a 42B per-call reservation, and no ledger shows the transfer.

Changes (wasm SHA-256 `bfbe2b5ce03aeae1e24447b917685a2d9e94a2bf05b77c34957ee4ff181a5270`, 4,511,800 bytes; interface and dashboard unchanged):

- Every ICRC-1 transfer goes through `swaps::icrc1_transfer`, which classifies the outcome. `LedgerRejected` (the ledger refused; `Duplicate` is excluded) and `NotSent` (the call future resolved on its first poll, which in ic-cdk 0.13.6 happens only when `ic0.call_perform` refuses the message) are certain no-ops. Anything else is an unknown outcome.
- `fund_volume_subaccount`, `withdraw_volume_subaccount` and `withdraw` release the lock on a certain no-op and flag it only on an unknown outcome. Both are written to the activity log.
- `withdraw` and `volume_swap` reply manually and report failure with an explicit reject, so nothing traps once the lock is held. Their candid signature is still `-> ()`.
- `volume_swap` keeps the lock on any failure after its input has left the volume subaccount, reports an unreturned output as a failure instead of success, and refuses dust before taking the lock.
- The recovery endpoint now requires the id to be the prefix followed by a `u64`.

Two reviews before deploy. The first found a regression in the initial version (a never-sent transfer would have committed a flagged, unreleasable lock) and that a refund after a failed swap step does not prove the venue left the input alone; both were fixed. The second returned "safe to ship".

Quiet window 18:18:36 UTC, stopped, upgraded, hash verified while stopped, started about 18:19. Readback: state intact (7,802 volume trades, `icusd_price_usd6=985000`, no lock, no execution), scheduler ticking. Canary at 18:19:51: `fund_volume_subaccount` for 10,000,000 ICP returned `LedgerRejected("Transfer: InsufficientFunds { balance: Nat(1496331758) }")`, the lock read back `null`, and the refusal is in the activity log. Before this deploy the same call left the bot wedged.

Remaining limits, by design:

- A flagged `withdraw-*`, `volume-swap-*` or `volume-withdraw-*` lock still needs a code upgrade to clear. That now requires a transfer that was sent and never answered clearly, or a `volume_swap` that failed after its first step.
- A trap from the system itself inside a reply callback (instruction limit, out of memory) can still strand an unflagged lock.
- The `NotSent` path has not been exercised live; it rests on reading the ic-cdk source and must be re-checked on any ic-cdk upgrade.

## Not verified live

- Route executions have since run under the new scheduler: `route-execution-12-56` completed both legs (prepare to submit 9 to 10 s, as before) and `route-execution-12-55` aborted on a venue slippage rejection with the same 9 s gap.
- The trap paths (aborted tick, lost one-shot) are covered by source-level tests, not behavioural ones.

## Open items for the operator

- The observation is now the largest cost (~60B/day). It is set by routes × ladder sizes × the 600 s cadence, which is trading policy and was not changed.
- The 2026-09-28 snapshot costs about 6.5B/day and predates both upgrades.
- `next_action` selects only stable and ICP winners (`route_scheduler.rs`), so a winner that exists only in the ckBTC or ckETH book is never auto-selected. This predates these changes.
- `get_bot_health` is retired and traps; diagnosis uses the query endpoints.
