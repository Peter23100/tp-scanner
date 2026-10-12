//! The engine thread: one `Engine`, one market-data source (live or replay), and everything the
//! windows see, published as events and kept in a shared snapshot for windows that open later.
//!
//! The thread owns the engine outright; windows reach it only through [`Cmd`]s. A filter error, a
//! bad bar or a dropped connection is counted and shown, never fatal: the engine already skips a
//! bad symbol, and this loop survives a panicking command by reporting it.
//!
//! Signals are a boundary: they leave as immutable values (events, a JSONL day file). Nothing here
//! holds broker credentials or can reach an order path.

use crate::layout::{self, FilterMeta};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tp_marketdata::source::{MarketEvent, MarketSource, ReplaySource};
use tp_marketdata::time::Ts;
use tp_massive::daily_cache::RawDaily;
use tp_massive::ws::{FeedState, LiveSource, Poll};
use tp_scan_engine::compile::{Compiled, Kind, Program};
use tp_scan_engine::engine::{Engine, EngineError, ListChange, Signal, Sink};
use tp_scan_engine::instrument::JsonlSink;
use tp_scan_engine::state::Reference;

/// What the live scan subscribes to: every US stock's per-second bar.
pub const LIVE_CHANNELS: &str = "A.*";

/// Signals kept for windows that open later (newest last).
const KEEP_SIGNALS: usize = 5000;
/// Rows per list sent to the screens.
const LIST_ROWS: usize = 100;

#[derive(Debug, Clone, Serialize)]
pub struct SignalDto {
    pub id: u64,
    pub filter: String,
    pub symbol: String,
    pub side: &'static str,
    pub at_ms: i64,
    pub time: String,
    pub last: Option<f64>,
    pub columns: Vec<(String, Option<f64>)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListRowDto {
    pub symbol: String,
    pub last: Option<f64>,
    pub rank: Option<f64>,
    pub since: String,
    pub columns: Vec<(String, Option<f64>)>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ReplayDto {
    pub date: String,
    pub speed: f64,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct StatusDto {
    /// "idle", "loading", "live" or "replay".
    pub mode: String,
    /// The feed's state in words.
    pub feed: String,
    /// True when bars are flowing (live and authenticated, or a replay running).
    pub ok: bool,
    pub bars: u64,
    /// The market clock: the newest bar's end, New York time.
    pub market_time: Option<String>,
    pub market_ms: Option<i64>,
    /// Live only: how far the newest bar trails the wall clock.
    pub delay_s: Option<i64>,
    pub bad_messages: u64,
    pub reconnects: u32,
    pub signals: u64,
    pub errors: u64,
    pub error_samples: Vec<String>,
    pub warnings: Vec<String>,
    pub replay: Option<ReplayDto>,
    pub filters: usize,
    /// The engine's history starts here (a reload or source change resets it).
    pub since: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Snapshot {
    pub signals: VecDeque<SignalDto>,
    pub lists: HashMap<String, Vec<ListRowDto>>,
    pub status: StatusDto,
}

/// What the windows share with the engine thread.
#[derive(Default)]
pub struct Shared {
    pub snapshot: Snapshot,
    /// What the panels need to know about each loaded filter.
    pub filters: Vec<FilterMeta>,
    /// Why the filters didn't load (the app runs on, with none or the previous set).
    pub filters_error: Option<String>,
    /// Chart window label → the symbol it shows, so the engine forwards that symbol's bars.
    pub chart_symbols: HashMap<String, String>,
    /// What the side tools (diagnose, filter tests) need to run a filter on their own: the loaded
    /// program, the references, the replay day and when the running feed started.
    pub program: Program,
    pub daily: Option<Arc<RawDaily>>,
    pub references: Arc<Vec<(String, Reference)>>,
    pub replay_date: Option<chrono::NaiveDate>,
    pub feed_start: Option<Ts>,
    /// Link colour → the symbol its charts show. The single copy: windows set it by click, panels
    /// with Follow on set it on each new fire. `links_dirty` asks the app to save it.
    pub links: HashMap<String, String>,
    pub links_dirty: bool,
    /// Filter → the link colours of the panels showing it with Follow on.
    pub follow: HashMap<String, Vec<String>>,
}

pub enum Cmd {
    Reload(Sender<Result<String, String>>),
    Live { realtime: bool },
    Replay { date: chrono::NaiveDate, speed: f64, start_at: Option<chrono::NaiveTime> },
    SetSpeed(f64),
    Stop,
    Explain { filter: String, symbol: String, reply: Sender<Result<String, String>> },
}

#[derive(Serialize, Clone)]
struct ChartBar {
    symbol: String,
    t: i64,
    o: f64,
    h: f64,
    l: f64,
    c: f64,
    v: f64,
}

#[derive(Serialize, Clone)]
struct Tick<'a> {
    lists: &'a HashMap<String, Vec<ListRowDto>>,
    status: &'a StatusDto,
}

fn hms(t: Ts) -> String {
    t.ny().format("%H:%M:%S").to_string()
}

struct AppSink {
    app: AppHandle,
    shared: Arc<Mutex<Shared>>,
    next_id: u64,
    jsonl: Option<JsonlSink<BufWriter<File>>>,
    errors: VecDeque<String>,
    /// Whether a fire may move a followed link now: live, or a replay at a watchable pace (not
    /// at max speed, not in its fast-forward to the start time).
    follow_now: bool,
    /// Link colour → when a fire last moved it (wall clock): at most one switch a second per colour,
    /// so a burst of fires can't strobe the charts.
    followed_at: HashMap<String, Instant>,
}

/// The least time between two follow switches of one link colour.
const FOLLOW_GAP: Duration = Duration::from_millis(1000);

impl Sink for AppSink {
    fn signal(&mut self, s: &Signal) {
        self.next_id += 1;
        let dto = SignalDto {
            id: self.next_id,
            filter: s.filter.to_string(),
            symbol: s.symbol.to_string(),
            side: layout::side_str(s.side),
            at_ms: s.at.millis(),
            time: hms(s.at),
            last: s.last,
            columns: s.columns.iter().map(|(l, v)| (l.to_string(), *v)).collect(),
        };
        if let Some(j) = self.jsonl.as_mut() {
            j.signal(s);
        }
        let _ = self.app.emit("scan-signal", &dto);
        let mut g = self.shared.lock().expect("shared");
        if self.follow_now {
            let colors = g.follow.get(&*s.filter).cloned().unwrap_or_default();
            for c in colors {
                if g.links.get(&c).is_some_and(|cur| **cur == *s.symbol) || self.followed_at.get(&c).is_some_and(|t| t.elapsed() < FOLLOW_GAP) {
                    continue;
                }
                self.followed_at.insert(c.clone(), Instant::now());
                g.links.insert(c.clone(), s.symbol.to_string());
                g.links_dirty = true;
                let _ = self.app.emit("link-changed", serde_json::json!({"color": c, "symbol": &*s.symbol}));
            }
        }
        let sig = &mut g.snapshot.signals;
        sig.push_back(dto);
        while sig.len() > KEEP_SIGNALS {
            sig.pop_front();
        }
    }

    fn list_change(&mut self, _c: &ListChange<'_>) {}

    fn error(&mut self, symbol: &str, e: &EngineError) {
        if self.errors.len() >= 20 {
            self.errors.pop_front();
        }
        self.errors.push_back(format!("{symbol}: {e}"));
    }
}

enum Source {
    Idle,
    Live(LiveSource),
    Replay(Replay),
}

struct Replay {
    src: ReplaySource,
    date: chrono::NaiveDate,
    speed: f64,
    /// Bars before this are run at full speed (skip to the interesting part of the day).
    fast_until: Option<Ts>,
    pending: Option<MarketEvent>,
    /// Pacing anchor: this bar time was due at this wall instant.
    anchor: Option<(i64, Instant)>,
    done: bool,
}

/// Daily references and float, loaded once and shared by every engine the thread builds.
struct Refs {
    daily: Option<Arc<RawDaily>>,
    /// Float, security type and exchange per symbol.
    references: Arc<Vec<(String, Reference)>>,
    warnings: Vec<String>,
}

fn load_refs(data: &std::path::Path) -> Refs {
    let mut warnings = vec![];
    let today = Ts::from_millis(now_ms()).expect("now").ny().date_naive();
    let dir = data.join("daily");
    let daily = if dir.exists() {
        match RawDaily::load(&dir, today - chrono::Duration::days(tp_massive::daily_cache::DEFAULT_CALENDAR_DAYS + 30)) {
            Ok(r) if r.days > 0 => Some(Arc::new(r)),
            Ok(_) => {
                warnings.push("daily cache is empty: prior close, ATR, ADV unknown (run `tp-scan sync`)".into());
                None
            }
            Err(e) => {
                warnings.push(format!("daily cache unreadable: {e}"));
                None
            }
        }
    } else {
        warnings.push("no daily cache: prior close, ATR, ADV unknown, most filters can't fire (run `tp-scan sync`)".into());
        None
    };
    let fp = data.join("float.csv");
    let floats = match std::fs::read_to_string(&fp) {
        Ok(t) => t
            .lines()
            .skip(1)
            .filter_map(|l| {
                let (s, f) = l.split_once(',')?;
                Some((s.to_string(), f.trim().parse::<f64>().ok().filter(|x| x.is_finite() && *x >= 0.0)?))
            })
            .collect(),
        Err(_) => {
            warnings.push("no float data: float filters can't fire (run `tp-scan sync`)".into());
            vec![]
        }
    };
    // Ticker types and exchanges (baskets). Fetched here once if a sync hasn't fetched them yet.
    if !tp_massive::tickers::is_current(data) {
        if let Ok(rest) = tp_massive::Secret::from_env(tp_massive::API_KEY_VAR).and_then(tp_massive::rest::Rest::new) {
            if let Err(e) = tp_massive::tickers::refresh(&rest, data) {
                warnings.push(format!("ticker reference not fetched ({e}): basket types and exchanges read unknown"));
            }
        }
    }
    let tickers = tp_massive::tickers::load(data);
    if tickers.is_empty() {
        warnings.push("no ticker reference: basket types and exchanges read unknown (run `tp-scan sync`)".into());
    }
    let references = Arc::new(Reference::merge(&floats, &tickers));
    // Feeds Starter doesn't carry. Their filters read unknown and never fire; say so.
    warnings.push("no halt, imbalance or NYSE IND feed yet: those panels stay empty".into());
    Refs { daily, references, warnings }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub struct Runner {
    app: AppHandle,
    shared: Arc<Mutex<Shared>>,
    filters_dir: PathBuf,
    data_dir: PathBuf,
    program: Program,
    engine: Engine,
    refs: Refs,
    source: Source,
    sink: AppSink,
    since: Option<Ts>,
    /// The newest bar end the engine has read: the market clock in a replay.
    latest: Option<Ts>,
    rx: Receiver<Cmd>,
}

impl Runner {
    /// Load the filters and layout and get ready. A load error is returned so the app can show it
    /// and keep running with an empty engine.
    pub fn new(app: AppHandle, shared: Arc<Mutex<Shared>>, filters_dir: PathBuf, data_dir: PathBuf, rx: Receiver<Cmd>) -> Runner {
        {
            let mut g = shared.lock().expect("shared");
            g.snapshot.status.mode = "loading".into();
            g.snapshot.status.feed = "loading daily cache".into();
        }
        let refs = load_refs(&data_dir);
        let (program, err) = match load_program(&filters_dir) {
            Ok(p) => (p, None),
            Err(e) => (Program::default(), Some(e)),
        };
        let engine = Engine::new(&program).unwrap_or_else(|_| Engine::new(&Program::default()).expect("empty program"));
        let sink = AppSink { app: app.clone(), shared: shared.clone(), next_id: 0, jsonl: None, errors: VecDeque::new(), follow_now: false, followed_at: HashMap::new() };
        {
            let mut g = shared.lock().expect("shared");
            g.daily = refs.daily.clone();
            g.references = refs.references.clone();
        }
        let mut r = Runner { app, shared, filters_dir, data_dir, program, engine, refs, source: Source::Idle, sink, since: None, latest: None, rx };
        r.publish_filters(err);
        r.reset_engine();
        r
    }

    fn publish_filters(&mut self, load_error: Option<String>) {
        let compiled = Compiled::new(&self.program);
        let mut g = self.shared.lock().expect("shared");
        if let Some(e) = load_error {
            g.filters_error = Some(e);
            return;
        }
        match compiled {
            Ok(c) => {
                g.filters = layout::filter_meta(&self.program, &c);
                g.filters_error = None;
                g.program = self.program.clone();
            }
            Err(e) => g.filters_error = Some(format!("filters don't compile: {e}")),
        }
    }

    /// A fresh engine with the current program and references. History starts over.
    fn reset_engine(&mut self) {
        self.engine = Engine::new(&self.program).unwrap_or_else(|_| Engine::new(&Program::default()).expect("empty"));
        if let Some(d) = self.refs.daily.clone() {
            self.engine.set_daily_provider(tp_scan_engine::daily::provider_from(move |sym, session| {
                d.adjusted_history(sym, session, tp_scan_engine::daily::HISTORY_DAYS)
            }));
        }
        for (s, r) in self.refs.references.iter() {
            self.engine.set_reference(s, r.clone());
        }
        self.since = None;
        self.latest = None;
        self.sink.errors.clear();
        {
            let mut g = self.shared.lock().expect("shared");
            g.snapshot.signals.clear();
            g.snapshot.lists.clear();
            g.feed_start = None;
        }
        let _ = self.app.emit("scan-reset", ());
    }

    pub fn run(mut self) {
        let mut last_tick = Instant::now() - Duration::from_secs(2);
        loop {
            // A panic inside the engine (a bug, never expected) must not take the scanner down
            // silently: report it, start a fresh engine, keep the windows alive.
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                while let Ok(cmd) = self.rx.try_recv() {
                    if !self.command(cmd) {
                        return false;
                    }
                }
                self.pump();
                if last_tick.elapsed() >= Duration::from_millis(1000) {
                    last_tick = Instant::now();
                    self.tick();
                }
                true
            }));
            match r {
                Ok(true) => {}
                Ok(false) => return,
                Err(p) => {
                    let what = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                    self.stop_source();
                    self.reset_engine();
                    self.refs.warnings.push(format!("the engine crashed and was restarted (source stopped): {what}"));
                }
            }
        }
    }

    /// Returns false when the thread should end.
    fn command(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Reload(reply) => {
                let r = match load_program(&self.filters_dir) {
                    Ok(p) => {
                        let n = p.filters.len();
                        self.program = p;
                        self.publish_filters(None);
                        self.reset_engine();
                        let _ = self.app.emit("filters-changed", ());
                        Ok(format!("{n} filters loaded; history restarts now"))
                    }
                    // A bad edit keeps the running engine.
                    Err(e) => Err(format!("not reloaded, still running the previous filters:\n{e}")),
                };
                let _ = reply.send(r);
            }
            Cmd::Live { realtime } => {
                self.stop_source();
                self.reset_engine();
                self.shared.lock().expect("shared").replay_date = None;
                match tp_massive::Secret::from_env(tp_massive::API_KEY_VAR) {
                    Ok(key) => {
                        let url = if realtime { tp_massive::ws::REALTIME_URL } else { tp_massive::ws::DELAYED_URL };
                        // Per-second bars for the whole market (`A.*`): filters see each second as it
                        // closes, not each minute. (Trades, `T.*`, aren't on the Starter plan.)
                        self.source = Source::Live(LiveSource::start(key, url, LIVE_CHANNELS));
                        self.open_signal_log();
                    }
                    Err(e) => self.set_feed_error(e.to_string()),
                }
            }
            Cmd::Replay { date, speed, start_at } => {
                self.stop_source();
                self.reset_engine();
                match self.open_replay(date) {
                    Ok(src) => {
                        let fast_until = start_at.and_then(|t| Ts::from_ny_local(date, t).ok());
                        self.source = Source::Replay(Replay { src, date, speed, fast_until, pending: None, anchor: None, done: false });
                        self.shared.lock().expect("shared").replay_date = Some(date);
                    }
                    Err(e) => self.set_feed_error(e),
                }
            }
            Cmd::SetSpeed(s) => {
                if let Source::Replay(r) = &mut self.source {
                    r.speed = s;
                    r.anchor = None;
                }
            }
            Cmd::Stop => self.stop_source(),
            Cmd::Explain { filter, symbol, reply } => {
                let r = match self.engine.explain(&filter, &symbol) {
                    Some(ex) => Ok(ex.to_string()),
                    None => Err(format!("no filter `{filter}`, or no bars yet for `{symbol}`")),
                };
                let _ = reply.send(r);
            }
        }
        true
    }

    fn set_feed_error(&mut self, e: String) {
        let mut g = self.shared.lock().expect("shared");
        g.snapshot.status.mode = "idle".into();
        g.snapshot.status.feed = e;
        g.snapshot.status.ok = false;
    }

    fn stop_source(&mut self) {
        if let Source::Live(mut l) = std::mem::replace(&mut self.source, Source::Idle) {
            l.stop();
        }
        if let Some(j) = self.sink.jsonl.take() {
            let _ = j.finish();
        }
    }

    fn open_signal_log(&mut self) {
        let day = Ts::from_millis(now_ms()).expect("now").ny().date_naive();
        let dir = self.data_dir.join("signals");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(format!("{day}.jsonl"));
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
            self.sink.jsonl = Some(JsonlSink::new(BufWriter::new(f), false));
        }
    }

    fn open_replay(&self, date: chrono::NaiveDate) -> Result<ReplaySource, String> {
        let path = tp_massive::flatfiles::local_path(&self.data_dir, date);
        let path = if path.exists() {
            path
        } else {
            let id = tp_massive::Secret::from_env(tp_massive::S3_KEY_ID_VAR).map_err(|e| e.to_string())?;
            let secret = tp_massive::Secret::from_env(tp_massive::S3_SECRET_VAR).map_err(|e| e.to_string())?;
            {
                let mut g = self.shared.lock().expect("shared");
                g.snapshot.status.mode = "loading".into();
                g.snapshot.status.feed = format!("downloading {date}");
            }
            let _ = self.emit_tick_now();
            tp_massive::flatfiles::fetch_minute_aggs(&id, &secret, &self.data_dir, date).map_err(|e| e.to_string())?
        };
        {
            let mut g = self.shared.lock().expect("shared");
            g.snapshot.status.mode = "loading".into();
            g.snapshot.status.feed = format!("reading {date}");
        }
        let _ = self.emit_tick_now();
        let f = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let rd = std::io::BufReader::new(flate2::read::GzDecoder::new(f));
        tp_marketdata::flatfile::read_minute_aggs(rd).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn emit_tick_now(&self) -> Option<()> {
        let g = self.shared.lock().ok()?;
        self.app.emit("scan-tick", Tick { lists: &g.snapshot.lists, status: &g.snapshot.status }).ok()
    }

    fn on_event(&mut self, ev: &MarketEvent, watched: &HashSet<String>) {
        let at = ev.available_at();
        if self.since.is_none() {
            self.since = Some(at);
            if let MarketEvent::Bar { bar, .. } = ev {
                self.shared.lock().expect("shared").feed_start = Some(bar.start);
            }
        }
        if self.latest.is_none_or(|t| at > t) {
            self.latest = Some(at);
        }
        self.engine.on_event(ev, &mut self.sink);
        let MarketEvent::Bar { symbol, bar, .. } = ev else { return };
        if watched.contains(&**symbol) {
            let _ = self.app.emit(
                "chart-bar",
                ChartBar {
                    symbol: symbol.to_string(),
                    t: bar.start.millis(),
                    o: bar.open.value(),
                    h: bar.high.value(),
                    l: bar.low.value(),
                    c: bar.close.value(),
                    v: bar.volume,
                },
            );
        }
    }

    fn watched(&self) -> HashSet<String> {
        self.shared.lock().expect("shared").chart_symbols.values().cloned().collect()
    }

    /// Move market data into the engine for at most ~100 ms, then return so commands and ticks run.
    fn pump(&mut self) {
        let watched = self.watched();
        let budget = Instant::now() + Duration::from_millis(100);
        match std::mem::replace(&mut self.source, Source::Idle) {
            Source::Idle => {
                self.source = Source::Idle;
                std::thread::sleep(Duration::from_millis(50));
            }
            Source::Live(mut l) => {
                self.sink.follow_now = true;
                while Instant::now() < budget {
                    match l.next_timeout(Duration::from_millis(50)) {
                        Poll::Event(e) => self.on_event(&e, &watched),
                        Poll::Timeout => break,
                        Poll::Closed => break,
                    }
                }
                self.source = Source::Live(l);
            }
            Source::Replay(mut r) => {
                while !r.done && Instant::now() < budget {
                    if r.pending.is_none() {
                        match r.src.next_event() {
                            Some(Ok(e)) => r.pending = Some(e),
                            Some(Err(e)) => {
                                self.sink.error("", &EngineError::Source(e));
                                continue;
                            }
                            None => {
                                r.done = true;
                                break;
                            }
                        }
                    }
                    let ev = r.pending.as_ref().expect("filled above");
                    let due = ev.available_at().millis();
                    let fast = r.speed <= 0.0 || r.fast_until.is_some_and(|t| due < t.millis());
                    self.sink.follow_now = !fast;
                    if !fast {
                        let (t0, w0) = *r.anchor.get_or_insert((due, Instant::now()));
                        let wait_ms = ((due - t0) as f64 / r.speed) as i64 - w0.elapsed().as_millis() as i64;
                        if wait_ms > 0 {
                            std::thread::sleep(Duration::from_millis(wait_ms.min(50) as u64));
                            break;
                        }
                    }
                    let ev = r.pending.take().expect("present");
                    self.on_event(&ev, &watched);
                }
                self.source = Source::Replay(r);
            }
        }
    }

    fn tick(&mut self) {
        let mut lists = HashMap::new();
        for f in self.engine.compiled().filters().iter().filter(|f| f.kind == Kind::List) {
            let rows = self.engine.list(&f.name).unwrap_or_default();
            lists.insert(
                f.name.to_string(),
                rows.iter()
                    .take(LIST_ROWS)
                    .map(|r| ListRowDto {
                        symbol: r.symbol.to_string(),
                        last: r.last,
                        rank: r.rank,
                        since: r.since.ny().format("%H:%M").to_string(),
                        columns: r.columns.iter().map(|(l, v)| (l.to_string(), *v)).collect(),
                    })
                    .collect(),
            );
        }
        let st = self.engine.stats();
        let mut status = StatusDto {
            signals: st.signals,
            errors: st.errors,
            error_samples: self.sink.errors.iter().cloned().collect(),
            warnings: self.refs.warnings.clone(),
            filters: self.engine.compiled().filters().len(),
            since: self.since.map(|t| t.ny().format("%H:%M").to_string()),
            ..Default::default()
        };
        match &self.source {
            Source::Idle => {
                let prev = self.shared.lock().expect("shared").snapshot.status.clone();
                status.mode = "idle".into();
                status.feed = if prev.mode == "idle" && !prev.feed.is_empty() { prev.feed } else { "stopped".into() };
            }
            Source::Live(l) => {
                let fs = l.status();
                status.mode = "live".into();
                status.ok = fs.state == FeedState::Live;
                status.feed = match &fs.state {
                    FeedState::Connecting => "connecting".into(),
                    FeedState::Authenticating => "authenticating".into(),
                    FeedState::Live => format!("live ({})", if fs.url.contains("delayed") { "15-min delayed" } else { "real-time" }),
                    FeedState::Reconnecting { attempt, wait_ms } => format!(
                        "reconnecting (try {attempt}, {:.0}s){}",
                        *wait_ms as f64 / 1000.0,
                        fs.last_error.as_ref().map_or(String::new(), |e| format!(": {e}"))
                    ),
                    FeedState::AuthFailed(m) => format!("key refused: {m}"),
                    FeedState::Stopped => "stopped".into(),
                };
                status.bars = fs.bars;
                status.bad_messages = fs.bad_messages;
                status.reconnects = fs.reconnects;
                if let Some(t) = fs.last_bar_end {
                    status.market_time = Some(hms(t));
                    status.market_ms = Some(t.millis());
                    status.delay_s = Some((now_ms() - t.millis()) / 1000);
                }
                if fs.reconnects > 0 {
                    status.warnings.push(format!(
                        "{} reconnect(s) this session: bars missed while disconnected are not backfilled",
                        fs.reconnects
                    ));
                }
            }
            Source::Replay(r) => {
                status.mode = "replay".into();
                status.ok = !r.done;
                status.feed = if r.done { format!("replay of {} finished", r.date) } else { format!("replaying {}", r.date) };
                status.bars = st.events;
                status.replay = Some(ReplayDto { date: r.date.to_string(), speed: r.speed, done: r.done });
                if let Some(t) = self.latest {
                    status.market_time = Some(hms(t));
                    status.market_ms = Some(t.millis());
                }
            }
        }
        if let Some(j) = self.sink.jsonl.as_mut() {
            j.flush();
        }
        let mut g = self.shared.lock().expect("shared");
        g.snapshot.lists = lists;
        g.snapshot.status = status;
        let _ = self.app.emit("scan-tick", Tick { lists: &g.snapshot.lists, status: &g.snapshot.status });
    }

}

fn load_program(dir: &std::path::Path) -> Result<Program, String> {
    let p = tp_scan_engine::lang::load_dir(dir).map_err(|e| e.to_string())?;
    Engine::new(&p).map_err(|e| e.to_string())?;
    Ok(p)
}
