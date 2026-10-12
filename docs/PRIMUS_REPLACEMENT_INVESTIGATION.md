# Primus replacement — feasibility investigation

Investigated 2026-09-04 against the brief in §1–9 of the user's spec. **No code written.**
Vendor facts researched against live docs that date; re-verify before committing money.

Headline: **this is not a hard systems problem at your scale, and that is the most useful thing to
know.** The whole live engine fits in roughly one CPU core and a few hundred MB. The difficulty is
concentrated in three places that have nothing to do with throughput — historical data management,
expression-language design, and backtest correctness. Budget accordingly: the instinct to fear
"5,000 symbols at 1Hz" is misplaced, and the instinct that backtesting is the hard part is right.

---

## 1. Answering §8 — Data

### One vendor covers essentially the whole of §3, and it is Massive (ex-Polygon.io)

Checked against the §3 list. Massive endpoints, all under one subscription and one symbology:

| §3 requirement | Massive | Notes |
|---|---|---|
| Consolidated real-time trades + quotes, all US equities | ✅ WS `T`, `Q`, `A` (per-second aggs), `AM` (per-minute) | wildcard `*` = full market |
| Pre/post market, separably | ✅ | aggregates explicitly cover pre/regular/post |
| **Trading halt status** | ⚠️ **LULD via WebSocket — LIVE ONLY** | see gap below |
| Daily adjusted OHLCV | ✅ custom bars, `adjusted` flag | |
| **Adjustment factors exposed** | ✅ `/stocks/corporate-actions/splits`, `/dividends` | factors derivable, not baked in — meets the §3 ask |
| Shares outstanding / float | ✅ `/stocks/fundamentals/float`, ticker-overview | |
| Corporate action calendar | ✅ splits + dividends endpoints | |
| Historical 1-min bars, multi-year | ✅ custom bars **+ daily S3 flat files** | flat files matter enormously, see below |
| Historical tick trades | ✅ `/stocks/trades-quotes/trades` + flat files | |
| *(bonus, not requested)* short interest / short volume | ✅ `/stocks/fundamentals/short-interest`, `/short-volume` | plausibly useful for a squeeze filter |

**Databento cannot serve this brief alone.** Its market data is better in every technical respect —
direct prop feeds from all venues, four timestamps per event at sub-microsecond accuracy, published
p90 590µs over internet — but it sells market data only. **No float, no shares outstanding, no
corporate action calendar, no short interest.** §3 requires all of those, so Databento means two
vendors, two symbologies, and reconciling adjustments across them. That is exactly the class of
problem §2 lists as a reason for leaving the incumbent.

This also aligns with the equity trading-feed decision already scoped in
`MASSIVE_EQUITY_SCOPING.md` — **one vendor, one subscription family, one symbology across the
trading app and the scanner.**

### ⚠️ The one real data gap: historical halts

LULD halt events are live-only. §5 wants halt correctness, and §6 wants backtests that reflect
reality — so replaying a halted session would silently misrepresent exactly the most volatile names,
which are the ones the scanner exists to find.

Mitigations, in order of preference, all needing verification:
1. Check whether historical **trade condition codes** in the tick archive carry halt/resumption
   indicators (they often do). This would be free if true.
2. Reconstruct halts heuristically: a liquid symbol with zero prints for N seconds, bracketed by a
   volatility band. Approximate, but auditable.
3. Source a halt archive separately (Nasdaq publishes historical halt data).

**Resolve this before building the backtester**, not after — it changes what a "correct" replay
means.

### Flat files are the single most valuable thing on the list

Massive publishes **daily S3 flat-file archives** for trades, minute aggregates, and day aggregates.
This is the difference between a backtest corpus you bulk-download once and one you paginate out of a
REST API over weeks. It makes §6 tractable. Design the historical store around flat files from day
one; use the REST history endpoints only for gap-filling.

### Storage — a number worth having up front

| corpus | rough size |
|---|---|
| 1-minute bars, ~8,000 symbols, extended hours, **per year** | ~50 GB raw, **~15–25 GB columnar-compressed** |
| 3 years of minute bars | **~50–75 GB compressed** |
| Tick trades, **per year** | ~190 GB compressed |

**Recommendation: multi-year minute bars, tick for a recent window only** (say 3–6 months). §6 says
"1-minute bars, ideally sub-minute or tick" — minute bars satisfy the backtest for everything except
the tick-precision event triggers in §5, and those can be validated on the shorter tick window. This
keeps the corpus on one ordinary SSD instead of demanding a storage tier.

---

## 2. Answering §8 — Scale. The surprise is that it is easy

### The evaluation load is ~20% of one core

40 expressions × 5,000 symbols × 1 Hz = **200,000 expression-evaluations/sec**.

An AST interpreter over ~20 clauses, where each clause reads a *precomputed* aggregate and does one
comparison, costs on the order of 0.5–1µs. So 200,000 × 1µs ≈ **0.2 core-seconds per second**. Even
five times pessimistic, that is one core.

**This is not the bottleneck, and designing as though it were would be the main way to waste months
here.**

### Ingest volume: subscribe to per-second aggregates, not the trade firehose

The most consequential architectural choice in the whole design:

| subscription | messages/sec | verdict |
|---|---|---|
| `T.*` full trade firehose | 10,000–50,000+ at peak | heavy, and mostly unnecessary |
| **`A.*` per-second aggregates, full market** | **~5,000** (one per active symbol per second) | ~1 MB/s, ~8 Mbps — **trivial** |

Your target cadence is 1 second. Massive's per-second aggregate channel *is* that cadence, delivered
pre-aggregated. Taking `A.*` as the backbone collapses the ingest problem to nothing.

**Cost:** you lose intra-second detail. The §5 event triggers ("new 15-minute low") at true tick
precision need trades. **Hybrid design:** `A.*` for the full universe, `T.*` for a focused subset
(current alerts, watchlist, positions) where instant-precision matters. This directly satisfies §5's
"evaluate continuously enough that instants are never missed" for the symbols that matter, without
paying firehose cost for 5,000 names that mostly do not.

### Memory: two-tier ring buffers, ~150–250 MB total

The naive design fails: 1-second bars for a 12-hour session is 43,200 bars/symbol ≈ 2.1 MB, times
5,000 symbols = **10 GB**. Don't do that.

**Two-tier instead:**

| tier | resolution | span | per symbol |
|---|---|---|---|
| fine | 1-second bars | last ~5 min (300) | ~14 KB |
| coarse | 1-minute bars | last ~120 min (120) | ~6 KB |
| daily reference | ATR14, ADV20, prior week/month H/L, float | loaded at startup | ~1 KB |

≈ **20–30 KB per symbol → 100–150 MB for 5,000 symbols.** Add volume-profile histograms (§3, price
bins per symbol per session) at ~5–20 KB each and you land around **150–250 MB**. Comfortable.

**Rolling-window accuracy consequence, and it is a real design decision to make consciously:** a
rolling 120-minute window built from 1-minute bars is accurate to ±1 minute at the edge — a 0.8%
error. Clock-aligned windows, the thing §2 and §5 identify as the highest-value structural fix, can
be wrong by up to 100% of the window. **1-minute granularity on long windows already captures
essentially all of the available win.** Short windows (1–5 min) get 1-second granularity, so they are
accurate to the second where it matters.

### Where the crossover actually is (§8's question)

Recomputing a 120-minute aggregate from 1-minute bars is ~120 operations. Doing that for 5,000
symbols × 6 timeframes × 1 Hz = **3.6M ops/sec — still affordable.** So:

- **Incremental (rolling sums) is better and not hard**, but
- **naive recompute is affordable at this scale**, which means you can build the simple correct thing
  first and optimise only if measurement says to.

That is an unusually comfortable position and worth exploiting: correctness first, with real
confidence that performance will not force a rewrite.

---

## 3. Answering §8 — Expression evaluation

**Parse to AST, then "compile" to a tree of Rust closures.** Not an interpreter walking an AST node
by node at evaluation time, and emphatically not a JIT.

- Walking an AST re-dispatches on node type on every evaluation — 200,000 times a second, that is
  wasted branch prediction for no benefit.
- Closure-tree compilation (each AST node becomes a `Box<dyn Fn(&Ctx) -> Value>` built once at parse
  time) removes the dispatch, is ~50 lines of technique, and gets most of a real compiler's win.
- Cranelift/LLVM would be **strictly wasted effort** at 20% of a core.

**Introspection (§2's second complaint) should be designed in here, not bolted on.** Build the
evaluator so that every boolean node can optionally record its own value and its children's values
during an evaluation. Then "which clause was false" is a debug-mode re-evaluation that returns a
tree, not a forensic exercise. This is cheap if designed in from the start and painful to retrofit —
it is the single most important structural decision in the evaluator.

---

## 4. Answering §8 — Architecture. Separate service, shared crates

**The scanner must be a separate process from TradierPro.** Reasons, strongest first:

1. **Fault isolation on the order path.** CLAUDE.md's standing principle is that a panel failure must
   never take down others; a scanner is far more dangerous than a panel. It runs user-authored
   expressions, and §2 documents that the incumbent *dies on runtime exceptions from single symbols*.
   That must never share a process with order placement.
2. **Measured memory pressure on the trading box.** TradierPro is already 49 processes / 4.67 GB,
   with 19.6 GB committed against 15.8 GB physical — the machine is paging today (see
   `MASSIVE_EQUITY_SCOPING.md`). It cannot host a scanner.
3. **Different lifecycle.** Backtests peg CPU for minutes. The trading app must stay responsive
   during exactly those minutes.
4. **It already lives on a different computer.**

**But share code as a Cargo workspace**, not by copy-paste:

```
tradier-pro/            (workspace root)
  crates/
    tp-providers/       ← extracted from src-tauri/src/providers/, shared
    tp-marketdata/      ← bars, rolling aggregates, indicators (NEW, Rust)
    tp-scan-engine/     ← language, evaluator, replay driver (NEW)
  src-tauri/            ← trading app, depends on tp-providers
  scanner/              ← scanner service, depends on all three
```

**⚠️ Reality check on reuse:** the ~18k lines of existing Rust are **less reusable than they look**.
`providers/` (~8,400 lines) is genuinely reusable — WS supervisors, reconnect/backoff, staleness
watchdogs, the hard-won reliability machinery. But the indicator math you would most want —
`calcPOC`, `calcValueArea`, VWAP bands — **lives in TypeScript in `App.tsx`, not Rust**, despite
CLAUDE.md's "math runs in Rust" principle. It must be ported. The good news is the algorithms are
already worked out and numerically validated against TOS, so this is transcription plus tests, not
design.

### ⚠️ This revises the relay recommendation in `MASSIVE_EQUITY_SCOPING.md`

That doc recommended hosting the relay on the trading PC. **A full-market scanner changes the
calculus**: the scanner needs `A.*` (whole market), TradierPro needs ~100 symbols. With one shared
WebSocket connection, a trading-PC relay would push the entire firehose across the LAN to the
scanner. Hosting it on the scanner PC keeps the big stream local — but then trading's market data
depends on the scanner machine being up, which is a reliability coupling on the trading path.

**Cleanest answer: buy the second WebSocket connection from Massive and skip the relay entirely.**
Each machine connects independently, no firehose over the LAN, no cross-machine coupling. Price that
in the same support email as the multi-device licence question — it is now clearly worth paying for
if it is at all reasonable.

---

## 5. Answering §8 — Language design

**Design a new syntax. Do not constrain it for migration compatibility.** The brief's own reasoning
is right: ~40 expressions, translating them is roughly a day. Letting a day of translation work
compromise a language you will live in for years is a bad trade — especially when §4 lists three
additions (named intermediates, rolling-by-default, aggregate helpers) that an incumbent-compatible
grammar would fight.

Design notes worth fixing early:

- **Rolling is the default; clock-aligned needs an explicit marker.** Make the safe thing the short
  thing to write. This is §5's highest-value item and it should be visible in the syntax.
- **Named intermediates need to be evaluated once per symbol per cycle**, not once per reference —
  `ATRStock(...)` appearing five times must cost one computation. That is a memoisation requirement
  on the evaluation context, and it is easy to get wrong in a way that quietly costs 5x.
- **Units and adjustment state belong in the type system.** §2's worst data bug — a $188 stock
  showing a $53 ATR because adjusted and unadjusted values mixed silently — is preventable if
  adjusted and unadjusted prices are *different types* that cannot be compared without an explicit
  conversion. This is the highest-leverage correctness idea available and costs almost nothing if
  done at the start.
- **Every primitive gets a doc comment and a unit test at definition time.** §9 asks for documented,
  testable semantics; the only way that holds is if it is structurally impossible to add a primitive
  without both.

---

## 6. Answering §8 — Build order. The brief's instinct is right, with one refinement

The proposed sequence — backtesting first, keep Primus live — is correct, and for a better reason
than the brief states: **replay is deterministic and available at 3am.** Building the evaluator
against historical data means you can develop and test it without market hours, and validate against
known outcomes before anything depends on it live.

**The refinement:** make the replay driver and the live driver two implementations of one trait
**from the very first commit**. §6 asks for live and replay to share the evaluation code; the way to
guarantee that structurally is to give the evaluator a single input interface and never let it know
which side it is on. Retrofitting this after a live path exists is how the two silently diverge.

Suggested sequence:

| # | Stage | Delivers |
|---|---|---|
| 1 | Historical store: flat-file ingest, adjustment pipeline, symbol universe | a queryable local corpus |
| 2 | `tp-marketdata`: ring buffers, rolling aggregates, ATR/ADV/VWAP/EMA/volume-profile — ported from the TS originals, with tests | trustworthy primitives |
| 3 | Language + evaluator + **introspection**, driven by the replay trait | expressions that run on history |
| 4 | **Instrumentation mode** — log every measurement value for every candidate | **§6's cheap win: a sweepable dataset before the backtester exists** |
| 5 | Backtest harness: no-lookahead replay, parameter sweeps, MFE/MAE capture, export | the reason for the project |
| 6 | Live ingest driver (same trait), full-market `A.*` + focused `T.*` | live scanning |
| 7 | Alerting, lists, rate limiting, **loud failure surfacing** | Primus parity |
| 8 | Replay trainer (reuses stage 3's driver + a UI) | §7 |

**One challenge to the brief:** §6 describes instrumentation mode as running *live*. Run it on
**history** instead, at stage 4. It is the same logging, it needs no live ingest, it works on years
of data instead of days, and it arrives months earlier. This is the single biggest schedule win
available.

---

## 7. Effort and timeline — the actual question

Solo developer with AI assistance, working around a trading day, no hard deadline (Primus runs to
next May ≈ 8 months).

| Stage | Estimate |
|---|---|
| 1. Historical store + adjustment pipeline | 2–4 weeks |
| 2. Market-data primitives (port + test) | 3–5 weeks |
| 3. Language + evaluator + introspection | 5–7 weeks |
| 4. Instrumentation mode | 1–2 weeks |
| 5. Backtest harness + sweeps + outcome capture | 4–6 weeks |
| 6. Live ingest | 2–3 weeks |
| 7. Alerting / lists / failure surfacing | 3–4 weeks |
| 8. Replay trainer | 2–3 weeks |
| **Total** | **22–34 focused weeks** |

Translating that honestly:

- **Full-time equivalent: 5–8 months.** You are not full-time.
- **Realistic part-time, around trading: 9–15 months to all eight stages.**
- **But stages 1–5 — the actual reason for the project — are 15–24 weeks, and land well inside the
  Primus window.** That is the target to hold.

**Recommended milestone plan:**
- **By ~month 4:** stages 1–4. You can set thresholds from distributions instead of guesses. This
  alone retires §2's biggest complaint and months of guess-deploy-wait cycles.
- **By ~month 6–7:** stage 5. Full backtesting with forward outcomes.
- **After May, unhurried:** stages 6–8. Run live scanning in parallel with Primus for a full month
  before trusting it, and do not cancel the subscription until it has been proven side by side.

Estimates at this range carry real uncertainty — ±40% is honest, and scope creep in stage 3
(language design is where projects like this expand without anyone noticing) is the most likely way
this slips.

---

## 7b. Migrating the existing 40 filters — what ports and what cannot

### ⚠️ DO THIS WEEK: get the filter definitions out of the vendor and into git

The 40 expressions are, right now, **the most valuable artefact in this project and they live inside
a subscription that expires next May.** They are the accumulated output of months of
guess-deploy-watch cycles — they encode which setups matter, which primitives are needed (§4's table
was derived from them), and roughly where the thresholds sit.

If they exist only in the vendor's cloud, they are a dependency with an expiry date. **Export all 40
verbatim — plus every static list definition — commit them to this repo as reference material, and
do it long before the subscription lapses.** This costs an hour and de-risks the entire project. It
does not depend on any other decision here.

Capture alongside each expression: what it is *supposed* to find, and any observed
false-positive/false-negative behaviour. That commentary is worth as much as the expression and
exists nowhere but in the user's head.

### What ports mechanically (≈ a day, as the brief estimated)

§4 establishes the expressions are boolean conjunctions/disjunctions over numeric comparisons — no
loops, no arrays, no cross-evaluation state. That shape maps onto any sane new grammar almost
one-to-one. Structure, clause order, nesting, and intent all survive translation.

### What does NOT port: the thresholds

**Every threshold was tuned against semantics the replacement deliberately changes.** A number
calibrated for a clock-aligned 120-minute window is not the right number for a rolling one. A
relative-volume threshold tuned against cumulative-since-open RVOL is meaningless against the
burst-rate measure §5 requires. An ATR threshold set while the incumbent's ATR was silently wrong
across splits is calibrated to a bug.

**This is not a loss — it is the project's purpose.** §2's headline complaint is that every threshold
was set by guessing because there was no backtesting. Stage 4 (instrumentation on history) produces
the distributions to set them from data. The thresholds you "lose" in migration are precisely the
ones you most wanted to stop guessing at.

### What genuinely cannot be ported: the semantics nobody knows

§2 lists primitives with undocumented behaviour the vendor will not explain — a trailing boolean
argument of unknown meaning, an unknown VWAP band calculation, an unknown rolling-vs-clock-aligned
120-minute window, an unanswered question about whether a counter resets daily.

**A faithful port of those clauses is not merely hard, it is impossible** — you cannot reimplement
behaviour you cannot specify. And attempting fidelity would be the wrong goal anyway: a faithful
translation reproduces the exact defects being escaped.

Treat each such clause as a **specification question to answer deliberately**: what did I *intend*
this to mean? Then implement that, document it, and unit-test it (§9). Roughly the ~5 known-opaque
clauses need this treatment; the rest translate cleanly.

### Do not build a translator

For 40 expressions, hand-translate with review. A mechanical translator would need to encode the
incumbent's semantics — including the unknown ones — and would faithfully carry over the
clock-alignment and cumulative-RVOL defects. Hand translation forces confronting each ambiguous
clause exactly once, which is the desired outcome. The brief's own judgement that compatibility is
not worth constraining the design for is correct, and this is the reason.

### Their real role: the conformance test set

The 40 expressions are the **acceptance criterion for stage 3**. The new language is finished when it
can express all 40 — no sooner, and, importantly, no later. That is a concrete, bounded definition of
done for the stage most at risk of scope creep (see risk 1), and it is available on day one.

## 7c. Precision - "only tradeable symbols" (added 2026-09-04 on user clarification)

The brief optimises heavily for **not missing signals** - rolling windows, event persistence, burst
volume. The user's stated daily pain is the opposite: **too many fires that are not tradeable, and
time wasted hand-filtering them.** That is a precision problem, and it deserves first-class design
attention rather than being treated as a threshold-tuning afterthought.

**The good news: the tool that fixes it is the tool already planned.** An outcome-labelled dataset -
every fire, its full measurement vector, and its forward MFE/MAE (SS6) - is simultaneously the answer
to "where should this threshold be" and "which measurements separate a tradeable fire from a
distraction." Stages 4-5 deliver both stated pains at once. This strongly reinforces the build order
rather than changing it.

Three concrete design additions:

### (a) A shared `Tradeable()` predicate - the highest-leverage single item

Most untradeable fires are not subtle: spread too wide, price too low, average daily volume too thin,
already extended, too close to the close, hard-to-borrow when the signal is short-side. These are the
same conditions across all 40 filters.

Define **one shared named expression** every filter ANDs with. Tuning tradeability then becomes a
single edit in one place instead of 40 parallel edits - and, critically, it becomes a thing that can
be **swept as a unit** in the backtester. SS4 already asks for named intermediates; this is their most
valuable application and should be a library-level construct, not a per-filter copy.

### (b) A rank expression alongside the boolean

A boolean filter cannot say "this fire is better than that one", so every fire arrives looking
equally urgent - which is exactly what forces manual triage.

Let each filter optionally declare a **numeric rank expression** evaluated on fire. The language
already supports numeric expressions (every comparison has numeric operands), so this is a near-zero
grammar addition with a large workflow payoff: alerts arrive sorted by expected quality instead of
by arrival time. Start with a hand-written rank, then refine it from the outcome-labelled dataset.

### (c) Per-symbol suppression that is smarter than a rate limit

SS4's control primitive is rate limiting per symbol. Precision also wants: suppress if this symbol
already fired *any* filter recently; suppress if it fired and the outcome has not yet resolved;
optionally suppress names already alerted and dismissed today. Cheap to add, directly removes
repeat-offender noise from the queue.

**Measure precision explicitly.** Once stage 5 exists, every filter should report fires/day, and the
distribution of forward MFE at fixed horizons. A filter that fires 40 times a day with a median MFE
indistinguishable from noise is not a filter, and today there is no way to know that.

---

## 7d. Automated trading - forward compatibility (user goal, stated 2026-09-04)

**This lifts a standing CLAUDE.md exclusion.** "Automated or algorithmic trading" was listed under
Out of Scope; the user has now named it as an eventual goal, so CLAUDE.md has been updated to record
it as a stated future direction rather than an excluded one. **It is NOT part of this project's
scope** - but a few decisions made now cost nothing and would be expensive to retrofit.

**The one hard architectural rule: the scanner never places orders.** It holds no broker credentials
and has no order-path access. It emits signals across a process boundary; a separate, deliberate
execution component consumes them and owns every guard. This preserves the fault isolation that
justifies a separate process in the first place (SS4) - and the order-path incident history in this
project's memory is a long argument for keeping anything that evaluates user-authored expressions
away from order placement.

**Cheap now, expensive later:**

1. **Signals are persisted, immutable, timestamped objects** - not ephemeral UI toasts. Each carries
   the full measurement vector and the evaluation snapshot that produced it. Without this there is no
   audit trail for an automated decision, and no way to replay one.
2. **Signal emission is a boundary, not a function call.** Whatever carries signals to the UI should
   be the same channel an executor would later subscribe to.
3. **Keep the evaluation deterministic and reproducible** - same inputs, same signal, always. Already
   required by SS6's shared-evaluator rule; automated trading raises the stakes.

**The brief already anticipated this without saying so.** SS7's replay trainer states the only
difference between it and a rule is "whether the decision comes from a rule or a human." That means
**the replay trainer IS the dry-run harness for automated strategies.** Building it well (stage 8)
delivers strategy backtesting for free - a rule plugged into the trainer's decision slot is an
automated strategy being tested with no lookahead. Design the decision slot as an interface with two
implementations (human hotkey, rule) exactly as the replay/live drivers are designed, and automated
trading becomes an increment rather than a project.

## 7e. What data subscription is actually needed to START (researched 2026-09-04)

**You do not need the $199 real-time tier to build this. $29/month covers stages 1-5 entirely.**

Massive US stocks individual tiers:

| tier | price | recency | history | rate limit | WebSockets | **flat files** |
|---|---|---|---|---|---|---|
| Basic | $0 | end-of-day | 2 yr | 5/min | no | **no** |
| **Starter** | **$29** | 15-min delayed | **5 yr** | **unlimited** | **yes** | **yes** |
| Developer | $79 | 15-min delayed | 10 yr | unlimited | yes | yes |
| Advanced | $199 | **real-time** | 20+ yr | unlimited | yes | yes |

**Why Starter is sufficient for the whole development phase:**

- **A 15-minute delay is irrelevant to historical data.** Backtesting reads bars from last week, last
  month, last year. The delay only affects the most recent 15 minutes, which no backtest touches.
- **5 years of history exceeds what SS6 asks for** ("a rolling multi-year window").
- **Flat files are included at Starter**, which is the corpus-download mechanism the whole historical
  store is designed around (SS1). This was the thing most at risk of being gated to a business plan.
  It is not.
- **Unlimited API rate limit** makes REST gap-filling practical. The free tier's 5 calls/min would
  not.
- **WebSockets are included, delayed.** This is the sleeper benefit: **stage 6 (live ingest) can be
  built and functionally validated on a delayed feed**, because the code path is byte-identical and
  only the recency differs. You would only be unable to measure *latency* - which is not what stage 6
  is proving.

**The $199 upgrade is needed exactly once: when live scanning goes into real use** (stage 7,
6+ months out). Running Starter through development instead of Advanced saves ~$170/month for
~6 months, roughly **$1,000**.

Free-tier note: stages 2-3 (primitives, language, evaluator) can technically be developed against
fixtures and a small sample pulled from the existing Tradier/Schwab access, at zero cost. But at $29
there is little reason to work blind - real data surfaces bad prints, halts, splits and sparse
symbols that fixtures never will, and the 50-75 GB corpus takes time to accumulate, so starting the
download early is worth more than the saving.

### Bundling consideration - one subscription may serve both projects

`MASSIVE_EQUITY_SCOPING.md` proposes buying **Stocks Advanced ($199)** to test whether a paid feed
fixes TradierPro's equity latency. If that purchase happens, **it also covers every scanner
development need** - same account, same flat files, deeper history, no additional data spend. The two
projects would share one subscription.

The only friction is the one-WebSocket-per-cluster limit: TradierPro and the scanner both streaming
means buying a second connection (see SS4). That question is already on the list for the support
email.

So the decision is really:

- **Scanner only, cheapest path:** Starter $29 now, upgrade to Advanced at stage 7.
- **Both projects:** Advanced $199 now, serving the TradierPro latency test immediately and the
  scanner corpus in parallel, plus a second WS connection.

**Verify before subscribing:** that the delayed WebSocket tier still permits the full-market wildcard
subscription, and Massive's actual flat-file coverage depth in years per tier.

## 8. Principal risks

1. **Language scope creep (highest).** Expression languages invite endless generality. §4 says no
   loops, no arrays, no cross-evaluation state — **hold that line hard**; it is what keeps stage 3
   at 5–7 weeks instead of 6 months.
2. **Historical halts (unresolved).** Settle before stage 5; it changes what a correct replay means.
3. **Backtest/live divergence.** Mitigated structurally by the shared-trait rule, and only by that.
   Any shortcut here reproduces the incumbent's core weakness in your own code.
4. **Corporate-action correctness.** §2 shows this is where the incumbent bleeds. The typed
   adjusted/unadjusted idea in §5 above is the cheapest available defence.
5. **Scanner-machine capacity — unmeasured.** The trading PC was measured; the scanner PC was not.
   Stages 1 and 5 want disk (50–75 GB corpus) and RAM. Measure it before stage 1.

## 9. What was NOT investigated

- The scanner PC's specs (not accessible from here)
- Massive's actual historical archive depth in years (docs did not state it — ask support)
- Whether historical trade condition codes carry halt indicators (the cheapest fix to risk 2)
- Exact Massive pricing for full-market historical/flat-file access, which may differ from the $199
  real-time tier — **this could move the cost materially and should be the first question asked**
