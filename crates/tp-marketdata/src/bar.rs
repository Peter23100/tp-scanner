//! OHLCV bars, and rolling fine bars up into coarser ones.

use crate::price::{AdjFactor, Adjusted, Basis, Price, Unadjusted};
use crate::time::{Phase, SessionDate, Ts};
use std::fmt;

/// One OHLCV bar in basis `B`, stamped with the instant its interval **starts**.
///
/// Volume is `f64` because adjusted volume is fractional after an odd-ratio split.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar<B: Basis> {
    pub start: Ts,
    pub open: Price<B>,
    pub high: Price<B>,
    pub low: Price<B>,
    pub close: Price<B>,
    pub volume: f64,
}

impl<B: Basis> Bar<B> {
    /// The trading phase, classified by the bar's start (see [`Phase`]).
    pub fn phase(&self) -> Phase {
        Phase::of(self.start)
    }

    pub fn session_date(&self) -> SessionDate {
        SessionDate::of(self.start)
    }

    /// `(high + low + close) / 3`, evaluated in that order so it is bit-identical to the
    /// TradierPro chart's VWAP input.
    pub fn typical_price(&self) -> Price<B> {
        (self.high + self.low + self.close) / 3.0
    }

    pub fn range(&self) -> Price<B> {
        self.high - self.low
    }
}

impl Bar<Unadjusted> {
    /// Restate onto the adjusted basis. Prices scale by the factor and volume by its inverse, so
    /// dollar volume is unchanged: after a 2-for-1 split, pre-split bars show half the price and
    /// twice the shares.
    pub fn adjust(self, f: AdjFactor) -> Bar<Adjusted> {
        Bar {
            start: self.start,
            open: self.open.adjust(f),
            high: self.high.adjust(f),
            low: self.low.adjust(f),
            close: self.close.adjust(f),
            volume: self.volume / f.get(),
        }
    }
}

/// Rolls bars up into a coarser period, e.g. 1-second bars into 1-minute bars.
///
/// A bar belongs to the output period containing its start: `floor(start / period) × period`.
/// The period must divide one hour. New York's UTC offset is a whole number of hours, so under
/// that condition UTC period boundaries fall on New York clock boundaries too.
///
/// Input bars must each fit inside one output period (input resolution ≤ period and dividing it).
/// That is the caller's contract; the aggregator cannot check it without knowing the input
/// resolution.
///
/// Completed bars come out two ways: [`Aggregator::push`] emits the previous period when a bar
/// from a later period arrives, and [`Aggregator::close_through`] emits the current period once
/// the clock has passed its end. Live use needs the second; a quiet symbol might not print again
/// for minutes.
pub struct Aggregator<B: Basis> {
    period_ms: i64,
    cur: Option<Bar<B>>,
    last_input_start: Option<Ts>,
}

impl<B: Basis> Aggregator<B> {
    pub fn new(period_ms: i64) -> Result<Self, AggError> {
        if period_ms <= 0 || 3_600_000 % period_ms != 0 {
            return Err(AggError::BadPeriod(period_ms));
        }
        Ok(Aggregator { period_ms, cur: None, last_input_start: None })
    }

    fn bucket(&self, ts: Ts) -> Ts {
        let b = ts.millis().div_euclid(self.period_ms) * self.period_ms;
        Ts::from_millis(b).expect("flooring moves toward zero-ish and stays in range")
    }

    /// Adds a bar. Returns the previous period's completed bar if `bar` starts a new period.
    /// Input starts must be strictly increasing.
    pub fn push(&mut self, bar: Bar<B>) -> Result<Option<Bar<B>>, AggError> {
        if let Some(prev) = self.last_input_start {
            if bar.start <= prev {
                return Err(AggError::OutOfOrder { prev, got: bar.start });
            }
        }
        self.last_input_start = Some(bar.start);
        let bucket = self.bucket(bar.start);
        match &mut self.cur {
            Some(c) if c.start == bucket => {
                c.high = c.high.max(bar.high);
                c.low = c.low.min(bar.low);
                c.close = bar.close;
                c.volume += bar.volume;
                Ok(None)
            }
            _ => {
                let done = self.cur.take();
                self.cur = Some(Bar { start: bucket, ..bar });
                Ok(done)
            }
        }
    }

    /// Emits the in-progress bar if its period has ended by `now`, i.e. `start + period <= now`.
    pub fn close_through(&mut self, now: Ts) -> Option<Bar<B>> {
        let ends = self.cur.as_ref()?.start.millis() + self.period_ms;
        if ends <= now.millis() {
            self.cur.take()
        } else {
            None
        }
    }

    /// Emits the in-progress bar unconditionally (end of a replay).
    pub fn flush(&mut self) -> Option<Bar<B>> {
        self.cur.take()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggError {
    BadPeriod(i64),
    OutOfOrder { prev: Ts, got: Ts },
}

impl fmt::Display for AggError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AggError::BadPeriod(p) => write!(f, "period {p}ms must be > 0 and divide one hour"),
            AggError::OutOfOrder { prev, got } => {
                write!(f, "bar at {}ms arrived after {}ms", got.millis(), prev.millis())
            }
        }
    }
}

impl std::error::Error for AggError {}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};

    pub(crate) fn at(h: u32, m: u32, s: u32) -> Ts {
        Ts::from_ny_local(
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
            NaiveTime::from_hms_opt(h, m, s).unwrap(),
        )
        .unwrap()
    }

    pub(crate) fn bar<B: Basis>(start: Ts, o: f64, h: f64, l: f64, c: f64, v: f64) -> Bar<B> {
        Bar {
            start,
            open: Price::new(o),
            high: Price::new(h),
            low: Price::new(l),
            close: Price::new(c),
            volume: v,
        }
    }

    #[test]
    fn seconds_roll_into_a_minute() {
        let mut agg = Aggregator::<Unadjusted>::new(60_000).unwrap();
        assert_eq!(agg.push(bar(at(9, 30, 0), 10.0, 10.5, 9.9, 10.2, 100.0)).unwrap(), None);
        assert_eq!(agg.push(bar(at(9, 30, 17), 10.2, 11.0, 10.1, 10.8, 50.0)).unwrap(), None);
        assert_eq!(agg.push(bar(at(9, 30, 59), 10.8, 10.9, 9.5, 9.7, 25.0)).unwrap(), None);
        let m = agg.push(bar(at(9, 31, 3), 9.7, 9.8, 9.6, 9.6, 1.0)).unwrap().unwrap();
        assert_eq!(m, bar(at(9, 30, 0), 10.0, 11.0, 9.5, 9.7, 175.0));
        assert_eq!(agg.flush().unwrap().start, at(9, 31, 0));
    }

    #[test]
    fn close_through_emits_only_after_the_period_ends() {
        let mut agg = Aggregator::<Unadjusted>::new(60_000).unwrap();
        agg.push(bar(at(9, 30, 5), 1.0, 1.0, 1.0, 1.0, 1.0)).unwrap();
        assert!(agg.close_through(at(9, 30, 59)).is_none());
        assert_eq!(agg.close_through(at(9, 31, 0)).unwrap().start, at(9, 30, 0));
        assert!(agg.close_through(at(9, 40, 0)).is_none());
    }

    #[test]
    fn a_gap_emits_no_phantom_bars() {
        let mut agg = Aggregator::<Unadjusted>::new(60_000).unwrap();
        agg.push(bar(at(9, 30, 0), 1.0, 1.0, 1.0, 1.0, 1.0)).unwrap();
        let done = agg.push(bar(at(9, 45, 0), 2.0, 2.0, 2.0, 2.0, 1.0)).unwrap().unwrap();
        assert_eq!(done.start, at(9, 30, 0));
        assert_eq!(agg.flush().unwrap().start, at(9, 45, 0));
    }

    #[test]
    fn out_of_order_and_bad_periods_are_errors() {
        let mut agg = Aggregator::<Unadjusted>::new(60_000).unwrap();
        agg.push(bar(at(9, 31, 0), 1.0, 1.0, 1.0, 1.0, 1.0)).unwrap();
        assert!(agg.push(bar(at(9, 30, 0), 1.0, 1.0, 1.0, 1.0, 1.0)).is_err());
        assert!(agg.push(bar(at(9, 31, 0), 1.0, 1.0, 1.0, 1.0, 1.0)).is_err());
        assert!(Aggregator::<Unadjusted>::new(7_000).is_err());
        assert!(Aggregator::<Unadjusted>::new(0).is_err());
    }

    #[test]
    fn adjusting_preserves_dollar_volume() {
        let b: Bar<Unadjusted> = bar(at(9, 30, 0), 188.0, 190.0, 186.0, 189.0, 1000.0);
        let a = b.adjust(AdjFactor::from_split(4.0, 1.0).unwrap());
        assert_eq!(a.high.value(), 47.5);
        assert_eq!(a.volume, 4000.0);
        assert_eq!(a.close.value() * a.volume, b.close.value() * b.volume);
    }

    #[test]
    fn typical_price_matches_the_chart_formula() {
        let b: Bar<Unadjusted> = bar(at(9, 30, 0), 1.0, 10.1, 9.7, 9.9, 1.0);
        assert_eq!(b.typical_price().value(), (10.1 + 9.7 + 9.9) / 3.0);
    }
}
