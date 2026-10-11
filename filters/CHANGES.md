# Filter changes

Every change to a filter's logic or thresholds, newest first: what changed, why (the trade or
observation that prompted it), and what verified it. One filter per entry. When fire counts move,
this is where to look.

## 2026-10-09: 19 recovered filters translated; 3 corrected because the loader refuses the originals

From spec/port/RECOVERED_EXPRESSIONS.md: 16 Contras (`contra.scan`), Breakout_Hold_Up/Down
(`right.scan`), PM_LowFloat_Coil_Up (`statics.scan`). As loaded, except:

| filter | Primus | here | why |
|---|---|---|---|
| Momo_VWAP_Divergence_Long | `Last - VWAP >= 0.75` alongside `<= -0.75` | clause deleted | can never be true; never fired |
| Contra_Long_VWAP_Divergence | `Last - VWAP = -1.25` | `<= -1.25` | equality on a price never holds |
| PM_LowFloat_Coil_Up | `Float <= 10000000` (millions) | `float <= 10_000_000` shares | filtered nothing |

**Verified:** all load; a capitulation scenario fires Contra_Long; the three originals are refused.
**Not verified on real data.** ⚠ Contra_Long's recovered text has an unmatched `)`: check in Primus.

## 2026-10-09: RVOL_CURVE and PM_CURVE are piecewise linear, not steps (`fields.scan`)

**Why:** each Primus IF branch is a linear ramp (filters.json); the master's PYPL figure (0.92 at
15:44) only holds interpolated. Steps read RVOL up to ~35% low early on. Affects every filter using
RVOL20DAYTIMEADJUSTED or PM_RVOL. **Verified:** curve values tested at 8 times incl. 15:44 = 0.92.
⚠ filters.json says 0.011 at 09:31; a straight line from the table gives 0.016. Check the first
segment against the verbatim IF chain.

## 2026-10-09: LowFloat_HOD_Break_Up rebuilt to the master's 2026-09-25 version (`right.scan`)

**Why:** MSGY (+202%) and ELPW (+65%) were missed on consecutive days. Cause: `atr` and `adv` are
stale exactly on a dormant low-float that just woke (MSGY ATR ≈ 0.12 gave the day-high tolerance two
cents), and ADV is inflated by reverse splits (PFSA 1-for-4). Master §4.21, §4A "stale-reference
problem".

| clause | was | now |
|---|---|---|
| today's volume (incl. premarket) | ≥ 250k | ≥ 500k |
| up from prior close | ≥ 10% | ≥ 20% |
| today's range | none | ≥ 10% of price |
| day-high tolerance | `day_high - 0.15 * atr` | `day_high - 0.15 * day_range` |
| completed 5-min bar range | none | ≥ 2% of price (the dial if the list runs long) |
| RVOL or burst | `RVOL ≥ 3 or burst` | burst required, RVOL dropped |
| completed-bar volume, second leg | `≥ adv * 0.05` | `≥ day_volume * 0.03` |

**Verified:** `check` passes; new scenario tests: a $2 low-float with a 0.12 ATR closing 3¢ under
each high fires, and the same tape with a 50M float doesn't (and `explain` blames the float).
**Not verified on real data** (no data subscription yet).
