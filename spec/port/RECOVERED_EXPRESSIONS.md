# Recovered filter expressions

Verbatim source recovered from the session transcript, 2026-10-09. These are the
filters that were collected in conversation but never written into the master's
code blocks — the master described them in prose instead. **This file closes that
gap; nothing here needs re-copying from Primus.**

Each expression ends with the filter's own name, exactly as Primus returns it.

Still genuinely missing and only obtainable from the Primus editor:
- `Contra Long All` and `Contra Short All` (six-branch ORs; analysed in the master §4C but not recoverable verbatim)
- `CatchALL_LF_HOD` (never collected in any session)
- plain `Halted` (never collected; only `Halted Static New Minute`)

---

## Contra_Short_A_FAST

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 1000000 AND
( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) )
  / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.40 AND
( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) ) >= 0.50 AND
ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) / Last(ALL_VENUES, CURRENT, NO) <= 0.15 AND
( Last(ALL_VENUES, CURRENT, NO) - MinuteHigh_I(ALL_VENUES, 5, P2, NO, False) )
  >= ( MinuteHigh_I(ALL_VENUES, 5, P2, NO, False) - MinuteHigh_I(ALL_VENUES, 5, P4, NO, False) ) * 2.0 AND
MinuteHigh_I(ALL_VENUES, 5, P2, NO, False) >= MinuteHigh_I(ALL_VENUES, 5, P4, NO, False) AND
MinuteVolume(ALL_VENUES, 5, CURRENT, NO) >= MinuteVolume(ALL_VENUES, 5, P1, NO) * 2.0 AND
NewMinuteHigh(5, NO) AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Contra_Short_B_Extended

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 1000000 AND
RSIStock(ALL_VENUES, NO, MINUTES_5, 14, CURRENT) >= 75 AND
( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 30, CURRENT, NO, False) )
  / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.50 AND
( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 30, CURRENT, NO, False) ) >= 0.50 AND
ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) / Last(ALL_VENUES, CURRENT, NO) <= 0.15 AND
RVOL20DAYTIMEADJUSTED >= 1.5 AND
Last(ALL_VENUES, CURRENT, NO) > VWAP(CURRENT, NO, VWAP, V1, PRIMARY) AND
NewMinuteHigh(5, NO) AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Contra_Short_C_Breakdown

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 1000000 AND
( MinuteHigh(ALL_VENUES, 15, CURRENT, NO, False) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) )
  / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.40 AND
( MinuteHigh(ALL_VENUES, 15, CURRENT, NO, False) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) ) >= 0.50 AND
ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) / Last(ALL_VENUES, CURRENT, NO) <= 0.15 AND
( Last(ALL_VENUES, CURRENT, NO) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) )
  / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.25 AND
Last(ALL_VENUES, CURRENT, NO) <= MinuteLow(ALL_VENUES, 5, P1, NO, False) AND
MinuteVolume(ALL_VENUES, 5, CURRENT, NO) >= MinuteVolume(ALL_VENUES, 5, P1, NO) AND
NewMinuteLow(5, NO) AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Contra_Cap_A_Fast

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 1000000 AND
( MinuteHigh(ALL_VENUES, 15, CURRENT, NO, False) - Last(ALL_VENUES, CURRENT, NO) )
  / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.40 AND
( MinuteHigh(ALL_VENUES, 15, CURRENT, NO, False) - Last(ALL_VENUES, CURRENT, NO) ) >= 0.50 AND
ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) / Last(ALL_VENUES, CURRENT, NO) <= 0.15 AND
( MinuteLow_I(ALL_VENUES, 5, P2, NO, False) - Last(ALL_VENUES, CURRENT, NO) )
  >= ( MinuteLow_I(ALL_VENUES, 5, P4, NO, False) - MinuteLow_I(ALL_VENUES, 5, P2, NO, False) ) * 2.0 AND
MinuteLow_I(ALL_VENUES, 5, P4, NO, False) >= MinuteLow_I(ALL_VENUES, 5, P2, NO, False) AND
MinuteVolume(ALL_VENUES, 5, CURRENT, NO) >= MinuteVolume(ALL_VENUES, 5, P1, NO) * 2.0 AND
NewMinuteLow(5, NO) AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Contra_Cap_B_Extended

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 1000000 AND
RSIStock(ALL_VENUES, NO, MINUTES_5, 14, CURRENT) <= 25 AND
( MinuteHigh(ALL_VENUES, 30, CURRENT, NO, False) - Last(ALL_VENUES, CURRENT, NO) )
  / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.50 AND
( MinuteHigh(ALL_VENUES, 30, CURRENT, NO, False) - Last(ALL_VENUES, CURRENT, NO) ) >= 0.50 AND
ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) / Last(ALL_VENUES, CURRENT, NO) <= 0.15 AND
RVOL20DAYTIMEADJUSTED >= 1.5 AND
Last(ALL_VENUES, CURRENT, NO) < VWAP(CURRENT, NO, VWAP, V1, PRIMARY) AND
NewMinuteLow(5, NO) AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Contra_Reclaim_C

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 1000000 AND
( MinuteHigh(ALL_VENUES, 15, CURRENT, NO, False) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) )
  / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.40 AND
( MinuteHigh(ALL_VENUES, 15, CURRENT, NO, False) - MinuteLow(ALL_VENUES, 15, CURRENT, NO, False) ) >= 0.50 AND
ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) / Last(ALL_VENUES, CURRENT, NO) <= 0.15 AND
( MinuteHigh(ALL_VENUES, 15, CURRENT, NO, False) - Last(ALL_VENUES, CURRENT, NO) )
  / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.25 AND
Last(ALL_VENUES, CURRENT, NO) >= MinuteHigh(ALL_VENUES, 5, P1, NO, False) AND
MinuteVolume(ALL_VENUES, 5, CURRENT, NO) >= MinuteVolume(ALL_VENUES, 5, P1, NO) AND
NewMinuteHigh(5, NO) AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Contra Long

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) > 500000 AND
RSIStock(ALL_VENUES, NO, MINUTES_5, 14, CURRENT) < 25 AND
1.0 * DayVolume(ALL_VENUES, 1, CURRENT, NO) / ( AvgDayVolume(ALL_VENUES, 20, NO) * ( TickTime - 570.0)/ 390 ) > 1.5 AND
(MinuteVolume(ALL_VENUES, 5, CURRENT, NO)* 24.0) / MinuteVolume(ALL_VENUES, 120, CURRENT, NO) > 1.5 AND
(Last(ALL_VENUES, CURRENT, NO) - MinuteClose_I(ALL_VENUES, 6, P5, NO)) < -.40 AND
MinuteClose_I(ALL_VENUES, 5, P1, NO) < MinuteOpen_I(ALL_VENUES, 5, P1, NO) AND
MinuteClose_I(ALL_VENUES, 5, P2, NO) < MinuteOpen_I(ALL_VENUES, 5, P2, NO) AND
MinuteClose_I(ALL_VENUES, 5, P3, NO) < MinuteOpen_I(ALL_VENUES, 5, P3, NO) AND
MinuteClose_I(ALL_VENUES, 5, P4, NO) < MinuteOpen_I(ALL_VENUES, 5, P4, NO) AND
Last(ALL_VENUES, CURRENT, NO) - VWAP(CURRENT, NO, VWAP, V1, PRIMARY) < -0.75 AND
NewMinuteLow(5, NO) ) AND
LAST>5 AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Contra Short

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 500000 AND
(Last(ALL_VENUES, CURRENT, NO) - MinuteOpen_I(ALL_VENUES, 1, P5, NO)) > .65 AND
(Last(ALL_VENUES, CURRENT, NO) / MinuteOpen_I(ALL_VENUES, 1, P5, NO)) > 1.02 AND
(MinuteVolume(ALL_VENUES, 5, CURRENT, NO)* 24.0) / MinuteVolume(ALL_VENUES, 120, CURRENT, NO) > 1.77 AND
Last(ALL_VENUES, CURRENT, NO) - VWAP(CURRENT, NO, VWAP, V1, PRIMARY) > 1.00 AND
Last >5 AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Contra Long V5

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) > 500000 AND
1.0 * DayVolume(ALL_VENUES, 1, CURRENT, NO) / ( AvgDayVolume(ALL_VENUES, 20, NO) * ( TickTime - 570.0)/ 390 ) > 1.25 AND
(MinuteVolume(ALL_VENUES, 5, CURRENT, NO)* 24.0) / MinuteVolume(ALL_VENUES, 120, CURRENT, NO) > 1.75 AND
RSIStock(ALL_VENUES, NO, MINUTES_5, 14, CURRENT) <= 25 AND
(Last(ALL_VENUES, CURRENT, NO) - MinuteOpen_I(ALL_VENUES, 6, P5, NO)) <= .50 AND
MinuteClose_I(ALL_VENUES, 5, P1, NO) < MinuteOpen_I(ALL_VENUES, 5, P1, NO) AND
MinuteClose_I(ALL_VENUES, 5, P2, NO) < MinuteOpen_I(ALL_VENUES, 5, P2, NO) AND
MinuteClose_I(ALL_VENUES, 5, P3, NO) < MinuteOpen_I(ALL_VENUES, 5, P3, NO) AND
MinuteClose_I(ALL_VENUES, 5, P4, NO) < MinuteOpen_I(ALL_VENUES, 5, P4, NO) AND
Last(ALL_VENUES, CURRENT, NO) - VWAP(CURRENT, NO, VWAP, V1, PRIMARY) <= -0.75 AND
MaxTriggersCount(MINUTES, 1, 1) AND
LAST >5
```

## Contra Short V5

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) > 500000 AND
1.0 * DayVolume(ALL_VENUES, 1, CURRENT, NO) / ( AvgDayVolume(ALL_VENUES, 20, NO) * ( TickTime - 570.0)/ 390 ) > 1.25 AND
(MinuteVolume(ALL_VENUES, 5, CURRENT, NO)* 24.0) / MinuteVolume(ALL_VENUES, 120, CURRENT, NO) > 1.75 AND
RSIStock(ALL_VENUES, NO, MINUTES_5, 14, CURRENT) >= 75 AND
(Last(ALL_VENUES, CURRENT, NO) - MinuteOpen_I(ALL_VENUES, 6, P5, NO)) >= -.50 AND
MinuteClose_I(ALL_VENUES, 5, P1, NO) > MinuteOpen_I(ALL_VENUES, 5, P1, NO) AND
MinuteClose_I(ALL_VENUES, 5, P2, NO) > MinuteOpen_I(ALL_VENUES, 5, P2, NO) AND
MinuteClose_I(ALL_VENUES, 5, P3, NO) > MinuteOpen_I(ALL_VENUES, 5, P3, NO) AND
MinuteClose_I(ALL_VENUES, 5, P4, NO) > MinuteOpen_I(ALL_VENUES, 5, P4, NO) AND
Last(ALL_VENUES, CURRENT, NO) - VWAP(CURRENT, NO, VWAP, V1, PRIMARY) > 0.75 AND
NewMinuteHigh(10, NO) AND
LAST>5 AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Contra Long V8

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) > 500000 AND
1.0 * DayVolume(ALL_VENUES, 1, CURRENT, NO) / ( AvgDayVolume(ALL_VENUES, 20, NO) * ( TickTime - 570.0)/ 390 ) > 1.25 AND
(MinuteVolume(ALL_VENUES, 5, CURRENT, NO)* 24.0) / MinuteVolume(ALL_VENUES, 120, CURRENT, NO) > 1.75 AND
RSIStock(ALL_VENUES, NO, MINUTES_5, 14, CURRENT) <= 25 AND
(Last(ALL_VENUES, CURRENT, NO) - MinuteOpen_I(ALL_VENUES, 6, P5, NO)) <= -.50 AND
MinuteClose_I(ALL_VENUES, 5, P1, NO) < MinuteOpen_I(ALL_VENUES, 5, P1, NO) AND
MinuteClose_I(ALL_VENUES, 5, P2, NO) < MinuteOpen_I(ALL_VENUES, 5, P2, NO) AND
MinuteClose_I(ALL_VENUES, 5, P3, NO) < MinuteOpen_I(ALL_VENUES, 5, P3, NO) AND
MinuteClose_I(ALL_VENUES, 5, P4, NO) < MinuteOpen_I(ALL_VENUES, 5, P4, NO) AND
Last(ALL_VENUES, CURRENT, NO) / VWAP(CURRENT, NO, VWAP, V1, PRIMARY) < .98 AND
Last(ALL_VENUES, CURRENT, NO) <= BollingerBands(CURRENT, MINUTES_5, 20, ALL_VENUES, NO, LowerBand, V2) AND
Last(ALL_VENUES, CURRENT, NO) - VWAP(CURRENT, NO, VWAP, V1, PRIMARY) <= -0.75 AND
( NewMinuteHigh(5, NO) OR
NewMinuteLow(5, NO) ) AND
LAST>5 AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Contra Short V8

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) > 500000 AND
1.0 * DayVolume(ALL_VENUES, 1, CURRENT, NO) / ( AvgDayVolume(ALL_VENUES, 20, NO) * ( TickTime - 570.0)/ 390 ) > 1.25 AND
(MinuteVolume(ALL_VENUES, 5, CURRENT, NO)* 24.0) / MinuteVolume(ALL_VENUES, 120, CURRENT, NO) > 1.75 AND
RSIStock(ALL_VENUES, NO, MINUTES_5, 14, CURRENT) >= 75 AND
(Last(ALL_VENUES, CURRENT, NO) - MinuteOpen_I(ALL_VENUES, 6, P5, NO)) >= .50 AND
MinuteClose_I(ALL_VENUES, 5, P1, NO) > MinuteOpen_I(ALL_VENUES, 5, P1, NO) AND
MinuteClose_I(ALL_VENUES, 5, P2, NO) > MinuteOpen_I(ALL_VENUES, 5, P2, NO) AND
MinuteClose_I(ALL_VENUES, 5, P3, NO) > MinuteOpen_I(ALL_VENUES, 5, P3, NO) AND
MinuteClose_I(ALL_VENUES, 5, P4, NO) > MinuteOpen_I(ALL_VENUES, 5, P4, NO) AND
Last(ALL_VENUES, CURRENT, NO) / VWAP(CURRENT, NO, VWAP, V1, PRIMARY) > 1.02 AND
Last(ALL_VENUES, CURRENT, NO) >= BollingerBands(CURRENT, MINUTES_5, 20, ALL_VENUES, NO, UpperBand, V2) AND
Last(ALL_VENUES, CURRENT, NO) - VWAP(CURRENT, NO, VWAP, V1, PRIMARY) > 0.75 AND
NewMinuteHigh(10, NO) AND
LAST>5 AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Contra Long Parabolic

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 500000 AND
RSIStock(ALL_VENUES, NO, MINUTES_5, 14, CURRENT) <= 30 AND
(Last(ALL_VENUES, CURRENT, NO) - MinuteClose_I(ALL_VENUES, 6, P5, NO)) <= -0.50 AND
((Last(ALL_VENUES, CURRENT, NO) - MinuteClose_I(ALL_VENUES, 6, P5, NO)) / MinuteClose_I(ALL_VENUES, 6, P5, NO) * 100) < -5.0 AND
MinuteClose_I(ALL_VENUES, 5, P1, NO) < MinuteOpen_I(ALL_VENUES, 5, P1, NO) AND
MinuteClose_I(ALL_VENUES, 5, P2, NO) < MinuteOpen_I(ALL_VENUES, 5, P2, NO) AND
(MinuteVolume(ALL_VENUES, 5, CURRENT, NO)* 24.0) / MinuteVolume(ALL_VENUES, 120, CURRENT, NO)> 1.5 AND
Last(ALL_VENUES, CURRENT, NO) - VWAP(CURRENT, NO, VWAP, V1, PRIMARY) <= 0.75 AND
( NewMinuteHigh(5, NO) OR
NewMinuteLow(5, NO) ) AND
LAST>5 AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Contra Short Parabolic

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 750000 AND
(Last(ALL_VENUES, CURRENT, NO) - MinuteOpen_I(ALL_VENUES, 1, P5, NO)) > .50 AND
(Last(ALL_VENUES, CURRENT, NO) / MinuteOpen_I(ALL_VENUES, 1, P5, NO)) > 1.015 AND
(MinuteVolume(ALL_VENUES, 5, CURRENT, NO)* 24.0) / MinuteVolume(ALL_VENUES, 120, CURRENT, NO) > 1.75 AND
Last(ALL_VENUES, CURRENT, NO) - VWAP(CURRENT, NO, VWAP, V1, PRIMARY) > 0.75 AND
LAST>5 AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Momo VWAP Divergence Long

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 500000 AND
(Last(ALL_VENUES, CURRENT, NO) - MinuteOpen_I(ALL_VENUES, 6, P5, NO)) < -.50 AND
Last(ALL_VENUES, CURRENT, NO) / VWAP(CURRENT, NO, VWAP, V1, PRIMARY) <= 0.97 AND
Last(ALL_VENUES, CURRENT, NO) - VWAP(CURRENT, NO, VWAP, V1, PRIMARY) <= -0.75 AND
(Last(ALL_VENUES, CURRENT, NO) - BollingerBands(CURRENT, MINUTES_5, 20, ALL_VENUES, NO, LowerBand, V2)) / ( SMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_20, CURRENT) - BollingerBands(CURRENT, MINUTES_5, 20, ALL_VENUES, NO, LowerBand, V2)) <= 0.10 AND
Last(ALL_VENUES, CURRENT, NO) - VWAP(CURRENT, NO, VWAP, V1, PRIMARY) >= 0.75 AND
MaxTriggersCount(MINUTES, 1, 1)
```

## Contra Long VWAP Divergence

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 500000 AND
(Last(ALL_VENUES, CURRENT, NO) - MinuteOpen_I(ALL_VENUES, 6, P5, NO)) < -.60 AND
Last(ALL_VENUES, CURRENT, NO) / VWAP(CURRENT, NO, VWAP, V1, PRIMARY) <= 0.97 AND
(Last(ALL_VENUES, CURRENT, NO) - BollingerBands(CURRENT, MINUTES_5, 20, ALL_VENUES, NO, LowerBand, V2)) / ( SMAStock(ALL_VENUES, NO, MINUTES_5, PERIOD_20, CURRENT) - BollingerBands(CURRENT, MINUTES_5, 20, ALL_VENUES, NO, LowerBand, V2)) <= 0.10 AND
Last(ALL_VENUES, CURRENT, NO) - VWAP(CURRENT, NO, VWAP, V1, PRIMARY) = - 1.25 AND
MaxTriggersCount(MINUTES, 1, 1) AND
LAST>5
```

## Breakout_Hold_Up

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 1000000 AND
Last(ALL_VENUES, CURRENT, NO) >= 10 AND
( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.50 AND
RVOL20DAYTIMEADJUSTED >= 2.0 AND
Last(ALL_VENUES, CURRENT, NO) >= ClosePrice(PRIMARY, P1, NO) AND
(
  ( Last(ALL_VENUES, CURRENT, NO) >= MonthHigh_I(ALL_VENUES, 1, P1, NO) AND
ClosePrice(PRIMARY, P1, NO) < MonthHigh_I(ALL_VENUES, 1, P1, NO) ) OR
( Last(ALL_VENUES, CURRENT, NO) - ClosePrice(PRIMARY, P1, NO) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 1.0
) AND
(
  ( ( MinuteHigh_I(ALL_VENUES, 5, P1, NO, False) - MinuteLow_I(ALL_VENUES, 5, P1, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.40 AND
( MinuteHigh_I(ALL_VENUES, 5, P1, NO, False) - MinuteLow_I(ALL_VENUES, 5, P1, NO, False) ) / ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.15 AND
MinuteClose_I(ALL_VENUES, 5, P1, NO) > MinuteOpen_I(ALL_VENUES, 5, P1, NO) AND
( MinuteVolume(ALL_VENUES, 5, P1, NO) >= MinuteVolume(ALL_VENUES, 5, P2, NO) * 2.0 OR
MinuteVolume(ALL_VENUES, 5, P1, NO) >= AvgDayVolume(ALL_VENUES, 20, NO) * 0.026 ) ) OR
( ( Last(ALL_VENUES, CURRENT, NO) - MinuteLow_I(ALL_VENUES, 15, CURRENT, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.60 AND
( Last(ALL_VENUES, CURRENT, NO) - MinuteLow_I(ALL_VENUES, 15, CURRENT, NO, False) ) / ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.15 AND
( MinuteVolume(ALL_VENUES, 15, CURRENT, NO) >= MinuteVolume(ALL_VENUES, 15, P1, NO) * 2.0 OR
MinuteVolume(ALL_VENUES, 15, CURRENT, NO) >= AvgDayVolume(ALL_VENUES, 20, NO) * 0.077 ) )
) AND
( MinuteHigh_I(ALL_VENUES, 1, P1, NO, False) - MinuteLow_I(ALL_VENUES, 1, P1, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) <= 0.25 AND
( MinuteHigh_I(ALL_VENUES, 1, P2, NO, False) - MinuteLow_I(ALL_VENUES, 1, P2, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) <= 0.25 AND
MaxTriggersCount(MINUTES, 5, 1)
```

## Breakout_Hold_Down

```
DayVolume(ALL_VENUES, 1, CURRENT, NO) >= 1000000 AND
Last(ALL_VENUES, CURRENT, NO) >= 10 AND
( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.50 AND
RVOL20DAYTIMEADJUSTED >= 2.0 AND
Last(ALL_VENUES, CURRENT, NO) <= ClosePrice(PRIMARY, P1, NO) AND
(
  ( Last(ALL_VENUES, CURRENT, NO) <= MonthLow_I(ALL_VENUES, 1, P1, NO) AND
ClosePrice(PRIMARY, P1, NO) > MonthLow_I(ALL_VENUES, 1, P1, NO) ) OR
( ClosePrice(PRIMARY, P1, NO) - Last(ALL_VENUES, CURRENT, NO) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 1.0
) AND
(
  ( ( MinuteHigh_I(ALL_VENUES, 5, P1, NO, False) - MinuteLow_I(ALL_VENUES, 5, P1, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.40 AND
( MinuteHigh_I(ALL_VENUES, 5, P1, NO, False) - MinuteLow_I(ALL_VENUES, 5, P1, NO, False) ) / ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.15 AND
MinuteClose_I(ALL_VENUES, 5, P1, NO) < MinuteOpen_I(ALL_VENUES, 5, P1, NO) AND
( MinuteVolume(ALL_VENUES, 5, P1, NO) >= MinuteVolume(ALL_VENUES, 5, P2, NO) * 2.0 OR
MinuteVolume(ALL_VENUES, 5, P1, NO) >= AvgDayVolume(ALL_VENUES, 20, NO) * 0.026 ) ) OR
( ( MinuteHigh_I(ALL_VENUES, 15, CURRENT, NO, False) - Last(ALL_VENUES, CURRENT, NO) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) >= 0.60 AND
( MinuteHigh_I(ALL_VENUES, 15, CURRENT, NO, False) - Last(ALL_VENUES, CURRENT, NO) ) / ( DayHigh(ALL_VENUES, 1, CURRENT, NO) - DayLow(ALL_VENUES, 1, CURRENT, NO) ) >= 0.15 AND
( MinuteVolume(ALL_VENUES, 15, CURRENT, NO) >= MinuteVolume(ALL_VENUES, 15, P1, NO) * 2.0 OR
MinuteVolume(ALL_VENUES, 15, CURRENT, NO) >= AvgDayVolume(ALL_VENUES, 20, NO) * 0.077 ) )
) AND
( MinuteHigh_I(ALL_VENUES, 1, P1, NO, False) - MinuteLow_I(ALL_VENUES, 1, P1, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) <= 0.25 AND
( MinuteHigh_I(ALL_VENUES, 1, P2, NO, False) - MinuteLow_I(ALL_VENUES, 1, P2, NO, False) ) / ATRStock(ALL_VENUES, NO, DAILY, 14, CURRENT) <= 0.25 AND
MaxTriggersCount(MINUTES, 5, 1)
```

## PM_LowFloat_Coil_Up

```
Float <= 10000000 AND
AdjustedClosePrice(P1) >= 1.00 AND
AdjustedClosePrice(P1) <= 50 AND
PremarketDayVolume(CURRENT) >= 100000 AND
PM_RVOL >= 5.0 AND
( Last(ALL_VENUES, CURRENT, YES) - ClosePrice(PRIMARY, P1, NO) ) / ClosePrice(PRIMARY, P1, NO) >= 0.15 AND
( PremarketDayHigh(CURRENT) - PremarketDayLow(CURRENT) ) / ClosePrice(PRIMARY, P1, NO) >= 0.05 AND
Last(ALL_VENUES, CURRENT, YES) >= VWAP(CURRENT, YES, VWAP, V1, ALL_VENUES) AND
Last(ALL_VENUES, CURRENT, YES) >= PremarketDayHigh(CURRENT) - 0.25 * ( PremarketDayHigh(CURRENT) - PremarketDayLow(CURRENT) ) AND
( MinuteHigh_I(ALL_VENUES, 1, CURRENT, YES, False) - MinuteLow_I(ALL_VENUES, 1, CURRENT, YES, False) ) / ( PremarketDayHigh(CURRENT) - PremarketDayLow(CURRENT) ) <= 0.15 AND
( MinuteHigh_I(ALL_VENUES, 1, P1, YES, False) - MinuteLow_I(ALL_VENUES, 1, P1, YES, False) ) / ( PremarketDayHigh(CURRENT) - PremarketDayLow(CURRENT) ) <= 0.15 AND
MaxTriggersCount(MINUTES, 5, 1)
```
