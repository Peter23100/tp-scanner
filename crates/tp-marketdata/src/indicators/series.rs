//! Indicators read at the end of a series of closes, computed from scratch on each call.
//!
//! The scanner keeps the last few hundred 5-minute closes per symbol and asks for "EMA7 now",
//! "RSI14 now". Recomputing over the kept series is exact, needs no per-indicator state, and at a
//! few hundred values is cheap (the investigation's "naive recompute is affordable" finding). The
//! price paid: an EMA or Wilder average started from a finite series differs from one started at
//! the dawn of time by `(1 − α)^(len − n)`, which for 200 closes and n = 20 is ~1e-8.
//!
//! All functions return `None` when the series is too short, never a guess.

use super::ma::Ema;

/// Mean of the last `n` values.
pub fn sma_last(xs: &[f64], n: usize) -> Option<f64> {
    if n == 0 || xs.len() < n {
        return None;
    }
    Some(xs[xs.len() - n..].iter().sum::<f64>() / n as f64)
}

/// EMA with `α = 2 / (n + 1)`, seeded with the SMA of the first `n` values (see [`Ema`]).
pub fn ema_last(xs: &[f64], n: usize) -> Option<f64> {
    if n == 0 || xs.len() < n {
        return None;
    }
    let mut e = Ema::<f64>::new(n);
    let mut v = None;
    for &x in xs {
        v = e.update(x);
    }
    v
}

/// Wilder's RSI over `n` periods. Needs `n + 1` values (n changes).
///
/// The first average gain/loss is the simple mean of the first `n` changes; after that each is
/// `prev + (x − prev) / n`. RSI = `100 − 100 / (1 + gain/loss)`. With no losses it is 100; with
/// no movement at all it is 50.
pub fn rsi_last(xs: &[f64], n: usize) -> Option<f64> {
    if n == 0 || xs.len() < n + 1 {
        return None;
    }
    let (mut gain, mut loss) = (0.0, 0.0);
    for w in xs[..=n].windows(2) {
        let d = w[1] - w[0];
        if d > 0.0 {
            gain += d;
        } else {
            loss -= d;
        }
    }
    gain /= n as f64;
    loss /= n as f64;
    for w in xs[n..].windows(2) {
        let d = w[1] - w[0];
        gain += (d.max(0.0) - gain) / n as f64;
        loss += ((-d).max(0.0) - loss) / n as f64;
    }
    Some(if loss == 0.0 {
        if gain == 0.0 {
            50.0
        } else {
            100.0
        }
    } else {
        100.0 - 100.0 / (1.0 + gain / loss)
    })
}

/// Bollinger bands at the series end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bollinger {
    pub mid: f64,
    pub upper: f64,
    pub lower: f64,
}

/// SMA(`n`) ± `k` × the **population** standard deviation of the same `n` values (the TOS
/// convention). Primus's variant is undocumented, so no match to it is claimed.
pub fn bollinger_last(xs: &[f64], n: usize, k: f64) -> Option<Bollinger> {
    let mid = sma_last(xs, n)?;
    let tail = &xs[xs.len() - n..];
    let var = tail.iter().map(|x| (x - mid).powi(2)).sum::<f64>() / n as f64;
    let sd = var.sqrt();
    Some(Bollinger { mid, upper: mid + k * sd, lower: mid - k * sd })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_series_are_unknown() {
        assert_eq!(sma_last(&[1.0, 2.0], 3), None);
        assert_eq!(ema_last(&[1.0, 2.0], 3), None);
        assert_eq!(rsi_last(&[1.0, 2.0, 3.0], 3), None, "needs n + 1 closes");
        assert_eq!(bollinger_last(&[1.0], 2, 2.0), None);
        assert_eq!(sma_last(&[1.0], 0), None);
    }

    #[test]
    fn sma_and_ema_read_the_series_end() {
        let xs = [1.0, 2.0, 3.0, 5.0, 5.0];
        assert_eq!(sma_last(&xs, 3), Some(13.0 / 3.0));
        // Same numbers as ma.rs's EMA test: 2, 3.5, 4.25.
        assert_eq!(ema_last(&xs, 3), Some(4.25));
    }

    #[test]
    fn rsi_extremes_and_a_hand_computed_value() {
        assert_eq!(rsi_last(&[1.0, 2.0, 3.0, 4.0], 3), Some(100.0));
        assert_eq!(rsi_last(&[4.0, 3.0, 2.0, 1.0], 3), Some(0.0));
        assert_eq!(rsi_last(&[2.0, 2.0, 2.0, 2.0], 3), Some(50.0));
        // Changes +1, -1, +1 -> gain 2/3, loss 1/3; then -1: gain (2/3)(2/3)=4/9, loss (1/3 + (1 - 1/3)/3)=5/9.
        let r = rsi_last(&[10.0, 11.0, 10.0, 11.0, 10.0], 3).unwrap();
        let rs: f64 = (4.0 / 9.0) / (5.0 / 9.0);
        assert!((r - (100.0 - 100.0 / (1.0 + rs))).abs() < 1e-12, "{r}");
    }

    #[test]
    fn bollinger_uses_population_sd() {
        let b = bollinger_last(&[0.0, 9.0, 2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0], 8, 2.0).unwrap();
        // Last 8: 2,4,4,4,5,5,7,9 -> mean 5, population sd 2.
        assert_eq!((b.mid, b.upper, b.lower), (5.0, 9.0, 1.0));
    }
}
