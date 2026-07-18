# Rumi Arb Bot — Dashboard Design System

Approved direction (2026-07-18). Single-file dashboard (`src/arb_bot/src/dashboard.html`,
`include_str!`-embedded — `cargo clean` before deploy after edits).

## Feel

A **pit console for a market keeper** — phosphor instruments on dark glass. Everything on
screen is a reading, a threshold, or a lever. Quiet structure, data-forward, zero decoration.
Not analytics, not SaaS.

## Tokens (existing — use them, never inline hex)

- Surfaces: `--base`, `--surface-1/2/3` (whisper-quiet elevation steps)
- Text: `--text-primary/secondary/tertiary/muted` (use all four levels)
- Depth: borders-only (`--border` progression); no shadows
- Accent: `--accent` teal `#00d4aa` — the only brand color
- Semantic: `--profit` / `--loss` / `--warning` / `--info` — color must mean something
- Type: `--font-ui` IBM Plex Sans, `--font-data` IBM Plex Mono (tabular numerals for data)
- Spacing: `--sp-1…10`; radius `--radius-sm/md/lg`

Known debt: ~239 inline `style=` attrs bypass tokens (raw hex `#e06b9f`, `#b45309`…).
Phase 3 purges them. New code must not add more.

## Signature — the venue-pair chip (Phase 3)

Every strategy is a meeting of two venues → two-tone split pill: left half venue A hue,
right half venue B hue, strategy letter centered. Used in spread rows, chart series,
trade attribution, force-exec buttons, health gates.

Venue hues: ICPSwap indigo, PartyDEX violet, Rumi/icUSD teal-green, BOB ember-orange,
ICP neutral silver. Hue + monogram replaces per-token logo images (no more 95KB base64).

## Rejected defaults

- Logo-image-per-token → hue + monogram registry
- One mega-chart with 12 series → small multiples per venue family + combined overview
- Flat settings form → levers grouped by touch frequency (daily / weekly / once-ever)
- Pause+Resume button pairs → single state-reflecting toggles (Phase 3)

## Phased plan

1. **Truth & safety** (branch `feat/dashboard-phase1-truth-safety`): config form hydration,
   health in poll + loading states, paused-strategy hide/unhide toggle (user-requested;
   persisted in localStorage), global wedge banner, incident toolkit card
   (clear_cycle_lock / recover_partydex_balance / backfill_trade_legs), scroll reset on
   view switch, 3USD asterisk footnote, dynamic snapshot-interval copy, BOB swap coverage.
2. **Registry & structure**: `VENUES`/`STRATEGIES` JS registry generates spread rows, legend,
   dry-run/force-exec grids, chart series, trade badges. Nav regroup: Overview (cockpit),
   Markets, Charts, Money, Ops, Volume. Trade→strategy attribution, leg pairing.
   Registry entries declare a **quote axis**: dollar-stable venues share the USD-per-ICP
   strip in STRATEGY PRICES; non-stable assets (BOB today, more coming) each get their own
   generated row-group (spot vs pool vs synthetic ref). New asset = registry entry, never
   a hand-added tile.
3. **Craft & signature**: venue-pair chips, chart small multiples, inline-style purge,
   confirmations on money-moving actions, unified empty/loading/error states.

## Constraints

- Single file, no build step. Client-side templating only.
- Candid is hand-maintained in three places (Rust, `.did`, dashboard IDL block) —
  run `scripts/check-candid.sh` after any IDL edit.
- Scales past strategy S: never assume A–S is final.
