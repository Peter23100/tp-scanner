//! The replay's side inputs: daily history and float.

use chrono::{NaiveDate, NaiveTime};
use std::collections::HashMap;
use std::path::Path;
use tp_marketdata::bar::Bar;
use tp_marketdata::price::{Adjusted, Price};
use tp_marketdata::time::Ts;
use tp_scan_engine::engine::DailyProvider;

pub const DAILY_HEADER: &str = "symbol,date,open,high,low,close,volume";
pub const FLOAT_HEADER: &str = "symbol,float_shares";

fn rows<'a>(path: &Path, text: &'a str, header: &str) -> Result<Vec<(usize, Vec<&'a str>)>, String> {
    let mut out = vec![];
    let mut saw_header = false;
    for (i, raw) in text.lines().enumerate() {
        let l = raw.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if !saw_header {
            if l != header {
                return Err(format!("{}:{}: expected header `{header}`", path.display(), i + 1));
            }
            saw_header = true;
            continue;
        }
        out.push((i + 1, l.split(',').map(str::trim).collect()));
    }
    Ok(out)
}

fn num(path: &Path, line: usize, s: &str, what: &str) -> Result<f64, String> {
    s.parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
        .ok_or_else(|| format!("{}:{line}: {what} `{s}` is not a number", path.display()))
}

/// Split-adjusted daily bars by symbol, oldest first. Refuses the file on any bad row, a repeated
/// date, or a date out of order.
pub fn read_daily(path: &Path) -> Result<HashMap<String, Vec<Bar<Adjusted>>>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out: HashMap<String, Vec<Bar<Adjusted>>> = HashMap::new();
    for (line, f) in rows(path, &text, DAILY_HEADER)? {
        if f.len() != 7 {
            return Err(format!("{}:{line}: expected 7 columns", path.display()));
        }
        let date = NaiveDate::parse_from_str(f[1], "%Y-%m-%d").map_err(|e| format!("{}:{line}: date: {e}", path.display()))?;
        let start = Ts::from_ny_local(date, NaiveTime::MIN).map_err(|e| format!("{}:{line}: {e}", path.display()))?;
        let [o, h, l, c, v] = [2, 3, 4, 5, 6].map(|k| num(path, line, f[k], ["open", "high", "low", "close", "volume"][k - 2]));
        let (o, h, l, c, v) = (o?, h?, l?, c?, v?);
        if !(l <= o.min(c) && o.max(c) <= h && v >= 0.0) {
            return Err(format!("{}:{line}: not a sane bar (low ≤ open, close ≤ high; volume ≥ 0)", path.display()));
        }
        let bars = out.entry(f[0].to_string()).or_default();
        if bars.last().is_some_and(|b| b.start >= start) {
            return Err(format!("{}:{line}: {} dates must increase", path.display(), f[0]));
        }
        bars.push(Bar { start, open: Price::new(o), high: Price::new(h), low: Price::new(l), close: Price::new(c), volume: v });
    }
    Ok(out)
}

/// See [`tp_scan_engine::daily::history_provider`].
pub fn provider(daily: HashMap<String, Vec<Bar<Adjusted>>>) -> DailyProvider {
    tp_scan_engine::daily::history_provider(daily)
}

pub fn read_float(path: &Path) -> Result<Vec<(String, f64)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = vec![];
    for (line, f) in rows(path, &text, FLOAT_HEADER)? {
        if f.len() != 2 {
            return Err(format!("{}:{line}: expected 2 columns", path.display()));
        }
        let shares = num(path, line, f[1], "float_shares")?;
        if shares < 0.0 {
            return Err(format!("{}:{line}: negative float", path.display()));
        }
        out.push((f[0].to_string(), shares));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tp_marketdata::time::SessionDate;

    fn tmp(name: &str, text: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("tp-scan-cli-{}-{name}", std::process::id()));
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn daily_references_never_see_the_session_they_serve() {
        let p = tmp(
            "daily.csv",
            "symbol,date,open,high,low,close,volume\nXYZ,2026-06-11,10,11,9,10,100\nXYZ,2026-06-12,10,12,9,11,200\nXYZ,2026-06-15,11,99,1,50,300\n",
        );
        let mut prov = provider(read_daily(&p).unwrap());
        let d = prov("XYZ", SessionDate(NaiveDate::from_ymd_opt(2026, 6, 15).unwrap())).unwrap();
        assert_eq!(d.prev_close.value(), 11.0, "the 06-15 bar itself is excluded");
        assert_eq!(d.prev_high.unwrap().value(), 12.0);
        assert!(prov("XYZ", SessionDate(NaiveDate::from_ymd_opt(2026, 6, 11).unwrap())).is_none(), "no prior day");
        assert!(prov("NOPE", SessionDate(NaiveDate::from_ymd_opt(2026, 6, 15).unwrap())).is_none());
    }

    #[test]
    fn bad_files_are_refused_whole() {
        let bad = [
            "symbol,date,open,high,low,close\n",
            "symbol,date,open,high,low,close,volume\nXYZ,2026-06-12,10,9,11,10,1\n",
            "symbol,date,open,high,low,close,volume\nXYZ,2026-06-12,10,11,9,10,1\nXYZ,2026-06-12,10,11,9,10,1\n",
            "symbol,date,open,high,low,close,volume\nXYZ,2026-13-01,10,11,9,10,1\n",
        ];
        for (i, t) in bad.iter().enumerate() {
            assert!(read_daily(&tmp(&format!("bad{i}.csv"), t)).is_err(), "{t}");
        }
        assert_eq!(read_float(&tmp("f.csv", "symbol,float_shares\nWFF,4500000\n")).unwrap(), vec![("WFF".to_string(), 4.5e6)]);
        assert!(read_float(&tmp("f2.csv", "symbol,float_shares\nWFF,-1\n")).is_err());
    }
}
