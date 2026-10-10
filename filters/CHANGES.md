# Filter changes

Every change to a filter's logic or thresholds, newest first: what changed, why (the trade or
observation that prompted it), and what verified it. One filter per entry. When fire counts move,
this is where to look.

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
