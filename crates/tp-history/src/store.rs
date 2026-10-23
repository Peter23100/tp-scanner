//! Where each series lives on disk.
//!
//! ```text
//! <root>/2026-10-16/stocks/NVDA.trades.tph       NVDA.quotes.tph   NVDA.bars1s.tph   NVDA.done.json
//! <root>/2026-10-16/options/NVDA.trades.tph      (every NVDA contract that day, by contract id)
//!                          NVDA.quotes.tph  NVDA.bars1s.tph  NVDA.contracts.json  NVDA.done.json
//! ```
//!
//! A (day, symbol) is **available only once its `done.json` manifest exists**. The downloader writes
//! the manifest last, after every series has landed, so a download interrupted halfway is never
//! mistaken for a complete day. Root: `TP_HISTORY_DIR` if set, else `%LOCALAPPDATA%\tp-history`.

use crate::HistoryError;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const ROOT_VAR: &str = "TP_HISTORY_DIR";

pub fn default_root() -> Result<PathBuf, HistoryError> {
    if let Ok(d) = std::env::var(ROOT_VAR) {
        return Ok(PathBuf::from(d));
    }
    std::env::var("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join("tp-history"))
        .map_err(|_| HistoryError::Io(format!("LOCALAPPDATA is not set; set {ROOT_VAR} to a folder")))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Asset {
    Stock,
    /// Every option contract of one underlying.
    Options,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Series {
    Trades,
    Quotes,
    Bars1s,
    Bars1m,
}

impl Series {
    pub fn suffix(self) -> &'static str {
        match self {
            Series::Trades => "trades",
            Series::Quotes => "quotes",
            Series::Bars1s => "bars1s",
            Series::Bars1m => "bars1m",
        }
    }
}

/// A ticker as a file name. Uppercase; `/` and space become `.` so `BRK/B`, `BRK B` and `BRK.B`
/// are one file; anything else outside `A-Z 0-9 . -` is refused rather than guessed at. Windows
/// device names (`CON`, `NUL`, `COM1`…) get a leading `_`, or Windows would refuse the file.
pub fn file_symbol(symbol: &str) -> Result<String, HistoryError> {
    let s: String = symbol.trim().to_ascii_uppercase().chars().map(|c| if c == '/' || c == ' ' { '.' } else { c }).collect();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Err(HistoryError::Format(format!("symbol {symbol:?} can't be a file name")));
    }
    let stem = s.split('.').next().unwrap_or("");
    let reserved = matches!(stem, "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    Ok(if reserved { format!("_{s}") } else { s })
}

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Store {
        Store { root: root.into() }
    }

    pub fn open_default() -> Result<Store, HistoryError> {
        Ok(Store::new(default_root()?))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn dir(&self, date: NaiveDate, asset: Asset) -> PathBuf {
        self.root.join(date.to_string()).join(match asset {
            Asset::Stock => "stocks",
            Asset::Options => "options",
        })
    }

    pub fn series_path(&self, date: NaiveDate, asset: Asset, symbol: &str, series: Series) -> Result<PathBuf, HistoryError> {
        Ok(self.dir(date, asset).join(format!("{}.{}.tph", file_symbol(symbol)?, series.suffix())))
    }

    pub fn contracts_path(&self, date: NaiveDate, underlying: &str) -> Result<PathBuf, HistoryError> {
        Ok(self.dir(date, Asset::Options).join(format!("{}.contracts.json", file_symbol(underlying)?)))
    }

    pub fn manifest_path(&self, date: NaiveDate, asset: Asset, symbol: &str) -> Result<PathBuf, HistoryError> {
        Ok(self.dir(date, asset).join(format!("{}.done.json", file_symbol(symbol)?)))
    }

    /// The manifest, if this (day, symbol) finished downloading.
    pub fn manifest(&self, date: NaiveDate, asset: Asset, symbol: &str) -> Result<Option<Manifest>, HistoryError> {
        let p = self.manifest_path(date, asset, symbol)?;
        match std::fs::read(&p) {
            Ok(b) => serde_json::from_slice(&b).map(Some).map_err(|e| HistoryError::Format(format!("{}: {e}", p.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Mark a (day, symbol) complete. Call only after every series in `m` is on disk.
    pub fn write_manifest(&self, m: &Manifest) -> Result<(), HistoryError> {
        let p = self.manifest_path(m.date, m.asset, &m.symbol)?;
        for s in &m.series {
            let sp = self.series_path(m.date, m.asset, &m.symbol, s.series)?;
            if !sp.exists() {
                return Err(HistoryError::Format(format!("manifest lists {} but it isn't on disk", sp.display())));
            }
        }
        write_json_atomic(&p, m)
    }

    pub fn available_days(&self) -> Vec<NaiveDate> {
        let mut out: Vec<NaiveDate> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|n| NaiveDate::parse_from_str(n, "%Y-%m-%d").ok()))
            .collect();
        out.sort();
        out
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeriesCount {
    pub series: Series,
    pub records: u64,
}

/// Written last: the proof a (day, symbol) downloaded completely.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub date: NaiveDate,
    pub asset: Asset,
    pub symbol: String,
    pub series: Vec<SeriesCount>,
    /// Where it came from, e.g. "massive rest v3".
    pub source: String,
    /// UTC, RFC 3339.
    pub fetched_at: String,
}

/// One option contract in an underlying's day. `id` is what records carry as `contract`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Contract {
    pub id: u32,
    /// OCC symbol, unpadded (`NVDA261016C00180000`).
    pub occ: String,
    pub expiration: NaiveDate,
    /// 'C' or 'P'.
    pub right: char,
    pub strike: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ContractTable {
    pub contracts: Vec<Contract>,
}

impl ContractTable {
    /// The id for `occ`, adding it if new. Ids start at 1: 0 means "the stock itself".
    pub fn id_for(&mut self, occ: &str, expiration: NaiveDate, right: char, strike: f64) -> u32 {
        let occ = occ.replace(' ', "");
        if let Some(c) = self.contracts.iter().find(|c| c.occ == occ) {
            return c.id;
        }
        let id = self.contracts.len() as u32 + 1;
        self.contracts.push(Contract { id, occ, expiration, right, strike });
        id
    }

    pub fn by_id(&self, id: u32) -> Option<&Contract> {
        id.checked_sub(1).and_then(|i| self.contracts.get(i as usize)).filter(|c| c.id == id)
    }

    /// Whitespace-insensitive (padded and unpadded OCC match).
    pub fn by_occ(&self, occ: &str) -> Option<&Contract> {
        let occ = occ.replace(' ', "");
        self.contracts.iter().find(|c| c.occ == occ)
    }

    pub fn load(path: &Path) -> Result<ContractTable, HistoryError> {
        let b = std::fs::read(path).map_err(|e| HistoryError::Io(format!("{}: {e}", path.display())))?;
        serde_json::from_slice(&b).map_err(|e| HistoryError::Format(format!("{}: {e}", path.display())))
    }

    pub fn save(&self, path: &Path) -> Result<(), HistoryError> {
        write_json_atomic(path, self)
    }
}

fn write_json_atomic<T: Serialize>(path: &Path, v: &T) -> Result<(), HistoryError> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let json = serde_json::to_vec_pretty(v).map_err(|e| HistoryError::Format(e.to_string()))?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::tests::{temp_dir, trade};
    use crate::file::write_all;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn symbols_become_safe_file_names() {
        assert_eq!(file_symbol("nvda").unwrap(), "NVDA");
        assert_eq!(file_symbol("BRK/B").unwrap(), "BRK.B");
        assert_eq!(file_symbol("BRK B").unwrap(), "BRK.B");
        assert_eq!(file_symbol("BRK.B").unwrap(), "BRK.B");
        assert_eq!(file_symbol("CON").unwrap(), "_CON");
        assert_eq!(file_symbol("COM1").unwrap(), "_COM1");
        assert_eq!(file_symbol("COMP").unwrap(), "COMP");
        assert!(file_symbol("A\\B").is_err());
        assert!(file_symbol("..").is_ok_and(|s| s == ".."), "dots alone are harmless inside the stem");
        assert!(file_symbol("").is_err());
    }

    #[test]
    fn a_day_is_available_only_after_its_manifest() {
        let st = Store::new(temp_dir("store"));
        let day = d("2026-10-16");
        assert_eq!(st.manifest(day, Asset::Stock, "NVDA").unwrap(), None);
        let p = st.series_path(day, Asset::Stock, "NVDA", Series::Trades).unwrap();
        write_all(&p, &[trade(1, 1.0)]).unwrap();
        let m = Manifest {
            date: day,
            asset: Asset::Stock,
            symbol: "NVDA".into(),
            series: vec![SeriesCount { series: Series::Trades, records: 1 }],
            source: "test".into(),
            fetched_at: "2026-10-19T00:00:00Z".into(),
        };
        st.write_manifest(&m).unwrap();
        assert_eq!(st.manifest(day, Asset::Stock, "NVDA").unwrap(), Some(m));
        assert_eq!(st.available_days(), vec![day]);
    }

    #[test]
    fn a_manifest_for_missing_series_is_refused() {
        let st = Store::new(temp_dir("store2"));
        let m = Manifest {
            date: d("2026-10-16"),
            asset: Asset::Options,
            symbol: "NVDA".into(),
            series: vec![SeriesCount { series: Series::Quotes, records: 9 }],
            source: "test".into(),
            fetched_at: "x".into(),
        };
        assert!(st.write_manifest(&m).is_err());
        assert!(!st.manifest_path(m.date, m.asset, "NVDA").unwrap().exists());
    }

    #[test]
    fn contract_ids_are_stable_and_whitespace_blind() {
        let mut t = ContractTable::default();
        let exp = d("2026-10-16");
        let a = t.id_for("NVDA  261016C00180000", exp, 'C', 180.0);
        let b = t.id_for("NVDA261016P00180000", exp, 'P', 180.0);
        assert_eq!((a, b), (1, 2));
        assert_eq!(t.id_for("NVDA261016C00180000", exp, 'C', 180.0), 1);
        assert_eq!(t.by_occ("NVDA  261016P00180000").unwrap().id, 2);
        assert_eq!(t.by_id(2).unwrap().right, 'P');
        assert!(t.by_id(0).is_none(), "0 is the stock itself, never a contract");
        let p = temp_dir("contracts").join("NVDA.contracts.json");
        t.save(&p).unwrap();
        assert_eq!(ContractTable::load(&p).unwrap(), t);
    }
}
