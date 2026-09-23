//! Simple and exponential moving averages, incremental.
//!
//! Both work on anything [`Linear`]: raw `f64` (volume, for ADV) or a `Price<B>`, where the basis
//! carries through. An SMA of adjusted closes is an adjusted price.

use crate::price::{Basis, Price};
use std::collections::VecDeque;

/// Values that can be averaged: added, and scaled by a plain number.
pub trait Linear: Copy {
    fn zero() -> Self;
    fn add(self, o: Self) -> Self;
    fn sub(self, o: Self) -> Self;
    fn scale(self, k: f64) -> Self;
}

impl Linear for f64 {
    fn zero() -> Self {
        0.0
    }
    fn add(self, o: Self) -> Self {
        self + o
    }
    fn sub(self, o: Self) -> Self {
        self - o
    }
    fn scale(self, k: f64) -> Self {
        self * k
    }
}

impl<B: Basis> Linear for Price<B> {
    fn zero() -> Self {
        Price::new(0.0)
    }
    fn add(self, o: Self) -> Self {
        self + o
    }
    fn sub(self, o: Self) -> Self {
        self - o
    }
    fn scale(self, k: f64) -> Self {
        self * k
    }
}

/// Mean of the last `n` values. `None` until `n` values have arrived.
///
/// Re-sums the window on every update: exact, with no drift, and at n ≤ a few hundred it's cheap
/// enough (the investigation's "naive recompute is affordable" finding).
#[derive(Debug, Clone)]
pub struct Sma<T> {
    n: usize,
    buf: VecDeque<T>,
}

impl<T: Linear> Sma<T> {
    pub fn new(n: usize) -> Self {
        assert!(n > 0, "SMA length must be positive");
        Sma { n, buf: VecDeque::with_capacity(n) }
    }

    pub fn update(&mut self, x: T) -> Option<T> {
        if self.buf.len() == self.n {
            self.buf.pop_front();
        }
        self.buf.push_back(x);
        (self.buf.len() == self.n).then(|| {
            let s = self.buf.iter().fold(T::zero(), |a, &v| a.add(v));
            s.scale(1.0 / self.n as f64)
        })
    }
}

/// Exponential moving average with `α = 2 / (n + 1)`.
///
/// **Seeding:** `None` for the first `n − 1` values; the `n`-th output is the SMA of the first
/// `n` values; after that each output is `prev + α·(x − prev)`. That is the common convention, but
/// platforms differ on it (some seed with the first value). No match to TradierPro's klinecharts
/// EMA or to TOS is claimed until it has been measured.
#[derive(Debug, Clone)]
pub struct Ema<T> {
    n: usize,
    alpha: f64,
    seed: Sma<T>,
    value: Option<T>,
}

impl<T: Linear> Ema<T> {
    pub fn new(n: usize) -> Self {
        Ema { n, alpha: 2.0 / (n as f64 + 1.0), seed: Sma::new(n), value: None }
    }

    pub fn length(&self) -> usize {
        self.n
    }

    pub fn update(&mut self, x: T) -> Option<T> {
        self.value = match self.value {
            Some(prev) => Some(prev.add(x.sub(prev).scale(self.alpha))),
            None => self.seed.update(x),
        };
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::price::Adjusted;

    #[test]
    fn sma_waits_for_a_full_window_then_slides() {
        let mut s = Sma::<f64>::new(3);
        let out: Vec<_> = [1.0, 2.0, 3.0, 4.0, 5.0].into_iter().map(|x| s.update(x)).collect();
        assert_eq!(out, vec![None, None, Some(2.0), Some(3.0), Some(4.0)]);
    }

    #[test]
    fn ema_seeds_with_sma_then_smooths() {
        let mut e = Ema::<f64>::new(3); // α = 0.5
        let out: Vec<_> = [1.0, 2.0, 3.0, 5.0, 5.0].into_iter().map(|x| e.update(x)).collect();
        assert_eq!(out, vec![None, None, Some(2.0), Some(3.5), Some(4.25)]);
    }

    #[test]
    fn price_basis_carries_through() {
        let mut s = Sma::<Price<Adjusted>>::new(2);
        s.update(Price::new(10.0));
        let out: Price<Adjusted> = s.update(Price::new(20.0)).unwrap();
        assert_eq!(out.value(), 15.0);
    }
}
