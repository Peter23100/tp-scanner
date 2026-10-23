//! The local market-history store, shared by tp-scanner and TradierPro's backtest mode
//! (TradierPro `BACKTEST_PLAN.md` §7).
//!
//! Data is downloaded from Massive once and kept: every (day, symbol) is written once, atomically,
//! and never rewritten. Readers replay it without look-ahead.
//!
//! - [`record`]: the fixed-width record types — [`record::Trade`], [`record::Quote`],
//!   [`record::Bar`] — and how each encodes.
//! - [`file`]: the block file a series lives in: minute-sized zstd blocks with a CRC each and an
//!   index, so a jump to 14:05 decompresses one block, not the day.
//! - [`cursor`]: reading a series against a clock. It never yields a record stamped after the time
//!   it was asked for; that is the one place look-ahead is prevented.
//! - [`clock`]: the replay clock — sim time from wall time, speed, pause and seek.
//! - [`store`]: where each series lives on disk.
//! - [`universe`]: which symbols a day downloads (the day's major movers).
//!
//! **Times** are UTC nanoseconds since the Unix epoch, as Massive stamps them (SIP timestamp).
//! Sessions are derived from them on New York time by the caller, never from the machine clock.
//! **Prices** are as traded that day (unadjusted).

pub mod clock;
pub mod cursor;
pub mod file;
pub mod record;
pub mod store;
pub mod universe;

use std::fmt;

#[derive(Debug)]
pub enum HistoryError {
    Io(String),
    /// The file is not a history file, is from a newer version, or is damaged.
    Format(String),
    /// A block's checksum didn't match: the file is damaged on disk.
    Corrupt { path: String, block: usize },
    /// Records handed to a writer were not in time order.
    OutOfOrder { prev_ns: i64, next_ns: i64 },
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HistoryError::Io(e) => write!(f, "{e}"),
            HistoryError::Format(e) => write!(f, "not a readable history file: {e}"),
            HistoryError::Corrupt { path, block } => write!(f, "{path}: block {block} failed its checksum (damaged file — delete it and download the day again)"),
            HistoryError::OutOfOrder { prev_ns, next_ns } => write!(f, "records out of time order: {next_ns} after {prev_ns}"),
        }
    }
}

impl std::error::Error for HistoryError {}

impl From<std::io::Error> for HistoryError {
    fn from(e: std::io::Error) -> Self {
        HistoryError::Io(e.to_string())
    }
}
