//! Massive (ex-Polygon.io) market data for the scanner.
//!
//! - [`ws::LiveSource`]: the live [`MarketSource`](tp_marketdata::source::MarketSource), full-market
//!   minute bars over the WebSocket.
//! - [`rest::Rest`]: reference data over REST (daily bars, splits, float, chart history, holidays).
//! - [`daily_cache`]: the daily-bar store that daily references are built from.
//! - [`flatfiles`]: one-day minute-bar files for replay.
//!
//! **Keys.** The API key is read from `MASSIVE_API_KEY`, the flat-file keys from
//! `MASSIVE_S3_KEY_ID` / `MASSIVE_S3_SECRET`. They are never logged, printed or written to disk by
//! this crate; their `Debug` form is redacted. The scanner holds market-data keys only, never broker
//! credentials.

pub mod daily_cache;
pub mod flatfiles;
pub mod rest;
pub mod ws;

use std::fmt;
use std::path::PathBuf;

/// A secret read from the environment. `Debug` and `Display` never show it.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn from_env(var: &str) -> Result<Secret, MassiveError> {
        match std::env::var(var) {
            Ok(v) if !v.trim().is_empty() => Ok(Secret(v.trim().to_string())),
            _ => Err(MassiveError::MissingKey(var.to_string())),
        }
    }

    /// For tests and callers that hold the key some other way.
    pub fn new(s: impl Into<String>) -> Secret {
        Secret(s.into())
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

pub const API_KEY_VAR: &str = "MASSIVE_API_KEY";
pub const S3_KEY_ID_VAR: &str = "MASSIVE_S3_KEY_ID";
pub const S3_SECRET_VAR: &str = "MASSIVE_S3_SECRET";

#[derive(Debug)]
pub enum MassiveError {
    MissingKey(String),
    Http(String),
    /// The server answered with a non-success status. The body is truncated and never contains
    /// the key (the key travels in a header, not the URL).
    Status { status: u16, url: String, body: String },
    Parse(String),
    Io(String),
}

impl fmt::Display for MassiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MassiveError::MissingKey(v) => write!(
                f,
                "{v} is not set. Set it once with `setx {v} <your key>` in a terminal, then open a new terminal"
            ),
            MassiveError::Http(e) => write!(f, "network: {e}"),
            MassiveError::Status { status, url, body } => write!(f, "HTTP {status} from {url}: {body}"),
            MassiveError::Parse(e) => write!(f, "unexpected response: {e}"),
            MassiveError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MassiveError {}

/// Where the scanner keeps its local data: `%LOCALAPPDATA%\tp-scanner`, or `TP_SCANNER_DATA` if
/// set (tests and second installs).
pub fn data_dir() -> Result<PathBuf, MassiveError> {
    if let Ok(d) = std::env::var("TP_SCANNER_DATA") {
        return Ok(PathBuf::from(d));
    }
    std::env::var("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join("tp-scanner"))
        .map_err(|_| MassiveError::Io("LOCALAPPDATA is not set; set TP_SCANNER_DATA to a folder".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_never_print() {
        let s = Secret::new("abc123");
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert!(!format!("{:?}", Some(s)).contains("abc"));
    }
}
