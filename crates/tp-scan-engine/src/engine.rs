//! The scan loop: market events in, signals out.
//!
//! On every bar for a symbol, the engine updates that symbol's state and evaluates every filter
//! against it (one memo shared by all filters for that symbol at that instant). A `True` result
//! outside the filter's cooldown emits a [`Signal`].
//!
//! **A bad symbol never stops the scan.** The incumbent dies on runtime exceptions from single
//! symbols. Here a per-symbol error is counted, reported to the sink, and that one event is
//! skipped. Everything else carries on.
//!
//! **Signals are a boundary, not a call.** They go to a [`Sink`] as immutable values carrying
//! their full measurement vector. Whatever consumes them (an alert window, a log, one day an
//! executor with its own guards) sits on the far side. The scanner itself never holds broker
//! credentials and never touches the order path.
//!
//! Evaluation is event-driven: a symbol is re-evaluated when it prints. A symbol that goes quiet
//! is not re-evaluated as its windows decay. That's the right trade for condition-on-activity
//! filters, but it means "volume dried up" style conditions only fire on the next print, and a
//! list member that stops printing stays listed until it prints again. Lists only show members
//! seen in the engine's latest session, so yesterday's members don't linger into today.
//!
//! **Quiet is not unknown.** The engine assumes its feed carries every trade from the first event
//! it sees, so a symbol that first prints at 09:30 had no trades before that, and its 120-minute
//! window is covered, not unknown. True of a full-market feed or a replay of one. A feed that
//! adds symbols part-way through a session breaks the assumption.
//!
//! ## Triggers and lists
//!
//! A [`Kind::Trigger`] filter emits a [`Signal`] when true and out of cooldown (a Primus filter).
//! A [`Kind::List`] filter keeps a membership set: a symbol enters when the condition becomes
//! true and leaves when it isn't (false *or* unknown), and [`Engine::list`] returns the members
//! ranked (a Primus static). Both are evaluated only inside their [`crate::compile::Hours`].

use crate::compile::{CompileError, Compiled, Ctx, Explanation, Kind, Program, Side, Tri};
use crate::state::{AuctionSnapshot, DailyRef, Reference, StateError, SymbolState};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tp_marketdata::source::{MarketEvent, MarketSource, SourceError, Symbol};
use tp_marketdata::time::{SessionDate, Ts};

/// A filter fired. Immutable once created.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub filter: Arc<str>,
    pub symbol: Symbol,
    /// When the signal became knowable: the end of the bar that triggered it.
    pub at: Ts,
    pub side: Side,
    /// The newest bar's close at the signal.
    pub last: Option<f64>,
    /// The filter's display columns (Primus panel columns such as `ATR_POS`).
    pub columns: Vec<(Arc<str>, Option<f64>)>,
    /// Every number the filter read, labelled, in first-use order. What a later analysis needs
    /// to ask "which measurements separate the good signals from the noise".
    pub measurements: Vec<(Arc<str>, Option<f64>)>,
}

/// A member of a list.
#[derive(Debug, Clone, PartialEq)]
pub struct ListRow {
    pub symbol: Symbol,
    /// When it entered the list (this stay).
    pub since: Ts,
    /// Its newest evaluation.
    pub updated: Ts,
    pub rank: Option<f64>,
    pub last: Option<f64>,
    pub columns: Vec<(Arc<str>, Option<f64>)>,
}

/// A symbol entered or left a list.
#[derive(Debug, Clone, PartialEq)]
pub struct ListChange<'a> {
    pub filter: &'a Arc<str>,
    pub symbol: &'a Symbol,
    pub at: Ts,
    pub entered: bool,
}

/// Supplies a symbol's daily reference for a session, called the first time the symbol prints in
/// that session. `None` leaves the daily measures unknown for that session.
pub type DailyProvider = Box<dyn FnMut(&str, SessionDate) -> Option<DailyRef> + Send>;

/// One filter evaluated on one symbol at one instant (instrumentation mode).
#[derive(Debug)]
pub struct Evaluation<'a> {
    pub filter: &'a Arc<str>,
    pub symbol: &'a Symbol,
    pub at: Ts,
    pub result: Tri,
    pub fired: bool,
    pub measurements: &'a [(Arc<str>, Option<f64>)],
}

#[derive(Debug, Clone, PartialEq)]
pub enum EngineError {
    State(StateError),
    Source(SourceError),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::State(e) => write!(f, "{e}"),
            EngineError::Source(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// Where the engine's output goes.
pub trait Sink {
    fn signal(&mut self, s: &Signal);

    /// Instrumentation mode: called for EVERY evaluation, with its measurement vector. Only
    /// called when [`Sink::wants_evaluations`] is true, because computing every vector costs more
    /// than short-circuit evaluation.
    fn evaluation(&mut self, _e: &Evaluation<'_>) {}

    fn wants_evaluations(&self) -> bool {
        false
    }

    /// A symbol entered or left a list.
    fn list_change(&mut self, _c: &ListChange<'_>) {}

    /// A problem with one event. `symbol` is empty when the source itself failed.
    fn error(&mut self, _symbol: &str, _e: &EngineError) {}
}

/// Counters. `errors > 0` should be loud in any UI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunStats {
    pub events: u64,
    pub evaluations: u64,
    pub unknown: u64,
    pub signals: u64,
    pub errors: u64,
    pub ignored_events: u64,
    /// Evaluations skipped because the bar was outside the filter's hours.
    pub out_of_hours: u64,
    pub list_changes: u64,
}

struct Entry {
    state: SymbolState,
    last_fire: Vec<Option<Ts>>,
}

pub struct Engine {
    compiled: Compiled,
    symbols: HashMap<Symbol, Entry>,
    /// Per filter: the list's members (always empty for triggers).
    lists: Vec<HashMap<Symbol, ListRow>>,
    latest_session: Option<SessionDate>,
    daily_provider: Option<DailyProvider>,
    /// The start of the first bar the engine saw.
    feed_start: Option<Ts>,
    stats: RunStats,
}

impl Engine {
    pub fn new(p: &Program) -> Result<Engine, CompileError> {
        let compiled = Compiled::new(p)?;
        let lists = (0..compiled.filters().len()).map(|_| HashMap::new()).collect();
        Ok(Engine {
            compiled,
            symbols: HashMap::new(),
            lists,
            latest_session: None,
            daily_provider: None,
            feed_start: None,
            stats: RunStats::default(),
        })
    }

    /// Treat the feed as having started at `t` (before any bar arrives). A run over one symbol's
    /// bars sets the session's real feed start, so the time before that symbol's first print
    /// reads quiet exactly as it does in the full-market run, not unknown.
    pub fn set_feed_start(&mut self, t: Ts) {
        self.feed_start = Some(t);
    }

    pub fn compiled(&self) -> &Compiled {
        &self.compiled
    }

    fn entry(&mut self, symbol: &str) -> &mut Entry {
        let n = self.compiled.filters().len();
        self.symbols
            .entry(Arc::from(symbol))
            .or_insert_with(|| Entry { state: SymbolState::new(), last_fire: vec![None; n] })
    }

    /// Give `symbol` its daily reference (prior close, ATR, ADV) for a session.
    pub fn set_daily(&mut self, symbol: &str, d: DailyRef) {
        self.entry(symbol).state.set_daily(d);
    }

    /// Supply daily references on demand, as each symbol reaches each session. A multi-day replay
    /// needs this: one reference per session, each built only from days before it.
    pub fn set_daily_provider(&mut self, p: DailyProvider) {
        self.daily_provider = Some(p);
    }

    pub fn set_reference(&mut self, symbol: &str, r: Reference) {
        self.entry(symbol).state.set_reference(r);
    }

    pub fn set_auction(&mut self, symbol: &str, a: AuctionSnapshot) {
        self.entry(symbol).state.set_auction(a);
    }

    pub fn set_halted(&mut self, symbol: &str, session: SessionDate, halted: bool) {
        self.entry(symbol).state.set_halted(session, halted);
    }

    /// A list's current members, highest rank first (unknown ranks last, then by symbol). Only
    /// members evaluated in the engine's latest session. `None` if there's no list by that name.
    pub fn list(&self, name: &str) -> Option<Vec<&ListRow>> {
        let k = self.compiled.filter_index(name)?;
        if self.compiled.filters()[k].kind != Kind::List {
            return None;
        }
        let today = self.latest_session;
        let mut rows: Vec<&ListRow> =
            self.lists[k].values().filter(|r| Some(SessionDate::of(r.updated)) == today).collect();
        rows.sort_by(|a, b| {
            let key = |r: &ListRow| r.rank.filter(|x| x.is_finite());
            match (key(a), key(b)) {
                (Some(x), Some(y)) => y.total_cmp(&x),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| a.symbol.cmp(&b.symbol))
        });
        Some(rows)
    }

    pub fn stats(&self) -> &RunStats {
        &self.stats
    }

    pub fn state(&self, symbol: &str) -> Option<&SymbolState> {
        self.symbols.get(symbol).map(|e| &e.state)
    }

    pub fn on_event(&mut self, ev: &MarketEvent, sink: &mut dyn Sink) {
        self.stats.events += 1;
        let (symbol, resolution, bar) = match ev {
            MarketEvent::Bar { symbol, resolution, bar } => (symbol, *resolution, *bar),
            #[allow(unreachable_patterns)]
            _ => {
                self.stats.ignored_events += 1;
                return;
            }
        };
        let session = bar.session_date();
        if self.latest_session.is_none_or(|s| s < session) {
            self.latest_session = Some(session);
        }
        let feed_start = *self.feed_start.get_or_insert(bar.start);
        self.entry(symbol);
        let entry = self.symbols.get_mut(symbol).expect("created above");
        entry.state.assume_quiet_since(feed_start);

        if entry.state.session_date() != Some(session) {
            if let Some(p) = self.daily_provider.as_mut() {
                if let Some(d) = p(symbol, session) {
                    entry.state.set_daily(d);
                }
            }
        }
        if let Err(e) = entry.state.ingest(resolution, bar) {
            self.stats.errors += 1;
            sink.error(symbol, &EngineError::State(e));
            return;
        }
        let Entry { state, last_fire } = entry;
        let at = state.now().expect("set by ingest");
        let phase = bar.phase();
        let compiled = &self.compiled;
        let mut ctx = Ctx::new(compiled, state);
        let instrument = sink.wants_evaluations();
        for (k, f) in compiled.filters().iter().enumerate() {
            if !f.hours.admits(phase) {
                self.stats.out_of_hours += 1;
                continue;
            }
            let result = compiled.eval(k, &mut ctx);
            self.stats.evaluations += 1;
            if result == Tri::Unknown {
                self.stats.unknown += 1;
            }
            match f.kind {
                Kind::Trigger => {
                    let cooled = last_fire[k].is_none_or(|t| at.millis() - t.millis() >= f.cooldown_ms);
                    let fired = result == Tri::True && cooled;
                    if fired || instrument {
                        let measurements = compiled.measurements(k, &mut ctx);
                        if instrument {
                            sink.evaluation(&Evaluation { filter: &f.name, symbol, at, result, fired, measurements: &measurements });
                        }
                        if fired {
                            last_fire[k] = Some(at);
                            self.stats.signals += 1;
                            let columns = compiled.columns(k, &mut ctx);
                            let last = ctx_last(&mut ctx);
                            sink.signal(&Signal { filter: f.name.clone(), symbol: symbol.clone(), at, side: f.side, last, columns, measurements });
                        }
                    }
                }
                Kind::List => {
                    if instrument {
                        let measurements = compiled.measurements(k, &mut ctx);
                        sink.evaluation(&Evaluation { filter: &f.name, symbol, at, result, fired: false, measurements: &measurements });
                    }
                    let members = &mut self.lists[k];
                    if result == Tri::True {
                        let rank = compiled.rank(k, &mut ctx);
                        let columns = compiled.columns(k, &mut ctx);
                        let last = ctx_last(&mut ctx);
                        match members.get_mut(symbol) {
                            Some(row) => {
                                row.updated = at;
                                row.rank = rank;
                                row.last = last;
                                row.columns = columns;
                            }
                            None => {
                                members.insert(symbol.clone(), ListRow { symbol: symbol.clone(), since: at, updated: at, rank, last, columns });
                                self.stats.list_changes += 1;
                                sink.list_change(&ListChange { filter: &f.name, symbol, at, entered: true });
                            }
                        }
                    } else if members.remove(symbol).is_some() {
                        self.stats.list_changes += 1;
                        sink.list_change(&ListChange { filter: &f.name, symbol, at, entered: false });
                    }
                }
            }
        }
    }

    /// Drain a source. Source errors are counted and reported; the run continues.
    pub fn run(&mut self, src: &mut dyn MarketSource, sink: &mut dyn Sink) -> RunStats {
        while let Some(ev) = src.next_event() {
            match ev {
                Ok(ev) => self.on_event(&ev, sink),
                Err(e) => {
                    self.stats.errors += 1;
                    sink.error("", &EngineError::Source(e));
                }
            }
        }
        self.stats.clone()
    }

    /// Explain `filter` on `symbol` as of that symbol's newest bar.
    pub fn explain(&self, filter: &str, symbol: &str) -> Option<Explanation> {
        let k = self.compiled.filter_index(filter)?;
        let state = &self.symbols.get(symbol)?.state;
        let mut ctx = Ctx::new(&self.compiled, state);
        Some(self.compiled.explain(k, &mut ctx))
    }
}

fn ctx_last(ctx: &mut Ctx<'_>) -> Option<f64> {
    ctx.state().last_bar().map(|b| b.close.value())
}

/// Collects signals and errors in memory (tests, small replays).

#[derive(Debug, Default)]
pub struct VecSink {
    pub signals: Vec<Signal>,
    pub errors: Vec<(String, String)>,
    /// `(filter, symbol, at, entered)`.
    pub list_changes: Vec<(String, String, Ts, bool)>,
}

impl Sink for VecSink {
    fn signal(&mut self, s: &Signal) {
        self.signals.push(s.clone());
    }
    fn error(&mut self, symbol: &str, e: &EngineError) {
        self.errors.push((symbol.to_string(), e.to_string()));
    }
    fn list_change(&mut self, c: &ListChange<'_>) {
        self.list_changes.push((c.filter.to_string(), c.symbol.to_string(), c.at, c.entered));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{FilterDef, Hours};
    use crate::expr::{m, n};
    use crate::measure::Measure;
    use crate::state::tests::{b, t};
    use chrono::NaiveDate;
    use tp_marketdata::price::Price;
    use tp_marketdata::source::Resolution;

    fn bar(sym: &str, start: Ts, c: f64) -> MarketEvent {
        MarketEvent::Bar { symbol: Arc::from(sym), resolution: Resolution::Minute, bar: b(start, c, c, c, c, 100.0) }
    }

    fn list_program() -> Program {
        let mut f = FilterDef::new("over10", m(Measure::Last).gt(n(10.0)), 0);
        f.kind = Kind::List;
        f.hours = Hours::Rth;
        f.rank = Some(m(Measure::Last));
        f.columns = vec![(Arc::from("DOUBLE"), m(Measure::Last) * n(2.0))];
        let mut g = FilterDef::new("trig", m(Measure::Last).gt(n(10.0)), 60_000);
        g.hours = Hours::Rth;
        g.side = Side::Long;
        Program::default().filter(f).filter(g)
    }

    #[test]
    fn lists_track_membership_and_rank() {
        let mut e = Engine::new(&list_program()).unwrap();
        let mut sink = VecSink::default();
        e.on_event(&bar("AAA", t(9, 30, 0), 11.0), &mut sink);
        e.on_event(&bar("BBB", t(9, 30, 0), 15.0), &mut sink);
        e.on_event(&bar("CCC", t(9, 30, 0), 5.0), &mut sink);
        let rows: Vec<(&str, Option<f64>)> = e.list("over10").unwrap().iter().map(|r| (&*r.symbol, r.rank)).collect();
        assert_eq!(rows, vec![("BBB", Some(15.0)), ("AAA", Some(11.0))]);
        assert_eq!(e.list("over10").unwrap()[0].columns, vec![(Arc::from("DOUBLE"), Some(30.0))]);
        // BBB drops out, AAA's rank updates, CCC enters.
        e.on_event(&bar("BBB", t(9, 31, 0), 9.0), &mut sink);
        e.on_event(&bar("AAA", t(9, 31, 0), 12.0), &mut sink);
        e.on_event(&bar("CCC", t(9, 31, 0), 20.0), &mut sink);
        let rows: Vec<&str> = e.list("over10").unwrap().iter().map(|r| &*r.symbol).collect();
        assert_eq!(rows, vec!["CCC", "AAA"]);
        let aaa = e.list("over10").unwrap()[1].clone();
        assert_eq!((aaa.since, aaa.updated), (t(9, 31, 0), t(9, 32, 0)), "since = first entry, not re-entry");
        let entered: Vec<(&str, bool)> = sink.list_changes.iter().map(|(_, s, _, e)| (s.as_str(), *e)).collect();
        assert_eq!(entered, vec![("AAA", true), ("BBB", true), ("BBB", false), ("CCC", true)]);
        assert!(e.list("trig").is_none(), "a trigger is not a list");
        assert!(e.list("nope").is_none());
    }

    #[test]
    fn filters_only_run_in_their_hours_and_signals_carry_columns() {
        let mut e = Engine::new(&list_program()).unwrap();
        let mut sink = VecSink::default();
        e.on_event(&bar("AAA", t(9, 0, 0), 11.0), &mut sink); // premarket
        e.on_event(&bar("AAA", t(16, 5, 0), 11.0), &mut sink); // post
        assert!(sink.signals.is_empty());
        assert_eq!(e.list("over10").unwrap().len(), 0);
        assert_eq!(e.stats().out_of_hours, 4);
        let mut e = Engine::new(&list_program()).unwrap();
        e.on_event(&bar("AAA", t(9, 45, 0), 11.0), &mut sink);
        let s = &sink.signals[0];
        assert_eq!((s.side, s.last), (Side::Long, Some(11.0)));
    }

    #[test]
    fn yesterdays_members_are_not_shown_today() {
        let mut e = Engine::new(&list_program()).unwrap();
        let mut sink = VecSink::default();
        e.on_event(&bar("AAA", t(9, 30, 0), 11.0), &mut sink);
        let next_day = Ts::from_ny_local(NaiveDate::from_ymd_opt(2026, 6, 16).unwrap(), chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap()).unwrap();
        e.on_event(&bar("BBB", next_day, 11.0), &mut sink);
        let rows: Vec<&str> = e.list("over10").unwrap().iter().map(|r| &*r.symbol).collect();
        assert_eq!(rows, vec!["BBB"]);
    }

    #[test]
    fn the_daily_provider_is_asked_once_per_symbol_per_session() {
        let p = Program::default().filter(FilterDef::new("atr", m(Measure::Atr).gt(n(0.0)), 0));
        let mut e = Engine::new(&p).unwrap();
        let asked = Arc::new(std::sync::Mutex::new(vec![]));
        let log = asked.clone();
        e.set_daily_provider(Box::new(move |sym, session| {
            log.lock().unwrap().push((sym.to_string(), session));
            Some(DailyRef { atr: Some(Price::new(1.0)), ..DailyRef::bare(session, Price::new(10.0)) })
        }));
        let mut sink = VecSink::default();
        for i in 0..3 {
            e.on_event(&bar("AAA", t(9, 30 + i, 0), 11.0), &mut sink);
        }
        assert_eq!(asked.lock().unwrap().len(), 1);
        assert_eq!(sink.signals.len(), 3, "ATR known from the first bar");
    }
}

