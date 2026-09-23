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
//! filters, but it means "volume dried up" style conditions only fire on the next print.

use crate::compile::{CompileError, Compiled, Ctx, Explanation, Program, Tri};
use crate::state::{DailyRef, StateError, SymbolState};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tp_marketdata::source::{MarketEvent, MarketSource, SourceError, Symbol};
use tp_marketdata::time::Ts;

/// A filter fired. Immutable once created.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub filter: Arc<str>,
    pub symbol: Symbol,
    /// When the signal became knowable: the end of the bar that triggered it.
    pub at: Ts,
    /// Every number the filter read, labelled, in first-use order. What a later analysis needs
    /// to ask "which measurements separate the good signals from the noise".
    pub measurements: Vec<(Arc<str>, Option<f64>)>,
}

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
}

struct Entry {
    state: SymbolState,
    last_fire: Vec<Option<Ts>>,
}

pub struct Engine {
    compiled: Compiled,
    symbols: HashMap<Symbol, Entry>,
    pending_daily: HashMap<Symbol, DailyRef>,
    stats: RunStats,
}

impl Engine {
    pub fn new(p: &Program) -> Result<Engine, CompileError> {
        Ok(Engine { compiled: Compiled::new(p)?, symbols: HashMap::new(), pending_daily: HashMap::new(), stats: RunStats::default() })
    }

    pub fn compiled(&self) -> &Compiled {
        &self.compiled
    }

    /// Give `symbol` its daily reference (prior close, ATR, ADV) for a session.
    pub fn set_daily(&mut self, symbol: &str, d: DailyRef) {
        match self.symbols.get_mut(symbol) {
            Some(e) => e.state.set_daily(d),
            None => {
                self.pending_daily.insert(Arc::from(symbol), d);
            }
        }
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
        let n_filters = self.compiled.filters().len();
        if !self.symbols.contains_key(symbol) {
            let mut state = SymbolState::new();
            if let Some(d) = self.pending_daily.remove(symbol) {
                state.set_daily(d);
            }
            self.symbols.insert(symbol.clone(), Entry { state, last_fire: vec![None; n_filters] });
        }
        let entry = self.symbols.get_mut(symbol).expect("inserted above");
        if let Err(e) = entry.state.ingest(resolution, bar) {
            self.stats.errors += 1;
            sink.error(symbol, &EngineError::State(e));
            return;
        }
        let Entry { state, last_fire } = entry;
        let at = state.now().expect("set by ingest");
        let compiled = &self.compiled;
        let mut ctx = Ctx::new(compiled, state);
        let instrument = sink.wants_evaluations();
        for (k, f) in compiled.filters().iter().enumerate() {
            let result = compiled.eval(k, &mut ctx);
            self.stats.evaluations += 1;
            if result == Tri::Unknown {
                self.stats.unknown += 1;
            }
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
                    sink.signal(&Signal { filter: f.name.clone(), symbol: symbol.clone(), at, measurements });
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

/// Collects signals and errors in memory (tests, small replays).
#[derive(Debug, Default)]
pub struct VecSink {
    pub signals: Vec<Signal>,
    pub errors: Vec<(String, String)>,
}

impl Sink for VecSink {
    fn signal(&mut self, s: &Signal) {
        self.signals.push(s.clone());
    }
    fn error(&mut self, symbol: &str, e: &EngineError) {
        self.errors.push((symbol.to_string(), e.to_string()));
    }
}
