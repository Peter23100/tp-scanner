# Writing filters: the complete reference

Everything you can write in a `.scan` file, what each piece means exactly, how it maps to Primus
and to Trade Ideas, and a cookbook of ready-to-use patterns. **Every field, condition, filter and pattern
in the cookbook (§6) is compiled by the test suite** (`cargo test -p tp-scan-engine --test
language_doc`), so none of it is aspirational: the scanner accepts it today.

To change or add a filter, you can write it yourself, or describe it in plain words to Claude
("only stocks under $20", "make the hold tighter", "add a filter like Trade Ideas' Running Up
Now"). See [Asking Claude for a change](#asking-claude-for-a-change) at the end.

---

## 1. The five things a file can contain

```
field     NAME = <number expression>          # a named number, usable anywhere, shows as a column
condition NAME = <true/false expression>      # a named yes/no test, usable inside any filter
trigger   NAME { settings...  when <test> }   # an ALERT: fires a signal (Primus filter, TI alert window)
list      NAME { settings...  when <test> }   # a STANDING LIST: who qualifies right now (Primus static, TI top list)
basket    NAME { rules... }                   # WHICH STOCKS a filter may draw from (§2.1)
```

- Every `.scan` file in `filters/` is loaded together, so a field defined in `fields.scan` can be
  used in any file. Order doesn't matter: you can use a name before you define it.
- `#` starts a comment, to the end of the line. Write the *why* in comments: what the filter hunts
  and which trade made you change a threshold.
- Names: letters, digits, `_`. No spaces or `/` (Primus `M_Pop_Medium/EXT_Up` became
  `M_Pop_Medium_EXT_Up`). A name can't reuse a builtin (`last`, `high`, ...).

### Trigger vs list: the most important choice

| | `trigger` (alert) | `list` (standing list) |
|---|---|---|
| Primus | a filter | a static |
| Trade Ideas | Alert window (alerts + filters) | Top List window |
| Output | one signal at the moment it's true (then quiet for the cooldown) | a ranked membership: symbols enter and leave as the test changes |
| `cooldown` | yes | no (a member stays while it qualifies) |
| `rank` | no | yes: sorts members, highest first |

**A trigger fires every time it's evaluated true and out of cooldown.** A pure *state* test
("price above VWAP") fires again and again every cooldown period all day. To fire on the *moment*
something happens, include an **event**: `new_high(5m)`, `new_low(5m)`, or a "just crossed" pattern
(§6.4). This is the single biggest source of repeat alerts in the Primus set: six of its filters have
no event clause.

## 2. Filter settings

Written inside `{ }` before `when`, one per line, in any order. All optional; `when` is required and
comes last.

| setting | values | default | meaning |
|---|---|---|---|
| `side` | `long` / `short` / `neutral` | neutral | colour and direction of the signal |
| `screen` | any name: `left`, `right`, `top`, `statics`, ... | none | which panel shows it |
| `hours` | `rth` / `pre` / `extended` / `any` | `rth` | when the filter is evaluated at all (below) |
| `cooldown` | a duration: `30s`, `1m`, `5m`, `1h` | none (fires every evaluation) | min time between two signals for the same symbol. Primus `MaxTriggersCount(MINUTES, 5, 1)` = `cooldown 5m` |
| `show` | `FIELD1, FIELD2, ...` | none | columns shown next to the symbol (must be fields) |
| `rank` | a number expression | none | lists only: sort order, highest first |
| `basket` | a basket's name (§2.1) | none (every stock) | only stocks in that basket can fire |
| `when` | a true/false expression | (required) | the test itself |

| `hours` | evaluated |
|---|---|
| `rth` | 09:30–16:00 New York |
| `pre` | 04:00–09:30 |
| `extended` | 04:00–20:00 (pre + regular + after-hours) |
| `any` | always |

### 2.1 Baskets: which stocks a filter may draw from

A basket is a named set of qualifications, defined once and used by any number of filters, so
"low floats on the main exchanges, $1–50" is written (and changed) in one place. The app edits them
with checkboxes and min/max boxes (launcher → **Baskets**), and shows how many symbols pass before
you save; they live in `filters/baskets.scan`.

```
basket Example_LowFloat {
    types stock, adr
    exchanges nasdaq, nyse, amex
    price 1 to 50
    float to 10_000_000
    adv 100_000 to
}
```

| rule | values | reads |
|---|---|---|
| `types` | `stock`, `adr`, `etf`, `etn`, `preferred`, `warrant`, `right`, `unit`, `fund`, `other` | the ticker's security type (Massive reference data) |
| `exchanges` | `nasdaq`, `nyse`, `amex` (NYSE American), `arca` (NYSE Arca), `cboe`, `iex`, `otc`, `other` | its primary listing exchange |
| `price A to B` | dollars | `last` |
| `float A to B` | **shares** (as `float`, §5) | free float |
| `adv A to B` | shares per day | 20-day average daily volume |
| `atr A to B` | dollars | 14-day average true range |

- Leave either end off a range: `price 5 to` is "at least $5", `float to 20_000_000` "at most 20M".
  A rule left out restricts nothing; a basket needs at least one rule.
- A filter uses one with `basket NAME`. The basket's rules go **in front** of the filter's own
  `when`, each as its own clause, so "Why isn't X firing?" shows exactly which one blocked it
  (`type is stock, adr`, `last <= 50`, ...).
- **Missing data never passes.** A symbol with no float, or not in the ticker list, is *unknown* on
  those rules and can't fire, the same as any unknown value (§3).
- The ticker list (type and exchange) comes with `tp-scan sync` (`tickers.csv`); the app fetches
  it on first start if a sync hasn't.

## 3. Expressions

### Operators, loosest-binding first

| | | example |
|---|---|---|
| `or` | either side true | `a or b` |
| `and` | both true | `a and b` |
| `not` | flips true/false | `not halted` |
| `<  <=  >  >=` | comparison | `last >= 2` |
| `+  -` | add, subtract | `last - prev_close` |
| `*  /` | multiply, divide | `(last - prev_close) / prev_close` |
| `-x` | negative | `-0.75` |

- `and` binds tighter than `or`: `a or b and c` means `a or (b and c)`. **Use brackets whenever you
  mix them**; it costs nothing and prevents the classic bug.
- `AND` / `and`, `OR` / `or`, `NOT` / `not` are all accepted.
- **There is no `=`.** Equality on a computed price practically never holds (one Contra filter is
  dead from exactly that: `Last - VWAP = -1.25`). Use `<=` / `>=`, or a band:
  `x >= -1.30 and x <= -1.20`.
- Comparisons don't chain: write `last >= 2 and last <= 20`, not `2 <= last <= 20`.

### Numbers, times and durations

| write | means |
|---|---|
| `250_000` or `250000` | a number; `_` is just a separator |
| `.25` or `0.25` | a decimal |
| `9:30`, `15:45` | a time of day (24-hour, New York), for comparing with `time` |
| `30s`, `5m`, `2h` | a duration; **only** goes inside a function: `high(5m)` |

### Unknown is not zero

Every value can be *unknown*: no ATR history, a window with no data, division by zero, a clearing
price not yet published. **A filter never fires on unknown**, and `explain` shows the `?`. You
never need `+ 0.01` guards against dividing by zero (Primus needs them; here a zero divisor makes
the field unknown instead of infinite).

- `x and unknown` is unknown, unless `x` is false (then false).
- `x or unknown` is unknown, unless `x` is true (then true).
- `not unknown` is unknown.

So in `(RVOL20DAYTIMEADJUSTED >= 3 or volume(5m) * 24 >= volume(120m) * 3)`, the second leg still
works when RVOL is unknown.

## 4. Every building block

### 4.1 Price, today

| name | meaning | Primus | Trade Ideas |
|---|---|---|---|
| `last` | latest price (newest bar's close, any session) | `Last(...)`, `LAST` | Price |
| `open` | today's 09:30 opening price | `DayBar_Open(..., '9:30')` | (Change from Open uses it) |
| `day_high` / `day_low` | regular-hours high / low so far today | `DayHigh/DayLow(..., CURRENT, NO)` | (Position in Range uses it) |
| `vwap` | regular-hours VWAP | `VWAP(CURRENT, NO, ...)` | VWAP |
| `vwap_ext` | VWAP including the premarket | `VWAP(CURRENT, YES, ...)` | |
| `vwap_upper(k)` / `vwap_lower(k)` | VWAP ± k standard deviations (TradierPro chart bands), e.g. `vwap_upper(2)` | ⚠ not Primus's bands (broken there) | |
| `pm_high` / `pm_low` | premarket high / low | `PremarketDayHigh/Low` | Pre-market High/Low |

### 4.2 Price, previous days (split-adjusted)

| name | meaning | Primus | Trade Ideas |
|---|---|---|---|
| `prev_close` | yesterday's close | `ClosePrice(P1)`, `AdjustedClosePrice(P1)` | (Change from Close uses it) |
| `prev_high` / `prev_low` | yesterday's high / low | `DayHigh(..., P1, NO)` | Yesterday's High/Low |
| `prev_month_high` / `prev_month_low` | last calendar month's high / low | `MonthHigh_I(P1)` | |
| `atr` | 14-day ATR (completed days) | `ATRStock(..., DAILY, 14, ...)` | Average True Range |
| `daily_ema(n)` | EMA of daily closes, today's price as today's close: `daily_ema(20)` | `EMAStock(..., DAILY, PERIOD_20)` | (Distance from 20-day MA) |

### 4.3 Volume and float

| name | meaning | Primus | Trade Ideas |
|---|---|---|---|
| `day_volume` | shares today, regular hours only | `DayVolume(..., CURRENT, NO)` | Volume Today |
| `day_volume_ext` | shares today including premarket | `DayVolume(..., CURRENT, YES)` | |
| `pm_volume` | premarket shares | `PremarketDayVolume` | Pre-market Volume |
| `adv` | average daily volume (20 days) | `AvgDayVolume(..., 20, NO)` | Average Daily Volume |
| `float` | float, **in shares** (`20_000_000`, not `20`) | `FloatShares <= 20` (millions) | Float |

Relative volume is not a builtin: it's your own field, `RVOL20DAYTIMEADJUSTED` in `fields.scan`
(Primus's definition, time-of-day curve included). Use it like any number.

### 4.4 Windows: high, low, open, close and volume over a stretch of time

```
high(5m)            the last 5 minutes, rolling (at 10:07: 10:02–10:07)
high(5m, 1)         the 5 minutes before that (09:57–10:02)        Primus P1
high(5m, 2)         the 5 before those                             Primus P2
high(30m, clock)    the clock-aligned half hour in progress (10:00–10:07 at 10:07)
high(30m, clock, 1) the previous clock half hour (09:30–10:00)
```

Same shapes for `low(...)`, `volume(...)`, `open(...)` and `close(...)`. `open(5m)` is the first
price in the window and `close(5m)` the last, so `close(5m, 1) < open(5m, 1)` is "the five minutes
before the last five were red", and `close(1m, 5)` is the price five minutes ago. (Bare `open`,
without brackets, is still today's 09:30 open.) Spans can be seconds (`30s`), minutes or hours. The
span plus the back-offset must fit in **4 hours** of history. Seconds-level windows need 1-second
data (live feed, or replay with `--seconds`).

**Prefer rolling (the default).** A clock window splits a move across the boundary: a spike from
10:03 to 10:08 shows up as half a spike in each of two 5-minute bars. That's Primus's documented
"one spike in five is missed" problem. Use `clock` only when you mean a chart candle.

Windows include extended hours: `volume(120m)` at 10:00 reaches into the premarket.

| this | Primus | Trade Ideas |
|---|---|---|
| `high(5m)`, `low(5m)` | `MinuteHigh(..., 5, CURRENT, ...)` | (5 Minute Range uses them) |
| `high(5m, 1)` | `MinuteHigh(..., 5, P1, ...)` | |
| `volume(5m)` | `MinuteVolume(..., 5, CURRENT, ...)` | Volume 5 Minute |
| `open(5m, 1)`, `close(5m, 1)` | `MinuteOpen_I` / `MinuteClose_I(..., 5, P1, ...)` | (candle colour) |
| `close(1m, 5)` | | (the price behind Change 5 Minute) |

### 4.5 Indicators on 5-minute candles

| name | meaning | Primus | Trade Ideas |
|---|---|---|---|
| `ema(5m, n)` | n-period EMA on 5-minute candles: `ema(5m, 20)` | `EMAStock(..., MINUTES_5, PERIOD_n)` | 5 min EMA |
| `sma(5m, n)` | simple moving average | `SMAStock(..., MINUTES_5, ...)` | 5 min SMA |
| `rsi(5m, n)` | Wilder RSI, 0–100: `rsi(5m, 14)` | `RSIStock(..., MINUTES_5, 14)` | 5 min RSI |
| `bb_upper(5m, n, k)` / `bb_lower(5m, n, k)` | Bollinger bands: `bb_upper(5m, 20, 2)` | `BollingerBands(..., MINUTES_5, 20, ..., V2)` | (Position in Bollinger Bands) |

Only 5-minute candles exist so far (`ema(1m, 9)` is refused). Asking for 1/2/15-minute candles is a
small engine addition (§8).

### 4.6 Events: the "just happened" tests

| name | true when | Primus | Trade Ideas |
|---|---|---|---|
| `new_high(5m)` | the newest bar's high beats every high in the 5 minutes before it | `NewMinuteHigh(5, NO)` | New High (for the period) |
| `new_low(5m)` | the newest bar's low undercuts every low in the 5 minutes before it | `NewMinuteLow(5, NO)` | New Low |
| `halted` | halted right now | `IsHalt` | Halt |

`halted` and `new_high(...)` are true/false on their own: `when halted and day_volume > 0`.

### 4.7 Time

| name | meaning |
|---|---|
| `time` | time of day of the newest print, compared with times: `time >= 9:45 and time <= 15:30` |

### 4.8 Auctions and imbalances (closing/opening cross; feed not wired yet, reads unknown until it is)

| name | meaning | Primus |
|---|---|---|
| `imb_buy(nyse)`, `imb_buy(nasdaq)`, `imb_buy(any)` | buy-side imbalance shares | `ImbalanceBuyVolume(NYSE)` |
| `imb_sell(...)` | sell-side imbalance shares | `ImbalanceSellVolume(...)` |
| `clear_price` / `near_price` | auction clearing / indicative price | `ClearPricePrimary` / `NearPricePrimary` |
| `ind_bid` / `ind_ask` | NYSE indicative bid / ask | `IndBid` / `IndAsk` |

### 4.9 Helper functions

| function | meaning | example |
|---|---|---|
| `if(test, a, b)` | `a` when the test is true, else `b` | `if(time <= 10:00, 2.0, 1.5)` |
| `max(a, b)` / `min(a, b)` | the larger / smaller | `max(0.50, last * 0.02)` = "50¢ or 2%, whichever is bigger" |
| `abs(a)` | size without sign | `abs(last - vwap) / atr` |

### 4.10 Your own fields and conditions (already defined, use them anywhere)

From `fields.scan`: `RVOL20DAYTIMEADJUSTED`, `PM_RVOL`, `VolROC3`, `VolROC_1m_3`,
`VolFade_1m_Smooth`, `Spike_Vol_Pct`, `ATR_POS`, `ATR_NEG`, `Spike_Range_Pct`, `PM_RangePos`,
`VWAP_Stretch`, `Retrace_Pct_Up/Down`, `Retrace_1m_Up/Down`, `RNK_UP`, `RNK_DWN`, `Clear_ATR`,
`Near_ATR`, `Imb_Buy_Ratio`, `Imb_Sell_Ratio`; helpers `up5 up15 up30 dn5 dn15 dn30`, `day_range`,
`spike_range`; condition `volatile` (ATR ≥ 3% of price). From `contra.scan`: `CONTRA_RVOL` (the Contras'
own linear-day RVOL) and `BURST_120` (five-minute pace against the last two hours).

`common.scan` adds Trade Ideas-style fields: `chg_close_pct`, `gap_pct`,
`chg_open_pct`, `pos_in_range`, `pos_in_5m_range`, `chg_5m_pct`, `range_5m_pct`, `vwap_dist_pct`,
`float_rotation`, `pos_in_bb`, **`RVOL_NOW`** (the last half hour's volume against this stock's
normal pace for that half hour; a quiet morning can't dilute it; known 10:00–16:00), and the
conditions `tradeable`, `crossed_above_vwap`, `crossed_below_vwap`.

## 5. Units: the mistakes that silently break a filter

| | correct | wrong |
|---|---|---|
| float | `float <= 20_000_000` (shares) | `float <= 20` (Primus millions: matches nothing). **Refused when the filters load**, as is a float so large it filters nothing |
| percent | `(last - prev_close) / prev_close >= 0.10` (fraction: 0.10 = 10%) | `>= 10` (means +1000%) |
| time | `time >= 9:45` | `time >= 945` (minutes since midnight: 15:45) |
| price band | `max(0.50, last * 0.02)` (dollar AND percent floor) | a bare `0.50` (huge on a $2 stock, nothing on a $500 one) |

**Prefer same-session references over stale ones** (the 9/25 lesson, master §4A). `atr` and `adv`
are measured over past weeks and are wrong exactly when a stock wakes up: a dormant $2 name with
a 0.12 ATR gives `0.15 * atr` two cents of room. Same-session alternatives:

| instead of | consider |
|---|---|
| `day_high - 0.15 * atr` | `day_high - 0.15 * day_range` |
| `volume(5m, 1) >= adv * 0.03` | `volume(5m, 1) >= day_volume * 0.03` |

## 6. Cookbook

The fields and conditions below live in `filters/common.scan` and are ready to use in any filter.
The example triggers and lists live in `examples/recipes.scan`, which is **not** loaded with your
filters (so they never fire), and is compiled by the tests together with `filters/`. To use one,
copy it into a `filters/*.scan` file and rename it. **They're examples, not tuned setups: thresholds
are illustrative.**

### 6.1 Trade Ideas filters as fields

| Trade Ideas filter | here |
|---|---|
| Change from Close (%) | `field chg_close_pct = (last - prev_close) / prev_close * 100` |
| Gap (%) | `field gap_pct = (open - prev_close) / prev_close * 100` |
| Change from Open (%) | `field chg_open_pct = (last - open) / open * 100` |
| Position in Range (today, %) | `field pos_in_range = (last - day_low) / (day_high - day_low) * 100` |
| Position in 5 Minute Range | `field pos_in_5m_range = (last - low(5m)) / (high(5m) - low(5m)) * 100` |
| Change 5 Minute (%) | `field chg_5m_pct = (last - close(1m, 5)) / close(1m, 5) * 100` |
| 5 Minute Range (%) | `field range_5m_pct = (high(5m) - low(5m)) / last * 100` |
| Distance from VWAP (%) | `field vwap_dist_pct = (last - vwap) / vwap * 100` |
| Relative Volume | `RVOL20DAYTIMEADJUSTED` (yours) |
| Volume 5 Minute vs normal | `volume(5m) * 78 / adv` (78 five-minute bars in a session) |
| Float Rotation | `field float_rotation = day_volume_ext / float` |
| Position in Bollinger Bands | `field pos_in_bb = (last - bb_lower(5m, 20, 2)) / (bb_upper(5m, 20, 2) - bb_lower(5m, 20, 2)) * 100` |
| Average True Range ($ / %) | `atr`, `atr / last * 100` |

### 6.2 One shared "tradeable" gate

Your stated daily cost is untradeable fires. Put the universe rules in **one condition** and AND
every filter with it. Then tuning "what's tradeable" happens in one place:

```
condition tradeable = last >= 2 and last <= 500
                  and day_volume_ext >= 300_000
                  and atr >= 0.25
```

### 6.3 Trade Ideas alerts as triggers

**Running Up Now** (fast move on volume):
```
trigger Ex_Running_Up {
    side long
    screen recipes
    cooldown 3m
    when tradeable
     and up5 >= max(0.30, last * 0.015)
     and volume(5m) >= volume(5m, 1) * 2
     and new_high(5m)
}
```

**New High of Day** with a minimum move:
```
trigger Ex_New_HOD {
    side long
    screen recipes
    cooldown 5m
    when tradeable
     and new_high(30m)
     and last >= day_high - 0.02
     and chg_close_pct >= 3
}
```

**Consolidation Breakout** (tight 30 minutes, then out of it):
```
trigger Ex_Consolidation_Breakout {
    side long
    screen recipes
    cooldown 10m
    when tradeable
     and (high(30m, 1) - low(30m, 1)) <= 0.5 * atr
     and last >= high(30m, 1) + 0.05 * atr
     and volume(5m) >= volume(30m, 1) / 6 * 2
     and new_high(5m)
}
```

**Pullback from highs** (stock up on the day, now 3%+ off its high):
```
trigger Ex_Pullback_From_High {
    side long
    screen recipes
    cooldown 15m
    when tradeable
     and chg_close_pct >= 5
     and last <= day_high * 0.97
     and last >= vwap
     and new_low(5m)
}
```

### 6.4 "Just crossed": turning a state into an event

"Above VWAP" is state (true all day). "Crossed above VWAP" is an event: now above, and the minute
before was below:
```
condition crossed_above_vwap = last >= vwap and low(1m, 1) < vwap and high(1m, 2) < vwap
condition crossed_below_vwap = last <= vwap and high(1m, 1) > vwap and low(1m, 2) > vwap
```
The same shape works for any level: `prev_high`, `pm_high`, `daily_ema(20)`, `prev_month_high`.

**First time today** ("a same-day break", the §4.25 lesson): add `prev_close < LEVEL` so it
can't fire every day forever once cleared:
```
and last >= prev_month_high and prev_close < prev_month_high
```

### 6.5 Standing lists (Trade Ideas Top Lists, Primus statics)

**Top gainers, ranked by move in ATRs:**
```
list Ex_Top_Gainers {
    side long
    screen recipes
    rank RNK_UP
    show chg_close_pct, RVOL20DAYTIMEADJUSTED, pos_in_range
    when tradeable and chg_close_pct >= 4
}
```

**Premarket gappers** (evaluated in the premarket only):
```
list Ex_PM_Gappers {
    side long
    screen recipes
    hours pre
    rank PM_RVOL
    show chg_close_pct, PM_RVOL, float_rotation
    when last >= 1 and chg_close_pct >= 10 and pm_volume >= 100_000
}
```

### 6.6 Patterns worth knowing

| idea | write |
|---|---|
| dollar AND percent floor | `up5 >= max(0.50, last * 0.02)` |
| time-of-day threshold | `RVOL20DAYTIMEADJUSTED >= if(time <= 10:00, 3.0, 2.0)` |
| a band | `last >= 5 and last <= 50` |
| near a level | `abs(last - prev_high) <= 0.1 * atr` |
| accelerating volume | `volume(5m) >= volume(5m, 1) * 1.5 and volume(5m, 1) >= volume(5m, 2)` |
| higher highs / higher lows | `high(5m) > high(5m, 1) and low(5m) > low(5m, 1) and low(5m, 1) > low(5m, 2)` |
| quiet coil | `(high(15m) - low(15m)) <= 0.3 * atr` |
| held its gains | `last >= high(5m, 1) - spike_range * 0.25` |
| volume burst vs the last 2 hours | `volume(5m) * 24 >= volume(120m) * 3` |
| RSI stretched | `rsi(5m, 14) >= 80` |
| trend aligned | `ema(5m, 9) >= ema(5m, 20) and last >= ema(5m, 20)` |

## 7. Checking your work

```
cargo run -p tp-scan-cli -- check filters
```
lists every filter with its settings, or names the exact line and column of a mistake:
```
filters/right.scan:241:16: `last_price` is not defined (and isn't a builtin)
```

It also refuses, with the line, the mistakes that made Primus filters run "healthy" while never
firing: bounds on the same quantity that no value can meet
(`last - vwap <= -0.75 and last - vwap >= 0.75`: "did you mean `or`?"), `=` on a price, and a
float in the wrong units.

Once there is market data, `replay` runs a day through the filters, and
`--explain FILTER:SYMBOL` shows every clause with its value: which one blocked it, and by how much.
That answers "why didn't it catch X", the #2 complaint about Primus.

## 8. Not available yet (each is an engine addition; ask for it)

These are common in Trade Ideas but need data or a builtin the scanner doesn't have. Each is a
focused piece of work, not a language change.

| missing | Trade Ideas | needs |
|---|---|---|
| candles other than 5-minute | 1/2/15/60 min EMA, RSI, ... | keeping more candle series (small) |
| consecutive green/red candles | Consecutive Candles | a counting builtin (small) |
| multi-day highs/lows (5-day, 52-week) | Position in 5 Day / 52 Week Range | more daily history fields (small) |
| time since an event ("minutes since HOD") | | per-symbol event memory (medium) |
| bid, ask, spread, sizes | Spread, Bid/Ask Size | the quotes feed (data) |
| short interest / short float | Short Float | a data source (Massive has short interest) |
| shares outstanding, market cap | Market Cap | a data source |
| sector, industry | Sector | a data source |
| earnings date, news | Earnings, News | data sources |
| count of today's signals | | per-symbol signal history (medium) |

## Asking Claude for a change

Plain language works; the more specific, the better. Good requests:

- "M_Spike_Hold_Up fired on AAPL at 10:42 and it was junk because the spread was huge, tighten it."
- "Add a filter like Trade Ideas' Running Up Now, but only under $20 and only before 11:00."
- "Why didn't LowFloat_HOD_Break_Up catch MSGY on 9/24?" (needs data: replay + explain)
- "Make all the hold filters use day volume instead of ADV."

What happens every time:
1. Claude edits the `.scan` file and explains the change clause by clause.
2. `check` must pass; the test suite must pass. If a scenario test covers the filter, it is updated
   or a new one is added.
3. The change is noted in `filters/CHANGES.md` with the date and the reason (the trade that
   prompted it).
4. **One filter change at a time**, so if fire counts move, you know why.
