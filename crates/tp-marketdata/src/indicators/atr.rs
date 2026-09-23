//! Average True Range (Wilder), over **adjusted** bars only.
//!
//! ATR compares each bar to the previous close, so it spans dates and must be computed on one
//! adjustment basis. Taking `Bar<Adjusted>` makes the incumbent's split bug (a $188 stock showing
//! a $53 ATR) impossible to write: unadjusted bars don't typecheck here.

use crate::bar::Bar;
use crate::price::{Adjusted, Price};

/// True range: `max(high − low, |high − prev_close|, |low − prev_close|)`, or just `high − low`
/// when there is no previous close (the first bar).
pub fn true_range(bar: &Bar<Adjusted>, prev_close: Option<Price<Adjusted>>) -> Price<Adjusted> {
    let hl = bar.high - bar.low;
    match prev_close {
        None => hl,
        Some(pc) => hl.max((bar.high - pc).abs()).max((bar.low - pc).abs()),
    }
}

/// Incremental Wilder ATR of length `n`.
///
/// `None` for the first `n − 1` bars. The `n`-th output is the mean of the first `n` true ranges,
/// where the first bar's TR is its high − low. After that,
/// `ATR = (prev·(n − 1) + TR) / n`.
///
/// Platforms differ on how the first value is seeded, and the difference fades out after a few
/// multiples of `n` bars. No match to TOS's `ATR` study is claimed until it has been measured.
#[derive(Debug, Clone)]
pub struct Atr {
    n: usize,
    prev_close: Option<Price<Adjusted>>,
    seed_sum: Price<Adjusted>,
    seen: usize,
    value: Option<Price<Adjusted>>,
}

impl Atr {
    pub fn new(n: usize) -> Self {
        assert!(n > 0, "ATR length must be positive");
        Atr { n, prev_close: None, seed_sum: Price::new(0.0), seen: 0, value: None }
    }

    pub fn update(&mut self, bar: &Bar<Adjusted>) -> Option<Price<Adjusted>> {
        let tr = true_range(bar, self.prev_close);
        self.prev_close = Some(bar.close);
        let n = self.n as f64;
        self.value = match self.value {
            Some(prev) => Some((prev * (n - 1.0) + tr) / n),
            None => {
                self.seed_sum = self.seed_sum + tr;
                self.seen += 1;
                (self.seen == self.n).then(|| self.seed_sum / n)
            }
        };
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::tests::{at, bar};
    use crate::price::{AdjFactor, Unadjusted};

    #[test]
    fn true_range_uses_the_gap() {
        let b: Bar<Adjusted> = bar(at(9, 30, 0), 12.0, 12.5, 11.8, 12.2, 1.0);
        assert_eq!(true_range(&b, None).value(), 12.5 - 11.8);
        // Gap up from a 10.0 close: high − prev_close dominates.
        assert_eq!(true_range(&b, Some(Price::new(10.0))).value(), 2.5);
        // Gap down to a 14.0 close: |low − prev_close| dominates.
        assert!((true_range(&b, Some(Price::new(14.0))).value() - 2.2).abs() < 1e-12);
    }

    #[test]
    fn wilder_seed_then_smoothing() {
        let mut a = Atr::new(3);
        let mk = |h: f64, l: f64, c: f64| -> Bar<Adjusted> { bar(at(9, 30, 0), c, h, l, c, 1.0) };
        // TRs: 2 (h−l), 2, 2 → seed ATR 2; then TR 5 → (2·2 + 5)/3 = 3
        assert_eq!(a.update(&mk(12.0, 10.0, 11.0)), None);
        assert_eq!(a.update(&mk(12.0, 10.0, 11.0)), None);
        assert_eq!(a.update(&mk(12.0, 10.0, 11.0)).unwrap().value(), 2.0);
        assert_eq!(a.update(&mk(15.0, 10.0, 14.0)).unwrap().value(), 3.0);
    }

    #[test]
    fn a_split_does_not_inflate_atr_once_history_is_adjusted() {
        // $188 stock, 4-for-1 split. Adjusted history makes the post-split bars comparable.
        let f = AdjFactor::from_split(4.0, 1.0).unwrap();
        let pre: Vec<Bar<Unadjusted>> = (0..14).map(|_| bar(at(9, 30, 0), 188.0, 190.0, 186.0, 188.0, 1.0)).collect();
        let mut atr = Atr::new(14);
        let mut last = None;
        for b in &pre {
            last = atr.update(&b.adjust(f));
        }
        let post: Bar<Adjusted> = bar(at(9, 30, 0), 47.0, 47.5, 46.5, 47.0, 1.0);
        let after = atr.update(&post).unwrap();
        assert_eq!(last.unwrap().value(), 1.0);
        assert!(after.value() < 1.1, "ATR {after} should stay near $1, not jump toward $141");
    }
}
