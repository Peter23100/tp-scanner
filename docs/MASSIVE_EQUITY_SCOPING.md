# Massive equity DataProvider — scoping

Scoped 2026-09-04. **Not started. No code written.** Research is against Massive's live docs as of
that date; re-verify before spending money or time, vendor terms move.

Context: option quotes measured at **~830ms** behind Tradier's own feed stamp, equities ~550ms, our
own pipeline only ~40ms of it (CLAUDE.md → "Options data provider decision"). Decision was to go to
a paid feed. This doc scopes the **cheapest test of that decision**: equity only, one provider, one
new module.

⚠ **Massive is the company formerly known as Polygon.io.** polygon.io 301-redirects to massive.com.
Any "Polygon" reference in older notes means this vendor.

---

## Why equity-first, and why Massive first

**Equity-first** because it is the smallest change that answers the question that actually matters —
*does paying for data fix the latency?* — without touching the options path the user trades on. If
the answer is no, we stop, having spent $199 and a few days rather than a month.

**Massive over Databento**, despite Databento publishing far better latency (p90 590µs internet vs
Massive publishing nothing at all), because of what each vendor *sells*:

- Massive sells **derived data in the shapes this app already consumes** — snapshots, aggregate
  bars, an options chain snapshot carrying greeks/IV/OI. Near 1:1 with the existing `DataProvider`
  trait.
- Databento sells **raw market data** — MBO/MBP/BBO/trades and historical OHLCV. No computed greeks,
  no chain snapshot, no OI. The options half would have to be built, not integrated.

Databento remains the escalation if Massive's measured latency disappoints. Its official
tokio-based Rust client (`databento` on crates.io) would suit this backend well — that build is
plausible, just much larger. **Do not buy Databento first**: that pays the high integration cost
before knowing whether a paid feed helps at all.

Cost note: Massive bills **per asset class**. Equity real-time (Stocks Advanced) is $199/mo; adding
options later is a second $199/mo.

---

## The architecture already supports this

The assumption that this fights the "no half-states" atomic DATA toggle turned out to be **wrong**.
Per-feature routing already exists — every data command resolves through
`state.provider_for_feature(Feature::X)`, with `Feature` being `Quote, MultiQuote, Expirations,
ChainFull, ChainNarrow, History, Timesales, Stream` (`main.rs:47`).

So equity-only is a **routing configuration, not an architectural change**:

| Feature | Provider |
|---|---|
| `Quote`, `MultiQuote`, `History`, `Timesales`, `Stream` | **Massive** |
| `Expirations`, `ChainFull`, `ChainNarrow` | Tradier (unchanged) |

Options continue to stream through `OptionStreamProvider` on Tradier, untouched. The equity stream
already filters OCC symbols out of its union (the Task 4 `isOptSym` fix), so the equity/option split
is clean rather than tangled.

There is precedent for exactly this shape: L2 depth is Schwab-only, and daily history already routes
to Schwab when connected regardless of the DATA toggle.

---

## Trait mapping, method by method

`DataProvider` (`providers/mod.rs:150`) has 10 methods. Equity-only implements 8; the two option
methods return an "unsupported" error and routing never calls them.

### `get_quote` → `GET /v2/snapshot/locale/us/markets/stocks/tickers/{ticker}`

14 of `Quote`'s 16 fields map directly:

| `Quote` field | Massive source |
|---|---|
| `bid` / `ask` | `lastQuote.p` / `lastQuote.P` |
| `bidsize` / `asksize` | `lastQuote.s` / `lastQuote.S` |
| `last` | `lastTrade.p` |
| `open` / `high` / `low` | `day.o` / `day.h` / `day.l` |
| `volume` | `day.v` |
| `close` | `prevDay.c` — this is the **session-relative change base**, see `change_base_close` |
| `change` / `change_pct` | `todaysChange` / `todaysChangePerc` |

**Two gaps:**

1. **`description` is not in the snapshot.** It comes only from `get_quote` today and feeds the
   montage's company-name display. Needs a separate Ticker Details call, cached — a company name
   changes approximately never. Cheap, but if skipped the montage shows blank descriptions and it
   will look like a bug.
2. **`bidexch` / `askexch` are not returned by REST.** They ARE in the WebSocket quote message
   (`bx` / `ax`). So: REST leaves them empty, the stream fills them in. Acceptable, but means a
   freshly-loaded symbol has no exchange codes until its first tick.

### `get_quotes_multi` → Full Market Snapshot, or the multi-ticker form

Massive offers a full-market snapshot endpoint. For a watchlist-sized request the per-ticker
snapshot with a comma-separated list is the better fit; full-market is oversized for this use but is
the right primitive for the **future scanner**.

### `get_timesales` (intraday bars) and `get_history` (daily bars) → Custom Bars

`GET /v2/aggs/ticker/{ticker}/range/{multiplier}/{timespan}/{from}/{to}`

Parameters: `multiplier`, `timespan`, `from`, `to`, `adjusted` (default true), `sort`, `limit`.

✅ **Extended hours are INCLUDED.** The docs state custom bars cover "pre-market, regular market, and
after-hours sessions." This was the single biggest risk going in — `DATA=Schwab` silently kills all
pre/post chart bars because Schwab's timesales has no extended hours. Massive does not have that
defect. **Still verify on day one** with a 4:00–9:30am bar request; do not take the doc's word for
it in week three.

⚠ **Two traps, both of which this codebase has been bitten by before:**

- **`limit` defaults to 5,000, max 50,000.** A 1-minute request over ~10 days with extended hours is
  ~9,600 bars — the default would **silently truncate**. This is precisely the shape of the Tradier
  5-minute truncation bug. Set `limit` explicitly, and assert the returned count against the
  expected window rather than trusting it.
- **`t` is Unix MILLISECONDS**, at the start of the aggregate window. `OHLCVBar.time` is **Unix
  SECONDS** — every chart consumer multiplies by 1000. Divide on ingest. An off-by-1000 here puts
  every bar in 1970 or 56,000 AD, so it will be obvious, but it will cost an hour if unexpected.

`adjusted=true` is right for `get_history` (daily/swing). Consider `adjusted=false` for intraday, so
intraday bars match the live tape on a split day.

### `set_stream_symbols` / `stop_stream` → WebSocket

Channels: `Q` (NBBO quotes), `T` (trades), `A`/`AM` (per-second / per-minute aggregates).

Quote message fields: `bp`/`bs`/`bx` (bid price/size/exchange), `ap`/`as`/`ax` (ask), `sym`, `t`
(Unix ms), `q` (sequence number), `ev`, `c`, `i`, `z`.

Map: `Q` → `quote-update`, `T` → `trade-update` (the tape), plus `stream-status` on connect/error.

**Three properties worth calling out:**

1. **`t` is a feed-side Unix-ms timestamp**, exactly like Tradier's `biddate`/`askdate`. So
   **`[QUOTE-LAG]` works against Massive with no changes** — the acceptance test below is free.
2. **`q` is a sequence number.** Tradier has nothing equivalent. This makes genuine gap detection
   possible for the first time, which speaks directly to the reliability half of the question.
3. **Wildcard `*` subscribes to all tickers.** The future scanner runs on this same connection — no
   second integration. (Bandwidth for full-market equity is far more tractable than OPRA's ~37 Gbps,
   but must still be measured before assuming a home connection takes it.)

Auth is an API key (`MASSIVE_API_KEY` env var in their docs). **Ours goes in Tauri secure storage,
never an env var or localStorage**, per the architecture rules.

---

## Effort

| Piece | Size | Notes |
|---|---|---|
| REST half (`get_quote`, `get_quotes_multi`, `get_timesales`, `get_history`) | **Small** | Mirrors `tradier.rs` / `schwab.rs` closely |
| Ticker-details cache for `description` | **Trivial** | Easily forgotten; visible if missed |
| Streaming half (`set_stream_symbols`, `stop_stream`) | **The real work** | Reconnect, backoff, staleness watchdog, symbol diffing. Three existing implementations to copy the shape from |
| Routing + plumbing | **Small** | Third provider value, persistence, UI toggle, secure-storage credential |

A few focused days, dominated by the streaming supervisor. Not a one-liner, not a rewrite. The
reliability machinery is where the time goes, and skipping it is how you get a feed that works for
an hour and then silently stops — the exact failure mode already documented for Tradier.

---

## Acceptance test — costs nothing to build

`[QUOTE-LAG]` and `[FE-IPCLAG]` are already in the tree (commit `54fed39`). Run the candidate on the
**same box, same session**, and compare against the recorded baseline:

| baseline (Tradier, 2026-09-04) | value |
|---|---|
| option quotes | ~830ms (821/826/836/856, max ~1.6s) |
| **equity quotes** | **~550ms (542–651, max ~1.1s)** |
| our pipeline (`[FE-IPCLAG]`) | 6–51ms |

**Equity ~550ms is the number Massive must beat.** That is the whole point of the exercise.

Rules, learned the hard way:
- **Verify the system clock first** (`w32tm /stripchart`). The first-ever reading was 3269ms; the
  machine was 2.60s fast. 80% of that number was our own clock.
- **Read only `over5s=0` windows.** Subscribe-time snapshots replay last-known NBBO with true old
  timestamps and masquerade as multi-hour latency.
- Verify extended-hours bars explicitly, pre-market, on day one.

---

## Two-machine setup — the connection limit decides the architecture

The user runs the scanner on one computer and trading on another (10 screens total). This is a
real constraint on the design, discovered 2026-09-04, and it is NOT primarily a licensing question.

**★★★ Massive allows ONE simultaneous WebSocket connection per cluster, per account. Connecting a
second time to the same cluster DROPS the existing connection.** Two machines connecting directly
with the same API key would kick each other off in a permanent loop. REST is not limited this way
(both machines may call REST, subject to rate limits) — only the stream is singular. Additional
WebSocket connections can reportedly be purchased via their support team.

**Licence status: probably permitted, NOT confirmed.** The individual ToS grants use "solely for
your own personal, non-commercial, and non-business purposes" and prohibits "sharing an account or
API keys with others" — two of your OWN machines is not obviously "others". But the ToS is **silent
on multi-device personal use**, so do not treat this as settled. Redistribution and customer-facing
display require a Business plan. Confirm with support@massive.com before paying.

**Design: a local relay.** One machine holds the single vendor connection and fans out to both over
the LAN.

- **Put the relay on the TRADING machine.** Trading then gets localhost — no LAN hop on the
  latency-critical path — and the scanner absorbs the hop. Sub-millisecond on a LAN against a ~550ms
  feed is noise, but the ordering matters: the scanner tolerates latency far better than the order
  path.
- This is a larger version of what the backend already does — `app.emit` fans out to 24 webviews,
  measured at 1–5ms (`[FE-IPCLAG]`). The pattern is proven cheap here.
- **The real prize is decoupling.** A relay separates the market-data session from the trading app,
  so a TradierPro rebuild/relaunch no longer tears down and re-establishes the feed. Given the
  Tradier session-churn history (restart storms, orphan sessions, "too many sessions" backoff) that
  is an architectural gain in its own right, independent of the two-machine need.
- **Honest cost:** a new single point of failure — relay down means both machines lose data. And it
  is real work: a headless Rust binary sharing the provider module, plus a LAN transport.

**Cheaper alternative to price first:** just buy an additional WebSocket connection from support and
keep both machines independent. If that is inexpensive it beats building a relay. Ask in the same
email as the licence question.

⚠ Note this constraint likely applies to Databento too, in its own form, and Databento's Plus tier
($1,750/mo) is explicitly the one carrying "external distribution" — so a two-machine setup may be a
materially more expensive question there. Confirm before switching vendors on latency grounds alone.

## What the setup actually looks like, in two phases

**The relay is NOT needed to start.** Primus runs on its own vendor data and never touches a Massive
subscription. The one-connection limit only bites when the user's OWN scanner exists and wants the
same feed. So Phase 1 is a single machine, and the two-machine problem is deferred until it is real.

### Phase 1 — one machine, no relay (this is the $199 test)

```
TRADING PC
┌────────────────────────────────┐
│ TradierPro                     │──WS────→  Massive  (stocks cluster, 1 conn)
│   MassiveProvider  (equity)    │──REST──→  Massive  (snapshots, agg bars)
│   TradierProvider  (options)   │──WS────→  Tradier  (unchanged)
│   IBKR / TOS                   │
└────────────────────────────────┘

SCANNER PC
┌────────────────────────────────┐
│ Primus  — own vendor data      │   untouched, not involved
└────────────────────────────────┘
```

Nothing about the second machine changes. Equity quotes/charts/tape come from Massive, options stay
on Tradier, and the per-feature routing table is what splits them.

**User action list for Phase 1:**
1. Email support@massive.com with the two open questions (multi-device personal use; cost of an
   additional WebSocket connection). Both matter more later than now, but the answers shape Phase 2.
2. Subscribe to **Stocks Advanced, $199/mo** — the only individual tier with real-time equity. The
   cheaper tiers are 15-minute delayed and would prove nothing.
3. Generate an API key (massive.com/dashboard/keys) and hand it over ONCE — it goes into Tauri
   secure storage, never a file, env var, or localStorage.
4. Close the app for one rebuild, relaunch via the desktop shortcut.
5. Trade normally for a session, then compare `[QUOTE-LAG] equity` against the **~550ms** baseline.

That is the whole test. If it does not beat 550ms meaningfully, stop — the answer is that a paid
feed does not fix this, and Databento becomes the next question rather than a bigger Massive spend.

### Phase 2 — two machines, add the relay (only when the user's own scanner exists)

```
TRADING PC
┌──────────────────────────────────────────────┐
│  tp-relay.exe   ←──── ONE WS ────  Massive   │   headless, holds the only connection
│        │                                      │
│        ├── localhost ──→ TradierPro           │   no LAN hop on the trading path
│        │                                      │
└────────┼─────────────────────────────────────┘
         │  LAN   ws://trading-pc:9001
         ▼
SCANNER PC
┌──────────────────────────────────────────────┐
│  user's own scanner                           │
└──────────────────────────────────────────────┘
```

**Design note that keeps this cheap:** the relay re-broadcasts Massive's WS messages verbatim over a
local WebSocket. `MassiveProvider` then points at `ws://localhost:9001` instead of Massive's URL —
**same parser, different URL, one config value**. The relay is a transport, not a translator.

REST does not need relaying at all (it is rate-limited, not connection-limited), so both machines can
call REST directly. Only the stream is singular.

**User action list for Phase 2:**
1. Reserve a static LAN IP (or DHCP reservation) for the trading PC so the scanner has a stable
   address.
2. Windows Firewall inbound rule for the relay port, scoped to the LAN subnet only — never public.
3. Run the relay as a startup task or Windows service so it is up before TradierPro.
4. Point the scanner at `ws://<trading-pc>:9001`.

**Which machine hosts it?** Recommend the TRADING PC: trading keeps localhost, and a relay is a
separate process so it survives every TradierPro rebuild — the feed no longer dies each time the app
is rebuilt, which is a real gain given how often that happens. The one thing that would flip this
recommendation is if the trading PC is REBOOTED more often than the scanner PC; a reboot takes the
relay down for both. The LAN hop itself is sub-millisecond and irrelevant against a ~550ms feed, so
latency is not the deciding factor — stability is.

**Failure behaviour comes free.** If the relay dies, `MassiveProvider` fails its stream and the
existing DATA toggle can fall back to Tradier — degraded latency, but never a dead app. Worth wiring
the fallback deliberately rather than relying on it by accident.

## Relay resource cost — measured on the actual trading PC (2026-09-04)

**Machine:** i7-12700KF, 12 physical / 20 logical cores, 15.8 GB RAM.

**The relay itself is a rounding error.** A headless Rust binary holding one WebSocket and fanning
out to a couple of local consumers: order of **10–30 MB RAM and 1–2% of a single core** for the
trading symbol set (tens to low hundreds of symbols, a few thousand messages/sec at most). On a
20-thread CPU that is nothing. **No upgrade is needed for the relay.**

**⚠ But the machine is already under real memory pressure, independent of any of this:**

| metric | value |
|---|---|
| TradierPro + WebView2 | **49 processes, 4.67 GB** working set |
| Available memory | **3.1 GB of 15.8 GB** |
| Committed | **19.6 GB against 15.8 GB physical** (25.3 GB commit limit) |

Committed memory exceeds physical RAM, so the machine is paging. That is the cost of the
24-popout-window architecture — each Tauri webview is its own WebView2 process. It is not caused by
anything in this scope, and the relay would not meaningfully worsen it, but it is worth knowing for
two reasons:

1. **If a hardware upgrade is wanted, it is RAM (32 GB), and the reason is the windows, not the
   relay.** Paging can produce latency spikes, which is an unwelcome property on a machine whose
   whole purpose is reacting fast. Measured pipeline latency is currently fine (`[FE-IPCLAG]`
   1–51ms), so this is a risk factor, NOT a proven cause of anything observed.
2. **Do not plan to run the scanner on this machine.** The memory numbers say no, which independently
   reinforces the two-machine split the user already has.

**Firehose caveat.** The figures above are for the trading symbol set. A full-market wildcard (`*`)
subscription for the scanner is a materially heavier stream, and Massive's own guidance is that
there is no ticker limit "as long as you can consume the data fast enough."

That drives one design decision: make the relay a **filtering** relay, not a dumb pipe. A dumb pipe
is cheaper for the relay but would push the entire firehose into TradierPro, which would then have to
filter it — loading the memory-pressured trading app for no benefit. Parsing just the symbol field
and routing per-subscription keeps the firehose off the trading path entirely, and still sits well
within one core in Rust.

## Explicitly out of scope

- Options via Massive (a second $199/mo and a second integration — only after equity proves out)
- The filtering/scan system itself
- Removing or changing Tradier, Schwab, or IBKR in any way
- The fill-notification latency, which is a **broker REST polling** problem (1.5s blotter multi-poll,
  measured 0.46–0.88s floor) and is untouched by any data-vendor change

## Confirm with the vendor before paying

- Real-time equity WebSocket entitlement on Stocks Advanced at $199 — confirm it is not delayed
- Professional vs non-professional subscriber classification and its fee impact
- Rate limits on the real-time tier (not stated in public docs)
- Bandwidth of a full-market (`*`) equity subscription, for the scanner

Sources: massive.com/docs (REST stocks overview, single-ticker snapshot, custom bars, WS quotes),
massive.com/pricing, databento.com/pricing, databento.com/datasets/OPRA.PILLAR, docs.tradier.com.
