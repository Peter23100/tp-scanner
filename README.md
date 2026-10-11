# tp-scanner

A real-time equity scanner to replace Primus (subscription runs to ~May 2027).

The design and its reasoning are in TradierPro's `PRIMUS_REPLACEMENT_INVESTIGATION.md`. This repo
is separate from TradierPro on purpose. The scanner runs user-written expressions over the whole
market, and none of that may share a process, or a failure, with order placement. **The scanner
never holds broker credentials and never touches the order path.**

## Layout

| path | what |
|---|---|
| `crates/tp-marketdata` | session clock, typed prices, bars, rolling windows, the market-data input trait, indicators |
| `crates/tp-scan-engine` | per-symbol state, measurement catalog, the filter language (`lang.rs`), evaluator (memo + explain), scan loop with triggers and ranked lists, JSONL instrumentation |
| `crates/tp-scan-cli` | the `tp-scan` binary: `check` filter files, `replay` history, `sync` the Massive cache, `fetch-day`, `live` |
| `crates/tp-massive` | Massive data: the live WebSocket source, REST (daily bars, splits, float, holidays, chart history), the daily cache, flat files |
| `app/` | **the desktop app** (Tauri 2 + React, TradierPro's look): a window per panel and chart, TradierPro's chart, the filter tools. Its own Cargo workspace |
| `filters/display.json` | the Primus screens: the starting layout of the panel windows (after that, `panels.json` in the data dir) |
| `filters/baskets.scan` | named baskets (created by the app's basket editor when you make the first) |
| `ROADMAP.md` | the six-month plan to a full replacement and past it |
| `filters/` | the Primus filters and custom fields, translated. `filters/README.md` has the translation rules, what's missing, and where results will differ from Primus on purpose |
| `filters/LANGUAGE.md` | **how to write filters**: every building block, Primus and Trade Ideas equivalents, a cookbook |
| `filters/CHANGES.md` | every filter change, dated, with the reason |
| `examples/` | example filters from the cookbook; compiled by the tests, never loaded to run |
| `spec/primus/` | the user's Primus filter master (verbatim) and screenshots of the four screens |
| `tools/parity` | the TradierPro chart math (verbatim JS snapshot) and the generator for the expected outputs the Rust port is tested against |

## Rules that hold from the first commit

1. **Adjusted and unadjusted prices are different types.** `Price<Adjusted>` and
   `Price<Unadjusted>` can't be compared or mixed without an explicit `AdjFactor`. The
   incumbent's worst bug (a $188 stock with a $53 ATR) is a compile error here, and the
   `compile_fail` doctests in `price.rs` keep it that way.
2. **One door for market data.** Everything downstream consumes `MarketSource`. Replay is one
   implementation and live will be the other, so the two paths can't diverge.
3. **Every primitive documents its exact semantics and has a unit test**, both in the same commit
   that adds it.
4. **Sessions run on New York time**, derived from UTC instants, never from the machine clock.
5. **Windows are rolling time spans**, not clock-aligned buckets and not item counts.

## Running

### One-time setup

Keys live in environment variables, never in the repo (set them yourself; `setx` then open a new
terminal):

```
setx MASSIVE_API_KEY "..."      # Massive dashboard: API key
setx MASSIVE_S3_KEY_ID "..."    # Massive dashboard: Flat Files access key (replay downloads)
setx MASSIVE_S3_SECRET "..."
```

Then fill the local cache (`%LOCALAPPDATA%\tp-scanner`; first run ~260 daily files, after that
only new days):

```
cargo run --release -p tp-scan-cli -- sync
```

Run `sync` each morning before the session (it adds yesterday and refreshes splits and float).

### The app

```
cd app
npm install                       # once
npx tauri build --no-bundle       # -> app/src-tauri/target/release/tp-scanner.exe
npx tauri dev                     # or: run from source with hot reload
```

The launcher bar (TradierPro's menubar and status bar):

- **LEFT / RIGHT / TOP / FOURTH** open that Primus screen's panels and charts, each its own window,
  tiled on its own monitor the first time and wherever you put them after that. **Panels ▾** lists
  every panel (reopen, delete, reset to the Primus layout); **+ Panel** and **+ Chart** add one.
- **Baskets ▾**: named sets of qualifications (type, exchange, price, float, average volume, ATR)
  a filter can draw from; the editor shows how many symbols pass before you save.
- **LIVE** (Massive WebSocket; delayed on Starter) or **REPLAY** a past day (downloaded on first
  use) from a chosen time at 1x-300x or max speed.

In a panel: click a symbol to send it to every chart of the panel's link colour; with **Charts
follow new fires** on (the default) each new fire does it for you, and a chart's 🔒 holds what it
shows. **Right-click** for the panel menu: link colour, follow, sound, rename, the filters in the
panel, **Why isn't SYMBOL firing?** (the day minute by minute: when it would have fired, how often
each clause passed, the closest miss with the values), and **Adjust FILTER** (every number
editable, Test against a recorded day, Save with a backup and a CHANGES.md entry, Undo). Charts are
TradierPro's, with the scanner's signals drawn as arrows at the price that fired them (premarket
bars stay after the open only for a symbol that fired premarket). Windows have no OS title bar:
drag the header, double-click it to maximize; Ctrl + mouse wheel (or the menu's A−/A+) sizes a
panel's text. Signals are
logged to `%LOCALAPPDATA%\tp-scanner\signals\DAY.jsonl`. Windows, link colours and the running
source come back at the next launch.

### The engine and CLI

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

cargo run -p tp-scan-cli -- check filters
cargo run --release -p tp-scan-cli -- replay --filters filters --bars <day.csv[.gz]> [--massive] \
    --daily <daily.csv> [--float <float.csv>] [--jsonl out.jsonl [--instrument]] [--explain FILTER:SYMBOL]
cargo run --release -p tp-scan-cli -- replay --filters filters --bars <day.csv.gz> --massive --cache
cargo run --release -p tp-scan-cli -- fetch-day 2026-10-09
cargo run --release -p tp-scan-cli -- live --filters filters
```

The app is a separate Cargo workspace (`app/src-tauri`): tauri-build's static-CRT linker settings
break the engine crates' doctests when built in one workspace. Test it with `cargo test` inside
`app/src-tauri`.

`replay`'s input formats are documented at the top of `crates/tp-scan-cli/src/main.rs`. `--daily`
takes split-adjusted daily bars (`symbol,date,open,high,low,close,volume`). Each session's reference
is built only from days before it.

Parity fixtures only need regenerating when TradierPro's chart math changes:

```
node tools/parity/gen.mjs        # rewrites tools/parity/expected/*.json from reference.mjs
```

`make_fixture.mjs` rewrites the input CSV. It's deterministic, so rerunning it produces no diff.

## Status

Massive Stocks Starter (15-min delayed) being set up 2026-10-11. Until the keys are in, everything
has run on fixtures and synthetic data (the app was verified end to end on a synthetic day).

- **Built:** stage 2 primitives and stage 3. The filter language parses to a closure tree;
  measurements and fields are memoized once per symbol per cycle; logic is three-valued; `explain`
  shows which clause was false or unknown. Triggers with cooldowns, ranked lists (Primus statics),
  trading-hours gating, display columns. The scan loop (a bad symbol never stops the scan),
  stage-4 instrumentation (every evaluation's measurement vector as JSONL), and the `tp-scan`
  replay CLI.
- **Translated:** 61 Primus filters and all 21 custom fields (`filters/`), compiled and
  scenario-tested: the master's 2026-09-25 version plus the recovered Contra, Breakout_Hold and
  PM_LowFloat_Coil_Up expressions (`spec/port/`). Still needed from the Primus editor:
  Contra_Long_All / Contra_Short_All, the unnamed short-side divergence filter, plain Halted,
  CatchALL_LF_HOD. The RVOL / PM_RVOL curves are piecewise linear between the master's points;
  the verbatim IF chain would confirm the first segment.
- **Load-time checks:** bounds no value can meet, `=` on a price, and float in the wrong units are
  refused with the line (the defects that kept three Primus filters silent).
- **Measured** (`cargo run --release -p tp-scan-engine --example throughput [-- filters]`): 5,000
  symbols × the 42 translated filters is one full-market cycle in ~28 ms on one thread, ~3% of a
  core at 1 Hz (40 synthetic filters: ~12 ms). Synthetic minute bars; not yet measured on real data.
- **Flat-file reader** (`flatfile.rs`): written against the columns and nanosecond timestamps
  confirmed in Massive's docs; tested on hand-made rows only. ⚠ One thing to confirm with Massive:
  whether flat-file prices are split-adjusted (the reader assumes raw). The CLI reads `.gz`
  directly.
- **App** (`app/`): verified on Friday 2026-10-09's real data (replay): panel windows, menu, why,
  adjust/test/save/undo, TradierPro's chart with signal arrows, follow and hold, baskets.
- **Massive** (`tp-massive`): REST (Bearer), grouped daily, splits, float, holidays, tickers and
  the flat-file path all confirmed working 2026-10-11. ⚠ The live WebSocket
  (`delayed.massive.com`) is not yet seen: first live session Monday 2026-10-12.
- **Not built yet** (ROADMAP.md): reconnect and late-start backfill, market calendar, halts
  (LULD isn't on Starter), imbalances and NYSE IND (no source), alerts, TradierPro link.
