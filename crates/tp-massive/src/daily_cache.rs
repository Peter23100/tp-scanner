//! The daily-bar store: one file per trading day, every US stock, **as printed** (unadjusted).
//!
//! Stored raw because a raw day never changes: a split next month would silently invalidate a
//! cache of adjusted prices. Adjustment happens when a reference is built, onto the basis of the
//! session it serves, from the splits list ([`RawDaily::adjusted_history`]). The same store
//! therefore serves today's live session and any past session a replay asks for.
//!
//! Layout under `dir`: `YYYY-MM-DD.csv` (`symbol,open,high,low,close,volume`; a header-only file
//! marks a day the market was closed) and `splits.json`.

use crate::rest::{DayRow, Rest, Split};
use crate::MassiveError;
use chrono::{Datelike, Duration, NaiveDate, NaiveTime, Weekday};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tp_marketdata::bar::Bar;
use tp_marketdata::price::{AdjFactor, Adjusted, Price, Unadjusted};
use tp_marketdata::time::{SessionDate, Ts};

const HEADER: &str = "symbol,open,high,low,close,volume";

/// Calendar days fetched on a first run: comfortably more than 260 trading days.
pub const DEFAULT_CALENDAR_DAYS: i64 = 400;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RefreshReport {
    pub fetched: usize,
    pub already_cached: usize,
    pub closed: usize,
    pub splits: usize,
}

fn day_path(dir: &Path, d: NaiveDate) -> PathBuf {
    dir.join(format!("{d}.csv"))
}

fn is_weekend(d: NaiveDate) -> bool {
    matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

/// Fetch every weekday in `[before - calendar_days, before)` not already cached, then the splits.
/// `before` is normally today (New York): a day is only stored once it is over. `progress` gets
/// (done, total) as days complete.
pub fn refresh(
    rest: &Rest,
    dir: &Path,
    before: NaiveDate,
    calendar_days: i64,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<RefreshReport, MassiveError> {
    fs::create_dir_all(dir).map_err(|e| MassiveError::Io(format!("{}: {e}", dir.display())))?;
    let mut report = RefreshReport::default();
    let mut todo = vec![];
    for k in 1..=calendar_days {
        let d = before - Duration::days(k);
        if is_weekend(d) {
            continue;
        }
        if day_path(dir, d).exists() {
            report.already_cached += 1;
        } else {
            todo.push(d);
        }
    }
    let total = todo.len();
    let queue = Mutex::new(todo);
    let done = Mutex::new((0usize, 0usize, None::<MassiveError>));
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| loop {
                let Some(d) = queue.lock().expect("queue").pop() else { break };
                if done.lock().expect("done").2.is_some() {
                    break;
                }
                let r = rest.grouped_daily(d).and_then(|rows| write_day(dir, d, &rows).map(|()| rows.is_empty()));
                let mut g = done.lock().expect("done");
                match r {
                    Ok(closed) => {
                        g.0 += 1;
                        if closed {
                            g.1 += 1;
                        }
                        progress(g.0, total);
                    }
                    Err(e) => {
                        g.2.get_or_insert(e);
                    }
                }
            });
        }
    });
    let (fetched, closed, err) = done.into_inner().expect("done");
    if let Some(e) = err {
        return Err(e);
    }
    report.fetched = fetched;
    report.closed = closed;

    let splits = rest.splits_since(before - Duration::days(calendar_days + 10))?;
    report.splits = splits.len();
    write_splits(dir, &splits)?;
    Ok(report)
}

fn io(p: &Path, e: std::io::Error) -> MassiveError {
    MassiveError::Io(format!("{}: {e}", p.display()))
}

/// Written to a temporary name and renamed, so a crash never leaves a half day that looks whole.
pub fn write_day(dir: &Path, d: NaiveDate, rows: &[DayRow]) -> Result<(), MassiveError> {
    let mut s = String::with_capacity(rows.len() * 48 + 40);
    s.push_str(HEADER);
    s.push('\n');
    for r in rows {
        s.push_str(&format!("{},{},{},{},{},{}\n", r.symbol, r.open, r.high, r.low, r.close, r.volume));
    }
    let p = day_path(dir, d);
    let tmp = p.with_extension("csv.tmp");
    fs::write(&tmp, s).map_err(|e| io(&tmp, e))?;
    fs::rename(&tmp, &p).map_err(|e| io(&p, e))
}

fn write_splits(dir: &Path, splits: &[Split]) -> Result<(), MassiveError> {
    let v: Vec<serde_json::Value> = splits
        .iter()
        .map(|s| serde_json::json!({"ticker": s.symbol, "execution_date": s.execution_date.to_string(), "factor": s.factor.get()}))
        .collect();
    let p = dir.join("splits.json");
    let tmp = dir.join("splits.json.tmp");
    fs::write(&tmp, serde_json::to_string(&v).expect("json")).map_err(|e| io(&tmp, e))?;
    fs::rename(&tmp, &p).map_err(|e| io(&p, e))
}

/// Daily bars as printed, plus the splits needed to adjust them.
#[derive(Debug, Default)]
pub struct RawDaily {
    /// Oldest first.
    bars: HashMap<String, Vec<Bar<Unadjusted>>>,
    /// Per symbol, oldest first.
    splits: HashMap<String, Vec<(NaiveDate, AdjFactor)>>,
    pub days: usize,
}

impl RawDaily {
    /// Load every cached day on or after `from`. A malformed file fails the load, naming it.
    pub fn load(dir: &Path, from: NaiveDate) -> Result<RawDaily, MassiveError> {
        let mut dates: Vec<NaiveDate> = fs::read_dir(dir)
            .map_err(|e| io(dir, e))?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let n = e.file_name().into_string().ok()?;
                NaiveDate::parse_from_str(n.strip_suffix(".csv")?, "%Y-%m-%d").ok()
            })
            .filter(|d| *d >= from)
            .collect();
        dates.sort();
        let mut out = RawDaily::default();
        for d in dates {
            let p = day_path(dir, d);
            let text = fs::read_to_string(&p).map_err(|e| io(&p, e))?;
            let rows = parse_day(&text).map_err(|e| MassiveError::Parse(format!("{}: {e}", p.display())))?;
            if rows.is_empty() {
                continue;
            }
            out.days += 1;
            let start = Ts::from_ny_local(d, NaiveTime::MIN).map_err(|e| MassiveError::Parse(e.to_string()))?;
            for r in rows {
                out.bars.entry(r.symbol).or_default().push(Bar {
                    start,
                    open: Price::new(r.open),
                    high: Price::new(r.high),
                    low: Price::new(r.low),
                    close: Price::new(r.close),
                    volume: r.volume,
                });
            }
        }
        let sp = dir.join("splits.json");
        if sp.exists() {
            let text = fs::read_to_string(&sp).map_err(|e| io(&sp, e))?;
            let v: Vec<serde_json::Value> = serde_json::from_str(&text).map_err(|e| MassiveError::Parse(format!("{}: {e}", sp.display())))?;
            for s in v {
                let (Some(t), Some(d), Some(f)) = (
                    s.get("ticker").and_then(|x| x.as_str()),
                    s.get("execution_date").and_then(|x| x.as_str()).and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()),
                    s.get("factor").and_then(|x| x.as_f64()).and_then(AdjFactor::new),
                ) else {
                    continue;
                };
                out.splits.entry(t.to_string()).or_default().push((d, f));
            }
            for v in out.splits.values_mut() {
                v.sort_by_key(|x| x.0);
            }
        }
        Ok(out)
    }

    pub fn symbols(&self) -> usize {
        self.bars.len()
    }

    pub fn insert_for_test(&mut self, symbol: &str, bars: Vec<Bar<Unadjusted>>, splits: Vec<(NaiveDate, AdjFactor)>) {
        self.bars.insert(symbol.to_string(), bars);
        self.splits.insert(symbol.to_string(), splits);
    }

    /// The last `max_days` bars before `session`, adjusted onto `session`'s basis: a bar dated D
    /// is scaled by every split executing after D and on or before `session`. Splits after the
    /// session are not applied; that session's prints didn't know about them.
    pub fn adjusted_history(&self, symbol: &str, session: SessionDate, max_days: usize) -> Vec<Bar<Adjusted>> {
        let Some(bars) = self.bars.get(symbol) else { return vec![] };
        let cut = bars.partition_point(|b| b.session_date() < session);
        let hist = &bars[cut.saturating_sub(max_days)..cut];
        let splits: &[(NaiveDate, AdjFactor)] = self.splits.get(symbol).map_or(&[], |v| v.as_slice());
        hist.iter()
            .map(|b| {
                let d = b.session_date().0;
                let f = splits
                    .iter()
                    .filter(|(x, _)| *x > d && *x <= session.0)
                    .fold(AdjFactor::IDENTITY, |acc, (_, f)| acc.then(*f));
                b.adjust(f)
            })
            .collect()
    }
}

fn parse_day(text: &str) -> Result<Vec<DayRow>, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(HEADER) {
        return Err(format!("expected header `{HEADER}`"));
    }
    let mut out = vec![];
    for (i, l) in lines.enumerate() {
        if l.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = l.split(',').collect();
        let [s, o, h, lo, c, v] = f[..] else { return Err(format!("line {}: expected 6 fields", i + 2)) };
        let n = |x: &str| x.parse::<f64>().map_err(|_| format!("line {}: `{x}` is not a number", i + 2));
        out.push(DayRow { symbol: s.to_string(), open: n(o)?, high: n(h)?, low: n(lo)?, close: n(c)?, volume: n(v)? });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn tmpdir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("tp-massive-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn row(sym: &str, c: f64, v: f64) -> DayRow {
        DayRow { symbol: sym.into(), open: c, high: c, low: c, close: c, volume: v }
    }

    #[test]
    fn days_round_trip_and_closed_days_are_kept_but_skipped() {
        let dir = tmpdir("rt");
        write_day(&dir, d("2026-10-07"), &[row("AAA", 10.0, 100.0), row("BBB", 5.0, 50.0)]).unwrap();
        write_day(&dir, d("2026-10-08"), &[]).unwrap();
        write_day(&dir, d("2026-10-09"), &[row("AAA", 11.0, 110.0)]).unwrap();
        let raw = RawDaily::load(&dir, d("2026-01-01")).unwrap();
        assert_eq!(raw.days, 2);
        assert_eq!(raw.symbols(), 2);
        let h = raw.adjusted_history("AAA", SessionDate(d("2026-10-12")), 260);
        assert_eq!(h.iter().map(|b| b.close.value()).collect::<Vec<_>>(), vec![10.0, 11.0]);
        assert!(!dir.join("2026-10-07.csv.tmp").exists());
    }

    #[test]
    fn a_malformed_day_fails_the_load_naming_the_file() {
        let dir = tmpdir("bad");
        fs::write(dir.join("2026-10-07.csv"), "nope\n").unwrap();
        let e = RawDaily::load(&dir, d("2026-01-01")).unwrap_err().to_string();
        assert!(e.contains("2026-10-07.csv"), "{e}");
    }

    #[test]
    fn history_is_adjusted_onto_the_session_it_serves() {
        // A 1-for-4 reverse split executes 2026-09-02.
        let mut raw = RawDaily::default();
        let bar = |day: &str, c: f64| Bar {
            start: Ts::from_ny_local(d(day), NaiveTime::MIN).unwrap(),
            open: Price::new(c),
            high: Price::new(c),
            low: Price::new(c),
            close: Price::new(c),
            volume: 4000.0,
        };
        raw.insert_for_test(
            "PFSA",
            vec![bar("2026-08-31", 1.0), bar("2026-09-01", 1.1), bar("2026-09-02", 4.6)],
            vec![(d("2026-09-02"), AdjFactor::from_split(1.0, 4.0).unwrap())],
        );
        // For a session after the split: pre-split days are scaled x4, volume /4.
        let after = raw.adjusted_history("PFSA", SessionDate(d("2026-09-03")), 260);
        assert_eq!(after.iter().map(|b| b.close.value()).collect::<Vec<_>>(), vec![4.0, 4.4, 4.6]);
        assert_eq!(after[0].volume, 1000.0);
        // For the split day itself: the prior days are already on the new basis.
        let on = raw.adjusted_history("PFSA", SessionDate(d("2026-09-02")), 260);
        assert_eq!(on.iter().map(|b| b.close.value()).collect::<Vec<_>>(), vec![4.0, 4.4]);
        // For a session before the split: nothing is applied, and no lookahead.
        let before = raw.adjusted_history("PFSA", SessionDate(d("2026-09-01")), 260);
        assert_eq!(before.iter().map(|b| b.close.value()).collect::<Vec<_>>(), vec![1.0]);
    }

    #[test]
    fn weekends_are_never_fetched() {
        assert!(is_weekend(d("2026-10-10")));
        assert!(is_weekend(d("2026-10-11")));
        assert!(!is_weekend(d("2026-10-09")));
    }
}
