//! Ticker reference data: each symbol's security type and listing exchange, for baskets
//! (`types stock, adr`, `exchanges nasdaq, nyse`). Massive's `/v3/reference/tickers` codes are
//! mapped onto the engine's words (`tp_scan_engine::expr::SECURITY_TYPES` / `EXCHANGES`) here,
//! once, and kept as `tickers.csv` in the data dir (`symbol,type,exchange,name`; the company name
//! last, since it can hold commas, and is for display: the charts' watermark).
//!
//! A code we don't recognise maps to `other` rather than being guessed into a real category; a
//! missing field stays empty and reads unknown.

use crate::rest::Rest;
use crate::MassiveError;
use serde_json::Value;
use std::path::Path;

pub const FILE: &str = "tickers.csv";
/// The current file's header. A file with another (older) header is refetched.
pub const HEADER: &str = "symbol,type,exchange,name";

/// Whether `tickers.csv` exists in the current format.
pub fn is_current(data_dir: &Path) -> bool {
    std::fs::read_to_string(data_dir.join(FILE)).is_ok_and(|t| t.lines().next() == Some(HEADER))
}

/// Massive's security type code → the engine's word.
pub fn type_word(code: &str) -> &'static str {
    match code {
        "CS" | "OS" | "NYRS" => "stock",
        "ETF" | "ETV" | "ETS" => "etf",
        "ETN" => "etn",
        "ADRC" | "ADRP" | "ADRR" | "ADRW" | "GDR" => "adr",
        "PFD" => "preferred",
        "WARRANT" => "warrant",
        "RIGHT" => "right",
        "UNIT" => "unit",
        "FUND" => "fund",
        _ => "other",
    }
}

/// Massive's primary exchange (MIC) → the engine's word.
pub fn exchange_word(mic: &str) -> &'static str {
    match mic {
        "XNAS" => "nasdaq",
        "XNYS" => "nyse",
        "XASE" => "amex",
        "ARCX" => "arca",
        "BATS" => "cboe",
        "IEXG" => "iex",
        m if m.starts_with("OTC") || m == "XOTC" => "otc",
        _ => "other",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickerRef {
    pub symbol: String,
    pub security_type: Option<&'static str>,
    pub exchange: Option<&'static str>,
    pub name: Option<String>,
}

pub(crate) fn parse_ticker(r: &Value) -> Option<TickerRef> {
    let symbol = r.get("ticker")?.as_str()?.to_string();
    let field = |k: &str| r.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    // Line breaks can't be in a CSV row; anything else in a name is kept as is (it's the last field).
    let name = field("name").map(|n| n.replace(['\r', '\n'], " ").trim().to_string());
    Some(TickerRef { symbol, security_type: field("type").map(type_word), exchange: field("primary_exchange").map(exchange_word), name })
}

impl Rest {
    /// Every active US stock-market ticker with its type and exchange (paged, ~12 requests).
    pub fn tickers(&self) -> Result<Vec<TickerRef>, MassiveError> {
        let url = format!("{}/v3/reference/tickers?market=stocks&active=true&limit=1000", self.base);
        Ok(self.get_all(&url)?.iter().filter_map(parse_ticker).collect())
    }
}

/// Download and save `tickers.csv` (atomically). Returns how many symbols.
pub fn refresh(rest: &Rest, data_dir: &Path) -> Result<usize, MassiveError> {
    let rows = rest.tickers()?;
    let mut out = format!("{HEADER}\n");
    for t in &rows {
        out.push_str(&format!("{},{},{},{}\n", t.symbol, t.security_type.unwrap_or(""), t.exchange.unwrap_or(""), t.name.as_deref().unwrap_or("")));
    }
    std::fs::create_dir_all(data_dir).map_err(|e| MassiveError::Io(e.to_string()))?;
    let path = data_dir.join(FILE);
    let tmp = path.with_extension("csv.tmp");
    std::fs::write(&tmp, out).map_err(|e| MassiveError::Io(e.to_string()))?;
    std::fs::rename(&tmp, &path).map_err(|e| MassiveError::Io(e.to_string()))?;
    Ok(rows.len())
}

/// `tickers.csv` → `(symbol, type, exchange)`; empty fields are `None`. A missing file is empty.
pub fn load(data_dir: &Path) -> Vec<(String, Option<String>, Option<String>)> {
    let Ok(t) = std::fs::read_to_string(data_dir.join(FILE)) else { return vec![] };
    t.lines()
        .skip(1)
        .filter_map(|l| {
            let mut p = l.splitn(4, ',');
            let sym = p.next()?.trim();
            let opt = |s: Option<&str>| s.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
            (!sym.is_empty()).then(|| (sym.to_string(), opt(p.next()), opt(p.next())))
        })
        .collect()
}

/// Symbol → company name, from `tickers.csv`.
pub fn names(data_dir: &Path) -> std::collections::HashMap<String, String> {
    let Ok(t) = std::fs::read_to_string(data_dir.join(FILE)) else { return Default::default() };
    t.lines()
        .skip(1)
        .filter_map(|l| {
            let mut p = l.splitn(4, ',');
            let sym = p.next()?.trim();
            let name = p.nth(2)?.trim();
            (!sym.is_empty() && !name.is_empty()).then(|| (sym.to_string(), name.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codes_map_to_basket_words_and_unknowns_stay_unknown() {
        let r = parse_ticker(&json!({"ticker":"SPY","type":"ETF","primary_exchange":"ARCX"})).unwrap();
        assert_eq!((r.security_type, r.exchange), (Some("etf"), Some("arca")));
        let r = parse_ticker(&json!({"ticker":"BABA","type":"ADRC","primary_exchange":"XNYS"})).unwrap();
        assert_eq!((r.security_type, r.exchange), (Some("adr"), Some("nyse")));
        let r = parse_ticker(&json!({"ticker":"ODD","type":"SP","primary_exchange":"XCHI"})).unwrap();
        assert_eq!((r.security_type, r.exchange), (Some("other"), Some("other")), "unrecognised codes are `other`, never guessed");
        let r = parse_ticker(&json!({"ticker":"NEW"})).unwrap();
        assert_eq!((r.security_type, r.exchange), (None, None), "missing fields stay unknown");
        assert!(parse_ticker(&json!({"type":"CS"})).is_none());
    }

    #[test]
    fn the_file_round_trips() {
        let d = std::env::temp_dir().join(format!("tp-tickers-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(FILE), "symbol,type,exchange,name\nAAPL,stock,nasdaq,Apple Inc.\nBRK.B,stock,nyse,Berkshire Hathaway, Inc. Class B\nNEW,,,\n").unwrap();
        assert!(is_current(&d));
        assert_eq!(load(&d), vec![("AAPL".into(), Some("stock".into()), Some("nasdaq".into())), ("BRK.B".into(), Some("stock".into()), Some("nyse".into())), ("NEW".into(), None, None)]);
        let n = names(&d);
        assert_eq!(n.get("BRK.B").map(String::as_str), Some("Berkshire Hathaway, Inc. Class B"), "a comma in a name survives");
        assert!(!n.contains_key("NEW"));
        std::fs::write(d.join(FILE), "symbol,type,exchange\nAAPL,stock,nasdaq\n").unwrap();
        assert!(!is_current(&d), "the old format is refetched");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Every word produced here is one the basket language accepts (kept in step by hand: the
    /// engine doesn't depend on this crate).
    #[test]
    fn every_word_is_a_basket_word() {
        let types = ["stock", "etf", "etn", "adr", "preferred", "warrant", "right", "unit", "fund", "other"];
        let exchanges = ["nasdaq", "nyse", "amex", "arca", "cboe", "iex", "otc", "other"];
        for c in ["CS", "OS", "NYRS", "ETF", "ETV", "ETS", "ETN", "ADRC", "ADRP", "ADRR", "ADRW", "GDR", "PFD", "WARRANT", "RIGHT", "UNIT", "FUND", "SP"] {
            assert!(types.contains(&type_word(c)), "{c}");
        }
        for m in ["XNAS", "XNYS", "XASE", "ARCX", "BATS", "IEXG", "OTCM", "XCHI"] {
            assert!(exchanges.contains(&exchange_word(m)), "{m}");
        }
        assert_eq!(types, tp_scan_engine::expr::SECURITY_TYPES);
        assert_eq!(exchanges, tp_scan_engine::expr::EXCHANGES);
    }
}
