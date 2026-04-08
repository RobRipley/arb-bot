# Security Notes

This canister is currently operated as a **single-user personal bot**. The security model assumes:

- There is exactly one human operator (the owner principal).
- The operator's Internet Identity / key material is secure.
- Any principal in the admin list is fully trusted with all bot funds.

The items below are deliberate trust assumptions under that model. **If this canister is ever opened up to third-party depositors, or if additional admins are added, each item must be revisited and hardened.**

## Deferred hardening (revisit before multi-user)

### 1. `withdraw` is admin-gated, not owner-gated
`withdraw(token_ledger, to, amount)` lets any admin send any token to any principal. A compromised admin = total loss of funds.

**Before multi-user:** change `require_admin()` → `require_owner()` on `withdraw`, or add a withdrawal allowlist / rate limit / time lock.

### 2. `backfill_trade_legs` can rewrite history
Any admin can prepend arbitrary fake trade legs to the on-chain history, which would falsify displayed PnL.

**Before multi-user:** remove this method entirely, or restrict to owner with a one-shot flag that disables it after first use.

### 3. `set_config` is admin-gated
Any admin can change pool addresses, slippage, min_profit_usd, max_trade_size, etc. A malicious config could route trades to an attacker-controlled pool or disable safety limits. Owner is preserved, but everything else is mutable.

**Before multi-user:** gate `set_config` on `require_owner()`, or split into per-field setters with tighter validation.

### 4. `pool_deposit` / `pool_redeem` / `rumi_manual_swap`
All admin-gated direct trade execution. Same concern as withdraw — a compromised admin can intentionally execute losing trades.

**Before multi-user:** owner-only, or remove and rely exclusively on the automated arb cycle.

### 5. No per-depositor accounting
State tracks a single pool of funds owned by the bot. There is no concept of "user X deposited Y" — all balances are commingled.

**Before multi-user:** add a deposit/withdraw ledger keyed by principal, with share-based accounting for PnL distribution.

### 6. Unbounded storage growth
`trades`, `trade_legs`, `snapshots`, `errors`, `activity_log` grow forever. Acceptable for single-user (stable memory migration planned). For multi-user, add quotas or ring buffers to prevent any action from filling the 4 GiB heap.

### 7. Admin list has no audit trail
`add_admin` / `remove_admin` changes are not logged to `activity_log`.

**Before multi-user:** log every admin mutation with before/after state.

## What is already solid

- Every update method calls `require_admin()` (or `require_owner()` for admin management).
- Anonymous principal is explicitly rejected in `require_admin()`.
- Owner principal cannot be changed by `set_config` (preserved across config updates).
- Arb cycle has a reentrancy guard (`CYCLE_IN_PROGRESS`) that releases on drop, even if the callback traps.
- No secrets stored in canister state.
- Backup controller is set on the canister so the owner key is not a single point of failure for upgrades.
