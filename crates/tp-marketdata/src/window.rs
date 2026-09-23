//! Rolling windows over time — the ring buffers the scanner's rolling measurements read from.
//!
//! Windows are **time spans, not item counts**: a 5-minute window over a symbol that stopped
//! trading decays to empty as the clock moves, whereas a "last 5 bars" window would silently
//! stretch across a halt.
//!
//! ## Membership rule
//!
//! Stamp each item with the **end** of the interval it covers (a bar's start plus its
//! resolution). The window at time `now` holds every item whose end is after `now - span`, i.e.
//! every item that overlaps `(now - span, now]`. For one-minute bars:
//! at 09:35:00 a 5-minute window holds the bars that started 09:30…09:34 (ends 09:31…09:35); at
//! 09:35:30 it still holds all five, because the 09:30 bar ends inside the window. The error at
//! the edge is at most one item's width, which is the ±1-minute figure in the investigation.
//!
//! `now` is the later of the newest pushed stamp and the latest [`advance`](RollingSum::advance)
//! call.

use crate::time::Ts;
use std::collections::VecDeque;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowError {
    /// Items must be pushed in non-decreasing stamp order.
    OutOfOrder { last: Ts, got: Ts },
    /// NaN cannot be ordered, so it is refused rather than allowed to wedge a max/min window.
    Unordered,
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowError::OutOfOrder { last, got } => {
                write!(f, "item at {}ms pushed after {}ms", got.millis(), last.millis())
            }
            WindowError::Unordered => write!(f, "value is not comparable (NaN)"),
        }
    }
}

impl std::error::Error for WindowError {}

/// The clock and ordering bookkeeping shared by every window kind.
#[derive(Debug, Clone)]
struct Clock {
    span_ms: i64,
    now: Option<Ts>,
    last_push: Option<Ts>,
}

impl Clock {
    fn new(span_ms: i64) -> Self {
        assert!(span_ms > 0, "window span must be positive");
        Clock { span_ms, now: None, last_push: None }
    }

    fn check_push(&mut self, ts: Ts) -> Result<(), WindowError> {
        if let Some(last) = self.last_push {
            if ts < last {
                return Err(WindowError::OutOfOrder { last, got: ts });
            }
        }
        self.last_push = Some(ts);
        self.bump(ts);
        Ok(())
    }

    fn bump(&mut self, ts: Ts) {
        if self.now.is_none_or(|n| ts > n) {
            self.now = Some(ts);
        }
    }

    /// Items stamped at or before this are out of the window.
    fn cutoff(&self) -> Option<i64> {
        self.now.map(|n| n.millis() - self.span_ms)
    }
}

/// Sum of values over a trailing time span, such as rolling volume.
///
/// Maintained incrementally and fully re-summed every `4 × len` removals, so floating-point
/// drift from repeated add/subtract stays bounded.
#[derive(Debug, Clone)]
pub struct RollingSum {
    clock: Clock,
    items: VecDeque<(Ts, f64)>,
    sum: f64,
    removals_since_resum: usize,
}

impl RollingSum {
    pub fn new(span_ms: i64) -> Self {
        RollingSum { clock: Clock::new(span_ms), items: VecDeque::new(), sum: 0.0, removals_since_resum: 0 }
    }

    pub fn push(&mut self, end: Ts, v: f64) -> Result<(), WindowError> {
        if v.is_nan() {
            return Err(WindowError::Unordered);
        }
        self.clock.check_push(end)?;
        self.items.push_back((end, v));
        self.sum += v;
        self.evict();
        Ok(())
    }

    /// Moves the clock forward without adding anything. A no-op if `now` is not later.
    pub fn advance(&mut self, now: Ts) {
        self.clock.bump(now);
        self.evict();
    }

    fn evict(&mut self) {
        let Some(cut) = self.clock.cutoff() else { return };
        while let Some(&(t, v)) = self.items.front() {
            if t.millis() > cut {
                break;
            }
            self.items.pop_front();
            self.sum -= v;
            self.removals_since_resum += 1;
        }
        if self.items.is_empty() {
            self.sum = 0.0;
            self.removals_since_resum = 0;
        } else if self.removals_since_resum >= 4 * self.items.len() {
            self.sum = self.items.iter().map(|&(_, v)| v).sum();
            self.removals_since_resum = 0;
        }
    }

    /// The sum over the window; 0 when empty.
    pub fn value(&self) -> f64 {
        self.sum
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Which extreme a [`RollingExtreme`] tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extreme {
    Max,
    Min,
}

/// The maximum or minimum over a trailing time span, such as a rolling 15-minute high. Uses a
/// monotonic deque: amortised O(1) per push, and it holds only the values that can still become
/// the extreme.
///
/// Ties keep the **newer** item, so the extreme leaves the window as late as possible.
#[derive(Debug, Clone)]
pub struct RollingExtreme<T> {
    clock: Clock,
    kind: Extreme,
    deque: VecDeque<(Ts, T)>,
}

impl<T: PartialOrd + Copy> RollingExtreme<T> {
    pub fn new(kind: Extreme, span_ms: i64) -> Self {
        RollingExtreme { clock: Clock::new(span_ms), kind, deque: VecDeque::new() }
    }

    pub fn max(span_ms: i64) -> Self {
        Self::new(Extreme::Max, span_ms)
    }

    pub fn min(span_ms: i64) -> Self {
        Self::new(Extreme::Min, span_ms)
    }

    /// True when `old` can never again be the extreme once `new` is in the window.
    fn dominated(&self, old: &T, new: &T) -> bool {
        match self.kind {
            Extreme::Max => old <= new,
            Extreme::Min => old >= new,
        }
    }

    pub fn push(&mut self, end: Ts, v: T) -> Result<(), WindowError> {
        if v.partial_cmp(&v).is_none() {
            return Err(WindowError::Unordered);
        }
        self.clock.check_push(end)?;
        while let Some((_, back)) = self.deque.back() {
            if self.dominated(back, &v) {
                self.deque.pop_back();
            } else {
                break;
            }
        }
        self.deque.push_back((end, v));
        self.evict();
        Ok(())
    }

    pub fn advance(&mut self, now: Ts) {
        self.clock.bump(now);
        self.evict();
    }

    fn evict(&mut self) {
        let Some(cut) = self.clock.cutoff() else { return };
        while let Some(&(t, _)) = self.deque.front() {
            if t.millis() > cut {
                break;
            }
            self.deque.pop_front();
        }
    }

    /// The extreme over the window; `None` when the window is empty.
    pub fn value(&self) -> Option<T> {
        self.deque.front().map(|&(_, v)| v)
    }

    /// When the current extreme was stamped. Useful for "new high" events.
    pub fn value_at(&self) -> Option<Ts> {
        self.deque.front().map(|&(t, _)| t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::tests::at;

    const MIN: i64 = 60_000;

    #[test]
    fn five_minute_sum_follows_the_membership_rule() {
        let mut w = RollingSum::new(5 * MIN);
        // One-minute bars 09:30..09:34, stamped by their ends 09:31..09:35.
        for (i, m) in (31..=35).enumerate() {
            w.push(at(9, m, 0), (i + 1) as f64).unwrap();
        }
        assert_eq!(w.value(), 15.0);
        w.advance(at(9, 35, 30));
        assert_eq!(w.value(), 15.0, "09:30 bar still overlaps (09:30:30, 09:35:30]");
        w.advance(at(9, 36, 0));
        assert_eq!(w.value(), 14.0, "09:30 bar (end 09:31) has now left");
    }

    #[test]
    fn a_quiet_symbol_decays_to_empty() {
        let mut w = RollingSum::new(5 * MIN);
        w.push(at(9, 31, 0), 100.0).unwrap();
        w.advance(at(9, 50, 0));
        assert!(w.is_empty());
        assert_eq!(w.value(), 0.0);
    }

    #[test]
    fn rolling_max_and_min_track_across_eviction() {
        let mut hi = RollingExtreme::<f64>::max(3 * MIN);
        let mut lo = RollingExtreme::<f64>::min(3 * MIN);
        let seq = [(31, 5.0), (32, 9.0), (33, 7.0), (34, 6.0), (35, 8.0)];
        let mut highs = vec![];
        let mut lows = vec![];
        for (m, v) in seq {
            hi.push(at(9, m, 0), v).unwrap();
            lo.push(at(9, m, 0), v).unwrap();
            highs.push(hi.value().unwrap());
            lows.push(lo.value().unwrap());
        }
        assert_eq!(highs, vec![5.0, 9.0, 9.0, 9.0, 8.0]);
        assert_eq!(lows, vec![5.0, 5.0, 5.0, 6.0, 6.0]);
    }

    #[test]
    fn extreme_matches_brute_force_on_a_long_sequence() {
        // Deterministic pseudo-random walk; compare to a direct scan of the window.
        let span = 7 * MIN;
        let mut w = RollingExtreme::<f64>::max(span);
        let mut all: Vec<(i64, f64)> = vec![];
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let base = at(9, 30, 0).millis();
        for i in 0..2_000i64 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let v = (x % 1000) as f64;
            let t = base + i * 20_000;
            w.push(Ts::from_millis(t).unwrap(), v).unwrap();
            all.push((t, v));
            let expect = all
                .iter()
                .filter(|&&(s, _)| s > t - span)
                .map(|&(_, v)| v)
                .fold(f64::NEG_INFINITY, f64::max);
            assert_eq!(w.value().unwrap(), expect, "at step {i}");
        }
    }

    #[test]
    fn sum_stays_exact_over_many_evictions() {
        let mut w = RollingSum::new(10 * MIN);
        let base = at(4, 0, 0).millis();
        for i in 0..50_000i64 {
            w.push(Ts::from_millis(base + i * 1_000).unwrap(), 0.1).unwrap();
        }
        // 600 one-second items in a 10-minute window.
        assert_eq!(w.len(), 600);
        assert!((w.value() - 60.0).abs() < 1e-9, "{}", w.value());
    }

    #[test]
    fn out_of_order_and_nan_are_refused() {
        let mut w = RollingSum::new(MIN);
        w.push(at(9, 31, 0), 1.0).unwrap();
        assert!(w.push(at(9, 30, 0), 1.0).is_err());
        assert!(w.push(at(9, 31, 0), 1.0).is_ok(), "equal stamps are allowed");
        assert_eq!(w.push(at(9, 32, 0), f64::NAN), Err(WindowError::Unordered));
        let mut m = RollingExtreme::<f64>::max(MIN);
        assert_eq!(m.push(at(9, 31, 0), f64::NAN), Err(WindowError::Unordered));
    }
}
