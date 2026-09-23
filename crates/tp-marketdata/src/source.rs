//! The one door market data comes in through.
//!
//! [`MarketSource`] is the single input interface. Everything downstream (indicators, the
//! evaluator, instrumentation, the backtester) consumes it and never learns whether it is replaying
//! history or reading a live feed. [`ReplaySource`] is the first implementation; the live driver
//! (stage 6) will be the second. Keeping both behind one trait from the first commit is the only
//! structural guarantee that backtest and live can't quietly diverge.
//!
//! The trait is pull-based and synchronous on purpose: a replay then runs deterministically, and a
//! live driver can run its socket on its own thread and hand events over a channel.

use crate::bar::Bar;
use crate::price::{Price, Unadjusted};
use crate::time::Ts;
use chrono::{NaiveDate, NaiveTime};
use std::fmt;
use std::sync::Arc;

/// A ticker. Shared rather than cloned per event, because the same few thousand names repeat
/// every second.
pub type Symbol = Arc<str>;

/// The interval a bar covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Resolution {
    Second,
    Minute,
    Day,
}

impl Resolution {
    /// Width in milliseconds for the intraday resolutions. `None` for `Day`, whose width depends
    /// on the session.
    pub const fn millis(self) -> Option<i64> {
        match self {
            Resolution::Second => Some(1_000),
            Resolution::Minute => Some(60_000),
            Resolution::Day => None,
        }
    }
}

/// Something that happened in the market. Non-exhaustive: halts, trades and quotes join later
/// without breaking consumers.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MarketEvent {
    /// A completed bar, as the tape printed it (unadjusted).
    Bar { symbol: Symbol, resolution: Resolution, bar: Bar<Unadjusted> },
}

impl MarketEvent {
    /// When the event became knowable: a bar's **end**, not its start. A replay that exposes a bar
    /// at its start time has looked one bar ahead.
    pub fn available_at(&self) -> Ts {
        match self {
            MarketEvent::Bar { resolution, bar, .. } => {
                let w = resolution.millis().unwrap_or(0);
                bar.start.plus_millis(w).unwrap_or(bar.start)
            }
        }
    }
}

/// A stream of market events in time order.
pub trait MarketSource {
    /// The next event. `None` when the source is exhausted (end of a replay, or a live feed shut
    /// down). An `Err` reports a problem with one event; whether to continue is the caller's
    /// decision.
    fn next_event(&mut self) -> Option<Result<MarketEvent, SourceError>>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum SourceError {
    Parse { line: usize, msg: String },
    OutOfOrder { line: usize, msg: String },
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::Parse { line, msg } => write!(f, "line {line}: {msg}"),
            SourceError::OutOfOrder { line, msg } => write!(f, "line {line}: out of order: {msg}"),
        }
    }
}

impl std::error::Error for SourceError {}

/// Replays bars from a CSV fixture, in file order.
///
/// Format: a header line `symbol,start_ny,open,high,low,close,volume`, then one bar per line.
/// `start_ny` is New York wall time, `YYYY-MM-DD HH:MM:SS`, so fixtures can be written and read by
/// eye. Blank lines and lines starting with `#` are skipped.
///
/// The whole file is validated at construction, and any problem fails the load: bars must be
/// sane (`low ≤ open, close ≤ high`, all finite, volume ≥ 0) and start times must never decrease.
/// A replay that silently skips a bad row or reorders time is not a replay you can trust.
pub struct ReplaySource {
    events: std::vec::IntoIter<MarketEvent>,
}

const HEADER: &str = "symbol,start_ny,open,high,low,close,volume";

impl ReplaySource {
    pub fn from_csv(text: &str, resolution: Resolution) -> Result<Self, SourceError> {
        let mut events = Vec::new();
        let mut saw_header = false;
        let mut last: Option<Ts> = None;
        let mut interned: Vec<Symbol> = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let l = raw.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            if !saw_header {
                if l != HEADER {
                    return Err(SourceError::Parse { line, msg: format!("expected header `{HEADER}`") });
                }
                saw_header = true;
                continue;
            }
            let bar_fields: Vec<&str> = l.split(',').map(str::trim).collect();
            let [sym, start, o, h, lo, c, v] = bar_fields[..] else {
                return Err(SourceError::Parse { line, msg: format!("expected 7 fields, got {}", bar_fields.len()) });
            };
            if sym.is_empty() {
                return Err(SourceError::Parse { line, msg: "empty symbol".into() });
            }
            let start = parse_ny(start).map_err(|msg| SourceError::Parse { line, msg })?;
            let num = |s: &str, what: &str| -> Result<f64, SourceError> {
                let x: f64 = s
                    .parse()
                    .map_err(|_| SourceError::Parse { line, msg: format!("{what} `{s}` is not a number") })?;
                if !x.is_finite() {
                    return Err(SourceError::Parse { line, msg: format!("{what} is not finite") });
                }
                Ok(x)
            };
            let (o, h, lo, c, v) = (num(o, "open")?, num(h, "high")?, num(lo, "low")?, num(c, "close")?, num(v, "volume")?);
            if !(lo <= o && o <= h && lo <= c && c <= h) {
                return Err(SourceError::Parse { line, msg: format!("insane bar o={o} h={h} l={lo} c={c}") });
            }
            if v < 0.0 {
                return Err(SourceError::Parse { line, msg: "negative volume".into() });
            }
            if let Some(prev) = last {
                if start < prev {
                    return Err(SourceError::OutOfOrder {
                        line,
                        msg: format!("{}ms after {}ms", start.millis(), prev.millis()),
                    });
                }
            }
            last = Some(start);
            let symbol = match interned.iter().find(|s| &***s == sym) {
                Some(s) => s.clone(),
                None => {
                    let s: Symbol = Arc::from(sym);
                    interned.push(s.clone());
                    s
                }
            };
            events.push(MarketEvent::Bar {
                symbol,
                resolution,
                bar: Bar {
                    start,
                    open: Price::new(o),
                    high: Price::new(h),
                    low: Price::new(lo),
                    close: Price::new(c),
                    volume: v,
                },
            });
        }
        if !saw_header {
            return Err(SourceError::Parse { line: 0, msg: "empty file (no header)".into() });
        }
        Ok(ReplaySource { events: events.into_iter() })
    }
}

impl MarketSource for ReplaySource {
    fn next_event(&mut self) -> Option<Result<MarketEvent, SourceError>> {
        self.events.next().map(Ok)
    }
}

fn parse_ny(s: &str) -> Result<Ts, String> {
    let (d, t) = s.split_once(' ').ok_or_else(|| format!("start `{s}` is not `YYYY-MM-DD HH:MM:SS`"))?;
    let d = NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|e| format!("date `{d}`: {e}"))?;
    let t = NaiveTime::parse_from_str(t, "%H:%M:%S").map_err(|e| format!("time `{t}`: {e}"))?;
    Ts::from_ny_local(d, t).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = "\
# two symbols, one minute
symbol,start_ny,open,high,low,close,volume
AAPL,2026-06-15 09:30:00,200,201,199.5,200.5,1000
MSFT,2026-06-15 09:30:00,450,451,449,450.25,500

AAPL,2026-06-15 09:31:00,200.5,202,200.4,201.9,1200
";

    fn drain(mut s: impl MarketSource) -> Vec<MarketEvent> {
        let mut v = vec![];
        while let Some(e) = s.next_event() {
            v.push(e.unwrap());
        }
        v
    }

    #[test]
    fn replays_in_file_order() {
        let ev = drain(ReplaySource::from_csv(OK, Resolution::Minute).unwrap());
        assert_eq!(ev.len(), 3);
        let MarketEvent::Bar { symbol, bar, .. } = &ev[2];
        assert_eq!(&**symbol, "AAPL");
        assert_eq!(bar.close.value(), 201.9);
    }

    #[test]
    fn symbols_are_shared_not_copied() {
        let ev = drain(ReplaySource::from_csv(OK, Resolution::Minute).unwrap());
        let (MarketEvent::Bar { symbol: a, .. }, MarketEvent::Bar { symbol: b, .. }) = (&ev[0], &ev[2]);
        assert!(Arc::ptr_eq(a, b));
    }

    #[test]
    fn a_bar_is_available_at_its_end() {
        let ev = drain(ReplaySource::from_csv(OK, Resolution::Minute).unwrap());
        let MarketEvent::Bar { bar, .. } = &ev[0];
        assert_eq!(ev[0].available_at().millis() - bar.start.millis(), 60_000);
    }

    #[test]
    fn bad_rows_fail_the_whole_load() {
        let cases = [
            ("symbol,start_ny,open,high,low,close,volume\nX,2026-06-15 09:30:00,1,2,0.5,3,1\n", "close above high"),
            ("symbol,start_ny,open,high,low,close,volume\nX,2026-06-15 09:30:00,1,2,0.5,1,-1\n", "negative volume"),
            ("symbol,start_ny,open,high,low,close,volume\nX,2026-06-15 09:30:00,1,2,0.5,NaN,1\n", "NaN"),
            ("symbol,start_ny,open,high,low,close,volume\nX,2026-06-15 09:30:00,1,2,0.5\n", "short row"),
            ("symbol,start_ny,open,high,low,close,volume\nX,2026-03-08 02:30:00,1,1,1,1,1\n", "DST gap"),
            ("sym,time\n", "wrong header"),
            ("", "empty"),
        ];
        for (csv, why) in cases {
            assert!(ReplaySource::from_csv(csv, Resolution::Minute).is_err(), "{why} should fail");
        }
    }

    #[test]
    fn time_going_backward_fails_the_load() {
        let csv = "symbol,start_ny,open,high,low,close,volume\n\
                   X,2026-06-15 09:31:00,1,1,1,1,1\n\
                   X,2026-06-15 09:30:00,1,1,1,1,1\n";
        assert!(matches!(
            ReplaySource::from_csv(csv, Resolution::Minute),
            Err(SourceError::OutOfOrder { line: 3, .. })
        ));
    }
}
