# tp-scanner: project context

## Goal
A real-time, full-market equity scanner that replaces Primus (subscription ends ~May 2027). The
user is an active day trader who trades its signals, sibling project of TradierPro
(`C:\tradier-pro`, the trading terminal). The design and its reasoning:
`C:\tradier-pro\PRIMUS_REPLACEMENT_INVESTIGATION.md` (its in-tree workspace layout is superseded:
this is a separate repo by the user's choice).

What the user wants, in their words: translate the existing filters easily · **correct data** ·
adjust filters much more easily · **"tune the program so that it picks up only tradeable symbols:
i waste too much time filtering through the symbols manually"** · eventually automated trading.
Precision (fewer junk fires) is the daily pain; recall (not missing runners) is the other half.

## Hard rules
- **The scanner never holds broker credentials and never touches the order path.** It emits
  signals across a boundary. Automated trading is a stated future direction, NOT current scope:
  don't build toward it unasked.
- **Adjusted and unadjusted prices are different types** (`Price<Adjusted>` / `Price<Unadjusted>`).
  Never weaken this.
- **One market-data door** (`MarketSource`): replay and live are two impls of one trait.
- **Unknown is not zero.** A missing value is unknown and a filter never fires on unknown.
- Sessions run on New York time from UTC instants, never the machine clock. The user is in Puerto
  Rico (UTC-4, no DST).
- **Windows are rolling** time spans unless a filter asks for `clock` by name.
- Throughput is not the problem (measured ~28 ms per 5,000-symbol cycle, one thread). Build the
  simple correct thing; optimise only on measurement. No JIT/Cranelift.
- **Language scope creep is the #1 schedule risk.** No loops, arrays or user state in the filter
  language. New capability arrives as a new builtin measure (`measure.rs` + `lang.rs` `BARE`/`CALLS`
  + `LANGUAGE.md`), not new syntax.

## Changing filters: the workflow (the user asks in plain language, Claude edits)
The user describes a change ("tighten the hold", "add a filter like Trade Ideas' Running Up Now
under $20", "why didn't X fire on 9/24"). For each request:

1. **Read first:** the filter in `filters/*.scan`, its section in `spec/primus/Primus_Filters_MASTER_*.md`
   (intent, history, known defects), and `filters/LANGUAGE.md` for what's expressible.
2. **Show the change before saving** for anything beyond a one-threshold tweak: the old and new
   clauses, and what each change does in trading terms.
3. **One filter change at a time.** Stacked changes make fire-count movement unattributable.
4. Verify, all three, and report the results:
   ```
   cargo run -p tp-scan-cli -- check filters
   cargo test --workspace
   cargo clippy --workspace --all-targets -- -D warnings
   ```
   Add or update a scenario test in `crates/tp-scan-engine/tests/primus_filters.rs` when the
   change alters what the filter catches.
5. **Record it in `filters/CHANGES.md`**: date, what changed (old → new table), why (the trade),
   what verified it, and what wasn't verified (until there's a data subscription: "not verified on
   real data").
6. Keep `filters/README.md` (translation notes) and `LANGUAGE.md` true. The `language_doc` test
   fails if the cookbook drifts from `common.scan` / `examples/recipes.scan`.

Translation rules for anything new from Primus: `filters/README.md`. **Never transcribe a filter
from prose.** A filter the master only describes waits for the verbatim expression (a wrong
threshold is invisible). Hand-translate; don't build a Primus-syntax translator.

When the user supplies a newer master file, diff it against `spec/primus/`, replace the copy
(`git mv` to the new date, keep content verbatim), and translate only what changed.

## Where things live
| path | what |
|---|---|
| `crates/tp-marketdata` | clock, typed prices, bars, windows, indicators, Massive flat-file reader |
| `crates/tp-scan-engine` | language (`lang.rs`), measures (`measure.rs`), state, evaluator + explain, scan loop, JSONL instrumentation |
| `crates/tp-scan-cli` | `tp-scan check` / `tp-scan replay` (formats at the top of `main.rs`) |
| `filters/` | the live filter set; `LANGUAGE.md` reference, `CHANGES.md` log, `README.md` translation notes |
| `examples/recipes.scan` | cookbook filters; compiled by tests, never loaded to run |
| `spec/primus/` | the user's master, verbatim, plus the four screen PNGs |

## Status (2026-10-09)
- Built: stage 2 primitives, stage 3 language + evaluator + explain, stage 4 instrumentation, the
  replay CLI. 42 Primus filters + 21 fields translated; current to the master's 2026-09-25 version.
- **Waiting on the user (verbatim expressions):** the 14 Contras, Breakout_Hold_Up/Down,
  PM_LowFloat_Coil_Up, plain Halted, CatchALL_LF_HOD; the RVOL / PM_RVOL IF chains (curves were
  rebuilt from a table).
- **Waiting on a decision:** the display (Primus screens): technology undecided.
- **Waiting on data:** Massive Stocks Starter ($29/mo) is enough for development (history, flat
  files, delayed WebSocket); real-time ($199) only for live alerting. Open: are flat files
  split-adjusted (reader assumes raw), do historical trade conditions mark LULD halts, scanner PC
  disk/RAM never measured. No market calendar yet (half-days misclassify).

## Coding standards
Same as TradierPro: nothing is complete until it compiles, tests and clippy pass, and the
behaviour is verified; report what was verified and say plainly what wasn't. Stay in scope:
surface unrelated findings, don't fix them unasked. Prefer deleting dead code. Commit only when
the user asks; local git only, no remote.
