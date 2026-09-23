//! Reader for Massive (ex-Polygon) **US stocks minute-aggregate** flat files, one trading day
//! per file.
//!
//! What this relies on, confirmed from Massive's public docs (massive.com/docs/flat-files/stocks/
//! minute-aggregates, read 2026-09-23): the columns are `ticker,volume,open,close,high,low,
//! window_start,transactions` in that order, and `window_start` is a Unix timestamp in
//! **nanoseconds** marking the start of the minute.
//!
//! What is deliberately NOT assumed:
//! - **Compression.** The caller hands in a decompressed reader, so no guess about `.gz` is baked
//!   in.
//! - **Adjustment.** The docs don't say whether flat-file prices are split-adjusted. Bars come out
//!   as [`Unadjusted`] (as printed), which is right only if Massive's files are raw.
//!   ⚠ **Confirm with Massive before trusting any multi-day computation built on these.** If they
//!   turn out to be adjusted, this reader must emit `Bar<Adjusted>` instead, and the type system
//!   will then force every caller to handle it.
//! - **Row order.** Rows are sorted into time order here. A replay must run in time order, and a
//!   per-ticker file order would otherwise replay one ticker's whole day before the next.
//!
//! Every check fails the whole file, loudly: the exact header, a sane timestamp (a
//! millisecond/nanosecond mix-up lands in 1970 or the far future and is caught), sane OHLC,
//! non-negative volume. A corpus that silently drops rows is worse than one that refuses to load.
//!
//! Memory: a full-market day is a few million rows, held in memory while sorting (on the order
//! of a few hundred MB). A streaming k-way merge can replace this if measurement says it must.

use crate::bar::Bar;
use crate::price::{Price, Unadjusted};
use crate::source::{MarketEvent, ReplaySource, Resolution, SourceError, Symbol};
use crate::time::Ts;
use std::collections::HashMap;
use std::io::BufRead;
use std::sync::Arc;

pub const MINUTE_AGGS_HEADER: &str = "ticker,volume,open,close,high,low,window_start,transactions";

/// Plausible window_start range in ms: 2000-01-01 to 2100-01-01. Outside it the unit is wrong.
const SANE_MS: std::ops::Range<i64> = 946_684_800_000..4_102_444_800_000;

/// Read one day file into a time-ordered replay of one-minute bars.
pub fn read_minute_aggs(input: impl BufRead) -> Result<ReplaySource, SourceError> {
    let mut rows: Vec<(Ts, Symbol, Bar<Unadjusted>)> = Vec::new();
    let mut interned: HashMap<String, Symbol> = HashMap::new();
    let mut lines = input.lines().enumerate();
    match lines.next() {
        Some((_, Ok(h))) if h.trim() == MINUTE_AGGS_HEADER => {}
        Some((_, Ok(h))) => {
            return Err(SourceError::Parse { line: 1, msg: format!("header `{}` is not `{MINUTE_AGGS_HEADER}`", h.trim()) })
        }
        Some((_, Err(e))) => return Err(SourceError::Parse { line: 1, msg: e.to_string() }),
        None => return Err(SourceError::Parse { line: 0, msg: "empty file".into() }),
    }
    for (i, l) in lines {
        let line = i + 1;
        let l = l.map_err(|e| SourceError::Parse { line, msg: e.to_string() })?;
        let l = l.trim();
        if l.is_empty() {
            continue;
        }
        let f: Vec<&str> = l.split(',').collect();
        let [ticker, volume, open, close, high, low, window_start, _transactions] = f[..] else {
            return Err(SourceError::Parse { line, msg: format!("expected 8 fields, got {}", f.len()) });
        };
        let num = |s: &str, what: &str| -> Result<f64, SourceError> {
            s.parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .ok_or_else(|| SourceError::Parse { line, msg: format!("{what} `{s}` is not a finite number") })
        };
        let (v, o, c, h, lo) = (num(volume, "volume")?, num(open, "open")?, num(close, "close")?, num(high, "high")?, num(low, "low")?);
        if !(lo <= o && o <= h && lo <= c && c <= h) {
            return Err(SourceError::Parse { line, msg: format!("insane bar o={o} h={h} l={lo} c={c}") });
        }
        if v < 0.0 {
            return Err(SourceError::Parse { line, msg: "negative volume".into() });
        }
        let ns: i64 = window_start
            .parse()
            .map_err(|_| SourceError::Parse { line, msg: format!("window_start `{window_start}` is not an integer") })?;
        let ms = ns.div_euclid(1_000_000);
        if !SANE_MS.contains(&ms) || ns.rem_euclid(1_000_000) != 0 {
            return Err(SourceError::Parse {
                line,
                msg: format!("window_start {ns} is not a whole-millisecond nanosecond timestamp in 2000-2100 (wrong unit?)"),
            });
        }
        let start = Ts::from_millis(ms).expect("range-checked");
        if ticker.is_empty() {
            return Err(SourceError::Parse { line, msg: "empty ticker".into() });
        }
        let sym = interned.entry(ticker.to_string()).or_insert_with(|| Arc::from(ticker)).clone();
        let bar = Bar { start, open: Price::new(o), high: Price::new(h), low: Price::new(lo), close: Price::new(c), volume: v };
        rows.push((start, sym, bar));
    }
    // Time order, ties broken by ticker so the replay is deterministic.
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let events = rows
        .into_iter()
        .map(|(_, symbol, bar)| MarketEvent::Bar { symbol, resolution: Resolution::Minute, bar })
        .collect();
    ReplaySource::from_events(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MarketSource;
    use crate::time::Phase;

    // 1781530200000 ms = 2026-06-15 13:30 UTC = 09:30 New York.
    const DAY: &str = "\
ticker,volume,open,close,high,low,window_start,transactions
AAPL,1000,200,200.5,201,199.5,1781530200000000000,12
AAPL,1200,200.5,201.9,202,200.4,1781530260000000000,15
MSFT,500,450,450.25,451,449,1781530200000000000,7
MSFT,300,450.25,450,450.5,449.9,1781526600000000000,3
";

    fn drain(mut s: ReplaySource) -> Vec<(String, i64)> {
        let mut v = vec![];
        while let Some(e) = s.next_event() {
            let MarketEvent::Bar { symbol, bar, .. } = e.unwrap();
            v.push((symbol.to_string(), bar.start.millis()));
        }
        v
    }

    #[test]
    fn ticker_ordered_file_replays_in_time_order() {
        let ev = drain(read_minute_aggs(DAY.as_bytes()).unwrap());
        assert_eq!(
            ev,
            vec![
                ("MSFT".into(), 1781526600000),
                ("AAPL".into(), 1781530200000),
                ("MSFT".into(), 1781530200000),
                ("AAPL".into(), 1781530260000),
            ]
        );
    }

    #[test]
    fn nanoseconds_become_new_york_minutes() {
        let mut s = read_minute_aggs(DAY.as_bytes()).unwrap();
        let MarketEvent::Bar { bar, .. } = s.next_event().unwrap().unwrap();
        assert_eq!(bar.start.ny().format("%H:%M").to_string(), "08:30");
        assert_eq!(bar.phase(), Phase::Pre);
    }

    #[test]
    fn wrong_units_and_bad_rows_fail_loudly() {
        let with = |row: &str| format!("{MINUTE_AGGS_HEADER}\n{row}\n");
        let cases = [
            (with("AAPL,1000,200,200.5,201,199.5,1781530200000,12"), "milliseconds, not ns"),
            (with("AAPL,1000,200,200.5,201,199.5,1781530200000000001,12"), "sub-millisecond ns"),
            (with("AAPL,1000,200,203,201,199.5,1781530200000000000,12"), "close above high"),
            (with("AAPL,-1,200,200.5,201,199.5,1781530200000000000,12"), "negative volume"),
            (with("AAPL,1000,200,200.5,201,199.5,1781530200000000000"), "7 fields"),
            ("ticker,open,close\n".to_string(), "wrong header"),
            (String::new(), "empty"),
        ];
        for (csv, why) in cases {
            assert!(read_minute_aggs(csv.as_bytes()).is_err(), "{why} should fail");
        }
    }
}
