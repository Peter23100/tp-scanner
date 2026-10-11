# Roadmap: a full Primus replacement, and past it

Target: in daily use, replacing Primus, by **April 2027** (Primus lapses ~May 2027). Started building
the app 2026-10-11. Each item is built, verified on real data, then the next; nothing ships stacked
and unverified.

## Phase 1: running on real data (October 2026)
- [x] Engine, language, 61 filters (`filters/`), load-time checks.
- [x] Massive data door (`crates/tp-massive`): live WebSocket (`AM.*`), REST daily cache with
      per-session split adjustment, float, holidays, flat-file download.
- [x] `tp-scan sync | fetch-day | live`; `replay --cache`.
- [x] Desktop app (`app/`): four Primus screens from `filters/display.json`, trigger feeds and
      ranked lists, link colours, chart windows and chart panels, replay at any speed,
      Reload filters, right-click "why did/didn't this fire", resume on launch.
- [ ] **Massive keys set; `tp-scan sync` run; first live delayed session (Monday).**
- [ ] The `spec/port/fixtures.json` real-session baseline: replay each date, record what fired.
- [ ] Port corrections from `spec/port/filters.json`, one at a time, each with before/after.

## Phase 2: correctness gaps (November)
- [ ] **Reconnect backfill**: refill bars missed while disconnected over REST (today a gap reads
      as quiet).
- [ ] **Late start backfill**: launched after 04:00, fetch today's bars for active symbols.
- [ ] **Market calendar** from Massive holidays: half-days, closures (Phase is wall-clock only).
- [ ] **Halts**: LULD is not on Starter. Either the $199 tier, or detect halts from the tape
      (a liquid name with no prints for N minutes, bracketed by a gap).
- [ ] Imbalances / NYSE IND: find a source (Massive doesn't list NOII) or retire those panels.
- [ ] Confirm with Massive: flat files raw or adjusted; AM bar completeness vs trades.

## Phase 3: the daily driver (December–January)
- [ ] **Send to TradierPro**: clicking a symbol sets TradierPro's active symbol. Needs a few lines in
      TradierPro (its single-instance handler ignores arguments today). Ask before touching it.
- [ ] Alerts: per-panel sound and flash on a new fire, with quiet hours.
- [ ] Panel options in the UI: rows kept, dim-after, column widths, sort.
- [ ] Layout editing in the UI (drag panels), written back to `display.json`.
- [ ] Chart: more timeframes, daily chart, prior-day levels, premarket high/low lines, the
      filter's own reference lines (e.g. `pm_high`) drawn when you click its signal.
- [ ] Watchlists and per-symbol mute ("not this one today").

## Phase 4: better than Primus (February–April)
- [ ] **Signal outcomes**: for every fire, what the stock did over the next 5/15/30/60 minutes
      (MFE/MAE), per filter, per day. The precision dial the user asked for: "only tradeable
      symbols".
- [ ] **Filter report card** from replays across months: fires/day, hit rate, best hours, which
      clause kills the most good signals (from `explain`).
- [ ] **What-if**: change a threshold, replay N days, compare fire lists side by side, before
      editing the live filter.
- [ ] `tradeable` gate applied set-wide (spread, price, liquidity), tuned on outcomes.
- [ ] Replay trainer: step through a past day signal by signal, chart linked.
- [ ] Real-time data ($199) when going live for real (~March); latency measured against the
      TradierPro probes.

Automated trading stays out of scope until asked. The scanner never holds broker credentials and
never touches the order path; signals leave it as immutable records.
