//! Per-second days for replay. Massive's flat files are minute bars only; its REST API has every
//! stock's per-second bars (on Starter), one request per stock per day. A day is fetched once,
//! eight requests at a time, and kept as `flatfiles/second_aggs/YYYY-MM-DD.csv.gz` in the minute
//! files' own format (so the same reader replays it, as one-second bars).
//!
//! The stocks fetched are the ones in that day's minute file (every stock that traded). Measured
//! 2026-10-12 on a 40-stock sample of 2026-10-09: ~5M one-second bars for the market, tens of MB.

use crate::rest::Rest;
use crate::MassiveError;
use chrono::NaiveDate;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub fn local_path(data_dir: &Path, date: NaiveDate) -> PathBuf {
    data_dir.join("flatfiles").join("second_aggs").join(format!("{date}.csv.gz"))
}

/// Fetch every listed stock's per-second bars for `date` and write the day file (atomically: a
/// failed or interrupted fetch leaves nothing behind). A stock that fails three times fails the
/// day, naming it: a replay silently missing stocks would mislead.
pub fn fetch_day(rest: &Rest, data_dir: &Path, date: NaiveDate, symbols: &[String], progress: &(dyn Fn(usize, usize) + Sync)) -> Result<PathBuf, MassiveError> {
    const THREADS: usize = 8;
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let rows: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let failed: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|s| {
        for _ in 0..THREADS {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= symbols.len() || failed.lock().expect("failed").is_some() {
                    return;
                }
                let sym = &symbols[i];
                let mut last_err = None;
                let mut got = None;
                for attempt in 0..3 {
                    match rest.chart_bars(sym, "second", date, date, false) {
                        Ok(b) => {
                            got = Some(b);
                            break;
                        }
                        Err(e) => {
                            last_err = Some(e.to_string());
                            std::thread::sleep(std::time::Duration::from_millis(500 * (attempt + 1)));
                        }
                    }
                }
                match got {
                    Some(bars) => {
                        let mut out = String::with_capacity(bars.len() * 64);
                        for [t, o, h, l, c, v] in bars {
                            // ticker,volume,open,close,high,low,window_start(ns),transactions
                            out.push_str(&format!("{sym},{v},{o},{c},{h},{l},{},0\n", (t as i64) * 1_000_000));
                        }
                        rows.lock().expect("rows").push(out);
                        let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                        if n % 100 == 0 || n == symbols.len() {
                            progress(n, symbols.len());
                        }
                    }
                    None => {
                        *failed.lock().expect("failed") = Some(format!("{sym}: {}", last_err.unwrap_or_default()));
                        return;
                    }
                }
            });
        }
    });
    if let Some(e) = failed.into_inner().expect("failed") {
        return Err(MassiveError::Io(format!("per-second bars for {date} not fetched ({e})")));
    }
    let path = local_path(data_dir, date);
    std::fs::create_dir_all(path.parent().expect("dir")).map_err(|e| MassiveError::Io(e.to_string()))?;
    let tmp = path.with_extension("gz.tmp");
    {
        let f = std::fs::File::create(&tmp).map_err(|e| MassiveError::Io(e.to_string()))?;
        let mut gz = flate2::write::GzEncoder::new(std::io::BufWriter::new(f), flate2::Compression::fast());
        let io = |e: std::io::Error| MassiveError::Io(e.to_string());
        gz.write_all(b"ticker,volume,open,close,high,low,window_start,transactions\n").map_err(io)?;
        for chunk in rows.into_inner().expect("rows") {
            gz.write_all(chunk.as_bytes()).map_err(io)?;
        }
        gz.finish().map_err(io)?.flush().map_err(io)?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| MassiveError::Io(e.to_string()))?;
    Ok(path)
}

/// The stocks in a day's minute file (every stock that traded that day).
pub fn symbols_in(minute_file: &Path) -> Result<Vec<String>, MassiveError> {
    use std::io::BufRead;
    let f = std::fs::File::open(minute_file).map_err(|e| MassiveError::Io(format!("{}: {e}", minute_file.display())))?;
    let rd = std::io::BufReader::new(flate2::read::GzDecoder::new(f));
    let mut out: Vec<String> = vec![];
    let mut seen = std::collections::HashSet::new();
    for l in rd.lines().skip(1) {
        let l = l.map_err(|e| MassiveError::Io(e.to_string()))?;
        if let Some(t) = l.split(',').next() {
            if !t.is_empty() && seen.insert(t.to_string()) {
                out.push(t.to_string());
            }
        }
    }
    Ok(out)
}
