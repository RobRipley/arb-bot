# Dashboard Overhaul Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the current single-page GitHub-dark dashboard with a sidebar-navigated trading terminal interface featuring charts, better visual hierarchy, and a distinctive electric teal identity. Portfolio-worthy craft.

**Architecture:** Single HTML file (`dashboard.html`) served from the canister via `http_request`. Four views (Overview, Charts, Trades, Admin) rendered client-side with JS view-switching. TradingView lightweight-charts loaded from CDN for charting. All existing canister interactions preserved.

**Tech Stack:** Vanilla HTML/CSS/JS (no build step), TradingView lightweight-charts (CDN), IBM Plex Sans + IBM Plex Mono fonts (Google Fonts CDN), @dfinity/agent + auth-client (ESM imports, same as current).

---

## Design System

### Intent
Solo trader checking their personal ICP arb bot. Dense, precise, electric. A well-crafted instrument panel — not a consumer app, not a Bloomberg terminal. Something built for one person who built the machine it monitors. The feeling of infrastructure: reliable, purposeful, no decoration without meaning.

### Why These Fonts

**IBM Plex Sans + IBM Plex Mono.** Not Inter (generic), not JetBrains Mono (every dev dashboard). IBM Plex was designed for infrastructure — it's the typeface of systems that run things. The mono variant has a distinctive character at small sizes (the lowercase `l` with its serif, the `0` with its dot) that makes data feel *engineered* rather than just displayed. The sans pairs perfectly because they share the same skeleton. Together they say "this was built by someone who builds serious things."

### Texture & Atmosphere

The base background isn't flat — it has a subtle SVG noise grain overlay at 2-3% opacity. This is the difference between a flat CSS color and something that feels like a physical surface. Think brushed dark metal, the faceplate of an instrument. The noise is generated as an inline SVG data URI so there's no external asset to load.

```css
body::before {
  content: '';
  position: fixed;
  inset: 0;
  opacity: 0.025;
  background: url("data:image/svg+xml,..."); /* tiny noise SVG */
  pointer-events: none;
  z-index: 9999;
}
```

### Tokens (CSS Custom Properties)

```css
:root {
  /* Surfaces — cold blue-black, shifting only lightness */
  --base: #0c0e14;
  --surface-1: #12151e;
  --surface-2: #181c28;
  --surface-3: #1e2333;
  --surface-input: #0a0c11;

  /* Borders — rgba, disappear when you're not looking */
  --border: rgba(255, 255, 255, 0.06);
  --border-emphasis: rgba(255, 255, 255, 0.10);
  --border-focus: rgba(0, 212, 170, 0.4);

  /* Text — four levels */
  --text-primary: #e8ecf4;
  --text-secondary: #8b93a8;
  --text-tertiary: #5a6278;
  --text-muted: #3d4456;

  /* Accent — electric teal */
  --accent: #00d4aa;
  --accent-dim: rgba(0, 212, 170, 0.15);
  --accent-hover: #00eabc;

  /* Semantic */
  --profit: #00d4aa;    /* same as accent — profit IS the brand */
  --loss: #ff4757;
  --warning: #f0b232;
  --info: #5b8af0;

  /* Typography */
  --font-ui: 'IBM Plex Sans', -apple-system, sans-serif;
  --font-data: 'IBM Plex Mono', 'SF Mono', monospace;

  /* Spacing — 4px base */
  --sp-1: 4px;
  --sp-2: 8px;
  --sp-3: 12px;
  --sp-4: 16px;
  --sp-5: 20px;
  --sp-6: 24px;
  --sp-8: 32px;
  --sp-10: 40px;

  /* Radii — sharp-ish, technical */
  --radius-sm: 3px;
  --radius-md: 6px;
  --radius-lg: 10px;
}
```

### Typography Scale
- **Hero number** (P&L): IBM Plex Mono, 36px, weight 600, letter-spacing -0.02em
- **Section header**: IBM Plex Sans, 11px, weight 600, uppercase, 0.08em letter-spacing, --text-tertiary
- **Stat label**: IBM Plex Sans, 12px, weight 400, --text-secondary
- **Stat value**: IBM Plex Mono, 13px, weight 500, --text-primary
- **Table data**: IBM Plex Mono, 12px, weight 400
- **Table header**: IBM Plex Sans, 11px, weight 600, uppercase, --text-tertiary
- **Nav item**: IBM Plex Sans, 13px, weight 500

### Depth Strategy
Borders-only with noise texture for atmosphere. No shadows. `--border` (rgba 6%) for standard separation. `--border-emphasis` (rgba 10%) for interactive elements and hover states. Surface color shifts handle elevation. The noise grain adds perceived depth without any shadow.

### Signature Element
**Spread Thermometer** — a horizontal gauge in the Overview showing the current spread between pools, with the min-spread threshold marked as a vertical tick. The bar fills left or right from center based on spread direction. Color-coded: teal when |spread| exceeds threshold (opportunity), muted when below. Current spread value displayed as text alongside. This makes the core arb opportunity *visible* at a glance rather than being a number buried in a list.

### Sidebar Active Indicator
Active nav item has a **2px teal left-edge bar** (accent color, border-left) plus a subtle `--accent-dim` background. Not just a background color change — the vertical accent bar gives it physicality, like a selection indicator on hardware.

---

## Layout Architecture

```
┌──────────────────────────────────────────────┐
│ [Sidebar 200px]  │  [Content area]           │
│                  │                            │
│  ◉ Rumi Arb Bot │  ┌──────────────────────┐  │
│                  │  │  View-specific        │  │
│  ▎ Overview      │  │  content              │  │
│    Charts        │  │                       │  │
│    Trades        │  │                       │  │
│    Admin  🔒     │  │                       │  │
│                  │  │                       │  │
│  ──────────      │  │                       │  │
│  ● Running       │  │                       │  │
│  rob...xyz       │  │                       │  │
│  [Login/Logout]  │  └──────────────────────┘  │
└──────────────────────────────────────────────┘
```

- Sidebar: same `--base` background, right border only. Same surface, no color fragmentation.
- `▎` = teal left-edge indicator on active item
- Admin nav item hidden until logged in as admin (shows lock icon hint otherwise)
- Bot status indicator: small colored dot (green = running, amber = paused) with label
- Principal truncated with full text on hover/click-to-copy
- Content area: `--base` background, `--sp-6` padding, overflow-y scroll

---

## Views

### 1. Overview (default)

The at-a-glance hero page. Strong visual hierarchy — the most important number dominates.

**Row 1 — Hero P&L Strip (full width, single card)**
```
┌──────────────────────────────────────────────────────┐
│  NET P&L                                             │
│  $12.4567                47 trades · $4,200 volume   │
│  ▲ $2.10 (24h)           12 Rumi · 35 ICPSwap       │
└──────────────────────────────────────────────────────┘
```
- One wide card, not four competing cards
- P&L number at 36px IBM Plex Mono, colored (--profit or --loss)
- Supporting stats (trade count, volume, DEX breakdown) as --text-secondary on the right side
- 24h change as a small delta below the hero number (if we can compute it from snapshots)
- The P&L number is THE thing your eye hits first

**Row 2 — Spread Thermometer (full width)**
```
┌──────────────────────────────────────────────────────┐
│  SPREAD                                              │
│                                                      │
│  Strategy A — Rumi vs ICPSwap         +72 bps ●      │
│  ◄────────────────|─────██████████░░░─|──────────►   │
│                 -50              0           +50      │
│                                                      │
│  Strategy B — icUSD vs ckUSDC         -18 bps        │
│  ◄──────────░░────|───────────────────|──────────►   │
│                                                      │
└──────────────────────────────────────────────────────┘
```
- Full-width card with both strategies
- Each gauge: centered at 0, fills left (negative) or right (positive)
- `|` marks at ±min_spread_bps (threshold)
- Fill color: `--accent` when |spread| > threshold, `--text-muted` when below
- Small `●` dot (teal) next to the spread value when above threshold = "opportunity active"
- Spread value displayed as text right-aligned: `+72 bps ●` or `-18 bps`
- The gauge is pure CSS: container div, positioned inner div, width/margin set by JS

**Row 3 — Prices + Balances (2-column)**
```
┌────────────────────────────┬─────────────────────────┐
│  LIVE PRICES               │  BALANCES               │
│                            │                         │
│  STRATEGY A                │  3USD     1,234.5678    │
│  Rumi       $12.3456       │  ckUSDC     567.89      │
│  ICPSwap    $12.4012       │  icUSD       45.1234    │
│  VP         1.057234       │  ICP          2.3456    │
│                            │  ckUSDT       0.00      │
│  STRATEGY B                │                         │
│  icUSD pool $12.3890       │  ────────────────────   │
│  ckUSDC pool $12.4012      │  Total ≈ $1,849.23      │
│                            │                         │
│  Login to refresh ↻        │                         │
└────────────────────────────┴─────────────────────────┘
```
- Prices: show "Login to view" when unauthenticated (get_prices is an update call)
- Balances: always visible (query calls)
- Total estimate: approximate USD sum using latest price data
- Price values right-aligned in IBM Plex Mono
- Strategy sub-headers in --text-tertiary, 10px uppercase

**Row 4 — Recent Trades (compact, last 5)**
```
┌──────────────────────────────────────────────────────┐
│  RECENT TRADES                          View all →   │
│                                                      │
│  12:34  Leg 1  ICPSwap   567.89 ckUSDC → 45.12 ICP  │
│  12:34  Leg 2  Rumi      44.90 ICP → 562.34 3USD    │
│  09:12  Drain  Rumi      0.45 ICP → 5.62 3USD       │
│  ...                                                 │
└──────────────────────────────────────────────────────┘
```
- Compact single-line format per leg
- Type badge color-coded: Leg 1 = --info, Leg 2 = --accent, Drain = --warning
- "View all →" link navigates to Trades view
- If no trades: "No trades yet — bot is evaluating spreads every 3 minutes"

### 2. Charts

Full-page chart view. **Defaults to Spread History** on first load — the most operationally useful chart, showing whether opportunities are appearing and when the bot takes them.

**Data source:** `get_snapshots(offset, limit)` — returns `CycleSnapshot` records with all prices, balances, spreads, and trade indicators recorded every 3 minutes. This is a query call (no auth needed).

**Layout:**
```
┌──────────────────────────────────────────────────────┐
│  [Spread] [Prices] [P&L] [Balances] [VP]            │
│  [1H] [6H] [24H] [7D] [30D] [All]                   │
│                                                      │
│  ┌──────────────────────────────────────────────┐    │
│  │                                              │    │
│  │             Chart Area (500px)                │    │
│  │                                              │    │
│  │                                              │    │
│  └──────────────────────────────────────────────┘    │
│                                                      │
│  XX,XXX data points · Last updated 12:34:56          │
└──────────────────────────────────────────────────────┘
```

**Chart type tabs** — styled as pill buttons, active state uses --accent-dim background + --accent text:

1. **Spread History** (default)
   - Two line series: Strategy A spread (teal) and Strategy B spread (--info blue)
   - Horizontal reference line at ±min_spread_bps (dashed, --text-muted)
   - Trade markers: teal dots on the Strategy A line where `traded == true && strategy_used == "A"`, blue dots for Strategy B
   - Y-axis: basis points

2. **Price Comparison**
   - Two overlaid line series: Rumi ICP/USD (teal) and ICPSwap ICP/USD (--info blue)
   - When lines diverge = spread opportunity; when they converge = bot traded
   - Y-axis: USD price

3. **Cumulative P&L**
   - Area chart with teal fill at low opacity
   - Data source: `get_trade_legs()` — compute running sum of `bought_usd_value - sold_usd_value - fees_usd`
   - Y-axis: USD (6 dec)
   - This is THE money chart — shows the bot's total earnings over time

4. **Balance Composition**
   - Individual line series per token (all converted to USD terms):
     - ICP: `balance_icp * icpswap_icp_price_ckusdc / 1e6` (from same snapshot)
     - 3USD: `balance_3usd * virtual_price / 1e18 / 100`
     - ckUSDC: `balance_ckusdc / 1e6` (already USD)
     - icUSD: `balance_icusd / 1e8` (≈ $1 each)
   - Plus a thicker line for total
   - Colors: 3USD = teal, ckUSDC = --info, icUSD = --warning, ICP = --text-secondary, Total = --text-primary

5. **Virtual Price**
   - Single line chart: `virtual_price / 1e18` over time
   - Slow-moving (VP changes gradually) — useful for understanding 3USD LP value

**Time range buttons** — filter `snapshotCache` by timestamp before rendering. Pill-style buttons, same styling as chart tabs but smaller.

**Chart theming:**
```js
const chartOptions = {
  layout: { background: { color: '#12151e' }, textColor: '#8b93a8', fontFamily: 'IBM Plex Mono' },
  grid: { vertLines: { color: 'rgba(255,255,255,0.03)' }, horzLines: { color: 'rgba(255,255,255,0.03)' } },
  crosshair: { mode: LightweightCharts.CrosshairMode.Normal },
  timeScale: { timeVisible: true, secondsVisible: false, borderColor: 'rgba(255,255,255,0.06)' },
  rightPriceScale: { borderColor: 'rgba(255,255,255,0.06)' },
};
```

**Empty state:** "Snapshot data is collecting. The bot records prices, balances, and spreads every 3 minutes. Charts will appear as data accumulates." (Because this feature was just deployed, there won't be data immediately.)

### 3. Trades

Full history view.

**Top — Filter chips + count**
```
[All] [Leg 1] [Leg 2] [Drain]    47 total legs
```
Chips are pill buttons, active = --accent-dim bg. Client-side filter on the fetched data.

**Main — Trade legs table (full width)**
```
┌──────┬──────┬─────────┬───────────────┬──────────────┬────────┬────────┬──────┬─────────┐
│ Time │ Type │ DEX     │ Sold          │ Bought       │ USD In │USD Out │ Fees │ Profit  │
├──────┼──────┼─────────┼───────────────┼──────────────┼────────┼────────┼──────┼─────────┤
│12:34 │ Lg 1 │ ICPSwap │ 567.89 ckUSDC │ 45.12 ICP    │$567.89 │   --   │$0.01 │   --    │
│12:34 │ Lg 2 │ Rumi    │ 44.90 ICP     │ 571.23 3USD  │   --   │$603.84 │  --  │ +$35.94 │
└──────┴──────┴─────────┴───────────────┴──────────────┴────────┴────────┴──────┴─────────┘
```
- Type column: color-coded text (not badges — too heavy for a dense table)
- Profit column: green/red, only shows on Leg 2/Drain (paired with preceding Leg 1)
- Pagination: `← Prev | Page 1 of 3 | Next →`
- IBM Plex Mono for all data cells, IBM Plex Sans for headers

**Bottom — Activity Log (collapsible)**
```
▾ ACTIVITY LOG                                [Filter: All ▾]
────────────────────────────────────────────────────────────
12:34:56  TRADE    COMPLETE RumiToIcpswap: 562.34 3USD → ...
12:31:56  ARB_SKIP A: Spread 12 bps < minimum 50 bps
12:28:56  ARB_SKIP A: Spread 8 bps < minimum 50 bps
...
```
- Category labels color-coded (same as current catColors map)
- Collapsible with triangle toggle
- Category filter dropdown

### 4. Admin (requires auth)

Three-column grid of cards.

**Bot Controls card:**
- Pause / Resume buttons (danger / success styling)
- Setup Approvals button
- Manual Arb Cycle button
- Dry Run button (primary) with expandable result panel below

**Swap Widget card:**
- Same routing logic as current (rumi, 3pool_deposit, 3pool_redeem, rumi+3pool, 3pool+rumi)
- From/To boxes with token selectors and amount inputs
- Flip button (↕)
- Route indicator ("via Rumi AMM + 3pool")
- Quote display
- Execute button

**Withdraw card:**
- Token selector, recipient principal input, amount input
- Withdraw button (danger styling)

**Deposit section (below the grid):**
- Shows II wallet balances (only when logged in)
- Deposit buttons per token
- Same functionality as current `doDeposit()`

**Config Display card:**
- Read-only display of key BotConfig values
- Owner principal, pool addresses (truncated), min_spread_bps, max_trade_size_usd, paused state
- Useful for quick reference without needing dfx

---

## Implementation Tasks

### Phase 1: Design System + Layout Shell

#### Task 1.1: Create the CSS design system
**File:** `src/arb_bot/src/dashboard.html` (full rewrite of `<style>` section)

Write all CSS:
1. Google Fonts import for IBM Plex Sans (400, 500, 600, 700) and IBM Plex Mono (400, 500, 600, 700)
2. All CSS custom properties from the tokens section above
3. CSS reset (`* { margin: 0; padding: 0; box-sizing: border-box; }`)
4. Body: `--base` background, `--text-primary` color, `--font-ui`, line-height 1.5
5. Noise grain overlay via `body::before` pseudo-element with inline SVG noise at 2.5% opacity
6. Layout grid: `body { display: grid; grid-template-columns: 200px 1fr; height: 100vh; }`
7. Sidebar styles: `.sidebar` — same `--base` bg, right border, flex column, space-between for nav top / status bottom
8. Nav items: `.nav-item` — padding, cursor, transition. `.nav-item.active` — 2px `--accent` border-left, `--accent-dim` background, `--accent` text color
9. Content area: `.content` — overflow-y auto, `--sp-6` padding
10. View switching: `section.view { display: none; } section.view.active { display: block; }`
11. Card: `.card` — `--surface-1` bg, `--border`, `--radius-lg`, `--sp-5` padding
12. Card header: `.card-header` — 11px uppercase IBM Plex Sans, `--text-tertiary`, 0.08em letter-spacing
13. Stat rows: `.stat-row`, `.stat-label`, `.stat-value`
14. Buttons: `.btn`, `.btn-primary` (--accent bg, dark text), `.btn-danger` (--loss), `.btn-success` (--profit), `.btn-sm`, `.btn-ghost` (transparent bg, border only)
15. Form controls: inputs/selects with `--surface-input` bg, `--border`, `--font-data`
16. Data table: `.data-table` — IBM Plex Mono, dense spacing, `--border` row separators
17. Badge/pill: `.pill` — small rounded label with tinted background
18. Toast: updated with new colors
19. Spinner animation
20. Utility classes: `.positive` (--profit), `.negative` (--loss), `.text-muted`, `.text-secondary`
21. Filter chip styles: `.chip`, `.chip.active`

#### Task 1.2: Write the HTML shell
**File:** `src/arb_bot/src/dashboard.html` (rewrite `<body>` structure)

```html
<body>
  <aside class="sidebar">
    <div class="sidebar-top">
      <div class="sidebar-brand">Rumi Arb Bot</div>
      <nav class="sidebar-nav">
        <a class="nav-item active" data-view="overview">Overview</a>
        <a class="nav-item" data-view="charts">Charts</a>
        <a class="nav-item" data-view="trades">Trades</a>
        <a class="nav-item" data-view="admin" id="nav-admin" style="display:none">Admin</a>
      </nav>
    </div>
    <div class="sidebar-bottom">
      <div class="sidebar-status">
        <span class="status-dot" id="status-dot"></span>
        <span id="status-label">Loading...</span>
      </div>
      <div id="principal-display" class="sidebar-principal"></div>
      <button class="btn btn-sm btn-ghost" id="auth-btn" style="width:100%">Login with II</button>
    </div>
  </aside>

  <main class="content">
    <section class="view active" id="view-overview"><!-- Phase 2 --></section>
    <section class="view" id="view-charts"><!-- Phase 3 --></section>
    <section class="view" id="view-trades"><!-- Phase 4 --></section>
    <section class="view" id="view-admin"><!-- Phase 5 --></section>
  </main>

  <div id="toast"></div>
</body>
```

JS: Click handler on nav items switches `.active` class on both nav items and view sections.

```js
document.querySelectorAll('.nav-item').forEach(item => {
  item.addEventListener('click', () => {
    document.querySelectorAll('.nav-item').forEach(n => n.classList.remove('active'));
    document.querySelectorAll('.view').forEach(v => v.classList.remove('active'));
    item.classList.add('active');
    document.getElementById('view-' + item.dataset.view).classList.add('active');
  });
});
```

Also wire up `switchView(name)` helper for programmatic navigation (e.g., "View all →" link).

#### Task 1.3: Port the JS module scaffolding
**File:** `src/arb_bot/src/dashboard.html`

Carry over from current dashboard:
- Import map (@dfinity/*)
- IDL definitions (all existing types + new CycleSnapshot)
- Constants (IC_HOST, TOKEN_INFO, POOL_COINS, etc.)
- State variables (canisterId, authClient, anonymousActor, etc.)
- Helper functions (bi, fmt$, fmtTok, fmtTime, etc.)
- Actor creation functions
- Auth flow (doLogin, doLogout, restoreSession, requireAuth)
- Toast function

Add new:
- `CycleSnapshot` IDL type
- `get_snapshots` in the service definition
- `switchView()` function
- `snapshotCache` variable

#### Task 1.4: Build to verify shell
**Command:**
```bash
cd /Users/robertripley/coding/rumi-arb-bot
PATH="$HOME/.cargo/bin:$HOME/Library/Application Support/org.dfinity.dfx/bin:$PATH" cargo build --target wasm32-unknown-unknown --release -p arb_bot
```

#### Task 1.5: Commit
```bash
git add src/arb_bot/src/dashboard.html
git commit -m "dashboard: design system + sidebar layout shell with IBM Plex, noise grain"
```

---

### Phase 2: Overview View

#### Task 2.1: Hero P&L strip
**File:** `src/arb_bot/src/dashboard.html`

Full-width card at the top of Overview. HTML:
```html
<div class="card hero-strip">
  <div class="card-header">NET P&L</div>
  <div class="hero-strip-content">
    <div class="hero-left">
      <div class="hero-number" id="hero-pnl">--</div>
    </div>
    <div class="hero-right">
      <div class="hero-stat"><span class="stat-label">Trades</span><span class="stat-value" id="hero-trades">--</span></div>
      <div class="hero-stat"><span class="stat-label">Volume</span><span class="stat-value" id="hero-volume">--</span></div>
      <div class="hero-stat"><span class="stat-label">DEXes</span><span class="stat-value" id="hero-dex">--</span></div>
    </div>
  </div>
</div>
```

CSS: `.hero-strip-content` is flex with space-between. `.hero-number` is 36px IBM Plex Mono weight 600, colored by P&L sign. `.hero-right` is a column of stat rows, smaller, right-aligned.

Wire to `get_summary()` data in `loadSummary()`.

#### Task 2.2: Spread Thermometer
**File:** `src/arb_bot/src/dashboard.html`

Full-width card below hero. Two gauges (Strategy A, Strategy B).

HTML per gauge:
```html
<div class="spread-row">
  <div class="spread-meta">
    <span class="spread-label">Strategy A — Rumi vs ICPSwap</span>
    <span class="spread-value" id="spread-a-value">--</span>
  </div>
  <div class="spread-gauge">
    <div class="spread-gauge-track">
      <div class="spread-gauge-fill" id="spread-a-fill"></div>
      <div class="spread-gauge-center"></div>
      <div class="spread-gauge-threshold spread-gauge-threshold-left" id="spread-a-thresh-l"></div>
      <div class="spread-gauge-threshold spread-gauge-threshold-right" id="spread-a-thresh-r"></div>
    </div>
  </div>
</div>
```

CSS:
```css
.spread-gauge-track {
  height: 6px;
  background: var(--surface-3);
  border-radius: 3px;
  position: relative;
  overflow: visible;
}
.spread-gauge-fill {
  height: 100%;
  border-radius: 3px;
  position: absolute;
  top: 0;
  transition: all 0.4s ease;
  /* width and left set by JS */
}
.spread-gauge-center {
  position: absolute;
  left: 50%;
  top: -2px;
  width: 1px;
  height: 10px;
  background: var(--text-muted);
}
.spread-gauge-threshold {
  position: absolute;
  top: -3px;
  width: 1px;
  height: 12px;
  background: var(--text-tertiary);
  /* left position set by JS based on min_spread_bps */
}
```

JS function:
```js
function updateSpreadGauge(fillEl, valueEl, threshLEl, threshREl, spreadBps, minSpreadBps) {
  const maxBps = 200; // gauge range ±200 bps
  const pct = Math.min(Math.abs(spreadBps) / maxBps, 1) * 50; // 0-50% of track
  const isAboveThreshold = Math.abs(spreadBps) >= minSpreadBps;
  const color = isAboveThreshold ? 'var(--accent)' : 'var(--text-muted)';

  if (spreadBps >= 0) {
    fillEl.style.left = '50%';
    fillEl.style.width = pct + '%';
  } else {
    fillEl.style.left = (50 - pct) + '%';
    fillEl.style.width = pct + '%';
  }
  fillEl.style.background = color;

  // Threshold markers
  const threshPct = Math.min(minSpreadBps / maxBps, 1) * 50;
  threshLEl.style.left = (50 - threshPct) + '%';
  threshREl.style.left = (50 + threshPct) + '%';

  // Value text
  const sign = spreadBps >= 0 ? '+' : '';
  const dot = isAboveThreshold ? ' ●' : '';
  valueEl.textContent = sign + spreadBps + ' bps' + dot;
  valueEl.style.color = isAboveThreshold ? 'var(--accent)' : 'var(--text-secondary)';
}
```

Wire to `loadPrices()` response.

#### Task 2.3: Prices + Balances cards
**File:** `src/arb_bot/src/dashboard.html`

Two-column grid below spread thermometer. `display: grid; grid-template-columns: 1fr 1fr; gap: var(--sp-4);`

Prices card: Strategy A section (Rumi, ICPSwap, VP) and Strategy B section (icUSD pool, ckUSDC pool). Show "Login to refresh" when not authed (prices still show from last fetch if any).

Balances card: All 5 token balances. Estimated USD total at bottom with a subtle top border separator.

USD total calculation:
```js
const totalUsd = (bi(bal3usd) / 1e8 * vp) + (bi(balCkusdc) / 1e6) + (bi(balIcusd) / 1e8) + (bi(balIcp) / 1e8 * icpPrice) + (bi(balCkusdt) / 1e6);
```
Where `vp` = virtual_price / 1e18, `icpPrice` = icpswap_icp_price_ckusdc / 1e6.

#### Task 2.4: Recent trades compact table
**File:** `src/arb_bot/src/dashboard.html`

Card with last 5 trade legs. Compact format — each row is a single line with time, type (color text), DEX, and the swap description. "View all →" span with click handler to `switchView('trades')`.

Load via `get_trade_legs(0, 5)`.

#### Task 2.5: Wire data loading
**File:** `src/arb_bot/src/dashboard.html`

Updated `loadAll()`:
```js
async function loadAll() {
  await loadConfig();
  loadSummary();
  loadBalances(currentConfig);
  loadTrades();
  loadErrors();
  loadActivity();
  if (authenticatedActor) loadPrices();
  loadRecentTrades(); // new — for overview
}
```

Auto-refresh: `setInterval(loadAll, 30000);`

#### Task 2.6: Build, verify, commit
```bash
cargo build --target wasm32-unknown-unknown --release -p arb_bot
git commit -m "dashboard: overview — hero P&L, spread thermometer, prices, balances, recent trades"
```

---

### Phase 3: Charts View

#### Task 3.1: lightweight-charts script tag + chart container HTML
**File:** `src/arb_bot/src/dashboard.html`

Add before the module script:
```html
<script src="https://unpkg.com/lightweight-charts@4.1.3/dist/lightweight-charts.standalone.production.js"></script>
```

Charts view HTML:
```html
<section class="view" id="view-charts">
  <div class="chart-controls">
    <div class="chart-tabs">
      <button class="chip active" data-chart="spread">Spread</button>
      <button class="chip" data-chart="prices">Prices</button>
      <button class="chip" data-chart="pnl">P&L</button>
      <button class="chip" data-chart="balances">Balances</button>
      <button class="chip" data-chart="vp">Virtual Price</button>
    </div>
    <div class="chart-ranges">
      <button class="chip chip-sm" data-range="3600">1H</button>
      <button class="chip chip-sm" data-range="21600">6H</button>
      <button class="chip chip-sm active" data-range="86400">24H</button>
      <button class="chip chip-sm" data-range="604800">7D</button>
      <button class="chip chip-sm" data-range="2592000">30D</button>
      <button class="chip chip-sm" data-range="0">All</button>
    </div>
  </div>
  <div class="chart-container" id="chart-container"></div>
  <div class="chart-footer" id="chart-footer"></div>
</section>
```

CSS: `.chart-container` — height 500px, width 100%, `--surface-1` background, `--border`, `--radius-lg`.

#### Task 3.2: Snapshot data fetching + caching
**File:** `src/arb_bot/src/dashboard.html`

```js
let snapshotCache = [];
let snapshotsLoaded = false;

async function loadSnapshots() {
  snapshotCache = [];
  let offset = 0;
  const batchSize = 2000;
  while (true) {
    const batch = await anonymousActor.get_snapshots(BigInt(offset), BigInt(batchSize));
    snapshotCache.push(...batch);
    if (batch.length < batchSize) break;
    offset += batchSize;
  }
  snapshotsLoaded = true;
}
```

Called when Charts view is first switched to (lazy load).

#### Task 3.3: Chart rendering engine
**File:** `src/arb_bot/src/dashboard.html`

Core functions:
- `createBaseChart(containerId)` — creates a LightweightCharts instance with the dark theme options
- `destroyChart()` — removes current chart instance
- `filterByRange(data, rangeSeconds)` — filters snapshot array by timestamp
- `renderSpreadChart(data)`, `renderPriceChart(data)`, `renderPnlChart()`, `renderBalanceChart(data)`, `renderVpChart(data)` — each creates the appropriate series

Wire chart tab clicks to destroy + re-render. Wire range buttons to re-filter + re-render.

Default: Spread chart, 24H range.

#### Task 3.4: Spread History chart implementation
```js
function renderSpreadChart(snapshots) {
  const chart = createBaseChart('chart-container');
  const seriesA = chart.addLineSeries({ color: '#00d4aa', lineWidth: 2, title: 'Strategy A' });
  const seriesB = chart.addLineSeries({ color: '#5b8af0', lineWidth: 2, title: 'Strategy B' });

  seriesA.setData(snapshots.map(s => ({
    time: Math.floor(bi(s.timestamp) / 1e9),
    value: bi(s.spread_a_bps),
  })));
  seriesB.setData(snapshots.map(s => ({
    time: Math.floor(bi(s.timestamp) / 1e9),
    value: bi(s.spread_b_bps),
  })));

  // Trade markers on Strategy A line
  const markers = snapshots
    .filter(s => s.traded)
    .map(s => ({
      time: Math.floor(bi(s.timestamp) / 1e9),
      position: 'aboveBar',
      color: s.strategy_used === 'A' ? '#00d4aa' : '#5b8af0',
      shape: 'circle',
      size: 1,
    }));
  seriesA.setMarkers(markers.filter(m => m.color === '#00d4aa'));
  seriesB.setMarkers(markers.filter(m => m.color === '#5b8af0'));

  // Min spread threshold reference line
  const minSpread = currentConfig ? bi(currentConfig.min_spread_bps) : 50;
  // lightweight-charts doesn't have native horizontal lines, use a baseline series or price line
  seriesA.createPriceLine({ price: minSpread, color: 'rgba(255,255,255,0.15)', lineWidth: 1, lineStyle: 2, title: 'threshold' });
  seriesA.createPriceLine({ price: -minSpread, color: 'rgba(255,255,255,0.15)', lineWidth: 1, lineStyle: 2 });

  chart.timeScale().fitContent();
}
```

#### Task 3.5: Price Comparison chart implementation
Two line series: Rumi USD price and ICPSwap USD price. Same pattern as spread chart.

```js
function renderPriceChart(snapshots) {
  const chart = createBaseChart('chart-container');
  const rumi = chart.addLineSeries({ color: '#00d4aa', lineWidth: 2, title: 'Rumi' });
  const icpswap = chart.addLineSeries({ color: '#5b8af0', lineWidth: 2, title: 'ICPSwap' });

  rumi.setData(snapshots.map(s => ({
    time: Math.floor(bi(s.timestamp) / 1e9),
    value: bi(s.rumi_icp_price_usd) / 1e6,
  })));
  icpswap.setData(snapshots.map(s => ({
    time: Math.floor(bi(s.timestamp) / 1e9),
    value: bi(s.icpswap_icp_price_ckusdc) / 1e6,
  })));

  chart.timeScale().fitContent();
}
```

#### Task 3.6: Cumulative P&L chart implementation
Area chart from trade legs data (not snapshots).

```js
async function renderPnlChart() {
  // Fetch ALL trade legs
  let allLegs = [];
  let offset = 0;
  while (true) {
    const batch = await anonymousActor.get_trade_legs(BigInt(offset), BigInt(2000));
    allLegs.push(...batch);
    if (batch.length < 2000) break;
    offset += 2000;
  }

  // Compute running P&L
  let cumPnl = 0;
  const data = allLegs.map(leg => {
    cumPnl += (bi(leg.bought_usd_value) - bi(leg.sold_usd_value) - bi(leg.fees_usd));
    return {
      time: Math.floor(bi(leg.timestamp) / 1e9),
      value: cumPnl / 1e6,
    };
  });

  const chart = createBaseChart('chart-container');
  const series = chart.addAreaSeries({
    lineColor: '#00d4aa',
    topColor: 'rgba(0, 212, 170, 0.3)',
    bottomColor: 'rgba(0, 212, 170, 0.02)',
    lineWidth: 2,
    title: 'Cumulative P&L',
  });
  series.setData(data);
  chart.timeScale().fitContent();
}
```

#### Task 3.7: Balance Composition chart implementation
Line series per token (converted to USD) plus total line.

#### Task 3.8: Virtual Price chart implementation
Simple single line series.

#### Task 3.9: Chart tab/range interaction wiring
Click handlers on chart tabs and range buttons. Store current chart type and range in state variables. Destroy and recreate chart on change.

#### Task 3.10: Empty state for no data
If `snapshotCache.length === 0`, show a centered message in the chart container instead of an empty chart:
```html
<div class="chart-empty">
  <div class="chart-empty-icon">📊</div>
  <div class="chart-empty-title">Collecting data...</div>
  <div class="chart-empty-text">The bot records prices, balances, and spreads every 3 minutes. Charts will appear as data accumulates.</div>
</div>
```

#### Task 3.11: Build, verify, commit
```bash
cargo build --target wasm32-unknown-unknown --release -p arb_bot
git commit -m "dashboard: charts view — spread, prices, P&L, balances, VP with lightweight-charts"
```

---

### Phase 4: Trades View

#### Task 4.1: Trade legs table with filter chips
**File:** `src/arb_bot/src/dashboard.html`

HTML for trades view with filter chips (All, Leg 1, Leg 2, Drain) and the full-width table. Same columns as current.

Filter chips use client-side filtering: when a chip is active, only show matching `leg_type` rows. This filters the already-fetched page of data.

Port the existing `loadTrades()` function with updated HTML rendering to use new CSS classes.

#### Task 4.2: Activity Log section
Port existing activity log with category filter. Styled consistently with new design system. Collapsible with triangle toggle.

#### Task 4.3: Build, verify, commit
```bash
cargo build --target wasm32-unknown-unknown --release -p arb_bot
git commit -m "dashboard: trades view — trade legs table with filters, activity log"
```

---

### Phase 5: Admin View

#### Task 5.1: Bot Controls card
Port Pause/Resume, Setup Approvals, Manual Arb, Dry Run with updated styling. Dry Run result panel uses new card/stat styling.

#### Task 5.2: Swap Widget card
Port the entire swap widget (routing logic, quote, execute). Updated styling with new form controls and buttons.

#### Task 5.3: Withdraw + Deposit
Port withdraw form and deposit section. Updated styling.

#### Task 5.4: Config display card
New — read-only display of `currentConfig` fields in a stat-row format.

#### Task 5.5: Auth flow wiring
- Admin nav item hidden by default, shown when `is_admin` returns true
- Login/Logout button in sidebar footer
- Session restore on page load
- My Wallet section moves into Admin view

Port all existing auth code (doLogin, doLogout, restoreSession).

#### Task 5.6: Build, verify, commit
```bash
cargo build --target wasm32-unknown-unknown --release -p arb_bot
git commit -m "dashboard: admin view — controls, swap, withdraw, deposit, config"
```

---

### Phase 6: Polish + Integration

#### Task 6.1: Loading + empty states
- Skeleton shimmer for stat values while loading (CSS animation on placeholder elements)
- Spinner states for buttons (port existing pattern)
- Empty states for tables ("No trades yet")
- "Last updated" timestamp in the footer or below each section

#### Task 6.2: Auto-refresh
- Overview: `setInterval(loadAll, 30000)` (same as current)
- Charts: refresh snapshot data every 60s, update series in place (no chart recreation)
- Price refresh triggered on each cycle

#### Task 6.3: Responsive
At `max-width: 900px`:
- Sidebar becomes a horizontal top bar with icon-only nav
- Card grids go single-column
- Tables get horizontal scroll wrapper
- Chart container reduces to 350px height

#### Task 6.4: Final build + deploy
```bash
cd /Users/robertripley/coding/rumi-arb-bot
PATH="$HOME/.cargo/bin:$HOME/Library/Application Support/org.dfinity.dfx/bin:$PATH" cargo build --target wasm32-unknown-unknown --release -p arb_bot
dfx deploy arb_bot --network ic
```

#### Task 6.5: Final commit + push
```bash
git add -A
git commit -m "dashboard: complete overhaul — trading terminal with charts, IBM Plex, noise grain"
git push
```

---

## File Map

| File | Changes |
|------|---------|
| `src/arb_bot/src/dashboard.html` | Complete rewrite (~2500-3000 lines). Same filename, same `include_str!` path. |
| `src/arb_bot/src/state.rs` | Already done — CycleSnapshot struct added |
| `src/arb_bot/src/arb.rs` | Already done — snapshot recording in run_arb_cycle() |
| `src/arb_bot/src/lib.rs` | Already done — get_snapshots endpoint added |
| `src/arb_bot/arb_bot.did` | Already done — CycleSnapshot type + get_snapshots |

## External Dependencies (CDN)

| Library | URL | Size |
|---------|-----|------|
| lightweight-charts | `https://unpkg.com/lightweight-charts@4.1.3/dist/lightweight-charts.standalone.production.js` | ~45KB gzip |
| IBM Plex Sans | Google Fonts CDN | ~18KB |
| IBM Plex Mono | Google Fonts CDN | ~18KB |
| @dfinity/* | esm.sh (same as current) | unchanged |

No build step. No npm. Everything loads from CDN. The HTML file is self-contained.

## Risk Notes

- **get_prices() requires auth** — Overview prices show last-known values or "Login to refresh" when unauthenticated. Snapshots (query) don't require auth, so charts always work.
- **No snapshot history yet** — Charts will be empty on first deploy. Data accumulates at 1 point per 3 minutes (480/day, ~14,400/month). Charts become useful within hours. Empty state message explains this.
- **File size** — Dashboard HTML grows from ~1333 lines to ~2500-3000 lines. Still well within canister limits (~200KB text). The lightweight-charts library is loaded externally.
- **lightweight-charts v4.1.3** — Pinned. Stable release with good dark mode support and line/area charts.
- **Cumulative P&L chart** requires fetching ALL trade legs — could be slow if there are thousands. Pagination fetches in batches of 2000. Cache the result.
