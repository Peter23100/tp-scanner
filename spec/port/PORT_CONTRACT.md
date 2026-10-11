# Scanner port contract

**Input for Claude Code.** This is the implementation contract for porting
PrimusScan filters into the replacement scanner. It is deliberately separate
from `Primus_Filters_MASTER_2026-09-05.md`, which holds the rationale, the
history, and the human-readable write-ups. Read this for *what to build*; read
the master for *why*.

Companion files:
- `filters.json` — every filter as structured data, grouped into families
- `fixtures.json` — regression cases with expected outcomes

---

## 1. The rule that matters most

**Do not port expressions verbatim.** Roughly a third of them contain defects
that are catalogued and understood. Each filter in `filters.json` carries a
`port` field:

| `port` value | Meaning |
|---|---|
| `as_is` | Reproduce the semantics exactly |
| `corrected` | Apply the listed `corrections[]`; the original is preserved in `original_defects[]` for reference |
| `skip` | Do not port — superseded or dead. Reason in `notes` |

A filter marked `corrected` that is ported verbatim will reproduce a known bug.
Two of them currently **never fire at all** and report healthy.

---

## 2. Primitive contract

### 2.1 Fully specified

| Primitive | Signature | Semantics |
|---|---|---|
| `Last` | `(venue, CURRENT, ext)` | Last trade price |
| `DayVolume` | `(venue, 1, CURRENT, ext)` | Cumulative session volume |
| `DayHigh` / `DayLow` | `(venue, 1, period, ext)` | Running session extreme, **includes the current tick**. `P1` = prior day |
| `ClosePrice` | `(PRIMARY, P1, ext)` | Prior session close, **unadjusted** |
| `AdjustedClosePrice` | `(P1)` | Prior session close, **split/dividend adjusted** |
| `AvgDayVolume` | `(venue, 20, ext)` | 20-session average volume — **see §3.2, this is distorted by splits** |
| `MinuteHigh` / `MinuteLow` | `(venue, N, period, ext, bool)` | Extreme over an N-minute window. `CURRENT` = forming window (running extreme, safe). `P1`..`P5` = completed windows |
| `MinuteOpen` / `MinuteClose` | `(venue, N, period, ext)` | Bar open/close. **`MinuteClose_I` is banned in source filters (Rule 0) but appears in 8+ legacy ones** |
| `MinuteVolume` | `(venue, N, period, ext)` | Volume over an N-minute window. `CURRENT` is a **forming** bar accumulating from zero |
| `MonthHigh_I` / `MonthLow_I` | `(venue, 1, period, ext)` | Calendar-month extreme. `P1` = prior *completed* month |
| `ATRStock` | `(venue, ext, DAILY, 14, CURRENT)` | 14-day ATR — **see §3.1, stale on dormant stocks** |
| `EMAStock` / `SMAStock` | `(venue, ext, interval, PERIOD_n, CURRENT)` | Moving averages |
| `RSIStock` | `(venue, ext, MINUTES_5, 14, CURRENT)` | RSI |
| `VWAP` | `(CURRENT, ext, VWAP, V1, venue)` | Session VWAP. Centerline accurate |
| `BollingerBands` | `(CURRENT, interval, 20, venue, ext, LowerBand\|UpperBand, V2)` | |
| `PremarketDayVolume/High/Low` | `(CURRENT)` | Premarket session values. **Freeze at 09:30** while `Last` keeps moving |
| `ImbalanceBuyVolume` / `ImbalanceSellVolume` | `(venue)` | Auction imbalance shares |
| `FloatShares` / `Float` | bare comparison | **Denominated in MILLIONS.** `<= 10` means 10,000,000 shares |
| `TickTime` | bare | Minutes from midnight. 570 = 09:30, 720 = 12:00, 960 = 16:00 |
| `IsHalt` | bare boolean | |
| `ClearPricePrimary` / `NearPricePrimary` | bare | Auction clearing / indicative price. **May be 0 when unpublished** |
| `IndBid` / `IndAsk` | bare | Auction indicative quote |
| `NewMinuteHigh` / `NewMinuteLow` | `(N, ext)` | Event: a new N-minute extreme at this instant |
| `MaxTriggersCount` | `(unit, n, count)` | Rate limit per symbol |
| `MaxEntriesTriggersCount` | `(HOURS, 8, 1)` | Distinct from the above — once per session per symbol |
| `IF` | `(cond, then, else)` | **Conditional. Nests at least 8 deep.** |

**Venue enum:** `ALL_VENUES`, `PRIMARY`, `NYSE`, `NASDAQ`, `ANY`.
⬜ *Open: is `ANY` a synonym for `ALL_VENUES`?*

**Extended-hours flag** (`YES`/`NO`) appears on most price and volume
primitives. `NO` excludes pre/post market.

**Bar indices:** `CURRENT`, `P1`…`P5`. **Intervals in use:** 1, 2, 3, 5, 6, 10,
15, 30, 60, 120 minutes. The aggregation layer must support all ten.

### 2.2 Undocumented — cannot be ported faithfully

These have never been explained by the vendor. For each, implement what the
*intent* appears to be and flag the assumption in code.

| Primitive | Problem |
|---|---|
| trailing `False` argument on `MinuteHigh`/`MinuteLow` | Meaning unknown. Present on most calls |
| `_I` suffix (`MinuteHigh_I` vs `MinuteHigh`) | Distinction unknown. **Both forms appear inside a single filter** (Contra_Short_A_FAST) |
| `VWAP_UpperBand` / `VWAP_LowerBand` | Formula unknown and bands are visibly wrong. Blowoff_Reversal depends on them for nearly all its selectivity |
| `MinuteVolume(120, …)` | Rolling or clock-aligned? Unknown |
| Result counter | Resets daily or accumulates? One static was disabled at 102,686 |

### 2.3 Engine semantics

**Evaluation is effectively continuous** (per-tick or near it) — *not* 20–30s
snapshots as earlier notes claimed. Evidence: across 98 fires from a
displacement filter, timestamp seconds span the whole 0–59 range with only 4 at
exactly `:00`. Instant-true conditions are **not** missed by sampling.

**Windows are clock-aligned, not rolling.** `MinuteHigh(30, CURRENT)` at 10:01
contains one minute of data. **A move spanning a boundary is measured as two
smaller moves — roughly one spike in five.**

> **The replacement should use rolling windows.** This is the single highest-value
> structural difference. Two of Peter's own filters already hand-roll them by
> summing N consecutive 1-minute bars back from `CURRENT` — see
> `VolFade_1m_Smooth` and the unnamed short-side divergence filter.

**Filters can fault mid-session** after repeated exceptions and are then silently
disabled. A disabled filter and a quiet market look identical.

> **Requirement: loud failure.** Any expression that faults must surface
> immediately and visibly.

**Load-time static checks are required.** Two filters in this set are
unsatisfiable and report `SET_RUNNING`:
- contradictory bounds on the same term (`X <= -0.75 AND X >= 0.75`)
- exact equality on a computed float (`X = -1.25`)

Both are trivially detectable. Not detecting them cost months of silent
non-production.

---

## 3. The stale-reference problem — read before porting any threshold

Two reference values are measured over a historical window and are **wrong
exactly when a stock does something interesting**. Both distortions hit the same
population: low-float runners, post-split names, stocks waking from dormancy —
which is the population most of these filters exist to find.

### 3.1 `ATRStock` is stale on a dormant stock

A stock that sat flat for months has a tiny ATR, so any `k × ATRStock` clause is
correspondingly tiny.

*Documented:* `Last >= DayHigh − 0.15 × ATRStock` allowed **2–3 cents** on MSGY
(flat 1.73–2.05 for months) and ELPW — demanding price be essentially exactly at
the day high. Also MU showing an $85 ATR and CRWD $53 on a $188 stock.

### 3.2 `AvgDayVolume` is inflated by reverse splits

When historical volumes are not adjusted alongside prices, ADV is wrong by the
split ratio.

*Documented:* PFSA's 1-for-4 split inflated ADV ~4×, making every ADV-normalised
clause ~4× too strict on a name that had just run +449%. ELPW shows a 54,972
print on its daily chart — the same artefact.

### 3.3 The fix pattern

**Normalise to same-session data.** Proven in `LowFloat_HOD_Break_Up` (rebuilt
2026-09-25):

| Instead of | Use |
|---|---|
| `k × ATRStock` for position tolerance | `k × ( DayHigh − DayLow )` |
| `AvgDayVolume × k` for bar volume | `DayVolume(CURRENT) × k` |

Neither can be distorted by a corporate action or a quiet trailing period.

**In the new scanner, prefer same-session normalisation by default** and treat
historical references as opt-in where genuinely needed.

---

## 4. Systematic defects to avoid reproducing

These recur across many filters. The spec files flag them per filter, but
understand the classes:

### 4.1 Forming-bar **range** clauses
A clause measuring the *range* of a forming bar — `MinuteHigh(1,CURRENT) −
MinuteLow(1,CURRENT)` — starts near zero each period and is **trivially true**
early in every bar.

*Affected:* M_Run_Hold pair, Quiet_Reversal pair, PM_Coil pair, `VolROC_1m_3`,
`VolROC3`.

Worst instance: Quiet_Reversal's volume-contraction clause
`MinuteVolume(15,CURRENT) < MinuteVolume(15,P1)` is trivially true early in every
quarter-hour — **inverting the filter's entire premise**.

> Forming-bar *extremes* (a running maximum) are safe. Forming-bar *ranges* and
> *accumulating volumes* are not.

### 4.2 Cumulative relative volume
`RVOL20DAYTIMEADJUSTED` divides cumulative session volume by a time-adjusted
expectation. A stock quiet until 2pm that then explodes still reads **below 1.0**
— a few minutes of burst cannot move a full-day ratio.

*Documented misses:* AMBA 12:30, CMG 12:45, CRM 14:30, WDAY 14:30, PYPL 15:44.

It also *over*-selects: a stock that gapped at 9:35 keeps a high reading all day
after going dead.

> **Build a time-local rate measure instead.** The curve's own segment slopes give
> the expected volume rate per minute (0.003/min in the 10:00–10:30 segment vs
> 0.00178/min in 11:30–13:00). See `custom_fields.RVOL20DAYTIMEADJUSTED` in
> `filters.json`.

### 4.3 Missing event gates
Six filters run as **pure state** on a throttle, so they re-fire continuously
while conditions hold — in one case into the evening on frozen values.

*Affected:* Contra Long V5, bare Contra Long, bare Contra Short, Contra Short
Parabolic, both Blowoff_Reversal, both Quiet_Reversal.

### 4.4 Permanent-state clauses in time-scoped filters
A clause comparing `DayBar_Open` to the prior close is true **all day**, so an
"Open Flush" filter with no `TickTime` gate runs from 09:30 to the close.

*Affected:* M_Open_Flush pair (no gate), PM_Coil trio (premarket references
freeze at 09:30 while `Last` keeps moving).

### 4.5 OR between ATR and percent legs
Lets the *easier* of two thresholds bind, which is backwards. High_ATR was
deliberately changed OR → AND because OR let a $3 chop through on the ATR leg
while cent-level wiggles passed on percent.

*Still OR:* M_Open_Flush pair.

---

## 5. Build order

Filters are grouped into **families** in `filters.json`. A family shares one
skeleton and differs only by parameters — port the skeleton once and
instantiate.

| Order | Family | Count | Why first |
|---|---|---|---|
| 1 | `sharp_move` | 6 | Most used; one skeleton, parameter table; exercises two-band logic, concentration, burst volume |
| 2 | `spike_hold` | 2 | Best-performing setup; completed-bar + retention logic |
| 3 | `low_float` | 3 | Already stale-reference-free; simplest correct examples |
| 4 | `breakout` | 6 | Monthly levels, same-day break semantics |
| 5 | `contra_abc` | 6 | Exact mirrors, clean; adds RSI, acceleration |
| 6 | `statics` | 13 | Membership lists rather than triggers |
| 7 | `m_series` | 6 | Large-cap suite; adds VWAP extension cap, EMA tags, session windows |
| 8 | `legacy` | 16 | Mostly `corrected` or `skip`; port last or not at all |

**Stop after family 3 and validate against `fixtures.json` before continuing.**

---

## 6. Validation

`fixtures.json` holds the regression set — named cases with the market
conditions and the expected outcome (`must_fire` / `must_not_fire`) per filter.

These are **not synthetic**. Each one is a real session where the outcome is
known and the reason is documented. A port that passes all of them reproduces
the useful behaviour without the defects.

The must-reject cases matter as much as the must-catch ones: they encode
distinctions that took weeks to find, such as *still moving* versus *move already
over* (NRG vs WGS — identical position, different last-minute range).
