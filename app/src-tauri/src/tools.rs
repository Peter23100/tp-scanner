//! Side tools that run one filter on its own, off the engine thread, so the live scan never
//! waits on them:
//!
//! - **diagnose**: a symbol over the day, minute by minute — when the filter would have fired, how
//!   often each clause passed, and the closest miss with the values that blocked it.
//! - **test**: an edited filter against the unedited one over a whole recorded day, before saving.
//! - **source / apply**: find a filter's block in its `.scan` file, and splice an edit back in,
//!   refusing anything that doesn't load.
//!
//! One filter on its own fires exactly as it does inside the full program (filters don't see each
//! other), provided it gets the same references and the same feed start.

use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tp_marketdata::source::{MarketEvent, MarketSource, Resolution};
use tp_marketdata::time::Ts;
use tp_massive::daily_cache::RawDaily;
use tp_scan_engine::compile::{ExplainNode, Kind, NodeKind, Program, TraceVal, Tri};
use tp_scan_engine::engine::{Engine, EngineError, ListChange, Signal, Sink};

/// What a side run needs, copied out of the engine thread's shared state.
#[derive(Clone)]
pub struct Context {
    pub program: Program,
    pub daily: Option<Arc<RawDaily>>,
    pub references: Arc<Vec<(String, tp_scan_engine::state::Reference)>>,
    pub replay_date: Option<chrono::NaiveDate>,
    pub feed_start: Option<Ts>,
    pub data_dir: PathBuf,
}

fn engine_for(p: &Program, cx: &Context, only_symbol: Option<&str>, feed_start: Option<Ts>) -> Result<Engine, String> {
    let mut e = Engine::new(p).map_err(|e| e.to_string())?;
    if let Some(d) = cx.daily.clone() {
        e.set_daily_provider(tp_scan_engine::daily::provider_from(move |sym, session| d.adjusted_history(sym, session, tp_scan_engine::daily::HISTORY_DAYS)));
    }
    for (s, r) in cx.references.iter().filter(|(s, _)| only_symbol.is_none_or(|o| o == s)) {
        e.set_reference(s, r.clone());
    }
    if let Some(t) = feed_start {
        e.set_feed_start(t);
    }
    Ok(e)
}

/// A recorded day's events, kept for the next call (the filter-tuning loop tests the same day
/// over and over; reading it is the slow part).
static DAY: Mutex<Option<(chrono::NaiveDate, Arc<Vec<MarketEvent>>)>> = Mutex::new(None);

fn day_events(data_dir: &Path, date: chrono::NaiveDate) -> Result<Arc<Vec<MarketEvent>>, String> {
    if let Some((d, ev)) = DAY.lock().expect("day").as_ref() {
        if *d == date {
            return Ok(ev.clone());
        }
    }
    let path = tp_massive::flatfiles::local_path(data_dir, date);
    let f = std::fs::File::open(&path).map_err(|e| format!("{}: {e} (replay {date} once to download it)", path.display()))?;
    let rd = std::io::BufReader::new(flate2::read::GzDecoder::new(f));
    let mut src = tp_marketdata::flatfile::read_minute_aggs(rd).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = vec![];
    while let Some(ev) = src.next_event() {
        if let Ok(ev) = ev {
            out.push(ev);
        }
    }
    let out = Arc::new(out);
    *DAY.lock().expect("day") = Some((date, out.clone()));
    Ok(out)
}

/// The day a test runs on: the replay's day, else the newest downloaded day.
fn test_day(cx: &Context) -> Result<chrono::NaiveDate, String> {
    if let Some(d) = cx.replay_date {
        return Ok(d);
    }
    let dir = cx.data_dir.join("flatfiles").join("minute_aggs");
    std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok()?.file_name().to_str()?.strip_suffix(".csv.gz").and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()))
        .max()
        .ok_or_else(|| "no recorded day to test on: run a Replay of a recent day first".to_string())
}

/// Signals and list entries, as they happen.
#[derive(Default)]
struct Collect {
    fires: Vec<(Ts, String, Option<f64>)>,
}

impl Sink for Collect {
    fn signal(&mut self, s: &Signal) {
        self.fires.push((s.at, s.symbol.to_string(), s.last));
    }
    fn list_change(&mut self, c: &ListChange<'_>) {
        if c.entered {
            self.fires.push((c.at, c.symbol.to_string(), None));
        }
    }
    fn error(&mut self, _symbol: &str, _e: &EngineError) {}
}

fn hm(t: Ts) -> String {
    t.ny().format("%H:%M").to_string()
}

// ---------------------------------------------------------------------------------------------
// Diagnose

#[derive(Debug, Serialize, Clone)]
pub struct ClauseStat {
    pub clause: String,
    /// In-hours minutes the clause was true / unknown, of `of`.
    pub passed: usize,
    pub unknown: usize,
    pub of: usize,
}

#[derive(Debug, Serialize, Clone)]
pub struct Miss {
    pub time: String,
    /// The clauses that stopped it, each with the values it compared.
    pub blocking: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Diagnosis {
    pub filter: String,
    pub symbol: String,
    pub day: String,
    pub source: String,
    pub kind: &'static str,
    pub hours: String,
    pub bars: usize,
    pub first_bar: Option<String>,
    pub minutes_in_hours: usize,
    /// Minutes the whole filter was true (a trigger fires on the first, then waits its cooldown).
    pub true_minutes: usize,
    /// Fires (signals, or list entries): time, last price.
    pub fires: Vec<(String, Option<f64>)>,
    pub clauses: Vec<ClauseStat>,
    pub closest: Option<Miss>,
    /// At the symbol's last bar.
    pub last: Option<Miss>,
}

/// "label" with the numbers a comparison read, e.g. `PM_RVOL >= 3.0   (PM_RVOL = 2.61)`.
fn clause_text(n: &ExplainNode) -> String {
    let vals: Vec<String> = n
        .children
        .iter()
        .filter(|c| c.kind == NodeKind::Num && c.label.parse::<f64>().is_err())
        .map(|c| match c.value {
            TraceVal::Num(Some(x)) => format!("{} = {}", c.label, fmt_num(x)),
            TraceVal::Num(None) => format!("{} = ? (unknown)", c.label),
            _ => c.label.clone(),
        })
        .collect();
    let state = match n.value {
        TraceVal::Bool(Tri::Unknown) => " [unknown]",
        _ => "",
    };
    if vals.is_empty() {
        format!("{}{state}", n.label)
    } else {
        format!("{}{state}   ({})", n.label, vals.join(", "))
    }
}

fn fmt_num(x: f64) -> String {
    let a = x.abs();
    if a >= 1e6 {
        format!("{:.2}M", x / 1e6)
    } else if a >= 1e4 {
        format!("{x:.0}")
    } else if a >= 1.0 {
        format!("{x:.2}")
    } else {
        format!("{x:.4}")
    }
}

fn top_clauses(root: &ExplainNode) -> Vec<&ExplainNode> {
    match root.kind {
        NodeKind::And => root.children.iter().collect(),
        _ => vec![root],
    }
}

/// Run `filter` over `symbol`'s bars for the day: the replay's day from its file, else today from
/// Massive REST.
pub fn diagnose(cx: &Context, rest: Option<&tp_massive::rest::Rest>, filter: &str, symbol: &str) -> Result<Diagnosis, String> {
    let symbol = symbol.trim().to_uppercase();
    let program = cx.program.only(filter).ok_or_else(|| format!("no filter `{filter}` loaded"))?;
    let def = &program.filters[0];
    let day = cx.replay_date.unwrap_or_else(|| Ts::from_millis(crate::scanner::now_ms()).expect("now").ny().date_naive());
    let four = Ts::from_ny_local(day, chrono::NaiveTime::from_hms_opt(4, 0, 0).expect("04:00")).ok();
    // The symbol's per-second bars over REST (one request), the live scan's own granularity.
    let per_second = rest.and_then(|r| r.chart_bars(&symbol, "second", day, day, false).ok()).filter(|b| !b.is_empty()).map(|bars| {
        let sym: tp_marketdata::source::Symbol = Arc::from(symbol.as_str());
        bars.iter()
            .filter_map(|&[t, o, h, l, c, v]| {
                let bar = tp_marketdata::bar::Bar { start: Ts::from_millis(t as i64)?, open: tp_marketdata::price::Price::new(o), high: tp_marketdata::price::Price::new(h), low: tp_marketdata::price::Price::new(l), close: tp_marketdata::price::Price::new(c), volume: v };
                Some(MarketEvent::Bar { symbol: sym.clone(), resolution: Resolution::Second, bar })
            })
            .collect::<Vec<_>>()
    });
    // A replay only ever looks at its own day up to now; a past day's bars beyond that are fine
    // for a diagnosis (it asks about the whole day), but live must not run past the clock: REST
    // ends at the delayed feed's edge anyway.
    let (source, events, feed_start): (String, Vec<MarketEvent>, Option<Ts>) = match (per_second, cx.replay_date) {
        (Some(ev), Some(_)) => (format!("Massive per-second bars for {day}"), ev, four),
        (Some(ev), None) => (format!("Massive per-second bars for {day} (delayed)"), ev, cx.feed_start.or(four)),
        (None, Some(d)) => {
            let all = day_events(&cx.data_dir, d)?;
            let start = all.first().map(|e| match e {
                MarketEvent::Bar { bar, .. } => bar.start,
                #[allow(unreachable_patterns)]
                _ => e.available_at(),
            });
            let mine = all.iter().filter(|e| matches!(e, MarketEvent::Bar { symbol: s, .. } if **s == *symbol)).cloned().collect();
            (format!("the {d} replay file (minute bars)"), mine, start)
        }
        (None, None) => {
            let rest = rest.ok_or("diagnosis needs MASSIVE_API_KEY")?;
            let bars = rest.minute_bars(&symbol, day, day).map_err(|e| e.to_string())?;
            let sym: tp_marketdata::source::Symbol = Arc::from(symbol.as_str());
            let ev = bars.into_iter().map(|bar| MarketEvent::Bar { symbol: sym.clone(), resolution: Resolution::Minute, bar }).collect();
            ("Massive minute bars (delayed)".into(), ev, cx.feed_start.or(four))
        }
    };
    if events.is_empty() {
        return Err(format!("no bars for {symbol} on {day}"));
    }
    let mut engine = engine_for(&program, cx, Some(&symbol), feed_start)?;
    let mut sink = Collect::default();
    let mut d = Diagnosis {
        filter: filter.to_string(),
        symbol: symbol.clone(),
        day: day.to_string(),
        source,
        kind: if def.kind == Kind::List { "list" } else { "trigger" },
        hours: format!("{:?}", def.hours).to_lowercase(),
        bars: events.len(),
        first_bar: None,
        minutes_in_hours: 0,
        true_minutes: 0,
        fires: vec![],
        clauses: vec![],
        closest: None,
        last: None,
    };
    let hours = def.hours;
    let mut best: Option<(usize, Miss)> = None;
    for ev in &events {
        engine.on_event(ev, &mut sink);
        let MarketEvent::Bar { bar, .. } = ev else { continue };
        d.first_bar.get_or_insert_with(|| hm(bar.start));
        let Some(ex) = engine.explain(filter, &symbol) else { continue };
        let at = hm(ev.available_at());
        let clauses = top_clauses(&ex.root);
        let failing: Vec<&ExplainNode> = {
            let f: Vec<_> = clauses.iter().copied().filter(|n| n.value == TraceVal::Bool(Tri::False)).collect();
            if f.is_empty() {
                clauses.iter().copied().filter(|n| n.value == TraceVal::Bool(Tri::Unknown)).collect()
            } else {
                f
            }
        };
        let miss = Miss { time: at, blocking: failing.iter().map(|n| clause_text(n)).collect() };
        if !hours.admits(bar.phase()) {
            d.last = Some(Miss { blocking: vec![format!("outside the filter's hours ({})", d.hours)], ..miss });
            continue;
        }
        d.minutes_in_hours += 1;
        if d.clauses.is_empty() {
            d.clauses = clauses.iter().map(|n| ClauseStat { clause: n.label.clone(), passed: 0, unknown: 0, of: 0 }).collect();
        }
        for (st, n) in d.clauses.iter_mut().zip(&clauses) {
            st.of += 1;
            match n.value {
                TraceVal::Bool(Tri::True) => st.passed += 1,
                TraceVal::Bool(Tri::Unknown) => st.unknown += 1,
                _ => {}
            }
        }
        if ex.result == Tri::True {
            d.true_minutes += 1;
        } else if best.as_ref().is_none_or(|(k, _)| failing.len() <= *k) {
            best = Some((failing.len(), miss.clone()));
        }
        d.last = Some(if ex.result == Tri::True { Miss { blocking: vec![], ..miss } } else { miss });
    }
    d.fires = sink.fires.iter().map(|(t, _, last)| (hm(*t), *last)).collect();
    d.closest = best.map(|(_, m)| m);
    Ok(d)
}

// ---------------------------------------------------------------------------------------------
// Test an edit

#[derive(Debug, Serialize, Default)]
pub struct Count {
    pub fires: usize,
    /// Symbol → fires, most first.
    pub symbols: Vec<(String, usize)>,
}

#[derive(Debug, Serialize)]
pub struct TestResult {
    pub day: String,
    pub before: Count,
    pub after: Count,
    /// Symbols that fire only after the edit / only before it.
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub seconds: f64,
}

fn count(p: &Program, cx: &Context, events: &[MarketEvent]) -> Result<Count, String> {
    let start = events.first().map(|e| match e {
        MarketEvent::Bar { bar, .. } => bar.start,
        #[allow(unreachable_patterns)]
        _ => e.available_at(),
    });
    let mut e = engine_for(p, cx, None, start)?;
    let mut sink = Collect::default();
    for ev in events {
        e.on_event(ev, &mut sink);
    }
    let mut by: HashMap<String, usize> = HashMap::new();
    for (_, s, _) in &sink.fires {
        *by.entry(s.clone()).or_default() += 1;
    }
    let mut symbols: Vec<(String, usize)> = by.into_iter().collect();
    symbols.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(Count { fires: sink.fires.len(), symbols })
}

/// Fires of `filter` as loaded vs. as edited (`edited`, a whole program), over a recorded day.
pub fn test(cx: &Context, filter: &str, edited: &Program) -> Result<TestResult, String> {
    let t0 = std::time::Instant::now();
    let before_p = cx.program.only(filter).ok_or_else(|| format!("no filter `{filter}` loaded"))?;
    let after_p = edited.only(filter).ok_or_else(|| format!("the edit no longer defines `{filter}`"))?;
    let day = test_day(cx)?;
    let events = day_events(&cx.data_dir, day)?;
    let (before, after) = std::thread::scope(|s| {
        let b = s.spawn(|| count(&before_p, cx, &events));
        let a = count(&after_p, cx, &events);
        (b.join().map_err(|_| "the test crashed".to_string()).and_then(|r| r), a)
    });
    let (before, after) = (before?, after?);
    let set = |c: &Count| c.symbols.iter().map(|s| s.0.clone()).collect::<std::collections::BTreeSet<_>>();
    let (b, a) = (set(&before), set(&after));
    Ok(TestResult {
        day: day.to_string(),
        added: a.difference(&b).cloned().collect(),
        removed: b.difference(&a).cloned().collect(),
        before,
        after,
        seconds: t0.elapsed().as_secs_f64(),
    })
}

// ---------------------------------------------------------------------------------------------
// The filter's source

#[derive(Debug, Serialize)]
pub struct FilterText {
    /// The file name inside filters/.
    pub file: String,
    pub block: String,
    /// Editable numbers: byte ranges inside `block`.
    pub literals: Vec<(usize, usize)>,
}

fn scan_files(dir: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "scan"))
        .collect();
    paths.sort();
    paths.into_iter().map(|p| std::fs::read_to_string(&p).map(|t| (p.clone(), t)).map_err(|e| format!("{}: {e}", p.display()))).collect()
}

/// What an edit replaces: a filter's block or a basket's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Filter,
    Basket,
}

impl ItemKind {
    fn locate(self, text: &str, name: &str) -> Option<tp_scan_engine::lang::FilterSource> {
        match self {
            ItemKind::Filter => tp_scan_engine::lang::locate_filter(text, name),
            ItemKind::Basket => tp_scan_engine::lang::locate_basket(text, name),
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            ItemKind::Filter => "filter",
            ItemKind::Basket => "basket",
        }
    }
}

/// New baskets go in this file.
pub const BASKETS_FILE: &str = "baskets.scan";

pub fn source(dir: &Path, kind: ItemKind, filter: &str) -> Result<FilterText, String> {
    for (p, text) in scan_files(dir)? {
        if let Some(fs) = kind.locate(&text, filter) {
            return Ok(FilterText {
                file: p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
                block: text[fs.start..fs.end].to_string(),
                literals: fs.literals.iter().map(|&(s, e)| (s - fs.start, e - fs.start)).collect(),
            });
        }
    }
    Err(format!("no {} `{filter}` in {}", kind.word(), dir.display()))
}

/// The edit spliced in: which file, its text before and after, and the whole program as edited.
/// Refused (nothing written anywhere) unless the edited files load and compile, and the block
/// still defines `filter` under the same name.
#[derive(Debug)]
pub struct Applied {
    pub path: PathBuf,
    pub old_text: String,
    pub new_text: String,
    pub old_block: String,
    pub program: Program,
}

pub fn apply(dir: &Path, kind: ItemKind, filter: &str, block: &str) -> Result<Applied, String> {
    let mut files = scan_files(dir)?;
    let found = files.iter().enumerate().find_map(|(i, (_, t))| kind.locate(t, filter).map(|fs| (i, fs.start, fs.end)));
    let (i, start, end) = match (found, kind) {
        (Some(x), _) => x,
        // A new basket: appended to baskets.scan (created if need be).
        (None, ItemKind::Basket) => {
            let path = dir.join(BASKETS_FILE);
            let i = match files.iter().position(|(p, _)| *p == path) {
                Some(i) => i,
                None => {
                    files.push((path, "# Baskets: the stocks a filter may draw from (`basket NAME` in a filter). LANGUAGE.md §2.\n".to_string()));
                    files.len() - 1
                }
            };
            let t = &mut files[i].1;
            if !t.ends_with('\n') {
                t.push('\n');
            }
            t.push('\n');
            let at = t.len();
            (i, at, at)
        }
        (None, ItemKind::Filter) => return Err(format!("no filter `{filter}` in {}", dir.display())),
    };
    let (path, old_text) = files[i].clone();
    let mut new_text = format!("{}{}{}", &old_text[..start], block.trim(), &old_text[end..]);
    if start == end && !new_text.ends_with('\n') {
        new_text.push('\n');
    }
    if kind.locate(&new_text, filter).is_none() {
        return Err(match kind {
            ItemKind::Filter => format!("the edit must still read `trigger {filter} {{ … }}` (or `list {filter} {{ … }}`): renaming or removing a filter isn't done here"),
            ItemKind::Basket => format!("the edit must read `basket {filter} {{ … }}`"),
        });
    }
    let mut texts: Vec<(String, String)> = files.iter().map(|(p, t)| (p.display().to_string(), t.clone())).collect();
    texts[i].1 = new_text.clone();
    let refs: Vec<(&str, &str)> = texts.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
    let program = tp_scan_engine::lang::parse_sources(&refs).map_err(|e| e.to_string())?;
    Engine::new(&program).map_err(|e| e.to_string())?;
    // A file that didn't exist before reads as empty, so Undo can't restore it; a save only adds.
    let old_on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    Ok(Applied { old_block: old_text[start..end].to_string(), path, old_text: old_on_disk, new_text, program })
}

#[derive(Debug, Serialize)]
pub struct BasketCount {
    /// Symbols with daily history that pass every rule / fail one / can't be decided (a rule's
    /// data is missing, e.g. no float or no ticker reference).
    pub pass: usize,
    pub fail: usize,
    pub unknown: usize,
    /// The passing symbols with the most average volume, for a feel of what's in.
    pub top: Vec<(String, f64)>,
    pub as_of: String,
}

/// How many symbols a basket lets through, judged on the latest daily data (price = last close).
pub fn basket_count(cx: &Context, def: &tp_scan_engine::compile::BasketDef) -> Result<BasketCount, String> {
    let daily = cx.daily.as_ref().ok_or("no daily cache: run `tp-scan sync`")?;
    let refs: HashMap<&str, &tp_scan_engine::state::Reference> = cx.references.iter().map(|(s, r)| (s.as_str(), r)).collect();
    let today = Ts::from_millis(crate::scanner::now_ms()).expect("now").ny().date_naive();
    let session = tp_marketdata::time::SessionDate(today);
    let (mut pass, mut fail, mut unknown) = (0, 0, 0);
    let mut top = vec![];
    for sym in daily.symbol_names() {
        let hist = daily.adjusted_history(sym, session, tp_scan_engine::daily::HISTORY_DAYS);
        let Ok(d) = tp_scan_engine::state::DailyRef::from_history(session, &hist, 14, 20) else { continue };
        let r = refs.get(sym);
        let mut verdict = Some(true);
        let mut and = |v: Option<bool>| {
            verdict = match (verdict, v) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            }
        };
        let member = |have: Option<&str>, set: &[String]| -> Option<bool> {
            if set.is_empty() {
                Some(true)
            } else {
                have.map(|h| set.iter().any(|s| s == h))
            }
        };
        and(member(r.and_then(|r| r.security_type.as_deref()), &def.types));
        and(member(r.and_then(|r| r.exchange.as_deref()), &def.exchanges));
        for (key, lo, hi) in &def.ranges {
            let v = match key.as_str() {
                "price" => Some(d.prev_close.value()),
                "float" => r.and_then(|r| r.float_shares),
                "adv" => d.adv,
                "atr" => d.atr.map(|a| a.value()),
                _ => None,
            };
            and(v.map(|v| lo.is_none_or(|l| v >= l) && hi.is_none_or(|h| v <= h)));
        }
        match verdict {
            Some(true) => {
                pass += 1;
                top.push((sym.to_string(), d.adv.unwrap_or(0.0)));
            }
            Some(false) => fail += 1,
            None => unknown += 1,
        }
    }
    top.sort_by(|a, b| b.1.total_cmp(&a.1));
    top.truncate(40);
    Ok(BasketCount { pass, fail, unknown, top, as_of: format!("daily data before {today}") })
}

/// Lines that differ between two blocks, for the change log: `- old` / `+ new`.
pub fn line_diff(old: &str, new: &str) -> String {
    let (o, n): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let mut out = vec![];
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    for l in &o {
        *counts.entry(l).or_default() += 1;
    }
    for l in &n {
        *counts.entry(l).or_default() -= 1;
    }
    for l in &o {
        if counts.get(l).copied().unwrap_or(0) > 0 {
            out.push(format!("- {}", l.trim_end()));
        }
    }
    for l in &n {
        if counts.get(l).copied().unwrap_or(0) < 0 {
            out.push(format!("+ {}", l.trim_end()));
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with(files: &[(&str, &str)]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tp-tools-{}-{}", std::process::id(), files[0].1.len()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (n, t) in files {
            std::fs::write(d.join(n), t).unwrap();
        }
        d
    }

    #[test]
    fn an_edit_is_spliced_in_and_checked() {
        let d = dir_with(&[("a.scan", "# keep me\ntrigger A {\n    side long\n    when last >= 5\n}\n\n# and me\ntrigger B { when last > 1 }\n")]);
        let s = source(&d, ItemKind::Filter, "A").unwrap();
        assert_eq!(s.file, "a.scan");
        assert_eq!(&s.block[s.literals[0].0..s.literals[0].1], "5");
        let ok = apply(&d, ItemKind::Filter, "A", "trigger A {\n    side long\n    when last >= 7\n}").unwrap();
        assert_eq!(ok.new_text, "# keep me\ntrigger A {\n    side long\n    when last >= 7\n}\n\n# and me\ntrigger B { when last > 1 }\n");
        assert_eq!(line_diff(&ok.old_block, "trigger A {\n    side long\n    when last >= 7\n}"), "-     when last >= 5\n+     when last >= 7");
        assert!(apply(&d, ItemKind::Filter, "A", "trigger A { when last >= }").unwrap_err().contains("a.scan"), "a broken edit names the file");
        assert!(apply(&d, ItemKind::Filter, "A", "trigger Z { when last > 1 }").unwrap_err().contains("renaming"));
        assert!(apply(&d, ItemKind::Filter, "A", "trigger A { when nope > 1 }").is_err(), "an unknown name doesn't load");
        // A new basket lands in baskets.scan; a filter can then use it.
        let nb = apply(&d, ItemKind::Basket, "Cheap", "basket Cheap {\n    price to 10\n}").unwrap();
        assert!(nb.path.ends_with(BASKETS_FILE));
        assert!(nb.new_text.contains("basket Cheap {\n    price to 10\n}\n"), "{}", nb.new_text);
        assert_eq!(nb.program.baskets.len(), 1);
        assert!(apply(&d, ItemKind::Basket, "Cheap", "basket Cheap { price to }").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
