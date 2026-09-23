//! Prices that know whether they have been adjusted for corporate actions.
//!
//! The incumbent scanner's worst bug showed a $188 stock with a $53 ATR: split-adjusted history
//! silently mixed with unadjusted live prices. Here that mix does not compile. `Price<Adjusted>`
//! and `Price<Unadjusted>` are distinct types. Arithmetic and comparison are defined only within
//! one basis, and crossing between them takes an explicit [`AdjFactor`].
//!
//! ```compile_fail,E0308
//! use tp_marketdata::price::{Adjusted, Price, Unadjusted};
//! let history = Price::<Adjusted>::new(47.0);
//! let live = Price::<Unadjusted>::new(188.0);
//! let _ = live < history; // comparing across bases is a type error
//! ```
//!
//! ```compile_fail,E0308
//! use tp_marketdata::price::{Adjusted, Price, Unadjusted};
//! let history = Price::<Adjusted>::new(47.0);
//! let live = Price::<Unadjusted>::new(188.0);
//! let _ = live - history; // so is arithmetic
//! ```
//!
//! The same operations within one basis compile, which shows it is the basis that the two
//! examples above are rejected for:
//!
//! ```
//! use tp_marketdata::price::{AdjFactor, Price, Unadjusted};
//! let before_split = Price::<Unadjusted>::new(188.0);
//! let live = Price::<Unadjusted>::new(47.5);
//! assert!(live < before_split);
//! // 4-for-1 split: the explicit conversion puts history on today's basis.
//! let adj = before_split.adjust(AdjFactor::from_split(4.0, 1.0).unwrap());
//! assert_eq!(adj.value(), 47.0);
//! ```
//!
//! `value()` is the deliberate escape hatch to a raw `f64`, for output and for code that takes
//! responsibility for the basis itself.

use std::cmp::Ordering;
use std::fmt;
use std::marker::PhantomData;
use std::ops::{Add, Div, Mul, Neg, Sub};

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Adjusted {}
    impl Sealed for super::Unadjusted {}
}

/// The adjustment basis a price is expressed in. Sealed: exactly two bases exist.
pub trait Basis: sealed::Sealed + Copy + fmt::Debug + PartialEq + 'static {
    const TAG: &'static str;
}

/// Prices restated onto the current share basis (splits and, where applied, dividends).
/// Use this for anything that compares across dates: ATR, multi-day ranges, prior highs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adjusted;

/// Prices as they printed on the tape that day. Live data arrives unadjusted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Unadjusted;

impl Basis for Adjusted {
    const TAG: &'static str = "adj";
}
impl Basis for Unadjusted {
    const TAG: &'static str = "unadj";
}

/// A price (or a price difference: a range, an ATR) in basis `B`.
pub struct Price<B: Basis> {
    v: f64,
    _basis: PhantomData<fn() -> B>,
}

impl<B: Basis> Price<B> {
    pub const fn new(v: f64) -> Self {
        Price { v, _basis: PhantomData }
    }

    /// The raw number. The escape hatch: whoever calls this owns the basis from here on.
    pub const fn value(self) -> f64 {
        self.v
    }

    pub fn abs(self) -> Self {
        Self::new(self.v.abs())
    }

    /// The larger of the two. `f64::max` semantics: NaN loses to a number.
    pub fn max(self, other: Self) -> Self {
        Self::new(self.v.max(other.v))
    }

    /// The smaller of the two. `f64::min` semantics: NaN loses to a number.
    pub fn min(self, other: Self) -> Self {
        Self::new(self.v.min(other.v))
    }

    pub fn is_finite(self) -> bool {
        self.v.is_finite()
    }
}

impl Price<Unadjusted> {
    /// Restate onto the adjusted basis: `adjusted = unadjusted × factor`.
    pub fn adjust(self, f: AdjFactor) -> Price<Adjusted> {
        Price::new(self.v * f.0)
    }
}

impl Price<Adjusted> {
    /// Undo [`Price::adjust`]: `unadjusted = adjusted ÷ factor`.
    pub fn unadjust(self, f: AdjFactor) -> Price<Unadjusted> {
        Price::new(self.v / f.0)
    }
}

// Manual impls: derives would demand the bounds on `B` itself, not just on `f64`.
impl<B: Basis> Clone for Price<B> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<B: Basis> Copy for Price<B> {}
impl<B: Basis> PartialEq for Price<B> {
    fn eq(&self, o: &Self) -> bool {
        self.v == o.v
    }
}
impl<B: Basis> PartialOrd for Price<B> {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        self.v.partial_cmp(&o.v)
    }
}
impl<B: Basis> fmt::Debug for Price<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.v, B::TAG)
    }
}
impl<B: Basis> fmt::Display for Price<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.v, f)
    }
}

impl<B: Basis> Add for Price<B> {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.v + o.v)
    }
}
impl<B: Basis> Sub for Price<B> {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.v - o.v)
    }
}
impl<B: Basis> Neg for Price<B> {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.v)
    }
}
impl<B: Basis> Mul<f64> for Price<B> {
    type Output = Self;
    fn mul(self, k: f64) -> Self {
        Self::new(self.v * k)
    }
}
impl<B: Basis> Div<f64> for Price<B> {
    type Output = Self;
    fn div(self, k: f64) -> Self {
        Self::new(self.v / k)
    }
}
/// The ratio of two prices on the same basis is a plain number, such as a move expressed in ATRs.
impl<B: Basis> Div for Price<B> {
    type Output = f64;
    fn div(self, o: Self) -> f64 {
        self.v / o.v
    }
}

/// The multiplier from the unadjusted basis to the adjusted one: `adjusted = unadjusted × factor`.
///
/// Always finite and strictly positive. Factors for successive corporate actions compose by
/// multiplication ([`AdjFactor::then`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdjFactor(f64);

impl AdjFactor {
    pub const IDENTITY: AdjFactor = AdjFactor(1.0);

    /// `None` unless `f` is finite and > 0.
    pub fn new(f: f64) -> Option<AdjFactor> {
        (f.is_finite() && f > 0.0).then_some(AdjFactor(f))
    }

    /// The factor to apply to pre-split prices for a `new_shares`-for-`old_shares` split.
    /// A 4-for-1 gives 0.25; a 1-for-10 reverse split gives 10.
    pub fn from_split(new_shares: f64, old_shares: f64) -> Option<AdjFactor> {
        AdjFactor::new(old_shares / new_shares)
    }

    pub const fn get(self) -> f64 {
        self.0
    }

    /// The factor for applying `self` and then `later`.
    pub fn then(self, later: AdjFactor) -> AdjFactor {
        AdjFactor(self.0 * later.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjust_round_trips() {
        let f = AdjFactor::from_split(4.0, 1.0).unwrap();
        let u = Price::<Unadjusted>::new(188.0);
        assert_eq!(u.adjust(f).value(), 47.0);
        assert_eq!(u.adjust(f).unadjust(f), u);
    }

    #[test]
    fn reverse_split_factor_is_above_one() {
        assert_eq!(AdjFactor::from_split(1.0, 10.0).unwrap().get(), 10.0);
    }

    #[test]
    fn factors_compose() {
        let a = AdjFactor::from_split(2.0, 1.0).unwrap();
        let b = AdjFactor::from_split(3.0, 1.0).unwrap();
        assert!((a.then(b).get() - 1.0 / 6.0).abs() < 1e-15);
    }

    #[test]
    fn invalid_factors_are_rejected() {
        assert!(AdjFactor::new(0.0).is_none());
        assert!(AdjFactor::new(-1.0).is_none());
        assert!(AdjFactor::new(f64::NAN).is_none());
        assert!(AdjFactor::new(f64::INFINITY).is_none());
        assert!(AdjFactor::from_split(0.0, 1.0).is_none());
    }

    #[test]
    fn same_basis_ratio_is_unitless() {
        let mv = Price::<Adjusted>::new(3.0);
        let atr = Price::<Adjusted>::new(2.0);
        assert_eq!(mv / atr, 1.5);
    }
}
