//! Session volume profile: the histogram, POC and value area.
//!
//! A **faithful port** of `sessionHistogram` / `pocBinOf` / `binCenter` / `calcPOC` /
//! `calcValueArea` in TradierPro's `src/App.tsx` (commit 8e8be1d), which were checked against TOS.
//! The port includes that code's known weakness: buckets are anchored to the session's own min
//! and max, so a new session extreme re-bins every earlier bar. Fixing that is its own piece of
//! work (TradierPro `scratch/PROFILE_PLAN.md`), and doing it here would make this port disagree
//! with the chart the user trades from. `tools/parity` checks the port against the TS
//! numerically.
//!
//! These are batch functions over a session's bars, because re-binning makes them inherently
//! batch. Which bars to pass (RTH only, in TradierPro's usage) is the caller's decision.

use crate::bar::Bar;
use crate::price::{Basis, Price};

/// The TS default bucket count.
pub const DEFAULT_BUCKETS: usize = 60;

/// Fraction of session volume the value area encloses.
pub const VA_FRACTION: f64 = 0.70;

/// Volume by price row. Row `i` spans `[min_p + i·b_sz, min_p + (i+1)·b_sz)`.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionHistogram {
    pub min_p: f64,
    pub b_sz: f64,
    pub vols: Vec<f64>,
}

impl SessionHistogram {
    pub fn buckets(&self) -> usize {
        self.vols.len()
    }

    /// Build the histogram: `buckets` rows between the session low and high, with each bar's
    /// volume spread evenly across the rows its low..high touches.
    ///
    /// `None` when there are no bars, `buckets` is 0, or the session has no price range. Bars with
    /// zero or NaN volume contribute nothing. NaN prices are ignored when finding the range, and a
    /// bar whose row span comes out NaN is skipped (in the TS, `Math.max(0, NaN)` is NaN and the
    /// fill loop never runs; Rust's `max` would have returned 0 instead, so the skip is explicit).
    pub fn build<B: Basis>(bars: &[Bar<B>], buckets: usize) -> Option<SessionHistogram> {
        if bars.is_empty() || buckets < 1 {
            return None;
        }
        let mut min_p = f64::INFINITY;
        let mut max_p = f64::NEG_INFINITY;
        for b in bars {
            if b.low.value() < min_p {
                min_p = b.low.value();
            }
            if b.high.value() > max_p {
                max_p = b.high.value();
            }
        }
        if !min_p.is_finite() || max_p <= min_p {
            return None;
        }
        let b_sz = (max_p - min_p) / buckets as f64;
        let mut vols = vec![0.0; buckets];
        let last = (buckets - 1) as f64;
        for b in bars {
            let vol = b.volume;
            if vol == 0.0 || vol.is_nan() {
                continue;
            }
            let b1 = ((b.low.value() - min_p) / b_sz).floor();
            let b2 = ((b.high.value() - min_p) / b_sz).floor();
            if b1.is_nan() || b2.is_nan() {
                continue;
            }
            let b1 = b1.max(0.0);
            let b2 = b2.min(last);
            let sp = b2 - b1 + 1.0;
            let per = vol / sp;
            let mut i = b1;
            while i <= b2 {
                vols[i as usize] += per;
                i += 1.0;
            }
        }
        Some(SessionHistogram { min_p, b_sz, vols })
    }

    /// The row holding the most volume. Ties go to the lowest row.
    pub fn poc_bin(&self) -> usize {
        let mut max_vol = f64::NEG_INFINITY;
        let mut poc = 0;
        for (i, &v) in self.vols.iter().enumerate() {
            if v > max_vol {
                max_vol = v;
                poc = i;
            }
        }
        poc
    }

    /// The row's center price, rounded to the cent (`Math.round(x·100)/100`; half rounds up for
    /// positive prices under both JS and Rust rounding).
    pub fn bin_center(&self, i: usize) -> f64 {
        ((self.min_p + (i as f64 + 0.5) * self.b_sz) * 100.0).round() / 100.0
    }
}

/// Point of control: the center of the highest-volume row, using [`DEFAULT_BUCKETS`] rows.
pub fn poc<B: Basis>(bars: &[Bar<B>]) -> Option<Price<B>> {
    let h = SessionHistogram::build(bars, DEFAULT_BUCKETS)?;
    Some(Price::new(h.bin_center(h.poc_bin())))
}

/// Value-area bounds, as row centers (the same convention as [`poc`], so `val ≤ poc ≤ vah`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueArea<B: Basis> {
    pub vah: Price<B>,
    pub val: Price<B>,
}

/// The value area: start at the POC row, then repeatedly annex whichever **pair** of rows (the
/// next two above or the next two below) carries more volume, until the enclosed rows hold
/// [`VA_FRACTION`] of the session's volume.
///
/// - **Tie:** equal volume above and below annexes upward.
/// - **Edge:** once one side runs out, the other side takes the remainder alone.
pub fn value_area<B: Basis>(bars: &[Bar<B>]) -> Option<ValueArea<B>> {
    let h = SessionHistogram::build(bars, DEFAULT_BUCKETS)?;
    let total: f64 = h.vols.iter().sum();
    if total.is_nan() || total <= 0.0 {
        return None;
    }
    let target = total * VA_FRACTION;
    let n = h.buckets();
    let poc = h.poc_bin();
    let (mut lo, mut hi) = (poc, poc);
    let mut acc = h.vols[poc];
    while acc < target && (lo > 0 || hi < n - 1) {
        let up = if hi < n - 1 { h.vols[hi + 1] } else { 0.0 } + if hi + 2 < n { h.vols[hi + 2] } else { 0.0 };
        let dn = if lo > 0 { h.vols[lo - 1] } else { 0.0 } + if lo > 1 { h.vols[lo - 2] } else { 0.0 };
        if hi < n - 1 && (up >= dn || lo == 0) {
            hi = (hi + 2).min(n - 1);
        } else if lo > 0 {
            lo = lo.saturating_sub(2);
        } else {
            break;
        }
        // Re-summed from scratch each step, exactly as the TS does, so the float totals agree.
        acc = h.vols[lo..=hi].iter().fold(0.0, |a, &v| a + v);
    }
    Some(ValueArea { val: Price::new(h.bin_center(lo)), vah: Price::new(h.bin_center(hi)) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::tests::{at, bar};
    use crate::price::Unadjusted;

    type B = Bar<Unadjusted>;

    #[test]
    fn no_range_or_no_bars_means_no_profile() {
        assert!(poc::<Unadjusted>(&[]).is_none());
        let flat: B = bar(at(9, 30, 0), 5.0, 5.0, 5.0, 5.0, 100.0);
        assert!(poc(&[flat]).is_none());
    }

    #[test]
    fn heavy_row_is_the_poc() {
        let bars: Vec<B> = vec![
            bar(at(9, 30, 0), 10.0, 16.0, 10.0, 15.0, 100.0),
            // Heavy volume concentrated near 12.
            bar(at(9, 31, 0), 12.0, 12.05, 12.0, 12.0, 10_000.0),
        ];
        let p = poc(&bars).unwrap().value();
        assert!((p - 12.0).abs() <= 0.1, "poc {p}");
    }

    #[test]
    fn value_area_brackets_the_poc_and_holds_70_percent() {
        let mut bars: Vec<B> = vec![];
        for (i, (lo, hi, v)) in [(10.0, 11.0, 50.0), (11.0, 12.0, 300.0), (11.5, 12.5, 400.0), (12.0, 13.0, 200.0), (13.0, 14.0, 50.0)]
            .into_iter()
            .enumerate()
        {
            bars.push(bar(at(9, 30 + i as u32, 0), lo, hi, lo, hi, v));
        }
        let p = poc(&bars).unwrap();
        let va = value_area(&bars).unwrap();
        assert!(va.val <= p && p <= va.vah);
        let h = SessionHistogram::build(&bars, DEFAULT_BUCKETS).unwrap();
        let total: f64 = h.vols.iter().sum();
        let row = |x: f64| ((x - h.min_p) / h.b_sz - 0.5).round() as usize;
        let inside: f64 = h.vols[row(va.val.value())..=row(va.vah.value())].iter().sum();
        assert!(inside >= VA_FRACTION * total - 1e-9);
    }

    #[test]
    fn zero_volume_bars_still_set_the_range() {
        let bars: Vec<B> = vec![
            bar(at(9, 30, 0), 10.0, 20.0, 10.0, 20.0, 0.0),
            bar(at(9, 31, 0), 15.0, 15.1, 15.0, 15.0, 100.0),
        ];
        let h = SessionHistogram::build(&bars, DEFAULT_BUCKETS).unwrap();
        assert_eq!(h.min_p, 10.0);
        assert_eq!(h.vols.iter().sum::<f64>(), 100.0);
    }
}
