//! Session VWAP with standard-deviation bands.
//!
//! A **faithful port** of `vwapBands` in TradierPro's `src/App.tsx` (commit 8e8be1d), the version
//! drawn on the user's charts and checked against TOS. It has two properties worth knowing, one
//! deliberately changed and one deliberately kept:
//!
//! 1. **Session reset: changed.** The TS resets when the bar's *UTC* date changes; this resets
//!    on the *New York session* date. For every RTH and premarket bar the two agree. They differ
//!    only for winter post-market bars after 19:00 New York, which are past UTC midnight. The TS
//!    resets there, and because those bars are extended hours it then draws nothing. This port
//!    keeps carrying the session's VWAP. `tests::winter_post_market_keeps_the_session_vwap` pins
//!    the difference.
//! 2. **Variance: kept.** Each bar adds `vol × (tp − vwap_now)²`, measured against the VWAP
//!    *as of that bar*, not the session's final VWAP. It's a running approximation rather than
//!    the textbook volume-weighted variance, and it's kept because it's what the user's charts
//!    (and their TOS comparison) show. Changing it is a decision for the user, not something to
//!    slip into a port.
//!
//! Extended-hours bars (any phase but RTH) are excluded from the accumulation, so VWAP starts at
//! the 09:30 bar. Before any RTH volume has printed, an extended bar yields `None` (the TS returns
//! NaN, which draws nothing) and a zero-volume RTH bar yields its own typical price with zero
//! width.

use crate::bar::Bar;
use crate::price::{Basis, Price};
use crate::time::{Phase, SessionDate};

/// VWAP and the standard deviation its bands are drawn at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VwapBands<B: Basis> {
    pub vwap: Price<B>,
    pub sd: Price<B>,
}

impl<B: Basis> VwapBands<B> {
    /// `vwap + k·sd` (k = 1 and 2 are the chart's bands).
    pub fn upper(&self, k: f64) -> Price<B> {
        self.vwap + self.sd * k
    }

    /// `vwap − k·sd`.
    pub fn lower(&self, k: f64) -> Price<B> {
        self.vwap - self.sd * k
    }
}

/// Incremental session VWAP. Feed bars in time order; one output per bar.
#[derive(Debug, Clone, Default)]
pub struct SessionVwap {
    session: Option<SessionDate>,
    cum_tpv: f64,
    cum_vol: f64,
    cum_var: f64,
}

impl SessionVwap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update<B: Basis>(&mut self, bar: &Bar<B>) -> Option<VwapBands<B>> {
        let date = bar.session_date();
        if self.session != Some(date) {
            *self = SessionVwap { session: Some(date), ..SessionVwap::default() };
        }
        let is_ext = bar.phase() != Phase::Rth;
        let tp = bar.typical_price().value();
        let vol = if bar.volume.is_nan() { 0.0 } else { bar.volume };
        if !is_ext && vol > 0.0 {
            self.cum_tpv += tp * vol;
            self.cum_vol += vol;
        }
        let v = if self.cum_vol > 0.0 {
            self.cum_tpv / self.cum_vol
        } else if is_ext {
            return None;
        } else {
            tp
        };
        if !is_ext && vol > 0.0 && self.cum_vol > 0.0 {
            self.cum_var += vol * (tp - v).powi(2);
        }
        let sd = if self.cum_vol > 0.0 { (self.cum_var / self.cum_vol).sqrt() } else { 0.0 };
        Some(VwapBands { vwap: Price::new(v), sd: Price::new(sd) })
    }
}

/// Batch form: one output per input bar.
pub fn vwap_bands<B: Basis>(bars: &[Bar<B>]) -> Vec<Option<VwapBands<B>>> {
    let mut s = SessionVwap::new();
    bars.iter().map(|b| s.update(b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::tests::{at, bar};
    use crate::price::Unadjusted;
    use crate::time::Ts;
    use chrono::{NaiveDate, NaiveTime};

    type B = Bar<Unadjusted>;

    #[test]
    fn premarket_is_excluded_and_draws_nothing() {
        let bars: Vec<B> = vec![
            bar(at(9, 0, 0), 5.0, 5.0, 5.0, 5.0, 1_000_000.0),
            bar(at(9, 30, 0), 10.0, 10.0, 10.0, 10.0, 100.0),
        ];
        let out = vwap_bands(&bars);
        assert!(out[0].is_none());
        assert_eq!(out[1].unwrap().vwap.value(), 10.0);
    }

    #[test]
    fn volume_weighting_and_bands() {
        let bars: Vec<B> = vec![
            bar(at(9, 30, 0), 10.0, 10.0, 10.0, 10.0, 100.0),
            bar(at(9, 31, 0), 12.0, 12.0, 12.0, 12.0, 300.0),
        ];
        let out = vwap_bands(&bars);
        let b = out[1].unwrap();
        assert_eq!(b.vwap.value(), 11.5);
        // cum_var = 100·0² (first bar, vs its own vwap 10) + 300·(12−11.5)² = 75; sd = √(75/400)
        assert!((b.sd.value() - (75.0f64 / 400.0).sqrt()).abs() < 1e-12);
        assert!((b.upper(2.0).value() - (11.5 + 2.0 * b.sd.value())).abs() < 1e-12);
    }

    #[test]
    fn zero_volume_rth_bar_before_any_volume_is_its_own_typical_price() {
        let out = vwap_bands::<Unadjusted>(&[bar(at(9, 30, 0), 9.0, 12.0, 9.0, 12.0, 0.0)]);
        let b = out[0].unwrap();
        assert_eq!(b.vwap.value(), 11.0);
        assert_eq!(b.sd.value(), 0.0);
    }

    #[test]
    fn resets_each_session() {
        let next = Ts::from_ny_local(
            NaiveDate::from_ymd_opt(2026, 6, 16).unwrap(),
            NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
        )
        .unwrap();
        let bars: Vec<B> = vec![bar(at(9, 30, 0), 10.0, 10.0, 10.0, 10.0, 100.0), bar(next, 50.0, 50.0, 50.0, 50.0, 1.0)];
        assert_eq!(vwap_bands(&bars)[1].unwrap().vwap.value(), 50.0);
    }

    #[test]
    fn winter_post_market_keeps_the_session_vwap() {
        // The deliberate difference from the TS: 19:30 EST is past UTC midnight, where the TS
        // resets and draws nothing. Here it belongs to the same session and carries the VWAP.
        let t = |h, m| {
            Ts::from_ny_local(NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(), NaiveTime::from_hms_opt(h, m, 0).unwrap())
                .unwrap()
        };
        let bars: Vec<B> = vec![bar(t(9, 30), 10.0, 10.0, 10.0, 10.0, 100.0), bar(t(19, 30), 11.0, 11.0, 11.0, 11.0, 5.0)];
        assert_eq!(vwap_bands(&bars)[1].unwrap().vwap.value(), 10.0);
    }
}
