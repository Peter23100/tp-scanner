# PrimusScan Filter Master
**Version 2026-09-24 — COLLECTION COMPLETE**
supersedes `Primus_Filters_MASTER_2026-07-15.md` (lost)

**All 58 filters, statics and premarket lists collected and verified, plus all
21 custom fields.** Nothing outstanding except the plain `Halted` expression and
whether `IndAsk` is defined or built-in.

This file is the single source of truth for filter definitions, platform
behaviour, and the regression set. It is written to be read by Claude Code as
the requirements input for the scanner replacement project, and by Peter as the
working reference.

### What changed on 2026-09-23/24
- M_Spike_Hold pair: 2.5% spike-bar percent floor added; running-low retention
  restored at 0.50
- Base_Break_Hold pair: rebuilt from scratch as a tightened M_Spike_Hold with a
  monthly-level break
- Breakout_Hold pair: left as-is by decision; diagnosis recorded in §4.25
- Top screen (§4B, 6) and the full Contra group (§4C, 14) collected
- Statics (§5, 13) and every custom field (§2, 21) collected
- **`DayBar_Open` confirmed WORKING** — the M_Open_Flush pair is not faulted
- **`IF()` conditionals discovered** — see §2 and the fix queue
- Two Contra filters found logically dead; one static's float clause found inert

## §0 How to read this

Every filter entry has the same shape:

| Field | Meaning |
|---|---|
| **Hunts** | One sentence: the setup this filter exists to find |
| **Type** | Trigger (emits alerts) or Static (maintains a list) |
| **Status** | See legend below |
| **Expression** | Verbatim, as loaded |
| **Clause notes** | What each non-obvious clause does and why |
| **Limits** | Known blind spots and failure modes |

**Status legend**

| | Meaning |
|---|---|
| ✅ VERIFIED | Expression pasted from Primus. Trustworthy. |
| ⚠️ UNCONFIRMED | Expression posted by Claude; may or may not be loaded. **Do not trust.** |
| ⬜ MISSING | Not yet collected |
| ❌ FAULTED | Currently broken in Primus |
| 🗑️ RETIRE | Still loaded, should be deleted |

> On 2026-08-14 `M_Spike_Up` was found running an expression posted two weeks
> earlier and never loaded. Every ⚠️ below carries that risk.

---

## §1 Platform behaviour — read this first

Hard-won facts about the Primus engine. **These are requirements for any
replacement**, either to reproduce or to deliberately fix.

### Evaluation model
- **Evaluation is effectively continuous — per-tick or near it.** Corrected
  2026-09-05. The earlier claim of 20–30 second snapshots came from a July
  session note and was never verified; the fire logs disprove it. Across 98
  M_Drop_Sensitive_Down fires the timestamp seconds are spread across the whole
  0–59 range with only 4 at exactly :00. A 20s cadence would put everything on
  00/20/40; a 30s cadence on 00/30. No grid exists.
- **M_Spike_Hold's fires cluster at :00 (52 of 65) for a different reason** — its
  inputs are almost all completed-bar values, so the setup can only *become*
  true at a bar boundary. That is about when conditions change, not when they
  are checked.
- **Consequences of the correction:** instant-true conditions like
  `NewMinuteLow(15)` and `Last >= DayHigh(CURRENT)` are NOT missed by sampling.
  Any diagnosis that blamed a missed fire on snapshot timing needs revisiting.
- Uses **forming-bar values**, not bar-close. Forming volume accumulates from
  zero, so any clause comparing current-bar volume to a completed bar becomes
  true only late in the bar.
- Filters can **fault mid-session** after repeated exceptions on one symbol, and
  are then silently disabled. A disabled filter and a quiet market look
  identical.

### Windows
- **All intraday windows are clock-aligned**, not rolling. `MinuteHigh(30,
  CURRENT)` at 10:01 contains one minute of data.
- **Consequence:** a move spanning a boundary is measured as two smaller moves.
  Roughly one spike in five. This is the single most valuable thing to fix in a
  replacement.

### Token semantics
| Token | Behaviour |
|---|---|
| `FloatShares` | **Denominated in millions.** `<= 10` means 10M shares. |
| `TickTime` | Minutes from midnight. 570 = 9:30, 750 = 12:30. |
| `MonthHigh_I` / `MonthLow_I` | `P1` = prior *completed* month. P1/P2/P3 all valid. |
| `DayHigh` / `DayLow` (CURRENT) | Running extremes including the current tick. |
| `MinuteHigh/Low(N, CURRENT)` | Running extreme of the forming bar — safe. |
| `MinuteHigh − MinuteLow` | A **range**, not directional. Direction must be stated separately. |
| `DayBar_Open` | ✅ **WORKS** (confirmed 2026-09-23 — M_Open_Flush pair running). Previously believed to throw a subscription exception; that is either fixed or was always context-dependent. Signature: `DayBar_Open(ALL_VENUES, 5, NO, '9:30')`. |
| Trailing `False` argument | **Meaning unknown.** Never explained by the vendor. |
| `MinuteVolume(120, …)` | Unknown whether rolling or clock-aligned. |
| `VWAP` | Centerline is accurate. `VWAP_UpperBand`/`LowerBand` are broken; formula unknown. |
| `MaxTriggersCount` | Suspected of interrupting Static list membership. Keep off statics. |

### Data quality
- **Corporate actions are inconsistently applied.** Adjusted and unadjusted
  values coexist across functions without documentation.
  - Observed: a $188 stock reporting a $53 ATR.
  - Observed: after a 1-for-4 reverse split, `AvgDayVolume(20)` inflated ~4x,
    silently blocking every ADV-normalised volume clause (PFSA, 2026-08-18).
- **Custom fields appear to use integer division.** `A / B * 100` where A < B
  truncates to zero. Multiply before dividing.

### Extended hours
- The `NO` flag excludes premarket. Consequences:
  - `RVOL20DAYTIMEADJUSTED` reads **0** premarket — any filter using it cannot
    fire before 9:30.
  - Any prior-window reference reads zero before 9:30, so window-over-window
    volume comparisons are unusable until ~9:40.
  - At the open, `AvgDayVolume` is the only non-zero volume baseline available.

---

## §2 Custom fields (21) ✅ ALL COLLECTED 2026-09-23

**Every one of these is Peter's own expression, not a Primus built-in.** That
matters: defects in them are his to fix, and the vendor cannot be asked about
them. It also means the replacement scanner must reproduce the *definitions*,
not just provide equivalent primitives.

### Conventions

**Scale is inconsistent.** Seventeen fields multiply by 100 and read as
percentages; four do not and read as raw ratios. `ATR_POS` and `VWAP_Stretch`
both measure distance in ATR units, so a stock half an ATR above its 5-minute
low and half an ATR below VWAP shows **50** in one column and **0.5** in the
other, on the same panel. Worth normalising.

**Divide-by-zero guards appear in four fields** (`+ 0.01` on the retrace pair,
`+ 1` on VolFade, `IndBid > 0` on the IND lists, the range clause in PM_Coil) and
are **missing from two that need them** (`VolROC_1m_3`, `VolROC3`).

**Naming is mixed case with underscores** — `Spike_Vol_Pct`, `Retrace_1m_Down` —
not the all-caps form used in older notes.

---

### Volume: relative

#### RVOL20DAYTIMEADJUSTED ⚠️ the field behind five missed trades
```
DayVolume(ALL_VENUES, 1, CURRENT, NO)
/ ( AvgDayVolume(ALL_VENUES, 20, NO) * <14-level nested IF curve on TickTime> )
```
Curve — fraction of the day's volume accumulated by that minute:

| Time | Fraction | | Time | Fraction |
|---|---|---|---|---|
| pre-9:30 | 0.01 | | 10:30 | 0.24 |
| 9:32 | 0.022 | | 11:30 | 0.37 |
| 9:35 | 0.040 | | 13:00 | 0.53 |
| 9:40 | 0.070 | | 14:30 | 0.70 |
| 9:45 | 0.095 | | 15:30 | 0.85 |
| 9:50 | 0.115 | | 16:00 | 1.00 |
| 9:55 | 0.133 | | | |
| 10:00 | 0.150 | | | |

Seven segments cover 9:30–10:00 alone, matching the trading window. A proper
U-shape: 15% of the day in the first half hour, 15% in the last.

**⚠️ THE CUMULATIVE-DILUTION DEFECT.** Because the numerator is cumulative and
the denominator approaches a full day, a stock quiet until 2pm that then explodes
still reads **below 1.0**. Documented misses: AMBA 12:30, CMG 12:45, CRM 14:30,
WDAY 14:30, PYPL 15:44.

*Worked example:* PYPL 2026-08-14, ADV ~15M, news at 15:44 where the curve reads
0.92 → denominator ~13.8M. Dead all session plus a 2.4M spike bar ≈ 10.4M
cumulative → **RVOL ≈ 0.75** against a 1.25 gate.

Premarket reads **0** because the numerator uses extended-hours `NO` — not
because of the curve. That is why PM_RVOL exists separately.

No divide-by-zero: the `<= 570` branch catches 9:30 exactly. (The hand-rolled
Contra RVOL does not — see §4C.)

**⭐ FIX AVAILABLE.** The curve's segment slopes give the expected volume *rate*
at each minute — 0.003/min in the 10:00–10:30 segment vs 0.00178/min in
11:30–13:00, so the open runs ~1.7× the midday rate. A **time-local** companion
is therefore buildable:
```
RVOL_Recent = MinuteVolume(ALL_VENUES, 30, CURRENT, NO)
            / ( AvgDayVolume(ALL_VENUES, 20, NO) * <slope at this minute> * 30 )
```
Immune to dilution, normalised to the stock's own average rather than to its
trailing 120 minutes the way the burst-leg workaround is, and more interpretable
— 3.0 would mean "three times normal pace *right now*".

#### PM_RVOL ⭐ contains the only `IF()` besides RVOL
```
PremarketDayVolume(CURRENT)
/ ( AvgDayVolume(ALL_VENUES, 20, NO) * 0.01 * <8-level nested IF on TickTime> )
```
Curve: 0.03 to 5:00am → 0.10 by 6:00 → 0.20 by 7:00 → 0.35 by 8:00 → 0.50 by
8:30 → 0.70 by 9:00 → 0.85 by 9:15 → 1.0 by 9:30. Accelerating: per-minute slope
rises from 0.0012 to 0.01.

Calibration: 1.0 normal, 3–5 active, 10+ gapper.

**⚠️ Caveat.** The `0.01` assumes total premarket volume ≈ 1% of ADV for *every*
stock. Large caps, ADRs and earnings reporters read high; illiquid names low. So
a fixed threshold like 3.0 does not mean the same thing across the universe.

### Volume: rate of change

Three fields, a clear progression — the last supersedes the other two.

| Field | Construction | Verdict |
|---|---|---|
| `VolROC3` | `( MinuteVolume(5,CURRENT) − MinuteVolume(5,P3) ) / MinuteVolume(5,P3)` | crudest |
| `VolROC_1m_3` | same on 1-minute bars | finer, same flaws |
| `VolFade_1m_Smooth` | `( (V_CURRENT+V_P1+V_P2) − (V_P3+V_P4+V_P5) ) / ( V_P3+V_P4+V_P5 + 1 )` | ⭐ best |

**Both ROC fields lack a divide-by-zero guard** — the only fields in the set
missing one they need. And both put the **forming** `CURRENT` bar alone in the
numerator, so each reads near −1 at the start of every bar and climbs through it.
A sawtooth. `VolROC3` is worse: its reset is every five minutes, not one.

**`VolFade_1m_Smooth` is the correct implementation of what Quiet_Reversal is
trying to do.** That filter's contraction clause —
`MinuteVolume(15,CURRENT) < MinuteVolume(15,P1)` — is trivially true early in
every quarter-hour. VolFade compares adjacent 3-minute windows and has no such
artefact. It is also the **second hand-rolled rolling window** in the set (see
§4C), the workaround for clock-aligned windows.

Mild bias remains: `MinuteVolume(1,CURRENT)` is one of three terms, so just after
a minute tick the recent sum is understated by ~⅓ and reads falsely *fading*.
`P1+P2+P3` vs `P4+P5+P6` would remove it.

**Open question:** none of the three appears in any collected filter. They may be
display-only — a correct tool sitting unused beside a filter with exactly the
problem it solves.

#### Spike_Vol_Pct
```
MinuteVolume(ALL_VENUES, 5, P1, NO) * 100 / AvgDayVolume(ALL_VENUES, 20, NO)
```
The completed spike bar as a percentage of ADV. **This is the fixed version** —
the original divided before multiplying and returned 0 on every row (integer
truncation, diagnosed 2026-09-04). Uses `P1`, matching M_Spike_Hold's own clause.

**⚠️** Divides by `AvgDayVolume`, so it inherits the reverse-split distortion:
after PFSA's 1-for-4 split, ADV was inflated ~4× and this field would read ~4×
**too low** — understating exactly the fire worth noticing.

### Displacement and position

All measure clauses the filters genuinely enforce, so each column is a direct
integrity check on its filter.

| Field | Expression | Reads against |
|---|---|---|
| `ATR_POS` | `( Last − MinuteLow(5,CURRENT) ) / ATRStock * 100` | sharp-move 5-min tier: ≥50 high band, ≥80 low band |
| `ATR_NEG` | `( MinuteHigh(5,CURRENT) − Last ) / ATRStock * 100` | same, down side |
| `Spike_Range_Pct` | `( MinuteHigh(5,P1) − MinuteLow(5,P1) ) / Last * 100` | M_Spike_Hold percent floor: **≥2.5** |
| `PM_RangePos` | `( Last(YES) − PremarketDayLow ) / ( PMHigh − PMLow + 0.01 ) * 100` | PM_Coil: ≥75 up, ≤25 down |
| `VWAP_Stretch` | `( VWAP − Last ) / ATRStock` | — (positive = below VWAP) |

**⭐ ATR_POS is a free diagnostic on M_Drive.** Its tiers are 0.75 / 1.0 / 1.25
ATR over 5 / 15 / 30 minutes, OR'd, but ATR_POS measures only the 5-minute leg.
**Any M_Drive fire showing ATR_POS well below 75 came from the 15- or 30-minute
branch** — and the 30-minute branch is 3.5× easier per minute. The 2026-09-04
log shows 33.22, 19.05, 22.10, 36.36, 33.94: all weak-branch fires.

**⭐ VWAP_Stretch could replace the percent-and-dollar VWAP pair in the Contra
filters.** V8 needs two clauses — `Last/VWAP < .98` AND `Last − VWAP <= -0.75` —
because neither percent nor dollars works alone across price levels (they cross
at $37.50). VWAP_Stretch normalises to ATR: scale-free in one clause.

`PM_RangePos` can exceed 100 or go negative after 9:30, since the premarket
references freeze while `Last` keeps moving. Informative, not broken — 150 means
half a premarket range above the premarket high.

### Retracement — the M_Spike_Hold instrumentation

| Field | Expression |
|---|---|
| `Retrace_Pct_Up` | `( MinuteHigh(5,P1) − MinuteLow(5,CURRENT) ) / ( range + 0.01 ) * 100` |
| `Retrace_Pct_Down` | `( MinuteHigh(5,CURRENT) − MinuteLow(5,P1) ) / ( range + 0.01 ) * 100` |
| `Retrace_1m_Up` | `( MinuteHigh(5,P1) − MinuteLow(1,P1) ) / ( range + 0.01 ) * 100` |
| `Retrace_1m_Down` | `( MinuteHigh(1,P1) − MinuteLow(5,P1) ) / ( range + 0.01 ) * 100` |

The `Retrace_Pct` pair uses the **running** extreme of the forming bar, so it
measures the retention clause bounded at **0.50**. The `1m` pair uses completed
1-minute `P1`, measuring the clause bounded at **0.25**.

**⭐ THE AMAT QUESTION, CLOSED.** On 2026-09-04 AMAT logged retrace 87.93 on what
looked like a clean opening ramp, and it was unclear whether the field, the
timestamp or the filter was wrong. **None of them.** At that time the filter had
no running-extreme clause — it had been replaced by the `Last`-based one on 09-03
— so the field was measuring an excursion the filter did not test. Price genuinely
did dip 87.93% back into the spike bar; `Last` had recovered by the sampled
instant. Two correct things measuring different quantities.

**They are aligned again.** With the running clause restored at 0.50, every
`Retrace_Pct` value in a current log must be **≤ 50** — matching the 09-21 logs
exactly (max 49.73 up, 48.29 down). **A value above 50 in a future log means the
clause has been dropped.** Free integrity check, no inspection needed.

**Negative `1m` values mean the minute after the spike traded entirely beyond the
spike bar** — the strongest continuation signature in the log. Possible because
the 1-minute `P1` bar sits *after* the 5-minute `P1` bar on the clock.

### Ranking — the Movers A-lists

```
RNK_UP  = ( Last(YES) − ClosePrice(P1) ) / ATRStock * 100
RNK_DWN = ( ClosePrice(P1) − Last(YES) ) / ATRStock * 100
```

**These are the same measure the lists gate on** (≥0.40 ATR), so every member
shows ≥40 and the list is sorted by the quantity it screens on. Biggest movers
rise to the top automatically.

**⭐ This reframes the overflow.** Movers_Up_Alist hit the result counter at
102,686 on 2026-09-21 and was disabled at 13:30. Because the list self-prioritises,
several hundred members is not a *readability* problem — it is purely a **counter**
problem. So a moderate threshold raise suffices: 0.75% → **2%** should cut
membership 3–5×, leaving comfortable headroom. The 3% originally suggested
over-corrects for a problem the sort already solves.

### Auction and imbalance

| Field | Expression |
|---|---|
| `Clear_ATR` | `( ClearPricePrimary − Last(YES) ) / ATRStock * 100` |
| `Near_ATR` | `( NearPricePrimary − Last(YES) ) / ATRStock * 100` |
| `Imb_Buy_Ratio` | `ImbalanceBuyVolume(ANY) / AvgDayVolume(20,NO) * 100` |
| `Imb_Sell_Ratio` | `ImbalanceSellVolume(ANY) / AvgDayVolume(20,NO) * 100` |

`Clear_ATR` / `Near_ATR` state the expected auction move **normalised to the
stock's own volatility** — 50 means half an ATR into the close whether the stock
is $12 or $600. The most actionable numbers on an imbalance screen. Near price is
indicative and updates continuously from ~15:50; clearing price is the actual
auction price and firms up at the end.

**⚠️ Neither guards against an unpublished clearing price.** If `ClearPricePrimary`
returns 0 before the auction prints, the field produces a large **negative**
reading rather than a blank — looking like a big expected down-move when it means
no data. **`IndBid > 0` in the NYSE IND lists is exactly this guard**, so the
pattern already exists in Peter's own expressions; these two simply lack it.

The `Imb_*_Ratio` pair is the field form of the imbalance statics' 3% gate, so
members show ≥3. They use venue `ANY` where the statics name NYSE or NASDAQ —
correct for a column shown on both panels.

### Token inventory from the fields

| Token | Note |
|---|---|
| `IF(cond, then, else)` | ⭐ **conditional** — only in RVOL and PM_RVOL |
| `PremarketDayVolume/High/Low(CURRENT)` | premarket session values; freeze at 9:30 |
| `ImbalanceBuyVolume/SellVolume(venue)` | venue enum: `ALL_VENUES · PRIMARY · NYSE · NASDAQ · ANY` — **is `ANY` a synonym for `ALL_VENUES`?** |
| `RSIStock(ALL_VENUES, NO, MINUTES_5, 14, CURRENT)` | first RSI |
| `BollingerBands(CURRENT, MINUTES_5, 20, ALL_VENUES, NO, Lower/UpperBand, V2)` | |
| `SMAStock(...PERIOD_20...)` | only simple MA; all else uses `EMAStock` |
| `MaxEntriesTriggersCount(HOURS, 8, 1)` | distinct from `MaxTriggersCount` |
| `MinuteVolume_I(..., YES/NO)` | explicit extended-hours flag |
| **Bare tokens (7)** | `LAST` / `Last >5` · `Vol5m` · `TickTime` · `IsHalt` · `ClearPricePrimary` · `NearPricePrimary` · `IndBid` / `IndAsk` |
| **Bar indices** | to `P5`; `P4` and `P5` both in use |
| **Intervals (10)** | 1 · 2 · 3 · 5 · 6 · 10 · 15 · 30 · 60 · 120 minutes |

---

## §3 Filter inventory

### LEFT SCREEN — sharp move family (10 filters) ✅ ALL VERIFIED 2026-09-05
| Filter | Type | Status |
|---|---|---|
| M_Spike_Up | Trigger | ✅ §4.1 |
| M_Spike_Down | Trigger | ✅ §4.2 |
| M_Pop_Sensitive_Up | Trigger | ✅ §4.3 |
| M_Drop_Sensitive_Down | Trigger | ✅ §4.4 |
| M_Pop_Medium/EXT_Up | Trigger | ✅ §4.5 |
| M_Drop_Medium/EXT_Down | Trigger | ✅ §4.6 |
| M_Drive_Up | Trigger | ✅ §4.7 |
| M_Drive_Down | Trigger | ✅ §4.8 |
| High_ATR_Pop | Trigger | ✅ §4.9 |
| High_ATR_Drop | Trigger | ✅ §4.10 |

Charts on this screen: GH, FAF, TYRA

### RIGHT SCREEN — breakout and hold families (14) ✅ ALL VERIFIED 2026-09-05
| Filter | Type | Status |
|---|---|---|
| M_Spike_Hold_Up | Trigger | ✅ §4.11 *(upd 09-23)* |
| M_Spike_Hold_Down | Trigger | ✅ §4.12 *(upd 09-23)* |
| M_Run_Hold_Up | Trigger | ✅ §4.13 |
| M_Run_Hold_Down | Trigger | ✅ §4.14 |
| M3_Breakout_Long | Trigger | ✅ §4.15 |
| M4_Breakdown_Short | Trigger | ✅ §4.16 |
| M5_Pullback_Long | Trigger | ✅ §4.17 🗑️ duplicate slot |
| M6_Bounce_Short | Trigger | ✅ §4.18 |
| M7_GrindUp | Trigger | ✅ §4.19 |
| M8_GrindDown | Trigger | ✅ §4.20 |
| LowFloat_HOD_Break_Up | Trigger | ✅ §4.21 |
| HOD_Base_Break_LQ | Trigger | ✅ §4.22 |
| Base_Break_Hold_Up | Trigger | 🔄 §4.23 *(rebuilt 09-23)* |
| Base_Break_Hold_Down | Trigger | 🔄 §4.24 *(rebuilt 09-23)* |
| Breakout_Hold_Up | Trigger | ✅ §4.25 |
| Breakout_Hold_Down | Trigger | ✅ §4.26 |

Charts: TJGC, COHU, DOCU, STNG, XYL

### TOP SCREEN — flushes and reversals (6) ✅ ALL VERIFIED 2026-09-23
| Filter | Type | Status |
|---|---|---|
| M_Open_Flush_Up | Trigger | ✅ §4.27 — **`DayBar_Open` works; NOT faulted** |
| M_OpenFlush_Down | Trigger | ✅ §4.28 — same |
| Blowoff_Reversal_Up | Trigger | ✅ §4.29 — depends on broken VWAP band; fires after hours |
| Blowoff_Reversal_Down | Trigger | ✅ §4.30 — same |
| Quite_Reversal_Up | Trigger | ✅ §4.31 — **name misspelled**; volume clause broken |
| Quiet_Reversal_Down | Trigger | ✅ §4.32 — same |

Charts: HUT, TSLA

### FOURTH AREA — legacy Contra and all statics
Contra panels are grouped four to a window with a `Filter Name` column.

| Static | Status | Displayed columns |
|---|---|---|
| Movers_Up_Alist / Movers_Down_Alist | ⬜ | `RNK_UP` / `RNK_DWN`, `PM_RangePos`, RVOL |
| LowFloat_Movers_List | ✅ §5.1 | `RNK_UP`, `PM_RangePos` |
| Halted | ⬜ | `ClearPrice`, `NearPricePri` |
| Halted Static New Minute | ⬜ | same |
| Imbalance_Nasdaq_Buy / Sell | ⬜ | `ImbBuyPc`, `ImbBuyVolIN$`, `NearPr`, `Near_ATR` |
| Imbalance_NYSE_Buy / Sell | ⬜ | same |
| NYSE IND UP / DOWN | ⬜ | `IndBid`, `IndAsk` |
| PM_Coil_Up / _Down | ⬜ | `PM_RangePos` |
| PM_LowFloat_Coil_Up | ⬜ | `PM_RangePos` |

Charts: BIAF ×2, BEX, TSLA

### Contra group (14) ✅ ALL VERIFIED 2026-09-23 — see §4C
**A/B/C generation** (clean, exact mirrors): Contra_Short_A_FAST · B_Extended ·
C_Breakdown · Contra_Cap_A_Fast · B_Extended · Contra_Reclaim_C

**V generation** (older, rough): Contra Long/Short V5 · V8 · Parabolic · All ·
bare Long/Short · Momo VWAP Divergence Long · Contra Long VWAP Divergence ·
unnamed short-side divergence filter

**🗑️ No longer recommended for retirement.** These catch extension and
exhaustion — a function absent from every filter in the current generation.
Retiring them deletes it. Modernise instead: fix the two dead filters, add the
missing triggers, replace the hand-rolled RVOL. See §4C.

## §4 Filter definitions

### The shared skeleton (§4.1–§4.6)

The six sharp-move filters are one design in six configurations. Rather than
repeat it, here is the common structure; each entry below gives only its
parameters and what makes it different.

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= {DAYVOL}
AND Last(ALL_VENUES, CURRENT, NO) >= 2
AND ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.50
AND ( RVOL20DAYTIMEADJUSTED >= 1.25
      OR MinuteVolume(ALL_VENUES, 5, CURRENT, NO) * 24 >= MinuteVolume(ALL_VENUES, 120, CURRENT, NO) * {BURST} )
AND MinuteVolume(ALL_VENUES, 5, CURRENT, NO) >= 5000
[optional day-direction gate]
[optional day-extreme gate]
AND ( <5-min tier> [OR <15-min tier>] )
AND NewMinuteHigh(15, NO)          -- or NewMinuteLow for down
AND MaxTriggersCount(MINUTES, 1, 1)
```

**Displacement** is `Last − MinuteLow(N, CURRENT)` for up filters and
`MinuteHigh(N, CURRENT) − Last` for down. Both measure how far price sits from
the current window's opposite extreme.

**Each tier is:** `displacement >= Last × {PCT}` **AND** `displacement >= 0.50`
**AND** a two-band ATR test:

```
( ATRStock / ClosePrice(P1) >= 0.03  AND  displacement / ATRStock >= {ATR_HIGH} )
OR
( ATRStock / ClosePrice(P1) <  0.03  AND  displacement / ATRStock >= {ATR_LOW} )
```

**The 15-min tier additionally carries the concentration clause:**
`5-min displacement >= 15-min displacement × 0.50`

#### Why the design is shaped this way

**The two-band ATR split is the core idea.** A single ATR multiple expressed as
a percentage is `multiple × (ATR/price)`, so the same threshold demands 0.7%
from a calm stock and 3.0% from a volatile one — simultaneously too loose and
too tight. Splitting the universe at 3% ATR-of-price and giving each side its
own multiple fixes both ends. The denominator is **yesterday's close**, not
`Last`, so a gapper isn't pushed into the wrong band by its own gap.

**Three floors ANDed — percent, dollar, ATR.** Whichever is largest binds.
Robust from $3 to $900.

**The volume OR exists because RVOL is cumulative** and therefore blocks
late-session moves (five documented misses — see §2). The burst leg is
time-local and immune to that.

**The concentration clause is the "fast" test.** Displacement says how far,
never how quickly; without it a fifteen-minute grind produces the same number as
a fifteen-minute crash. Added after TSEM, 2026-08-04.

**Known limitation, all six:** clock-aligned windows split a move that straddles
a boundary into two smaller measurements. Roughly one spike in five.

---

### §4.1 M_Spike_Up ✅ verified 2026-09-05

**Hunts:** a stock up on the day, at its high of day, on a sharp five-minute
thrust.

| Parameter | Value |
|---|---|
| DAYVOL | 150,000 |
| BURST | 2.0 |
| Tiers | 5-min only |
| 5-min PCT | 0.020 |
| 5-min ATR high / low | 0.50 / 0.80 |
| Day gate | `Last >= ClosePrice(P1)` |
| Day extreme | `Last >= DayHigh(CURRENT) − 0.15 × ATR` |
| Trigger | `NewMinuteHigh(15, NO)` |

**Day-extreme tolerance** — `− 0.15 × ATR` rather than a bare `>= DayHigh`,
which is a knife-edge test on a snapshot engine.

**Limit:** the day-extreme requirement makes this structurally morning-weighted,
since day highs are mostly set early. Drop/Pop carries afternoon coverage.

---

### §4.2 M_Spike_Down ✅ verified 2026-09-05
Exact mirror of §4.1. `Last <= ClosePrice(P1)`,
`Last <= DayLow(CURRENT) + 0.15 × ATR`, `NewMinuteLow(15, NO)`.

---

### §4.3 M_Pop_Sensitive_Up ✅ verified 2026-09-05

**Hunts:** a large, fast move up from a recent extreme over 5 or 15 minutes,
**regardless of direction on the day.** The reversal catcher.

| Parameter | Value |
|---|---|
| DAYVOL | 250,000 |
| BURST | 3.0 |
| Tiers | 5-min, 15-min |
| 5-min PCT / ATR high / ATR low | 0.025 / 0.50 / 0.80 |
| 15-min PCT / ATR high / ATR low | 0.040 / 0.90 / 1.40 |
| Day gate | **none — deliberate** |
| Trigger | `NewMinuteHigh(15, NO)` |

**No day gate is the point.** This catches sweeps against the day, which is the
reversal setup. Gap-up fades like IDXX and HSIC belong here, not on M_Spike.

**Limit:** no time-of-day bias whatsoever. If it's quiet in the afternoon, the
market isn't producing moves this size — the filter is not at fault.

---

### §4.4 M_Drop_Sensitive_Down ✅ verified 2026-09-05
Exact mirror of §4.3.

---

### §4.5 M_Pop_Medium/EXT_Up ✅ verified 2026-09-05

**Hunts:** the same setup as Sensitive, but large. A strength flag, not a
separate signal.

| Parameter | Value |
|---|---|
| DAYVOL | 250,000 |
| BURST | 3.0 |
| 5-min PCT / ATR high / ATR low | 0.050 / 0.90 / 1.60 |
| 15-min PCT / ATR high / ATR low | 0.080 / 1.40 / 2.40 |

**Strict subset of Sensitive** — every threshold is roughly doubled and nothing
else differs, so anything here also appears there. Treat as a highlight on the
Sensitive list rather than a second list to scan.

---

### §4.6 M_Drop_Medium/EXT_Down ✅ verified 2026-09-05
Exact mirror of §4.5.

---

### §4.7 M_Drive_Up ✅ verified 2026-09-05

**Hunts:** ⬜ *intent not yet stated by Peter.* From the expression: a stock up
on the day that has covered meaningful ground from a recent low across 5, 15 or
30 minutes.

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 250000
AND Last(ALL_VENUES, CURRENT, NO) >= 2
AND RVOL20DAYTIMEADJUSTED >= 1.25
AND Last(ALL_VENUES, CURRENT, NO) >= ClosePrice(PRIMARY, P1, NO)
AND ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.50
AND (
  ( ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 5, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.75
    AND ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 5, CURRENT, NO, False) ) / ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.15 )
  OR ( ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 1.0
    AND ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) ) / ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.15 )
  OR ( ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 30, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 1.25
    AND ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 30, CURRENT, NO, False) ) / ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.15 )
)
AND ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 30, CURRENT, NO, False) ) >= 0.50
AND NewMinuteHigh(5, NO)
AND MaxTriggersCount(MINUTES, 1, 1)
```

⚠️ **Previous-generation design.** Predates every fix applied to §4.1–§4.6:

- **No two-band logic, no percent floors.** Pure ATR multiples, so it is loose on
  calm names and tight on volatile ones — the exact inversion the bands fixed.
- **No burst leg.** Bare `RVOL >= 1.25`, so it inherits the full cumulative
  dilution problem and is blind to late-session movers.
- **The ELVA clause is present in all three tiers** — `move / day range >= 0.15`.
  As the day's range expands the required move grows with it, so a genuine trend
  day locks itself out. Removed from the six for exactly this reason.
- **Square-root tier scaling.** 0.75 / 1.0 / 1.25 across 5 / 15 / 30 min works
  out to 0.150 / 0.067 / 0.042 ATR per minute — the 30-minute branch is **3.5×
  easier per minute**. Since the tiers are OR'd, that branch is effectively the
  whole filter, making it a drift detector rather than a drive detector.
- **`NewMinuteHigh(5)`** rather than `(15)`, so it re-arms three times as often
  as the six. Combined with the 1-minute throttle, this is the likely cause of
  repeat firing (GH, GRI, BIAF on 2026-09-04).
- Two inert dollar constants at `0.50`.

**Overlaps M_Pop_Sensitive_Up substantially.** Whether that overlap is by design
or by accident is unresolved.

---

### §4.8 M_Drive_Down ✅ verified 2026-09-05
Exact mirror of §4.7, with the same five issues.

---

### §4.9 High_ATR_Pop ✅ verified 2026-09-05

**Hunts:** an expensive, genuinely volatile stock making a big dollar move.

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 1000000
AND Last(ALL_VENUES, CURRENT, NO) >= 10
AND ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 10
AND ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) / Last(ALL_VENUES, CURRENT, NO) <= 0.20
AND RVOL20DAYTIMEADJUSTED >= 1.25
AND (
  ( ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 5, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.30
    AND ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 5, CURRENT, NO, False) ) / Last(ALL_VENUES, CURRENT, NO) >= 0.02 )
  OR ( ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.50
    AND ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) ) / Last(ALL_VENUES, CURRENT, NO) >= 0.03 )
)
AND NewMinuteHigh(15, NO)
AND MaxTriggersCount(MINUTES, 1, 1)
```

**AND inside each tier, not OR.** The universe is already ATR-gated, so both
yardsticks should confirm. This is the opposite choice from the six, where the
percent leg is a rescue path — and it's deliberate. With an OR, VLO's $3 chop
qualified via the ATR leg while DFNS's $0.91 wiggles qualified via the percent
leg; the AND makes the harder leg bind in both cases.

**The volatility ceiling at 0.20** excludes wrecks whose ATR has become
meaningless relative to price — DFNS at 31% of price was firing constantly.

**⚠️ The effective price floor is $50, not $10.** `ATRStock >= 10` combined with
`ATR/Last <= 0.20` implies `Last >= 50` arithmetically, so the `Last >= 10`
clause is dead. In practice the universe is much higher — MU at $900 with an
$87 ATR is 9.7%.

**⚠️ No burst leg.** Bare `RVOL >= 1.25`, so this pair still carries the
cumulative-dilution problem — and it watches exactly the mega-caps that move on
late-session news (CRM, WDAY at 14:30; PYPL at 15:44). Adding the burst OR is a
one-line change using the clause already running in the six.

---

### §4.10 High_ATR_Drop ✅ verified 2026-09-05
Exact mirror of §4.9.

---

### §4.11 M_Spike_Hold_Up ✅ verified 2026-09-23

**Hunts:** a stock that spiked hard on volume in a completed five-minute bar and
then did not give any of it back. **Peter's best-performing setup.**

**Baseline — small tweaks only.**

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 250000
AND Last(ALL_VENUES, CURRENT, NO) >= 2
AND ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) / ClosePrice(PRIMARY, P1, NO) >= 0.025
AND Last(ALL_VENUES, CURRENT, NO) >= ClosePrice(PRIMARY, P1, NO)
AND RVOL20DAYTIMEADJUSTED >= 1.25
AND ( MinuteHigh(ALL_VENUES, 5, P1, NO, False) - MinuteLow(ALL_VENUES, 5, P1, NO, False) ) >= 0.50
AND ( MinuteHigh(ALL_VENUES, 5, P1, NO, False) - MinuteLow(ALL_VENUES, 5, P1, NO, False) ) >= Last(ALL_VENUES, CURRENT, NO) * 0.025
AND ( MinuteHigh(ALL_VENUES, 5, P1, NO, False) - MinuteLow(ALL_VENUES, 5, P1, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.40
AND ( MinuteVolume(ALL_VENUES, 5, P1, NO) >= MinuteVolume(ALL_VENUES, 5, P2, NO) * 1.25
      OR MinuteVolume(ALL_VENUES, 5, P1, NO) >= AvgDayVolume(ALL_VENUES, 20, NO) * 0.03 )
AND MinuteLow(ALL_VENUES, 1, P1, NO, False) >= MinuteHigh(ALL_VENUES, 5, P1, NO, False) - ( MinuteHigh(ALL_VENUES, 5, P1, NO, False) - MinuteLow(ALL_VENUES, 5, P1, NO, False) ) * 0.25
AND Last(ALL_VENUES, CURRENT, NO) >= MinuteHigh(ALL_VENUES, 5, P1, NO, False) - ( MinuteHigh(ALL_VENUES, 5, P1, NO, False) - MinuteLow(ALL_VENUES, 5, P1, NO, False) ) * 0.25
AND MinuteLow(ALL_VENUES, 5, CURRENT, NO, False) >= MinuteHigh(ALL_VENUES, 5, P1, NO, False) - ( MinuteHigh(ALL_VENUES, 5, P1, NO, False) - MinuteLow(ALL_VENUES, 5, P1, NO, False) ) * 0.50
AND MaxTriggersCount(MINUTES, 5, 1)
```

**The model (Peter, 2026-07-31):** spike, then a short coil, then continuation.
The coil is the stock *deciding*. If it's going to reverse it does so quickly;
if it doesn't reverse quickly it likely continues.

**Three magnitude floors on the spike bar** — dollar, percent and ATR. The
percent floor (`Last × 0.025`) was added 2026-09-23 and is the clause this
filter had been missing that all six sharp-move filters carry. It exists to
exclude large caps whose `0.40 ATR` is a small percentage move: on the 09-21
logs, ADI cleared the ATR test with a 1.35% bar, DVN with 1.19%, GE with 1.46%.
Roughly a third of fires were under 2.5%.

**Three retention clauses**, and they do different jobs:

| Clause | Bound | What it tests |
|---|---|---|
| `MinuteLow(1, P1)` | 0.25 | the last completed minute held in the far quarter |
| `Last` | 0.25 | price is in the far quarter **right now** |
| `MinuteLow(5, CURRENT)` | 0.50 | price never round-tripped more than half the bar |

The third is a **running minimum**, restored 2026-09-23 at 0.50. Its history
matters: it originally ran at 0.25, which meant a single dip disqualified the
entire remaining bar and the setup rolled to the next bar at a higher price
(PLTR fired 179.15 when the band was 176.04). It was removed entirely on 09-03
and replaced with the `Last` clause — which loosened the filter roughly sixfold,
since only 5 of 29 fires on the 09-04 log would have passed the old 0.25 bound.
Restoring it at 0.50 keeps both: `Last` says price is well positioned now, the
running low says it never gave it all back.

**Volatility floor at 0.025**, lowered from 0.03 on 2026-09-03 — SPCX and PLTR
both compute near 3.0% and were sitting on the boundary.

**⚠️ Instrumentation note.** `RETRACE_PCT_UP` computes the running-low formula,
so it now corresponds to a clause the filter actually enforces again. Any logged
value above 50 is a fire that no longer happens. Between 09-03 and 09-23 the
field and the filter measured different things, which is why logs from that
window show retrace values well above 25 (and why AMAT read 87.93 on what looked
like a clean ramp).

**Limits**
- **Expires in ~5 minutes.** Everything keys off `P1`; once the bar rolls the
  setup vanishes even if still valid. A stock that spikes and bases 20 minutes
  later is invisible (MOH, RNG — unfixed since July).
- Requires spike-then-pause. A continuous ramp (PLTR, EXLS) has no pause.
- **Repeat fires are the dominant noise source.** On 09-21, 48 of 91 fires came
  from 18 distinct names — JAGX six times, VKTX six, GRML five. The throttle is
  still 5 minutes; raising it to 30 costs nothing in coverage and is the next
  lever if the list stays unmanageable.

---

### §4.12 M_Spike_Hold_Down ✅ verified 2026-09-23
Exact mirror of §4.11 — same 0.025 volatility floor, same 2.5% percent floor,
same three retention clauses at 0.25 / 0.25 / 0.50.

---

### §4.13 M_Run_Hold_Up ✅ verified 2026-09-05

**Hunts:** a sustained fifteen-minute run that price is still holding near the
top of. A slower shape than M_Spike_Hold.

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 250000
AND Last(ALL_VENUES, CURRENT, NO) >= 2
AND RVOL20DAYTIMEADJUSTED >= 1.25
AND ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.50
AND ( MinuteHigh(ALL_VENUES, 15, CURRENT, NO, False) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.60
AND ( MinuteHigh(ALL_VENUES, 15, CURRENT, NO, False) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) ) >= 0.50
AND ( MinuteHigh(ALL_VENUES, 15, CURRENT, NO, False) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) ) / ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.15
AND ( MinuteVolume(ALL_VENUES, 15, CURRENT, NO) >= MinuteVolume(ALL_VENUES, 15, P1, NO) * 2.0 OR MinuteVolume(ALL_VENUES, 15, CURRENT, NO) >= AvgDayVolume(ALL_VENUES, 20, NO) * 0.077 )
AND Last(ALL_VENUES, CURRENT, NO) >= ClosePrice(PRIMARY, P1, NO)
AND Last(ALL_VENUES, CURRENT, NO) >= MinuteHigh(ALL_VENUES, 15, CURRENT, NO, False) - ( MinuteHigh(ALL_VENUES, 15, CURRENT, NO, False) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) ) * 0.25
AND ( MinuteHigh(ALL_VENUES, 1, CURRENT, NO, False) - MinuteLow(ALL_VENUES, 1, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) <= 0.25
AND ( MinuteHigh(ALL_VENUES, 1, P1, NO, False) - MinuteLow(ALL_VENUES, 1, P1, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) <= 0.25
AND Last(ALL_VENUES, CURRENT, NO) >= VWAP(CURRENT, NO, VWAP, V1, PRIMARY)
AND MaxTriggersCount(MINUTES, 5, 1)
```

⚠️ **Previous-generation design**, like M_Drive:
- No two-band ATR, no percent floors. Pure ATR at 0.60 plus a dead `$0.50`.
- No burst leg — bare `RVOL >= 1.25`.
- **ELVA present** (`15-min range / day range >= 0.15`).
- **First coil clause uses the FORMING 1-minute bar.** Right after each minute
  boundary that range is near zero, so it passes automatically for part of every
  minute. Only the `P1` clause does real work. Same defect fixed elsewhere by
  moving to P1/P2.

**Self-tightening ratchet.** The retention clause and the range clause reference
the same forming 15-minute window. As the window develops its range can only
grow, which raises the retention bar. The filter therefore gets progressively
harder to satisfy as each window matures, then resets — so fires cluster early
in each 15-minute block.

**Only filter in the hold family with a VWAP clause.**

---

### §4.14 M_Run_Hold_Down ✅ verified 2026-09-05
Exact mirror of §4.13, same four issues.

---

### §4.15 M3_Breakout_Long ✅ verified 2026-09-05

**Hunts:** a liquid large-cap breaking above **yesterday's high** to a new high
of day, in an uptrend, not yet extended from VWAP.

```
Last(ALL_VENUES, CURRENT, NO) >= 20
AND Last(ALL_VENUES, CURRENT, NO) < 600
AND AvgDayVolume(ALL_VENUES, 20, NO) >= 2000000
AND RVOL20DAYTIMEADJUSTED >= 2.0
AND ( Last(ALL_VENUES, CURRENT, NO) - ClosePrice(PRIMARY, P1, NO) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.5
AND ( Last(ALL_VENUES, CURRENT, NO) - ClosePrice(PRIMARY, P1, NO) ) >= Last(ALL_VENUES, CURRENT, NO) * 0.01
AND ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) / Last(ALL_VENUES, CURRENT, NO) <= 0.15
AND ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.50
AND Last(ALL_VENUES, CURRENT, NO) >= DayHigh(ALL_VENUES, 1, P1, NO)
AND Last(ALL_VENUES, CURRENT, NO) >= DayHigh(ALL_VENUES, 1, CURRENT, NO)
AND ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow_I(ALL_VENUES, 15, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.30
AND ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow_I(ALL_VENUES, 15, CURRENT, NO, False) ) / ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.15
AND Last(ALL_VENUES, CURRENT, NO) >= VWAP(CURRENT, NO, VWAP, V1, PRIMARY)
AND Last(ALL_VENUES, CURRENT, NO) <= VWAP(CURRENT, NO, VWAP, V1, PRIMARY) + 1.0 * ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT)
AND EMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_7, CURRENT) >= EMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_20, CURRENT)
AND MinuteVolume(ALL_VENUES, 5, CURRENT, NO) >= MinuteVolume(ALL_VENUES, 5, P1, NO) * 1.5
AND NewMinuteHigh(5, NO)
AND MaxTriggersCount(MINUTES, 1, 1)
```

**Distinctive features — three clauses that exist nowhere else:**

**`DayHigh(ALL_VENUES, 1, P1, NO)`** — prior day's high. A cleaner breakout level
than the monthly references, because it updates daily.

**VWAP extension cap** — `Last <= VWAP + 1.0 × ATR`. **The only anti-chase clause
in the entire set.** Bounds how extended a fire can be. If late fires reappear
in the M_Spike family, this clause is already written and proven here.

**Universe is deliberately narrow** — $20–600, ADV ≥ 2M, volatility ceiling at
15%. The only filter with an ADV floor and a two-sided price band.

**Better built than the other old-generation filters:** it has percent floors
alongside ATR (`0.5 ATR` **and** `1% of price`) and a volatility ceiling.

**Legacy issues:** ELVA present; and the volume clause uses the forming bar
against the completed prior one while `NewMinuteHigh(5)` fires at the instant of
the break — those two race each other.

---

### §4.16 M4_Breakdown_Short ✅ verified 2026-09-05
Exact mirror of §4.15, including the VWAP extension cap at `VWAP − 1.0 × ATR`.

---

### §4.17 M5_Pullback_Long ✅ verified 2026-09-05

**Hunts:** a liquid large-cap in an uptrend that pulled back to tag its
5-minute EMA7 and is now pushing back through the pullback bar's high.
**The only pullback-entry setup in the set.**

```
Last(ALL_VENUES, CURRENT, NO) >= 20
AND Last(ALL_VENUES, CURRENT, NO) < 600
AND AvgDayVolume(ALL_VENUES, 20, NO) >= 2000000
AND RVOL20DAYTIMEADJUSTED >= 2.0
AND TickTime >= 600
AND TickTime <= 920
AND Last(ALL_VENUES, CURRENT, NO) >= ClosePrice(PRIMARY, P1, NO) * 1.03
AND MinuteHigh(ALL_VENUES, 30, CURRENT, NO, False) >= DayHigh(ALL_VENUES, 1, P1, NO) * 0.97
AND Last(ALL_VENUES, CURRENT, NO) >= VWAP(CURRENT, NO, VWAP, V1, PRIMARY)
AND Last(ALL_VENUES, CURRENT, NO) <= VWAP(CURRENT, NO, VWAP, V1, PRIMARY) * 1.04
AND EMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_7, CURRENT) >= EMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_20, CURRENT)
AND MinuteLow(ALL_VENUES, 5, P1, NO, False) <= EMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_7, CURRENT) * 1.01
AND MinuteLow(ALL_VENUES, 5, P1, NO, False) >= EMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_7, CURRENT) * 0.97
AND Last(ALL_VENUES, CURRENT, NO) >= MinuteHigh(ALL_VENUES, 5, P1, NO, False)
AND MinuteVolume(ALL_VENUES, 5, CURRENT, NO) >= MinuteVolume(ALL_VENUES, 5, P1, NO) * 1.2
AND MaxTriggersCount(MINUTES, 5, 1)
```

**The EMA proximity pair is the heart of it** — the prior bar's low between
**0.97× and 1.01×** of EMA7. Tagged the average but didn't break far below it.
Nothing else in the set does anything like this.

**Then `Last >= MinuteHigh(5,P1)`** — taking out the pullback bar's high. The
resumption.

**Session window `TickTime` 600–920 = 10:00–15:20.** The only filter with one,
and it is right for this setup: a pullback-to-EMA needs an existing trend, which
rules out the open, and you don't want it firing into the close.

**VWAP cap is percentage-based (1.04)** rather than ATR-based as in M3.
Inconsistent — ATR-based is the better construction.

**One loose clause:** `MinuteHigh(30, CURRENT) >= DayHigh(P1) * 0.97` — traded
within 3% of yesterday's high at some point in the current 30-minute window. 3%
is loose on a $500 stock, and the window is clock-aligned so it resets.

🗑️ **Registry note:** this filter occupies two slots; the second was meant to be
M3. Check whether the copies are identical.

---

### §4.18 M6_Bounce_Short ✅ verified 2026-09-05
Exact mirror of §4.17. Prior bar's **high** between 0.99× and 1.03× of EMA7,
then `Last <= MinuteLow(5,P1)`. Same 10:00–15:20 window, same 1.2 volume
multiple, same percentage VWAP cap.

---

### §4.19 M7_GrindUp ✅ verified 2026-09-05

**Hunts:** an orderly uptrend making higher highs and higher lows, fully stacked
above its averages. Not a spike — a grind.

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 750000
AND Last(ALL_VENUES, CURRENT, NO) >= 5
AND RVOL20DAYTIMEADJUSTED >= 1.5
AND Last(ALL_VENUES, CURRENT, NO) >= ClosePrice(PRIMARY, P1, NO) * 1.02
AND Last(ALL_VENUES, CURRENT, NO) >= VWAP(CURRENT, NO, VWAP, V1, PRIMARY)
AND MinuteLow_I(ALL_VENUES, 5, P1, NO, False) >= VWAP(CURRENT, NO, VWAP, V1, PRIMARY)
AND EMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_7, CURRENT) >= EMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_20, CURRENT)
AND Last(ALL_VENUES, CURRENT, NO) >= EMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_20, CURRENT)
AND Last(ALL_VENUES, CURRENT, NO) >= EMAStock(ALL_VENUES, NO, DAILY, PERIOD_20, CURRENT)
AND MinuteHigh_I(ALL_VENUES, 5, P1, NO, False) >= MinuteHigh_I(ALL_VENUES, 5, P3, NO, False)
AND MinuteLow_I(ALL_VENUES, 5, P1, NO, False) >= MinuteLow_I(ALL_VENUES, 5, P3, NO, False)
AND NewMinuteHigh(5, NO)
AND MaxTriggersCount(MINUTES, 10, 1)
```

**Entirely about alignment, not magnitude.** No ATR anywhere, no percent floor,
no move-size requirement at all. It doesn't care how far the stock has gone,
only that the structure is intact — which is coherent for a grind filter. A
magnitude test would defeat the purpose.

**The pullbacks don't even touch VWAP** — `MinuteLow_I(5,P1) >= VWAP`.

**Full stack:** EMA7 > EMA20 on 5-min, price above 5-min EMA20, and price above
the **daily** EMA20. The only daily-EMA reference in the set.

**Stair-step test:** higher highs and higher lows across three bars — the same
structural shape drafted for Program_Buying on 2026-08-21, and it needs no
`MinuteClose`.

**Longest throttle in the set** at 10 minutes, appropriate for a slow setup.

**⚠️ Overlaps the unbuilt Program_Buying concept.** M7 finds the price structure
of institutional accumulation but has no volume-uniformity test and no elevated
volume requirement beyond `RVOL >= 1.5`. **Adding the uniformity clauses to M7
is a smaller move than building Program_Buying separately.** Check first whether
M7 caught SOFI on 2026-08-21.

---

### §4.20 M8_GrindDown ✅ verified 2026-09-05
Exact mirror of §4.19 — lower highs and lower lows, price below both EMAs and
the daily EMA20, prior bar's high below VWAP.

---

### §4.21 LowFloat_HOD_Break_Up ✅ verified 2026-09-23

**Hunts:** a low-float stock up big and breaking to new highs on heavy volume.
**No base requirement** — this is the parabolic runner filter.

```
FloatShares <= 20
AND AdjustedClosePrice(P1) >= 0.25
AND AdjustedClosePrice(P1) <= 50
AND DayVolume(ALL_VENUES, 1, CURRENT, YES) >= 250000
AND ( Last(ALL_VENUES, CURRENT, NO) - ClosePrice(PRIMARY, P1, NO) ) / ClosePrice(PRIMARY, P1, NO) >= 0.10
AND Last(ALL_VENUES, CURRENT, NO) >= DayHigh(ALL_VENUES, 1, CURRENT, NO) - 0.15 * ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT)
AND ( RVOL20DAYTIMEADJUSTED >= 3.0 OR MinuteVolume(ALL_VENUES, 5, CURRENT, NO) * 24 >= MinuteVolume(ALL_VENUES, 120, CURRENT, NO) * 3.0 )
AND ( MinuteVolume(ALL_VENUES, 5, P1, NO) >= MinuteVolume(ALL_VENUES, 5, P2, NO) * 1.5
      OR MinuteVolume(ALL_VENUES, 5, P1, NO) >= AvgDayVolume(ALL_VENUES, 20, NO) * 0.05 )
AND NewMinuteHigh(5, NO)
AND MaxTriggersCount(MINUTES, 5, 1)
```

**Rebuilt 2026-08-17** from a base-break filter into a runner filter. The base
block was deleted because WFF (+469%) never made one — *a coil-based filter
cannot catch a move that never pauses.*

**Changed since 09-05:** float 10M → **20M**, price floor $1.00 → **$0.25**, and
a **today's-volume gate** added: `DayVolume(ALL_VENUES, 1, CURRENT, YES) >=
250000`. Peter notes this filter has been revised several times.

**⭐ The today's-volume gate is the right structure**, matching §5.3. It replaces
what an `AvgDayVolume` floor would do, without excluding dormant low-floats — a
stock that normally trades nothing qualifies provided it is genuinely trading
today, premarket included. That is the WFF/VRME lesson applied.

**Deliberate extended-hours mix:** `NO` on price and the day-high clause since
the trigger is regular-hours; `YES` on `DayVolume` so premarket counts toward the
gate.

**⚠️ Still no split-immune volume leg.** Two problems with the
`AvgDayVolume(20) * 0.05` leg for this filter's target population:
- For a dormant low-float with 50k ADV, 5% is **2,500 shares** in a five-minute
  bar — trivially passed, so the leg discriminates nothing among exactly the
  names it should.
- After PFSA's 1-for-4 reverse split (2026-08-18), `AvgDayVolume(20)` was inflated
  ~4× by unadjusted historical share counts, making the same leg ~4× too strict on
  a name that had just done +449%.

The fix proposed 2026-08-18 was never added — see fix queue item 18:
```
OR MinuteVolume(ALL_VENUES, 5, P1, NO) >= DayVolume(ALL_VENUES, 1, CURRENT, NO) * 0.03
```
Both sides same-session, so no historical adjustment can distort it.

**⚠️ `FloatShares` is a real gate here** (unlike §5.6, where the old `Float <=
10000000` form is inert). Whether it populates for foreign small-cap issuers is
unverified, and WFF and PFSA are exactly the names targeted.

---

### §4.22 HOD_Base_Break_LQ ✅ verified 2026-09-05

**Hunts:** a liquid stock breaking out of a tight 30-minute base near the highs
to a new high of day, on genuinely heavy volume.

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 500000
AND Last(ALL_VENUES, CURRENT, NO) >= ClosePrice(PRIMARY, P1, NO) * 1.05
AND ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= Last(ALL_VENUES, CURRENT, NO) * 0.03
AND ( RVOL20DAYTIMEADJUSTED >= 2.0 OR MinuteVolume(ALL_VENUES, 5, CURRENT, NO) * 24 >= MinuteVolume(ALL_VENUES, 120, CURRENT, NO) * 3.0 )
AND EMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_7, CURRENT) >= EMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_20, CURRENT)
AND ( MinuteHigh_I(ALL_VENUES, 30, P1, NO, False) - MinuteLow_I(ALL_VENUES, 30, P1, NO, False) ) / ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) <= 0.20
AND MinuteHigh_I(ALL_VENUES, 30, P1, NO, False) >= DayHigh(ALL_VENUES, 1, CURRENT, NO) - 0.25 * ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) )
AND Last(ALL_VENUES, CURRENT, NO) >= MinuteHigh_I(ALL_VENUES, 30, P1, NO, False) + 0.15 * ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT)
AND Last(ALL_VENUES, CURRENT, NO) >= DayHigh(ALL_VENUES, 1, CURRENT, NO)
AND MinuteVolume(ALL_VENUES, 5, CURRENT, NO) >= MinuteVolume(ALL_VENUES, 5, P1, NO) * 1.5
AND MinuteVolume(ALL_VENUES, 5, CURRENT, NO) >= AvgDayVolume(ALL_VENUES, 20, NO) * 0.03
AND NewMinuteHigh(5, NO)
AND MaxTriggersCount(MINUTES, 5, 1)
```

**⭐ The only genuine break margin in the set** — `Last >= base high + 0.15 ×
ATR`. Price must *clear* the level by a real amount rather than touch it.
Base_Break and Breakout_Hold both use a bare `>=` against their monthly levels
and therefore fire on the touch. **If the monthly-level filters start showing
marginal breaks, this clause is already written and proven.**

**Volume is an AND, not an OR** — accelerating **and** heavy. Every other filter
uses OR. Deliberate, from the 2026-08-17 tightening.

**Tightened 2026-08-17:** break margin added; volume AND; day move 3% → 5%; day
range floor changed from `$0.50` to 3% of price; base 25% → 20%; near-highs
35% → 25%; throttle 1 min → 5 min.

**Known limitation:** the base uses a clock-aligned `MinuteHigh_I(30, P1)`, so a
base forming 10:15–10:45 is never a P1 bar. Roughly two alignment chances an
hour.

---

### §4.23 Base_Break_Hold_Up 🔄 REBUILT 2026-09-23

**Hunts:** a stock that gapped up and held, made a strong move on the day,
spiked hard on a completed five-minute bar, consolidated tightly, and is
breaking above **last month's high** for the first time today.

**This is M_Spike_Hold tightened, plus a monthly level.** The previous version —
a three-month flat-top test — was deleted, not tuned. See "why it was rebuilt"
below.

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 500000
AND Last(ALL_VENUES, CURRENT, NO) >= 5
AND ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) / ClosePrice(PRIMARY, P1, NO) >= 0.025
AND RVOL20DAYTIMEADJUSTED >= 2.0
AND Last(ALL_VENUES, CURRENT, NO) >= ClosePrice(PRIMARY, P1, NO) * 1.03
AND DayLow(ALL_VENUES, 1, CURRENT, NO) >= ClosePrice(PRIMARY, P1, NO)
AND Last(ALL_VENUES, CURRENT, NO) >= MonthHigh_I(ALL_VENUES, 1, P1, NO)
AND ClosePrice(PRIMARY, P1, NO) < MonthHigh_I(ALL_VENUES, 1, P1, NO)
AND ( MinuteHigh(ALL_VENUES, 5, P1, NO, False) - MinuteLow(ALL_VENUES, 5, P1, NO, False) ) >= 0.50
AND ( MinuteHigh(ALL_VENUES, 5, P1, NO, False) - MinuteLow(ALL_VENUES, 5, P1, NO, False) ) >= Last(ALL_VENUES, CURRENT, NO) * 0.035
AND ( MinuteHigh(ALL_VENUES, 5, P1, NO, False) - MinuteLow(ALL_VENUES, 5, P1, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.60
AND ( MinuteVolume(ALL_VENUES, 5, P1, NO) >= MinuteVolume(ALL_VENUES, 5, P2, NO) * 1.5
      OR MinuteVolume(ALL_VENUES, 5, P1, NO) >= AvgDayVolume(ALL_VENUES, 20, NO) * 0.05 )
AND ( MinuteHigh(ALL_VENUES, 1, P1, NO, False) - MinuteLow(ALL_VENUES, 1, P1, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) <= 0.20
AND ( MinuteHigh(ALL_VENUES, 1, P2, NO, False) - MinuteLow(ALL_VENUES, 1, P2, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) <= 0.20
AND MinuteLow(ALL_VENUES, 1, P1, NO, False) >= MinuteHigh(ALL_VENUES, 5, P1, NO, False) - ( MinuteHigh(ALL_VENUES, 5, P1, NO, False) - MinuteLow(ALL_VENUES, 5, P1, NO, False) ) * 0.25
AND Last(ALL_VENUES, CURRENT, NO) >= MinuteHigh(ALL_VENUES, 5, P1, NO, False) - ( MinuteHigh(ALL_VENUES, 5, P1, NO, False) - MinuteLow(ALL_VENUES, 5, P1, NO, False) ) * 0.25
AND MinuteLow(ALL_VENUES, 5, CURRENT, NO, False) >= MinuteHigh(ALL_VENUES, 5, P1, NO, False) - ( MinuteHigh(ALL_VENUES, 5, P1, NO, False) - MinuteLow(ALL_VENUES, 5, P1, NO, False) ) * 0.50
AND MaxTriggersCount(MINUTES, 15, 1)
```

#### Why it was rebuilt

The old version was both sparse and low-quality, for two separate reasons.

**Sparse:** `Last` above all three monthly highs *while yesterday's close was
below all three* fires on exactly one day per stock — and had to coincide with
`RVOL >= 2.0`, 1M day volume, two quiet minutes, and price within 0.40 ATR of
the day high, all at one instant.

**Low quality — the core defect: a monthly high is an extreme, not a level.**
It is one tick in a month. Three monthly highs landing within 2 ATR of each
other says almost nothing about whether the stock consolidated; a three-month
sawtooth that happened to peak at similar prices passed identically to a genuine
flat top. Three things compounded it: 2 ATR is not flat (on a $100 stock with a
$3 ATR the highs could span 6%); the 8 ATR depth bound applied only to `P1`, so
`P2` and `P3` could have had enormous ranges; and the four proximity clauses
compared `P1` to `P2` and `P1` to `P3` but never `P2` to `P3`, so those two could
sit 4 ATR apart.

#### Clause notes

**Monthly level — one month, not three.** `Last >= MonthHigh_I(P1)` with
`ClosePrice(P1) < MonthHigh_I(P1)` for the same-day break. Adding `P2` and `P3`
would make it a true three-month high if a stricter version is wanted later.

**⚠️ "Opened positive" is a proxy.** `DayLow(CURRENT) >= ClosePrice(P1)` means
the whole day's range sits above yesterday's close — the gap never filled. That
is **stricter** than "opened positive": a stock that dipped red for a minute at
9:31 and is now up 5% breaking out gets excluded. It was chosen because
`DayBar_Open` throws a subscription exception and there is no other working
day-open token.

**Tightenings from M_Spike_Hold:** volume 250k → 500k, price $2 → $5, RVOL
1.25 → 2.0, spike-bar percent 2.5% → 3.5%, spike-bar ATR 0.40 → 0.60, volume
legs 1.25×/3% → 1.5×/5%, throttle 5 min → 15 min. The three retention clauses
are unchanged at 0.25 / 0.25 / 0.50.

**Two consolidation clauses at 0.20 ATR** on the 1-minute `P1` and `P2` bars —
tighter than the 0.25 the old version used.

#### Known risks

- **This is a long conjunction and may run as dry as what it replaced.** A
  strong spike bar, two tight minutes, a gap that never filled and a first-time
  monthly break all at one instant is a lot to ask.
- **If dry, relax in this order:** the `DayLow` gap clause first (it is the proxy
  and the most arbitrary), then the two consolidation clauses. Leave the monthly
  break and the spike-bar thresholds alone — those are what make it the setup.
- **⚠️ Check for subset overlap with Breakout_Hold.** A July session concluded
  Base_Break_Hold was a strict subset of Breakout_Hold and retired it on those
  grounds. That finding was made against different expressions than either
  filter now runs, so it may be stale — but verify the rebuild isn't a subset
  again before relying on it.

---

### §4.24 Base_Break_Hold_Down 🔄 REBUILT 2026-09-23
Exact mirror of §4.23. Direction-dependent clauses flip:
`Last <= ClosePrice(P1) * 0.97`, `DayHigh(CURRENT) <= ClosePrice(P1)`,
`Last <= MonthLow_I(P1)`, `ClosePrice(P1) > MonthLow_I(P1)`, and the three
retention clauses measure from `MinuteLow(5, P1)` upward.

---

### §4.25 Breakout_Hold_Up ✅ verified 2026-09-05

**Hunts:** a stock taking out last month's high today and holding above it.

**Same-day break** — `Last >= MonthHigh_I(P1)` **AND** `ClosePrice(P1) <
MonthHigh_I(P1)`. Without the second half this fires every day forever once
cleared. Gap path: `(Last - ClosePrice(P1)) / ATR >= 1.0`.

**`DayBar_Open` is gone** — replaced by the gap leg above. That is why the
subscription fault stopped.

**Has a genuine thrust requirement**, unlike Base_Break: either a completed
5-min bar clearing 0.40 ATR with a directional close and volume, or a 15-min
displacement clearing 0.60 ATR with volume. A stock that broke out at 9:35 and
went dead cannot satisfy either at 2:20.

**Legacy issues:** ELVA in both branches; depends on `MinuteClose_I` /
`MinuteOpen_I` for the directional bar gate — still unresolved against the
Rule 0 `MinuteClose` ban, now three filters deep.

#### ⚠️ Diagnosed 2026-09-23 — left unchanged by decision

Peter's assessment: *"not finding great trades and they are way too late."*
Both halves have specific causes.

**Not breakouts.** The level clause is OR'd with a gap leg —
`( Last - ClosePrice(P1) ) / ATR >= 1.0`. **That makes the monthly breakout
optional.** Any stock up one ATR from yesterday's close qualifies with no level
involved, and that condition is *state* — true all day once the stock gaps. Most
fires are therefore gappers, not breakouts, despite the filter's name.

**Late, from three causes stacked:**
1. It requires **two** quiet 1-minute bars (`P1` and `P2`), one more than
   M_Spike_Hold, so it is a full minute later by construction — earliest
   possible fire is roughly seven minutes after a move starts.
2. **There is no trigger clause at all** — no `NewMinuteHigh`. It is a pure
   state test throttled at 1 per 5 min, so it fires at the first snapshot where
   the coil happens to be satisfied, which by definition is *after* the move has
   stopped. Nothing in it can fire early.
3. Branch 2 measures displacement from the current 15-minute window's low, which
   peaks at the **end** of a move and never expires within the window.

**Recommendation on file (not taken):** retire it. The rebuilt §4.23 covers the
same setup — spike bar, hold, same-day monthly break — with percent floors,
proper retention, no ELVA, one coil bar and no gap bypass. Removing Breakout_
Hold's gap leg and a coil bar would turn it into §4.23.

**Minimum fix if kept:** delete the gap OR leg. That alone makes it an actual
breakout filter.

---

### §4.26 Breakout_Hold_Down ✅ verified 2026-09-05
Exact mirror of §4.25.

---

## §4B Top screen — flushes and reversals (6) ✅ ALL VERIFIED 2026-09-23

### §4.27 M_Open_Flush_Up / §4.28 M_OpenFlush_Down ✅

**Hunts:** a stock that gapped at least 0.50 ATR at the open and is now pushing
hard in the gap direction off a very recent extreme.

```
AvgDayVolume(ALL_VENUES, 20, NO) >= 1000000
AND Last(ALL_VENUES, CURRENT, NO) >= 5
AND ( DayBar_Open(ALL_VENUES, 5, NO, '9:30') - ClosePrice(PRIMARY, P1, NO) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.50
AND (
  ( ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 1, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.20
    OR ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 1, CURRENT, NO, False) ) / Last(ALL_VENUES, CURRENT, NO) >= 0.02 )
  OR ( ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 2, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.30
    OR ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 2, CURRENT, NO, False) ) / Last(ALL_VENUES, CURRENT, NO) >= 0.03 )
  OR ( ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 3, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.40
    OR ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 3, CURRENT, NO, False) ) / Last(ALL_VENUES, CURRENT, NO) >= 0.04 )
)
AND ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 5, CURRENT, NO, False) ) >= 0.50
AND RVOL20DAYTIMEADJUSTED >= 1.5
AND NewMinuteHigh(5, NO)
AND MaxTriggersCount(MINUTES, 1, 1)
```

Down side mirrors exactly: `ClosePrice(P1) − DayBar_Open`, displacement from
window **highs**, `NewMinuteLow(5, NO)`.

**⭐ `DayBar_Open` WORKS.** Both filters are `SET_RUNNING` in the 09-16 and 09-21
logs. The subscription exception that got this token banned is either fixed or
was always context-dependent. **This master previously listed both as FAULTED —
that was wrong.**

Consequences: the `DayLow(CURRENT) >= ClosePrice(P1)` proxy in §4.23 can be
replaced with a real open test, and the `(Last − ClosePrice(P1))/ATR >= 1.0`
workaround in §4.25 can revert to the original open-versus-prior-close form.

**⚠️ No time gate despite "Open" in the name.** The gap clause compares
`DayBar_Open` to the prior close — permanent state, true all day. So it runs on
any gapper from 9:30 to the close, which is why IOT and DOCU fired repeatedly
from 10:05 through 15:55 in the 09-05 panels. Fix: a `TickTime <= 600` ceiling.

**⚠️ OR between the ATR and percent legs within each tier** — the known-bad
pattern. High_ATR was deliberately changed OR → AND because OR let VLO's $3 chop
through on the ATR leg while DFNS's cent-level wiggles passed on percent.

**New intervals:** 1, 2 and 3-minute windows.

---

### §4.29 Blowoff_Reversal_Up / §4.30 Blowoff_Reversal_Down ✅

**Hunts:** a stock up (or down) 3% on the day, beyond the VWAP band, still
extending.

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 1000000
AND Last(ALL_VENUES, CURRENT, NO) >= 10
AND ( Last(ALL_VENUES, CURRENT, NO) - ClosePrice(PRIMARY, P1, NO) ) / ClosePrice(PRIMARY, P1, NO) >= 0.03
AND Last(ALL_VENUES, CURRENT, NO) >= VWAP(CURRENT, NO, VWAP_UpperBand, V2, PRIMARY)
AND ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 5, CURRENT, NO, False) ) >= 0.50
AND (
  ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 5, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.15
  OR ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 10, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.30
  OR ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.50
  OR ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 30, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 1.0
)
AND MaxTriggersCount(MINUTES, 3, 1)
```

Down side uses `VWAP_LowerBand` and measures from window highs.

**⚠️ LOAD-BEARING ON A BROKEN FIELD.** The displacement tiers are very loose —
0.15 ATR over five minutes, against 0.50 in the sharp-move family — so
selectivity rests almost entirely on the VWAP band clause. The bands are
documented as visibly wrong with an unknown formula, and this pair has been
parked pending a developer answer since July. It is running and firing anyway.

**⚠️ No trigger clause, 3-minute throttle, fires after hours.** The 09-05 panels
show HUT at 19:22:47, 19:27:10, 19:31:24, 19:35:28, 19:39:22, 19:42:38,
19:45:59, 19:49:08, 19:52:46, 19:56:04 — every three to four minutes with prices
barely moving. That is one state re-firing on its throttle, not ten signals.
Untested: whether `Last(..., NO)` freezes at 16:00 or keeps updating on
after-hours prints.

**⭐ The one thing it gets right, and nothing else does: LINEAR tier scaling.**
0.15 / 0.30 / 0.50 / 1.0 across 5 / 10 / 15 / 30 minutes works out to roughly
0.030 / 0.030 / 0.033 / 0.033 ATR per minute. Every other tiered filter uses
square-root-ish scaling where the longest tier is far easier per minute
(M_Drive's 30-minute branch is 3.5× easier than its 5-minute). Because all four
tiers demand the same *rate*, the OR acts as a **partial workaround for
clock-aligned windows** — a move is caught whichever boundary it straddles.

---

### §4.31 Quite_Reversal_Up / §4.32 Quiet_Reversal_Down ✅

*(Name misspelled in Primus on the up side only.)*

**Hunts:** exhaustion at the extreme — extended on the day, beyond yesterday's
high (or low), overbought (or oversold), **and volume drying up**.

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 1000000
AND Last(ALL_VENUES, CURRENT, NO) >= 10
AND ( Last(ALL_VENUES, CURRENT, NO) - ClosePrice(PRIMARY, P1, NO) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 1.25
AND ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 60, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 1.0
AND MinuteVolume(ALL_VENUES, 15, CURRENT, NO) < MinuteVolume(ALL_VENUES, 15, P1, NO)
AND MinuteHigh(ALL_VENUES, 5, CURRENT, NO, False) >= DayHigh(ALL_VENUES, 1, P1, NO)
AND RSIStock(ALL_VENUES, NO, MINUTES_5, 14, CURRENT) >= 70
AND MaxTriggersCount(MINUTES, 3, 1)
```

Down side mirrors exactly, including the volume clause.

**⭐ The only volume-CONTRACTION clause in ~40 filters.** Everything else demands
expansion. This is the one genuine volume-divergence test in the set.

**⚠️ AND THE FORMING-BAR PROBLEM INVERTS IT.** `MinuteVolume(15, CURRENT)`
accumulates from zero at each quarter-hour boundary while `MinuteVolume(15, P1)`
is a complete block, so the comparison is **trivially true for the first several
minutes of every 15-minute block** and only becomes meaningful near the end of
one. The clause tests "we are early in the quarter hour" far more than it tests
"volume is drying up." With no trigger and a 3-minute throttle, fires cluster at
9:30-9:35, 9:45-9:50, 10:00-10:05 and so on.

**Fix — one line per side, matching the hold family's convention:**
```
AND MinuteVolume(ALL_VENUES, 15, P1, NO) < MinuteVolume(ALL_VENUES, 15, P2, NO)
```

**New interval:** 60 minutes. Second use of `DayHigh(ALL_VENUES, 1, P1, NO)`
(prior day's high) after M3/M4.

---

## §4C The Contra group (14) ✅ ALL VERIFIED 2026-09-23

Retained rather than retired. **The answer to "what do these catch that the
rebuilt family misses" is: extension and exhaustion.** Every filter in the
current generation is momentum-continuation or sharp-move. Nothing in it fades
an extended stock or measures acceleration. Retiring the Contras deletes that
function entirely.

### Two generations inside the group

**A/B/C generation — the later rewrite, clean.**
Contra_Short_A_FAST · B_Extended · C_Breakdown
Contra_Cap_A_Fast · B_Extended · Contra_Reclaim_C

Exact mirrors, every threshold matched. No `MinuteClose_I`, no hand-rolled RVOL,
no bare `LAST`. Structure is A = accelerating move, B = overbought/oversold
stretch, C = the break or reclaim after extension. Short side fades strength,
Cap side fades weakness.

> **Naming trap:** the long-side C is `Contra_Reclaim_C`, not
> `Contra_Cap_C_Reclaim`, so the six-filter structure is invisible from the
> filter list.

**V generation — older, rough.**
Contra Long/Short V5 · V8 · Parabolic · All · the bare Long/Short · two VWAP
Divergence filters

Approximate mirrors with drifting constants, hand-rolled RVOL, `MinuteClose_I`
throughout, bare `LAST`, unnormalized dollar thresholds.

### ⭐ Two dead filters

**Momo VWAP Divergence Long** — contains both `Last − VWAP <= -0.75` **and**
`Last − VWAP >= 0.75`. Logically unsatisfiable. **Reports `SET_RUNNING`.** Has
never fired since it was written. Likely intent: distance from VWAP of at least
0.75 in *either* direction, written as two bounds and joined with AND instead of
OR. Correct fix is to **delete** the `>= 0.75` line, since `Last / VWAP <= 0.97`
already forces the below-VWAP side.

**Contra Long VWAP Divergence** — contains `Last − VWAP = - 1.25`. **Equality on
a computed float.** Practically unsatisfiable. Reads like an attempted fix of the
above that lost its `<`. Fix: `<= -1.25`.

Both hold the best-constructed magnitude measure in the whole collection:

```
( Last − BB_Lower ) / ( SMA20 − BB_Lower ) <= 0.10
```

Bottom tenth of the lower half-band — scale-free across any price. **Never
evaluated.**

Once fixed, Contra Long VWAP Divergence is the *cleanest* filter in the group:
no `MinuteClose_I`, no hand-rolled RVOL, four clauses (down 60c+ over thirty
minutes, ≥3% below VWAP, ≥$1.25 below VWAP, bottom tenth of the lower band).

### ⭐ Systematic trigger asymmetry — the most actionable finding

| Filter | Trigger |
|---|---|
| Contra Short V5 | `NewMinuteHigh(10, NO)` |
| Contra Short V8 | `NewMinuteHigh(10, NO)` |
| Contra Long V5 | **none** |
| Contra Long V8 | `( NewMinuteHigh(5) OR NewMinuteLow(5) )` — near-inert |

Both short V-filters are properly gated; neither long one is. **The long Contra
list should flood while the short list stays controlled**, purely from the
missing event gate.

**Fix:** add `NewMinuteLow(10, NO)` to Contra Long V5, and replace Long V8's
two-sided pair with the same.

> The two-sided `NewMinuteHigh(5) OR NewMinuteLow(5)` is satisfiable almost
> continuously — within any forming 5-minute bar, new highs and lows occur
> constantly. It looks like an event gate and functions as almost none.

### Other defects

**Hand-rolled RVOL, divide-by-zero at 9:30.**
```
1.0 * DayVolume / ( AvgDayVolume(20) * ( TickTime - 570.0 ) / 390 ) > 1.5
```
At 9:30 exactly `TickTime − 570 = 0`. At 9:31 the denominator is 1/390 of a day,
so the ratio explodes and the gate passes trivially for the first several
minutes. Also uses a **linear** time curve where `RVOL20DAYTIMEADJUSTED` uses a
non-linear accumulation curve, so these filters' volume tests match no other
filter's.

**Rule 0 violations.** `MinuteClose_I` appears throughout the V generation — the
6-minute close and the four consecutive red/green bar tests. The July session
established it is prohibited in any form and fixed it in eight filters; these
were missed.

**"All" is not a union of the standalones.** Its branches use `DayVolume >= 750000`
where the standalones use `500000`, so All is **stricter**. Running both means
the standalones fire on names All misses. All also uses
`MaxEntriesTriggersCount(HOURS, 8, 1)` — once per session per name — where the
standalones use `MaxTriggersCount(MINUTES, 1, 1)`.

**Contra Long All branches 2 and 3** carry positive thresholds where their
siblings use negative (`Last − VWAP <= 0.75`; `(Last − MinuteOpen_I(6,P5)) <= .50`).
Branch 2's form also appears in Contra Long Parabolic, so it is probably
deliberate — the fast-drop variant where the 5%-in-30-minutes clause carries the
signal and the VWAP test is a loose ceiling. Branch 3 has no sibling
confirmation and remains suspect, though with RSI ≤ 25 and four red bars
required it excludes essentially nothing either way.

**Contra Short All is the messiest expression in the collection.**
- Branch 5 opens with `(` and is never closed — balance ends at +1
- No outer wrapper, so `MaxTriggersCount` and `LAST>5` sit inside branch 5 only;
  **branches 1-4 have no price floor and no per-minute throttle**
- `> 0.75AND` — no space between literal and keyword, in branches 2 and 3
- Branch 2 carries `MaxEntriesTriggersCount` **twice**
- Branch 4's Bollinger clause uses the *lower*-half formula with `>= 0.9`. The
  long side uses `<= 0.10`, a tight bottom-tenth test. The formula was never
  re-anchored to the upper half — only the comparison flipped — so the short
  side has a loose floor where the long side has a tight test

**Near-duplicates.** Contra Short and Contra Short Parabolic share an identical
six-clause structure; Parabolic is looser on four of five constants and stricter
only on volume (750k vs 500k). Above 750k, Parabolic fires on a superset of
Contra Short's names. Prime consolidation candidate.

**Naming inconsistency across sides.** "Contra Long Parabolic" is the RSI plus
5%-in-thirty-minutes variant; "Contra Short Parabolic" is a five-minute
fast-move filter with no RSI, no bar chain and no percent test. Same word,
different constructions.

**Six filters run with no trigger clause at all** — Contra Long V5, the bare
Contra Long and Contra Short, Contra Short Parabolic, plus both Blowoff and both
Quiet reversals. Pure state on a throttle means continuous re-firing while
conditions hold. **This is the single largest source of list repetition.**

### ⭐⭐ Techniques already present that solve known problems

**Hand-rolled ROLLING window** (unnamed short-side divergence filter):
```
( MinuteVolume_I(1,P5,YES) + MinuteVolume_I(1,P4,YES) + MinuteVolume_I(1,P3,NO)
+ MinuteVolume_I(1,P2,YES) + MinuteVolume_I(1,P1,YES) + MinuteVolume_I(1,CURRENT,YES) ) * 24
/ ( 1.0 * MinuteVolume(120, CURRENT, YES) ) > 1.5
```
Summing N consecutive 1-minute bars back from `CURRENT` **is a rolling window** —
the workaround for Primus's clock-aligned limitation, the defect blamed for
roughly one spike in five being split across two measurements. It could be
applied to the sharp-move displacement clauses today, without waiting for a new
scanner.

> Two bugs in that clause as written: six bars multiplied by **24** gives a
> 144-minute equivalent against a 120-minute denominator (should be **20**), so
> the stated 1.5 threshold behaves as about 1.25. And five terms use `YES` while
> **P3 uses `NO`** — harmless in regular hours, understates the sum by a sixth
> premarket.

**Percent-AND-dollar floors** (Contra Long/Short V8): `Last/VWAP < .98` **and**
`Last − VWAP <= -0.75` together enforce whichever is stricter at any price — the
crossover is $37.50. This is the pattern the sharp-move family took weeks to
arrive at. It was already here.

**Linear tier scaling** (Blowoff_Reversal) — see §4.29.

**Volume contraction** (Quiet_Reversal) — see §4.31.

### New primitives from this group

| Token | Note |
|---|---|
| `RSIStock(ALL_VENUES, NO, MINUTES_5, 14, CURRENT)` | first RSI anywhere |
| `BollingerBands(CURRENT, MINUTES_5, 20, ALL_VENUES, NO, LowerBand/UpperBand, V2)` | first Bollinger |
| `SMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_20, CURRENT)` | first simple MA; all else uses `EMAStock` |
| `MaxEntriesTriggersCount(HOURS, 8, 1)` | distinct from `MaxTriggersCount` |
| `MinuteVolume_I(..., YES/NO)` | explicit extended-hours flag |
| `Vol5m` | bare custom-field token, no arguments |
| `LAST>5` / `Last >5` | **three spellings** of the bare token across the group |
| `P5` | deepest bar index; `P4` also used |

---

## §4A Cross-filter consistency — deliberate or drift?

### The two generations

The single most useful finding from the 2026-09-05 collection: the filters do
**not** show uniform drift. They divide cleanly into two generations.

**Current generation** — two-band ATR, percent floors, burst volume leg, no ELVA:
sharp-move six · M_Spike_Hold pair · Breakout_Hold pair · Base_Break pair ·
High_ATR pair · HOD_Base_Break_LQ · LowFloat_HOD_Break_Up

**Previous generation** — pure ATR, no percent floors, no burst leg, ELVA present:
**M_Drive pair · M_Run_Hold pair**

That is four filters to bring forward or retire, not forty to triage.

**The M-series is its own thing** and better built than expected — a coherent
large-cap suite in three pairs covering setups the runner filters don't touch:

| Pair | Setup |
|---|---|
| M3 / M4 | Breakout above **yesterday's** high, with a VWAP extension cap |
| M5 / M6 | Pullback to EMA7 and resumption, 10:00–15:20 only |
| M7 / M8 | Orderly trend structure, **no magnitude test at all** |

### Parameter comparison

| Parameter | Spike | Sensitive | Med/EXT | M_Drive | High_ATR | M_Run_Hold | M3/M4 |
|---|---|---|---|---|---|---|---|
| DayVolume | 150k | 250k | 250k | 250k | 1M | 250k | ADV ≥ 2M |
| Burst leg | 2.0 | 3.0 | 3.0 | none | none | none | none |
| 5-min percent | 0.020 | 0.025 | 0.050 | none | 0.02 | none | 0.01 |
| Trigger window | 15 | 15 | 15 | **5** | 15 | none | 5 |
| Two-band ATR | yes | yes | yes | **no** | n/a (AND) | **no** | n/a |
| ELVA clause | no | no | no | **yes** | no | **yes** | **yes** |
| Throttle | 1 min | 1 min | 1 min | 1 min | 1 min | 5 min | 1 min |

### Open questions

1. **Spike is looser than Pop** on both burst (2.0 vs 3.0) and percent (2.0% vs
   2.5%), despite also carrying day-direction *and* day-extreme gates. Plausibly
   deliberate — those gates narrow it a lot — but it should be a choice.
2. **Nesting inversion.** Spike runs `DayVolume >= 150000`, Sensitive `250000`.
   Since Spike is otherwise a strict subset, a stock between 150k and 250k could
   fire Spike but not Sensitive. Academic in practice, but inverted.
3. **M_Drive and M_Run_Hold are a generation behind** on the same five counts.
   Bring forward or retire — M_Drive overlaps Sensitive heavily as it stands.
4. **High_ATR has no burst leg**, and watches precisely the mega-caps that move
   late in the session (CRM, WDAY 14:30; PYPL 15:44).
5. **ELVA survives in four filters** — M_Drive pair, M_Run_Hold pair,
   Breakout_Hold pair, M3/M4. Peter independently identified it as
   self-defeating on trending days; it was removed from the six for that reason.

### Clauses worth propagating

Two good ideas currently trapped in single filters:

**VWAP extension cap** (M3/M4) — `Last <= VWAP + 1.0 × ATR`. The only anti-chase
guard in the set. Directly addresses the late-fire complaint.

**Break margin** (HOD_Base_Break_LQ) — `Last >= level + 0.15 × ATR`. The only
genuine clear-the-level test. Base_Break and Breakout_Hold fire on the touch.

### Forming-bar clauses to audit

Clauses measuring a **forming** bar's *range* pass trivially right after a
boundary, because the range starts near zero:

- M_Run_Hold pair — first coil clause uses `MinuteHigh(1,CURRENT) − MinuteLow(1,CURRENT)`
- M3/M4, M5/M6, HOD_Base_Break_LQ — volume compares forming bar to completed
  prior bar while `NewMinuteHigh(5)` fires at the break instant; the two race

Forming-bar **extremes** (`MinuteHigh(5,CURRENT)` as a running maximum) are safe.
It is forming-bar **ranges** that are the problem.

## §5 Statics and premarket lists (13) ✅ ALL VERIFIED 2026-09-23

### §5.1 Movers_Up_Alist / §5.2 Movers_Down_Alist

```
Last(ALL_VENUES, CURRENT, YES) >= 5
AND Last(ALL_VENUES, CURRENT, YES) < 2500
AND AvgDayVolume(ALL_VENUES, 20, NO) >= 500000
AND ( RVOL20DAYTIMEADJUSTED >= 1.5 OR ( TickTime <= 570 AND PM_RVOL >= 2.0 ) )
AND ( Last(ALL_VENUES, CURRENT, YES) - ClosePrice(PRIMARY, P1, NO) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.40
AND ( Last(ALL_VENUES, CURRENT, YES) - ClosePrice(PRIMARY, P1, NO) ) >= Last(ALL_VENUES, CURRENT, YES) * 0.0075
```
Down side mirrors exactly.

**⚠️ THE OVERFLOW FILTER.** Movers_Up_Alist hit the result counter at **102,686**
on 2026-09-21 and was disabled at 13:30.

**Cause: 0.75% on the day is not a move.** The two magnitude clauses are also
near-equivalent — for a stock whose ATR is 2% of price, `0.40 ATR` *is* 0.8% of
price — so neither does independent work. The effective bar is roughly
`max(0.75%, 0.40 × ATR-percent)`, landing around 0.8–1.5% for most liquid names.
Against a universe of every US equity above 500k ADV, that admits several hundred
symbols at once on any decent day.

For scale: LowFloat_Movers_List uses 10%. The sharp-move family uses 2–2.5%
**over five minutes**. An "A-list" at 0.75% on the *day* is not an A-list.

**Fix: raise to 2%.** Not 3% — see the RNK note in §2. Because `RNK_UP`/`RNK_DWN`
sort by the same measure the list gates on, the list self-prioritises and several
hundred members is a *counter* problem, not a readability one. A 3–5× membership
cut leaves comfortable headroom on a four-hour run to 102,686.

**The down side carries identical thresholds** and has not tripped only because
down days are rarer. On a broad selloff it behaves the same — raise both.

Notable: an **upper** price bound at $2500 (only M3/M4 also have one). `Last(YES)`
so premarket counts, matching the `PM_RVOL` branch.

---

### §5.3 LowFloat_Movers_List ✅ current as of 2026-09-23

```
FloatShares <= 20
AND AdjustedClosePrice(P1) >= .25
AND AdjustedClosePrice(P1) <= 50
AND AvgDayVolume(ALL_VENUES, 20, NO) >= 5000
AND ( RVOL20DAYTIMEADJUSTED >= 2.0 OR ( TickTime <= 570 AND PM_RVOL >= 3.0 ) )
AND ( Last(ALL_VENUES, CURRENT, YES) - ClosePrice(PRIMARY, P1, NO) ) / ClosePrice(PRIMARY, P1, NO) >= 0.10
AND DayVolume(ALL_VENUES, 1, CURRENT, YES) >= 250000
```

**⭐ THE KEY STRUCTURAL FIX, AND THE RIGHT ONE.** The `AvgDayVolume` floor dropped
from 250,000 to **5,000** — effectively inert, a sanity gate — and was replaced by
a **today's-volume** floor of 250k using the `YES` flag. That directly solves the
WFF problem: a 250k *ADV* gate excluded exactly the dormant low-floats that
produce 400%+ days. A stock that normally trades nothing now qualifies provided
it is genuinely trading today, premarket included.

**This is the clause to propagate** — see §4.21.

Single-direction (up only) by design. No near-high or range clause, correct for a
movers list rather than a breakout list.

> **A Claude error, corrected.** On 2026-09-16 I proposed that the bare decimal
> `.25` without a leading zero was causing this filter's load fault. **Disproven**
> — it still contains `.25` and runs. The Contra filters are full of bare decimals
> (`-.40`, `-.50`, `.50`, `.65`) and all load. The cause of that double fault
> remains unknown.

---

### §5.4 PM_Coil_Up / §5.5 PM_Coil_Down ⭐ best-constructed filter in the collection

```
Last(ALL_VENUES, CURRENT, YES) >= 5
AND Last(ALL_VENUES, CURRENT, YES) < 2500
AND AvgDayVolume(ALL_VENUES, 20, NO) >= 1000000
AND PremarketDayVolume(CURRENT) >= 100000
AND PM_RVOL >= 3.0
AND ( Last(ALL_VENUES, CURRENT, YES) - ClosePrice(PRIMARY, P1, NO) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 1.0
AND ( Last(ALL_VENUES, CURRENT, YES) - ClosePrice(PRIMARY, P1, NO) ) >= Last(ALL_VENUES, CURRENT, YES) * 0.03
AND ( PremarketDayHigh(CURRENT) - PremarketDayLow(CURRENT) ) / ClosePrice(PRIMARY, P1, NO) >= 0.01
AND Last(ALL_VENUES, CURRENT, YES) >= VWAP(CURRENT, YES, VWAP, V1, ALL_VENUES)
AND Last(ALL_VENUES, CURRENT, YES) >= PremarketDayHigh(CURRENT) - 0.25 * ( PremarketDayHigh(CURRENT) - PremarketDayLow(CURRENT) )
AND ( MinuteHigh_I(ALL_VENUES, 1, CURRENT, YES, False) - MinuteLow_I(ALL_VENUES, 1, CURRENT, YES, False) ) / ( PremarketDayHigh(CURRENT) - PremarketDayLow(CURRENT) ) <= 0.15
AND ( MinuteHigh_I(ALL_VENUES, 1, P1, YES, False) - MinuteLow_I(ALL_VENUES, 1, P1, YES, False) ) / ( PremarketDayHigh(CURRENT) - PremarketDayLow(CURRENT) ) <= 0.15
AND MaxTriggersCount(MINUTES, 5, 1)
```

**Three things it gets right that nothing else does:**

**Dual floors applied consistently** — the gap needs `1.0 ATR` **and** `3% of
price`; premarket volume needs `100k shares` **and** `PM_RVOL ≥ 3.0`. Absolute
and relative on both.

**Coil tightness normalised to the premarket range**, not ATR. Correct scale —
premarket ranges are far smaller than daily ATR, so measuring against ATR would
call everything tight.

**A genuine divide-by-zero guard** — the `premarket range ≥ 1% of prior close`
clause protects the normalisation below it. Only such guard in any filter.

**⚠️ Forming-bar range** in the first coil clause (`MinuteHigh_I(1,CURRENT) −
MinuteLow_I(1,CURRENT)` starts near zero each minute, so it is trivially true
early). The `P1` version does the real work. **Fourth filter with this defect** —
see §4A.

**⚠️ No `TickTime` gate on a premarket filter.** `PremarketDayHigh/Low/Volume`
freeze at 9:30 while `Last` keeps updating, so after the open this silently
becomes "gapped up, above the premarket high, with quiet 1-minute bars" measured
against frozen references — and can fire all day. Fix: `TickTime <= 570`.

---

### §5.6 PM_LowFloat_Coil_Up ⚠️ float clause is inert

Same structure as §5.4 with `AdjustedClosePrice(P1)` $1–50, `PM_RVOL >= 5.0`, a
**15%** gap (percent-only, no ATR clause), and a premarket range ≥5% of prior
close. **No `AvgDayVolume` floor at all** — deliberate and correct, matching the
WFF lesson.

**⚠️ `Float <= 10000000` RESTRICTS NOTHING.** Primus expresses float in
**millions** — `Float <= 10` in §5.3 is the working 10-million gate. So this reads
as 10 *trillion* shares. **The filter has no float gate despite its name.**

This is the identical bug found in HOD_Base_Break_LF on 2026-08-13, which was
fixed by switching to `FloatShares <= 10`. **This filter never got the same fix**
— it is the only place the broken form survives.

**Fix: `Float <= 10`.**

It still works in practice because the $1–50 band, `PM_RVOL >= 5.0` and the 15%
gap select small caps indirectly — but by proxy, so genuinely low-float names are
not preferred over merely cheap ones.

---

### §5.7–§5.10 Imbalance statics

```
ImbalanceSellVolume(NYSE) >= AvgDayVolume(ALL_VENUES, 20, NO) * 0.03
```
All four are this single clause, parameterised only by venue (`NYSE` / `NASDAQ`)
and side (`Buy` / `Sell`). The `IN$` in the Nasdaq column header is display
formatting, not a different field — logic is share-based on both venues.

**Sound construction:** scaling to ADV makes 3% mean the same thing on a 500k-ADV
stock and a 50M one — the normalisation principle the sharp-move family took weeks
to reach.

**Gaps:** no absolute share floor (a 50k-ADV stock qualifies on 1,500 shares) and
no price floor. An absolute minimum ~25,000 shares would clear the noise.

**Time-windowed by data availability, not by a clause** — NYSE publishes closing
imbalances from ~15:50, so these sit empty most of the session.

The filter is a gate; the information is in the columns — `Imb_*_Ratio`,
`Clear_ATR` / `Near_ATR` (§2).

---

### §5.11 Halted Static New Minute

```
IsHalt AND DayVolume(ALL_VENUES, 1, CURRENT, NO) > 0
```
`IsHalt` is a bare boolean. The volume clause excludes names halted before ever
trading today — sensible, since a stock halted pre-open with no volume is not
actionable.

Displayed columns `ClearPrice` and `NearPricePri` are the valuable part: the halt
auction clearing price (0.00 until published) and the indicative price. **Directly
relevant** — WFF, PFSA and FGI all halted, and knowing where a halted runner will
reopen is worth more than knowing it halted.

⬜ The plain `Halted` static was not collected; the "New Minute" suffix may be a
refresh setting rather than an expression difference.

---

### §5.12 NYSE IND UP / §5.13 NYSE IND DOWN

```
TickTime < 720
AND ( ImbalanceBuyVolume(NYSE) > 0 OR ImbalanceSellVolume(NYSE) > 0 )
AND IndBid > AdjustedClosePrice(P1)
AND IndBid > 0
```
Down side flips the comparison to `<`.

**The `TickTime < 720` bound is noon, not 9:30** — almost certainly deliberate.
NYSE publishes indicative prices for **halt-resumption** auctions as well as the
opening auction, so a noon ceiling covers both. Given how often low-float runners
halt, that is the right design.

**⭐ Uses `AdjustedClosePrice(P1)`, not `ClosePrice(PRIMARY, P1, NO)`** — the
split-safe comparison, since the indicative bid is quoted in today's post-split
terms. **This filter handles corporate actions correctly where the PFSA case
showed others do not.**

**⭐ `IndBid > 0` is the missing-data guard** that `Clear_ATR` and `Near_ATR`
lack. The pattern already exists here — applying it there is a consistency fix,
not a new idea.

The imbalance clause is an OR across both sides: it only requires that *some*
imbalance exists, with direction taken from `IndBid` versus the prior close.
Correct, since the indication carries the direction.

**Neither filter uses `IndAsk`** — both judge on the bid alone. `IndAsk` appears
only as a displayed column. Internally consistent, and the bid is the firmer
number.

⬜ Open: whether `IndBid` zeroes after the opening auction or holds a stale value.
A stale value would leave dead names listed until noon.

---

## §6 Regression set

Every proposed change is checked against these **before** deployment.

### Must catch

| Symbol | Date | Shape | Notes |
|---|---|---|---|
| **SNDK** | 09-04 | Spike + perfect hold | Fired 09:39 @ 1640.17, closed 1740. Retrace 8.68 — lowest in the log. The archetype. |
| **BTSG** | 07-31 | Spike down, hold, continue | Fired 65.45 @ 9:37, ran to 60.32 |
| **CMG** | 08-04 | Midday news spike, no retrace | Wants a fire in the 36s; got 34.88 |
| **MQ** | 08-04 | Spike, 2-min hold, continue | Classic shape, midday |
| **EXLS** | 07-29 | Gap-and-go stair-step | Fired 33.08, ran to 35 |
| **NRG** | 08-04 | Sustained collapse, still active | 1-min bar $1.77 on $126 = still moving |
| **AMBA** | 07-31 | 12:30 M&A spike | Counterexample to any time gate |
| **WFF** | 08-17 | Low-float parabolic, +469% | No base — needs the no-base runner filter |
| **FGI** | 08-13 | Low-float runner, +161% | Halted on circuit breaker |
| **PFSA** | 08-18 | Low-float, +449% | Reverse split — ADV distortion case |

### Must reject

| Symbol | Date | Why |
|---|---|---|
| PG / XOM / KO | — | Low-ATR clog; 0.7% moves reading as significant |
| VLO | 08-04 | 1% chop on a $307 stock |
| IDXX / HSIC | 08-04 | Gap-up fades, still **up** on the day, on a down filter |
| WGS | 08-04 | Move already over — 1-min range 0.38% of price |
| DFNS | 08-04 | ATR 31% of price; percent legs become noise floor |
| TSEM | 08-04 | Steady grind, not a spike |
| APP | 07-31 | Fired on the first bar, no hold |
| PFG | 09-03 | Broke out at 9:35, fired at 2:20 while dead |

### Discriminators derived from these
- **NRG vs WGS** — both down, both near the low. Difference is the last
  completed 1-minute bar: 1.4% of price vs 0.38%. *Still moving* vs *over*.
- **SNDK vs AMAT** — same day, same filter. Retrace 8.68 vs 87.93/43.68. The
  measurement separates them before any outcome data exists.

---

## §7 Fix queue

Collection is complete. This replaces the checklist.

### Tier 1 — revive things that produce nothing

| # | Fix | Where | Effect |
|---|---|---|---|
| 1 | Delete the `>= 0.75` line | Momo VWAP Divergence Long | Filter is logically unsatisfiable and has **never fired**. Reports `SET_RUNNING`. |
| 2 | `= -1.25` → `<= -1.25` | Contra Long VWAP Divergence | Equality on a computed float. Never fires. Becomes the **cleanest filter in the Contra group**. |
| 3 | `Float <= 10000000` → `Float <= 10` | PM_LowFloat_Coil_Up | Float clause currently restricts nothing. |

### Tier 2 — stop things firing continuously

| # | Fix | Where |
|---|---|---|
| 4 | Add `NewMinuteLow(10, NO)` | Contra Long V5 (has no trigger at all) |
| 5 | Replace the two-sided pair with `NewMinuteLow(10, NO)` | Contra Long V8 |
| 6 | Add `TickTime <= 600` | M_Open_Flush pair — "Open" filter with no time gate |
| 7 | Add `TickTime <= 570` | PM_Coil pair, PM_LowFloat_Coil_Up |
| 8 | Raise 0.0075 → 0.02 | Movers_Up_Alist **and** Movers_Down_Alist (counter overflow) |

### Tier 3 — correct measurements that are wrong

| # | Fix | Where |
|---|---|---|
| 9 | `MinuteVolume(15,P1) < MinuteVolume(15,P2)` | Quite_Reversal_Up, Quiet_Reversal_Down — current clause is trivially true early in every quarter-hour, inverting the filter's premise |
| 10 | Multiplier `24` → `20` | unnamed short-side divergence filter — six bars against a 120-minute denominator; 1.5 threshold behaves as 1.25 |
| 11 | P3 flag `NO` → `YES` | same filter — five of six terms use YES |
| 12 | Close branch 5's paren; move `MaxTriggersCount`/`LAST>5` to an outer wrapper | Contra Short All — branches 1-4 currently have no price floor or throttle |
| 13 | Add a divide-by-zero guard, `CURRENT` → `P1` | `VolROC_1m_3`, `VolROC3` — or retire both in favour of `VolFade_1m_Smooth` |
| 14 | Add `ClearPricePrimary > 0` / `NearPricePrimary > 0` | `Clear_ATR`, `Near_ATR` — unpublished price reads as a large negative |

### Tier 4 — structural improvements now known to be possible

| # | Change | Rationale |
|---|---|---|
| 15 | Rewrite two-band ATR tiers using `IF()` | `displacement / ATRStock >= IF( ATRStock/ClosePrice(P1) >= 0.03, 0.50, 0.80 )` — roughly **halves** each sharp-move filter and removes most paren nesting |
| 16 | Build `RVOL_Recent` from the RVOL curve's slopes | A true time-local relative-volume measure, immune to the cumulative dilution behind five missed trades. Replaces the burst-leg workaround with something interpretable |
| 17 | Apply the hand-rolled rolling window to sharp-move displacement | Summing N 1-minute bars back from `CURRENT` defeats clock-aligned splitting — the technique already exists in two of Peter's own expressions |
| 18 | Add the split-immune volume leg | `MinuteVolume(5,P1) >= DayVolume(CURRENT) * 0.03` — LowFloat_HOD_Break_Up; proposed 2026-08-18 after PFSA, never added |
| 19 | Give Base_Break_Hold the thrust block Breakout_Hold already has | Addresses the PFG cause rather than the symptom |
| 20 | Replace the `DayLow >= ClosePrice(P1)` proxy with a real `DayBar_Open` test | §4.23 — the proxy was chosen only because `DayBar_Open` was believed broken |
| 21 | Normalise field scales | 17 fields use `× 100`, 4 do not; `ATR_POS` shows 50 where `VWAP_Stretch` shows 0.5 for the same magnitude |

### Deliberate decisions, not fixes

- **Contra group retained.** It catches extension and exhaustion — a function
  absent from every filter in the current generation.
- **Breakout_Hold left unchanged** despite diagnosis (§4.25).
- **M_Drive and M_Run_Hold** remain a generation behind (§4A). Bring forward or
  retire; they overlap the current family heavily.

## §8 Open questions

**For the vendor**
- What does the trailing `False` argument on the Minute functions control?
- Are `MinuteVolume(120, …)` windows rolling or clock-aligned?
- How are `VWAP_UpperBand` / `LowerBand` calculated?
- Does the Static result counter reset daily or accumulate?

**Determinable in-house**
- `RETRACE_PCT_UP` values don't match charts (AMAT 09-04, reads 87.93 on a clean
  ramp). Either the timestamp is the bar start rather than the fire instant, or
  the field formula differs from spec. **Blocks all threshold work.**
- `SPIKE_VOL_PCT` returns 0 — integer division.
- Does `FloatShares` populate for foreign small-cap issuers?

**Design decisions pending**
- Retrace bound: 0.25 (too tight, late entries) vs 0.50 (proposed) vs unbounded
  (too loose — 24 of 29 fires depend on it).
- Volatility floor 0.03 vs 0.025 — currently sits where the best setups live.
- Whether M_Spike's 5-minute expiry gets fixed (MOH/RNG, unfixed since July).
- Three overlapping lists per direction, or one list with a strength column.
