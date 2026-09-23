//! Market-data primitives for the scanner: the New York session clock, prices typed by adjustment
//! basis, bars, rolling time windows, the single market-data input trait, and indicators.
//!
//! Every primitive states its exact semantics in its doc comment and has unit tests. That is the
//! rule for adding one.

pub mod bar;
pub mod indicators;
pub mod price;
pub mod source;
pub mod time;
pub mod window;
