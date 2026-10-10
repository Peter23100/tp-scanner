# Filters

The Primus filters, hand-translated from `spec/primus/Primus_Filters_MASTER_2026-09-25.md` into the
scanner's language. **`LANGUAGE.md` is the reference for writing and changing filters**: every
building block, Primus and Trade Ideas equivalents, and a cookbook. `CHANGES.md` records every
filter change and why.

| file | what | count |
|---|---|---|
| `fields.scan` | the 21 custom fields (§2), plus a few named helpers the filters repeat | 21 + 10 |
| `left.scan` | sharp-move family (§4.1–§4.10) | 10 |
| `right.scan` | hold and breakout families (§4.11–§4.24) | 14 |
| `top.scan` | flushes and reversals (§4.27–§4.32) | 6 |
| `statics.scan` | movers, PM coils, imbalances, halts, NYSE IND (§5) | 12 |
| `common.scan` | shared fields and conditions for new filters (Trade Ideas-style measures, `tradeable`, VWAP crosses); not used by the translations | 9 + 3 |

`cargo test -p tp-scan-engine --test primus_filters` parses and compiles all of them and runs
scenario checks (a ramp fires M_Spike_Up; a ramp that holds fires M_Spike_Hold_Up; one that
round-trips doesn't, and `explain` names the retention clause that blocked it).

Check the files without data: `cargo run -p tp-scan-cli -- check filters`.

## Not translated yet: needs the verbatim expression

A transcription from prose is exactly the invisible-threshold risk the export exists to avoid, so
these wait for the pasted expression:

- **Contra group (14)**: §4C describes them but quotes none. Contra_Short_A_FAST, B_Extended,
  C_Breakdown, Contra_Cap_A_Fast, B_Extended, Contra_Reclaim_C, Contra Long/Short V5, V8,
  Parabolic, All, the bare Long/Short, Momo VWAP Divergence Long, Contra Long VWAP Divergence, and
  the unnamed short-side divergence filter.
- **Breakout_Hold_Up / _Down** (§4.25): prose only.
- **PM_LowFloat_Coil_Up** (§5.6): prose only.
- **Halted** (plain): never collected.
- **CatchALL_LF_HOD**: on the right screen in the screenshot, not in the master.

## Three things were reconstructed and need checking

- `RVOL_CURVE` (inside RVOL20DAYTIMEADJUSTED) is built from the §2 table. The step form, where each
  value holds up to and including its time, follows the `<= 570` branch the master quotes, but the
  other 13 branches weren't pasted.
- `PM_CURVE` (inside PM_RVOL) is built from the §2 prose.
- The M6_Bounce_Short constants that §4.18 doesn't state (day move 0.97, 30-minute 1.03, VWAP cap
  0.96) are mirrored from M5, and the Quiet_Reversal_Down RSI of 30 is mirrored from the up
  side's 70.

## Translation rules

Structure ports mechanically, and the rules below are applied uniformly.

| Primus | here |
|---|---|
| `MinuteHigh(ALL_VENUES, 5, CURRENT, NO, False)` | `high(5m)` (the trailing 5 minutes) |
| `MinuteHigh(..., 5, P1, ...)`, `P2`… | `high(5m, 1)`, `high(5m, 2)` (the 5 minutes before, and so on) |
| `MinuteHigh_I` | same as `MinuteHigh` (the difference is undocumented) |
| `MinuteVolume` / `MinuteLow` | `volume(...)` / `low(...)` |
| `DayHigh(..., 1, CURRENT, NO)` / `DayLow` | `day_high` / `day_low` (RTH, running) |
| `DayHigh(..., 1, P1, NO)` | `prev_high` |
| `ClosePrice(PRIMARY, P1, NO)`, `AdjustedClosePrice(P1)` | `prev_close` (always split-adjusted) |
| `MonthHigh_I(..., 1, P1, NO)` / `MonthLow_I` | `prev_month_high` / `prev_month_low` |
| `ATRStock(..., DAILY, 14, CURRENT)` | `atr` |
| `AvgDayVolume(..., 20, NO)` | `adv` |
| `DayVolume(..., CURRENT, NO)` / `YES` | `day_volume` / `day_volume_ext` |
| `Last(..., NO)`, `Last(YES)`, `LAST` | `last` |
| `VWAP(CURRENT, NO, VWAP, V1, PRIMARY)` | `vwap` |
| `VWAP(CURRENT, YES, VWAP, V1, ALL_VENUES)` | `vwap_ext` |
| `VWAP(..., VWAP_UpperBand, V2, ...)` | `vwap_upper(2)`, ⚠ a different formula (below) |
| `EMAStock(..., MINUTES_5, PERIOD_7, CURRENT)` | `ema(5m, 7)` |
| `EMAStock(..., DAILY, PERIOD_20, CURRENT)` | `daily_ema(20)` |
| `RSIStock(..., MINUTES_5, 14, CURRENT)` | `rsi(5m, 14)` |
| `DayBar_Open(ALL_VENUES, 5, NO, '9:30')` | `open` |
| `PremarketDayHigh/Low/Volume(CURRENT)` | `pm_high` / `pm_low` / `pm_volume` |
| `TickTime`, `570` | `time`, `9:30` |
| `FloatShares <= 20` (millions) | `float <= 20_000_000` (shares) |
| `NewMinuteHigh(15, NO)` | `new_high(15m)` |
| `MaxTriggersCount(MINUTES, 5, 1)` | `cooldown 5m` |
| `IsHalt` | `halted` |
| `ImbalanceBuyVolume(NYSE)` | `imb_buy(nyse)` |
| `IndBid`, `ClearPricePrimary`, `NearPricePrimary` | `ind_bid`, `clear_price`, `near_price` |
| the `NO` extended-hours flag on a trigger | `hours rth` (the default) |
| names with `/` or spaces | `_` (`M_Pop_Medium_EXT_Up`, `NYSE_IND_UP`) |

## Where results WILL differ from Primus, on purpose

These come from the engine, not from editing the filters. Every threshold was tuned against
Primus's behaviour, so expect different fire counts. Stage 4 (instrumentation on history) exists to
re-derive thresholds under these semantics.

1. **Rolling windows.** Primus's windows are clock-aligned, which splits roughly one spike in five
   across two measurements (§1). Here `high(5m)` at 10:07 covers 10:02–10:07. It also means:
   - "the completed spike bar" (`P1`) is the five minutes before the last five, at any minute,
     so M_Spike_Hold no longer only becomes true at bar boundaries (its :00 clustering goes);
   - forming-bar clauses that reset to zero each period no longer do. Quiet_Reversal's volume
     contraction (fix queue 9), M_Run_Hold's first coil clause and the VolROC sawtooth are
     honest comparisons of two full windows.
2. **Windows include extended hours.** `volume(120m)` at 10:00 reaches back into the premarket;
   Primus's `NO` flag reads zero there. The burst legs therefore work from the open, where Primus's
   read "unusable until ~9:40" (§1).
3. **Unknown is not zero.** A missing ATR, an unpublished clearing price or a division by zero is
   unknown, and a filter never fires on unknown. `Clear_ATR` / `Near_ATR` are blank before the
   auction prints rather than hugely negative (fix queue 14), and the VolROC fields can't divide
   by zero (fix queue 13).
4. **Split-adjusted daily references.** ATR, prior close and ADV come from adjusted history, so
   the PFSA reverse-split distortion (ADV 4× inflated) can't happen. That is the only intended
   change to ADV-normalised clauses.
5. **Hours gate evaluation.** A `hours rth` trigger isn't evaluated outside 09:30–16:00, so
   Blowoff_Reversal can't re-fire at 19:56 on a frozen state (§4.29).
6. **`vwap_upper(2)` is not Primus's band.** Primus's bands are documented as broken with an unknown
   formula. These are the TradierPro chart's VWAP ± 2 SD. Blowoff_Reversal will fire differently.
7. **`last >= day_high` depends on bar resolution.** `last` is the newest bar's close and
   `day_high` includes that bar's high, so the test passes only when a bar closes on its high.
   Live 1-second bars make that close to Primus's tick test; **minute-bar replays make it rare.**
   M3/M4 and HOD_Base_Break_LQ are exact-high tests. The M_Spike family's
   `day_high - 0.15 * atr` tolerance absorbs it.

## Not applied: the fix queue (§7)

Only the items above that the engine fixes by construction, plus item 18 (the split-immune volume
leg), which arrived in the master itself as the 2026-09-25 LowFloat_HOD_Break_Up rebuild. Everything
else in §7 (Movers 0.75% →
2%, `TickTime` gates on Open_Flush and PM_Coil, the Breakout_Hold gap leg, propagating the VWAP cap
or break margin, normalising field scales) is a decision to make once the translation runs on
history and can be compared, one change at a time.
