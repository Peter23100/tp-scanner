//! The scan engine: per-symbol state, the measurement catalog, filter expressions, the evaluator,
//! and the scan loop.
//!
//! Input comes only through `tp_marketdata::source::MarketSource`, so a history replay and a live
//! feed run exactly the same code.

pub mod compile;
pub mod engine;
pub mod expr;
pub mod instrument;
pub mod lang;
pub mod measure;
pub mod state;
