//! The session clock.
//!
//! Every session boundary in this crate is **America/New_York wall time**, derived from a UTC
//! instant. The machine's local clock is never consulted: the scanner's operator is in Puerto Rico
//! (UTC-4 all year, no DST), so from November to March their local time runs an hour ahead of the
//! market's, and any "local time" shortcut would put the open at the wrong bar for half the year.
//!
//! Known limitation, stated rather than hidden: [`Phase`] is wall-clock only. It knows nothing of
//! holidays or early closes, so on a 13:00 half-day the 13:00–16:00 bars would classify as
//! [`Phase::Rth`]. A market calendar is needed before live use; nothing in this slice depends on it.

use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use chrono_tz::America::New_York;
use chrono_tz::Tz;
use std::fmt;

/// An instant, as milliseconds since the Unix epoch (UTC). The crate's only timestamp type.
///
/// Construction is checked so every `Ts` is representable as a calendar date, which is what lets
/// [`Ts::ny`] be infallible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ts(i64);

impl Ts {
    /// `None` when `ms` lies outside chrono's representable range (hundreds of millennia away).
    pub fn from_millis(ms: i64) -> Option<Ts> {
        DateTime::from_timestamp_millis(ms).map(|_| Ts(ms))
    }

    /// Milliseconds since the Unix epoch.
    pub const fn millis(self) -> i64 {
        self.0
    }

    /// This instant on the New York wall clock.
    pub fn ny(self) -> DateTime<Tz> {
        DateTime::from_timestamp_millis(self.0)
            .expect("Ts is range-checked at construction")
            .with_timezone(&New_York)
    }

    /// The instant at which New York wall time reads `date time`.
    ///
    /// Errors when that wall time does not exist (the skipped hour at the spring DST change) or
    /// happens twice (the repeated hour in autumn). Market hours never fall in either, so an error
    /// here almost always means the input is not New York time.
    pub fn from_ny_local(date: NaiveDate, time: NaiveTime) -> Result<Ts, TimeError> {
        let local = NaiveDateTime::new(date, time);
        match New_York.from_local_datetime(&local) {
            LocalResult::Single(t) => Ok(Ts(t.timestamp_millis())),
            LocalResult::None => Err(TimeError::NonexistentLocalTime(local)),
            LocalResult::Ambiguous(..) => Err(TimeError::AmbiguousLocalTime(local)),
        }
    }

    /// This instant shifted by `ms` milliseconds; `None` if that leaves the representable range.
    pub fn plus_millis(self, ms: i64) -> Option<Ts> {
        self.0.checked_add(ms).and_then(Ts::from_millis)
    }
}

/// A New York calendar date — the date a session belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionDate(pub NaiveDate);

impl SessionDate {
    /// The New York date on which `ts` falls. A 19:30 post-market bar in January is 00:30 UTC the
    /// *next* day; it still belongs to the session it traded in, and this returns that session.
    pub fn of(ts: Ts) -> SessionDate {
        SessionDate(ts.ny().date_naive())
    }
}

impl fmt::Display for SessionDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Where an instant falls in the trading day, by New York wall clock. Each range includes its
/// start and excludes its end:
///
/// | phase    | New York time   |
/// |----------|-----------------|
/// | `Pre`    | 04:00 – 09:30   |
/// | `Rth`    | 09:30 – 16:00   |
/// | `Post`   | 16:00 – 20:00   |
/// | `Closed` | everything else |
///
/// A bar is classified by its **start** time, so the 09:30 one-minute bar is `Rth` and the 09:29
/// bar is `Pre`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    Pre,
    Rth,
    Post,
    Closed,
}

impl Phase {
    pub fn of(ts: Ts) -> Phase {
        let t = ts.ny().time();
        let hm = |h, m| NaiveTime::from_hms_opt(h, m, 0).expect("valid constant time");
        if t < hm(4, 0) {
            Phase::Closed
        } else if t < hm(9, 30) {
            Phase::Pre
        } else if t < hm(16, 0) {
            Phase::Rth
        } else if t < hm(20, 0) {
            Phase::Post
        } else {
            Phase::Closed
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeError {
    NonexistentLocalTime(NaiveDateTime),
    AmbiguousLocalTime(NaiveDateTime),
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeError::NonexistentLocalTime(t) => {
                write!(f, "{t} does not exist in New York (DST gap)")
            }
            TimeError::AmbiguousLocalTime(t) => {
                write!(f, "{t} happens twice in New York (DST overlap)")
            }
        }
    }
}

impl std::error::Error for TimeError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ny(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> Ts {
        Ts::from_ny_local(
            NaiveDate::from_ymd_opt(y, mo, d).unwrap(),
            NaiveTime::from_hms_opt(h, mi, 0).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn the_open_moves_in_utc_across_dst_but_not_in_new_york() {
        // DST began Sunday 2026-03-08. Friday's open is 14:30 UTC, Monday's is 13:30 UTC.
        let fri = ny(2026, 3, 6, 9, 30);
        let mon = ny(2026, 3, 9, 9, 30);
        assert_eq!(fri.ny().naive_utc().format("%H:%M").to_string(), "14:30");
        assert_eq!(mon.ny().naive_utc().format("%H:%M").to_string(), "13:30");
        assert_eq!(Phase::of(fri), Phase::Rth);
        assert_eq!(Phase::of(mon), Phase::Rth);
    }

    #[test]
    fn phase_boundaries_include_start_exclude_end() {
        let d = (2026, 6, 15);
        assert_eq!(Phase::of(ny(d.0, d.1, d.2, 3, 59)), Phase::Closed);
        assert_eq!(Phase::of(ny(d.0, d.1, d.2, 4, 0)), Phase::Pre);
        assert_eq!(Phase::of(ny(d.0, d.1, d.2, 9, 29)), Phase::Pre);
        assert_eq!(Phase::of(ny(d.0, d.1, d.2, 9, 30)), Phase::Rth);
        assert_eq!(Phase::of(ny(d.0, d.1, d.2, 15, 59)), Phase::Rth);
        assert_eq!(Phase::of(ny(d.0, d.1, d.2, 16, 0)), Phase::Post);
        assert_eq!(Phase::of(ny(d.0, d.1, d.2, 19, 59)), Phase::Post);
        assert_eq!(Phase::of(ny(d.0, d.1, d.2, 20, 0)), Phase::Closed);
    }

    #[test]
    fn winter_post_market_stays_in_its_session_despite_crossing_utc_midnight() {
        // 19:30 EST is 00:30 UTC the next day.
        let t = ny(2026, 1, 15, 19, 30);
        assert_eq!(t.ny().naive_utc().date(), NaiveDate::from_ymd_opt(2026, 1, 16).unwrap());
        assert_eq!(SessionDate::of(t).0, NaiveDate::from_ymd_opt(2026, 1, 15).unwrap());
        assert_eq!(Phase::of(t), Phase::Post);
    }

    #[test]
    fn dst_gap_and_overlap_are_errors_not_guesses() {
        let gap = Ts::from_ny_local(
            NaiveDate::from_ymd_opt(2026, 3, 8).unwrap(),
            NaiveTime::from_hms_opt(2, 30, 0).unwrap(),
        );
        assert!(matches!(gap, Err(TimeError::NonexistentLocalTime(_))));
        let overlap = Ts::from_ny_local(
            NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            NaiveTime::from_hms_opt(1, 30, 0).unwrap(),
        );
        assert!(matches!(overlap, Err(TimeError::AmbiguousLocalTime(_))));
    }

    #[test]
    fn out_of_range_millis_are_rejected() {
        assert!(Ts::from_millis(i64::MAX).is_none());
        assert!(Ts::from_millis(0).is_some());
    }
}
