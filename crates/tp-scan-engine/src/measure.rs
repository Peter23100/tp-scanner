//! The measurement catalog: the named quantities a filter can read.
//!
//! Each variant has one exact meaning, stated here and tested. The list was taken from the
//! primitives the exported Primus filters actually use (`Last`, `MinuteHigh/Low/Volume`,
//! `ATRStock`, `ClosePrice(P1)`, `AvgDayVolume`, `DayVolume`, `VWAP`, `PreMarketDay*`,
//! `DayBar_Open`, `NewMinuteHigh/Low`), with the deliberate semantic changes from the
//! investigation: windows are rolling unless marked clock-aligned, and daily references are
//! adjusted onto today's basis.
//!
//! **Not here yet, on purpose:** `RVOL20DAYTIMEADJUSTED` (a time-of-day-adjusted relative volume
//! whose exact definition must come from the user, not a guess) and the imbalance fields (they need
//! an imbalance feed).

use crate::state::{SymbolState, Win};
use std::fmt;

/// A numeric measurement. Evaluates to `None` ("unknown") rather than inventing a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Measure {
    /// Close of the newest bar.
    Last,
    /// Session volume so far. `extended: false` counts RTH only.
    DayVolume { extended: bool },
    /// Highest high in the window. Unknown if the window has no bars.
    High(Win),
    /// Lowest low in the window. Unknown if the window has no bars.
    Low(Win),
    /// Volume in the window (0 if covered but quiet).
    Volume(Win),
    /// The prior session's close, adjusted onto today's basis. Unknown without a daily reference
    /// built for today.
    PrevClose,
    /// Wilder ATR over the daily history, adjusted onto today's basis. Same condition.
    Atr,
    /// Mean daily volume over the daily history. Same condition.
    AvgDailyVolume,
    /// Session VWAP (RTH-anchored). Unknown before the first RTH volume.
    Vwap,
    /// Open of the first RTH bar. Unknown before 09:30.
    SessionOpen,
    PremarketHigh,
    PremarketLow,
    PremarketVolume,
}

impl Measure {
    pub fn eval(&self, s: &SymbolState) -> Option<f64> {
        match *self {
            Measure::Last => s.last_bar().map(|b| b.close.value()),
            Measure::DayVolume { extended } => s.day_volume(extended),
            Measure::High(w) => s.window(w)?.high.map(|p| p.value()),
            Measure::Low(w) => s.window(w)?.low.map(|p| p.value()),
            Measure::Volume(w) => s.window(w).map(|a| a.volume),
            Measure::PrevClose => s.daily_for_today().map(|d| d.prev_close.value()),
            Measure::Atr => s.daily_for_today()?.atr.map(|a| a.value()),
            Measure::AvgDailyVolume => s.daily_for_today()?.adv,
            Measure::Vwap => s.vwap().map(|v| v.vwap.value()),
            Measure::SessionOpen => s.session_open().map(|p| p.value()),
            Measure::PremarketHigh => s.premarket_high().map(|p| p.value()),
            Measure::PremarketLow => s.premarket_low().map(|p| p.value()),
            Measure::PremarketVolume => s.premarket_volume(),
        }
    }
}

impl fmt::Display for Measure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Measure::Last => write!(f, "last"),
            Measure::DayVolume { extended: false } => write!(f, "day_volume"),
            Measure::DayVolume { extended: true } => write!(f, "day_volume_ext"),
            Measure::High(w) => write!(f, "high({w})"),
            Measure::Low(w) => write!(f, "low({w})"),
            Measure::Volume(w) => write!(f, "volume({w})"),
            Measure::PrevClose => write!(f, "prev_close"),
            Measure::Atr => write!(f, "atr"),
            Measure::AvgDailyVolume => write!(f, "adv"),
            Measure::Vwap => write!(f, "vwap"),
            Measure::SessionOpen => write!(f, "open"),
            Measure::PremarketHigh => write!(f, "pm_high"),
            Measure::PremarketLow => write!(f, "pm_low"),
            Measure::PremarketVolume => write!(f, "pm_volume"),
        }
    }
}

/// A true/false event about the newest bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// The newest bar's high beats every high in the preceding `span_ms` (Primus `NewMinuteHigh`).
    NewHigh { span_ms: i64 },
    /// The newest bar's low undercuts every low in the preceding `span_ms`.
    NewLow { span_ms: i64 },
}

impl Event {
    pub fn eval(&self, s: &SymbolState) -> Option<bool> {
        match *self {
            Event::NewHigh { span_ms } => s.new_extreme(span_ms, true),
            Event::NewLow { span_ms } => s.new_extreme(span_ms, false),
        }
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = |ms: &i64| ms / 60_000;
        match self {
            Event::NewHigh { span_ms } => write!(f, "new_high({}m)", m(span_ms)),
            Event::NewLow { span_ms } => write!(f, "new_low({}m)", m(span_ms)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tests::minutes;
    use crate::state::DailyRef;
    use tp_marketdata::price::Price;

    #[test]
    fn catalog_reads_state() {
        let s = minutes(10);
        assert_eq!(Measure::Last.eval(&s), Some(18.5));
        assert_eq!(Measure::High(Win::rolling_min(5)).eval(&s), Some(19.0));
        assert_eq!(Measure::Volume(Win::rolling_min_back(5, 1)).eval(&s), Some(1500.0));
        assert_eq!(Measure::SessionOpen.eval(&s), Some(9.5));
        assert_eq!(Measure::PremarketHigh.eval(&s), None);
        assert_eq!(Measure::PremarketVolume.eval(&s), Some(0.0));
    }

    #[test]
    fn daily_measures_need_todays_reference() {
        let mut s = minutes(2);
        assert_eq!(Measure::Atr.eval(&s), None);
        s.set_daily(DailyRef {
            session: s.session_date().unwrap(),
            prev_close: Price::new(9.0),
            atr: Some(Price::new(0.8)),
            atr_len: 14,
            adv: None,
            adv_len: 20,
        });
        assert_eq!(Measure::Atr.eval(&s), Some(0.8));
        assert_eq!(Measure::PrevClose.eval(&s), Some(9.0));
        assert_eq!(Measure::AvgDailyVolume.eval(&s), None, "short history stays unknown");
    }

    #[test]
    fn labels_are_readable() {
        assert_eq!(Measure::High(Win::rolling_min(15)).to_string(), "high(15m)");
        assert_eq!(Measure::Volume(Win::rolling_min_back(5, 1)).to_string(), "volume(5m, 5m back)");
        assert_eq!(Measure::Low(Win::clock_min(30, 0)).to_string(), "low(30m clock)");
    }
}
