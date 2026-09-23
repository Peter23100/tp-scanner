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
| `crates/tp-scan-engine` | per-symbol state, measurement catalog, filter expressions, evaluator (memo + explain), scan loop, JSONL instrumentation |
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
```

Parity fixtures only need regenerating when TradierPro's chart math changes:

```
node tools/parity/gen.mjs        # rewrites tools/parity/expected/*.json from reference.mjs
```

`make_fixture.mjs` rewrites the input CSV. It's deterministic, so rerunning it produces no diff.

## Status

No market-data subscription yet; everything runs on fixtures and synthetic data.

- **Built:** stage 2 primitives. Most of the stage-3 evaluator: filters compile to a closure
  tree, measurements and definitions are memoized once per symbol per cycle, logic is
  three-valued, and `explain` shows which clause was false or unknown. Also the scan loop (a bad
  symbol never stops the scan), and stage-4 instrumentation (every evaluation's measurement
  vector as JSONL).
- **Measured** (`cargo run --release -p tp-scan-engine --example throughput`): 5,000 symbols ×
  40 filters is one full-market cycle in ~11 ms on one thread, ~1% of a core at 1 Hz. That uses
  synthetic minute bars; it's not yet measured on real data.
- **Not built, waiting on the user:**
  - A text syntax for filters. It gets designed against the exported Primus filters, which are
    its conformance set; until then, filters are built in Rust (see `expr.rs`).
  - `RVOL20DAYTIMEADJUSTED`: its exact definition has to come from the user.
- **Flat-file reader** (`flatfile.rs`): written against the columns and nanosecond timestamps
  confirmed in Massive's docs; tested on hand-made rows only. ⚠ Two things to confirm with
  Massive: whether flat-file prices are split-adjusted (the reader assumes raw), and the file
  compression (the reader takes an already-decompressed stream).
- **Not built, waiting on the subscription:** the historical store and adjustment pipeline
  (stage 1 beyond the reader), the live driver (stage 6), and a market calendar (holidays and
  half-days).
