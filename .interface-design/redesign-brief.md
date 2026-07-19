# Rumi Arb Bot — Ground-Up UI Redesign Brief

The current dashboard grew by accretion (4→13→14 strategies, venue by venue) with the UI
mirroring the candid API instead of the operator. This brief rethinks it from the user's
perspective. It supersedes screen-level details in system.md; tokens/signature carry over.

## The user (there is exactly one)

Robert. Solo operator of an arbitrage/market-keeper bot on ICP. He opens this page in four
modes, in descending frequency:

1. **Glance** (many times daily, often <10s, sometimes on phone, often logged out):
   "Is it healthy? Is it making money? Is anything stuck?"
2. **Diagnose** (rare, urgent, anxious): "Why isn't it trading / what's wedged / how do I
   unstick it?" Reference incident: a stranded-BOB wedge silently stalled the volume bot
   for 15 hours.
3. **Operate** (weekly-ish): toggle strategies/venues, tune slippage/intervals, move funds,
   force or dry-run a specific market.
4. **Grow** (recurring): add a new pool/venue → new strategy letters → volume pair.
   More assets are coming. The UI must scale by data, not by hand-edited markup.

Design law: **every screen must answer the mode-1 questions in the first second**, and no
state may be silent (nothing hidden because it's paused; nothing that looks live when it
isn't; nothing that looks dead while loading).

## What the product is

- 14 arb strategies (letters A–S, gaps are historical), each = a pairing of two venues
  (Rumi AMM, ICPSwap pools, PartyDEX pools) or a pool-vs-synthetic-reference (S: icUSD/BOB).
- A volume bot (3 pools) with its own subaccount, cadence, and spend caps.
- States that matter: bot paused; Rumi AMM paused (skips A/C/D/Q/R); Strategy S dry-run vs
  live; per-pool volume enabled/disabled; stranded funds; pending exits; cycle wedged.
- Three money pockets: arb main account, volume subaccount, operator wallet.

## Information architecture (replaces Overview/Charts/Trades/Health/Admin/Volume)

1. **Cockpit** (home) — answers mode 1 without scrolling:
   - Status strip: arb engine, volume engine, Rumi AMM, Strategy S mode, stranded funds,
     last cycle / next cycle countdown. Each item is a state chip, click → its home.
   - Net P&L hero + 24h delta; equity total across all three pockets.
   - "Attention" list: only markets/pools that are actionable (spread ≥ threshold, blocked
     gate, stranded) — empty state says "Nothing needs you."
   - Global alert banner (wedge/stuck) lives here and on every screen.
2. **Markets** — one row per strategy, generated from a registry, grouped by venue family
   (ICPSwap internal / PartyDEX × ICPSwap / Rumi × … / BOB). Each row: venue-pair chip,
   live spread gauge vs threshold, state badge (LIVE/PAUSED/DRY-RUN/UNCONFIGURED), last
   trade + P&L attribution, inline actions (Dry run · Force, confirm on Force). Row
   expands: mini spread chart, params, recent trades for THAT market. This kills the
   old DRY RUN / Force Exec button grids and the legend paragraph.
3. **Charts** — small multiples per venue family (not 12-series spaghetti); combined view
   optional; every series toggleable; labels never cover the axis. Click-through from a
   Markets row lands on that market isolated.
4. **Money** — all three pockets in one table (token × pocket grid with totals), plus
   Deposit / Withdraw / Swap / Fund-volume in one place. Footnotes for oddities (3USD has
   no subaccounts). Swap coverage matches reality (routes that exist server-side).
5. **Ops** — two clearly separated zones:
   - **Levers** (touched weekly): single state-reflecting toggles (Running/Paused,
     Rumi AMM, S live/dry-run, volume per-pool), slippage presets, intervals, caps.
     One toggle = one control showing current state; no Pause+Resume button pairs.
   - **Setup** (touched once per venue): pool principals, approvals, admins, fee tiers.
   - **Incident toolkit** card with the escape hatches + copyable dfx for the rest.
6. **Volume** — folded into Markets (volume pools are markets too, grouped under a
   "Volume" family) and Money (funding); its config lives in Ops › Levers. If a separate
   tab survives, it must justify itself; default is dissolution.

Trades/activity: a unified, filterable ledger reachable from Cockpit and from any Markets
row (pre-filtered). Legs of one arb render as a single grouped entry with combined P&L
and strategy attribution.

## Visual system

- Keep: dark instrument panel, IBM Plex Sans/Mono, quiet borders, whisper elevation,
  existing spacing scale. This is the product's identity; the redesign is IA + honesty,
  not a reskin. One accent (teal) for "alive/good"; amber = needs attention; red = losing
  money/stuck. Color always means something.
- **Signature: the venue-pair chip.** Two-tone split pill: left half venue A hue, right
  half venue B hue, strategy letter centered. Venue hues: Rumi/icUSD teal #00d4aa,
  ICPSwap indigo #5b8af0, PartyDEX violet #a78bfa, BOB ember #f0a05b, ICP silver #9aa4b2,
  synthetic-reference slate (dashed border). Chips appear in Markets rows, Cockpit
  attention list, charts legends, trade ledger, everywhere a strategy is named. Hue +
  monogram replaces logo images.
- States vocabulary (exactly one): LIVE (quiet), PAUSED (amber badge, muted row),
  DRY-RUN (info badge, live numbers), UNCONFIGURED (dashed outline), STALE (timestamp
  turns amber when data older than 2× cycle).
- **Cold load must be honest**: paint cached last-known data instantly from localStorage,
  stamped "as of <time>", with a thin refresh progress indication; skeletons only on
  true first-ever visit. Never a dead minute.

## Amendments (post red-team — these override anything above that conflicts)

1. **States vocabulary is now seven**: LIVE, PAUSED, DRY-RUN, UNCONFIGURED, STALE,
   **BLOCKED** (enabled but a gate is failing — e.g. insufficient balance, stranded
   recovery; amber, shows the first failing gate inline), and **STUCK** (cycle in
   progress > 3× interval; red, always escalates to the global banner). A transient
   "cycling…" treatment (subtle pulse on the state chip) covers normal in-progress.
   The stranded-BOB incident renders as BLOCKED + banner, never as PAUSED.
2. **Two row templates in Markets, one chrome**: arb rows (spread gauge, threshold,
   P&L attribution, Dry run/Force) and **cost-center rows** for volume pools (next
   direction, cadence, daily cost vs cap meter, idle threshold, trade count — NO
   P&L field, a volume pool has a cost, not a profit). Same visual chrome, chips,
   and expand behavior; different fact sets. This is the honest version of
   "volume dissolves into Markets."
3. **PAUSED badges carry their reason**: "PAUSED · Rumi AMM" / "PAUSED · bot" /
   "PAUSED · manual". Five rows sharing one root cause must be visibly one cause;
   the family group header for Rumi-gated rows shows the shared kill-switch state.
4. **Force lives in the expanded row only** (not the collapsed row) — glancing and
   executing are different postures; one deliberate click separates them. The Force
   confirm restates the live numbers ("Force M: buy ~$40 ICPSwap icUSD → sell
   PartyDEX ckUSDC, expected +$0.31"), never a generic yes/no. Dry run stays on the
   collapsed row (harmless).
5. **Toggles have three visual states**: on / off / applying (disabled, spinner,
   "applying…"). A toggle disables on click and re-enables only after a confirmed
   refetch. Never an optimistic flip.
6. **Expanded-row fact lists are enumerated**: arb rows — mini spread chart, params
   (min spread, max trade, slippage), last 5 trades with P&L, Dry run/Force; volume
   rows — the seven gate-card facts (next direction, trade size, daily cost/cap,
   input balance, min required, last price, current price) + Run cycle/Rebalance.
7. **Setup distinguishes its two gaps**: pool principal set/unset AND approvals
   run/not-run are separate visible facts per venue ("principal ✓ · approvals
   pending" is a distinct, dangerous state that must never look configured).
8. **Attention list staleness**: when logged out (or data older than one cycle),
   attention items carry the "as of" treatment — actionability itself can be stale,
   and says so.
9. **Withdraw gets the strongest confirm in the app** (restates token, amount,
   destination principal); Swap/Deposit confirm lighter. Adjacent placement in
   Money is fine only because the confirms differ.
10. **Family partition rule**: group by shared gating axis — "Rumi AMM" family =
    everything `rumi_amm_paused` gates (A/C/D/Q/R); "ICPSwap internal" (B/F);
    "PartyDEX × ICPSwap" (K–P); "BOB" (S); "Volume" (cost-center rows). A new
    venue adds one hue + one family assignment to the registry — accepted as the
    only manual design step per venue.

11. **(Robert, prototype review) Real logos return.** The venue-pair chip stays as the
    strategy identity, but real token logos accompany token names (Markets pair labels,
    Money token column, Ledger strategy cells, Cockpit balances) and real venue logos
    accompany venue names (Markets sublabels/family headers, Ledger venue column).
    Assets: icUSD + 3USD (Rumi-provided), ckUSDC/ckUSDT/ICP (dfinity kit; use the
    white ICP variant on dark), BOB, ICPSwap (ICS), PartyDEX (Partyhats). Rumi AMM
    venue = 3pool logo. Synthetic reference keeps the dashed slate treatment (no logo).
    Keep inlined assets small (downscale PNGs; total budget well under the old 95KB).
12. **(Robert) Cockpit shows arb-bot token balances** — glanceable "do we have what we
    need" card with token logos.
13. **(Robert) Money's Swap is a real panel** (from-token + balance + Max, amount, to-token,
    estimated receive with route note), then the light confirm — not confirm-only.
14. **(Robert) Ledger gets a legend** for strategy letters plus token/venue logos in the
    strategy and venue columns.

15. **(Robert, round 2) Mirrored logo convention + legend by alignment.** In Markets/Ledger
    pair lines: no ICP logos (pure repetition); the left pool's distinguishing token logo
    sits far-left, the right pool's far-right ("[icUSD] icUSD/ICP ⇔ ckUSDC/ICP [ckUSDC]");
    venue lines mirror the opposite way, logos toward the center ×
    ("ICPSwap [logo] × [logo] ICPSwap"). Synthetic ref: never a logo. Cockpit bot-balances
    is a vertical list. The strategy legend uses NO logos — readability there comes from a
    strict columnar grid (chip | left pool | ⇔ | right pool | venues) grouped in Markets
    order, every ⇔ on one vertical line.

## Prototype requirements (for the build agent)

Self-contained static HTML file(s), no external requests, realistic mock data (use these
live-derived values: Net P&L $203.02, 2025 legs, $23.4k volume; equity ≈ $850 arb +
$81 volume; Rumi AMM paused ⇒ A/C/D/Q/R paused; S live; spreads: B 41, F 27, K 42,
L 57●, M 84●, N 29, O 14, P 12, S 46 bps vs threshold 50; balances ICP 2.37 / icUSD
528.41 / ckUSDC 262.14 / ckUSDT 54.47 / 3USD 0.02 / BOB 607.91). Interactive enough to
communicate: view switching, row expansion, the incident-mode demo toggle (shows wedge
banner + stranded state + how Cockpit/Markets degrade), paused show/hide, confirm on
Force. Desktop 1440 and mobile 375 both composed, not merely unbroken.
