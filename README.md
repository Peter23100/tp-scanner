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
| `crates/tp-scan-cli` | the `tp-scan` binary: `check` filter files, `replay` history through them |
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

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

cargo run -p tp-scan-cli -- check filters
cargo run --release -p tp-scan-cli -- replay --filters filters --bars <day.csv[.gz]> [--massive] \
    --daily <daily.csv> [--float <float.csv>] [--jsonl out.jsonl [--instrument]] [--explain FILTER:SYMBOL]
```

`replay`'s input formats are documented at the top of `crates/tp-scan-cli/src/main.rs`. `--daily`
takes split-adjusted daily bars (`symbol,date,open,high,low,close,volume`). Each session's reference
is built only from days before it.

Parity fixtures only need regenerating when TradierPro's chart math changes:

```
node tools/parity/gen.mjs        # rewrites tools/parity/expected/*.json from reference.mjs
```

`make_fixture.mjs` rewrites the input CSV. It's deterministic, so rerunning it produces no diff.

## Status

No market-data subscription yet; everything runs on fixtures and synthetic data.

- **Built:** stage 2 primitives and stage 3. The filter language parses to a closure tree;
  measurements and fields are memoized once per symbol per cycle; logic is three-valued; `explain`
  shows which clause was false or unknown. Triggers with cooldowns, ranked lists (Primus statics),
  trading-hours gating, display columns. The scan loop (a bad symbol never stops the scan),
  stage-4 instrumentation (every evaluation's measurement vector as JSONL), and the `tp-scan`
  replay CLI.
- **Translated:** 42 Primus filters and all 21 custom fields (`filters/`), compiled and
  scenario-tested, current to the master's 2026-09-25 version. Waiting on verbatim expressions: the 14 Contras, Breakout_Hold,
  PM_LowFloat_Coil_Up, plain Halted, CatchALL_LF_HOD. The RVOL / PM_RVOL curves were rebuilt from
  the master's table and prose and need checking against the pasted IF chains.
- **Measured** (`cargo run --release -p tp-scan-engine --example throughput [-- filters]`): 5,000
  symbols × the 42 translated filters is one full-market cycle in ~28 ms on one thread, ~3% of a
  core at 1 Hz (40 synthetic filters: ~12 ms). Synthetic minute bars; not yet measured on real data.
- **Flat-file reader** (`flatfile.rs`): written against the columns and nanosecond timestamps
  confirmed in Massive's docs; tested on hand-made rows only. ⚠ One thing to confirm with Massive:
  whether flat-file prices are split-adjusted (the reader assumes raw). The CLI reads `.gz`
  directly.
- **Not built, waiting on the user:** a display (the Primus screens). The engine produces signals
  and ranked lists; nothing draws them yet.
- **Not built, waiting on the subscription:** the historical store and adjustment pipeline
  (stage 1 beyond the reader), the live driver (stage 6), feeds for halts, imbalances and float
  (the measures exist and read unknown until set), and a market calendar (holidays and half-days).
