//! Indicators. Stateful ones are incremental (`update` per bar), so the replay and live paths
//! run the same code; batch helpers are thin folds over them. The volume profile is the
//! exception: re-binning on a new session extreme makes it inherently batch, and so is `series`,
//! which reads an indicator at the end of a kept series of closes.

pub mod atr;
pub mod ma;
pub mod profile;
pub mod series;
pub mod vwap;
