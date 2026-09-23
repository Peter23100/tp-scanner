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

Stage 2 (market-data primitives), first slice. No market-data subscription yet; everything runs
on fixtures. The filter language waits for the exported Primus filter list, because those
filters are its conformance set.
