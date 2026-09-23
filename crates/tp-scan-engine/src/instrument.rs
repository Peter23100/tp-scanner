//! Instrumentation mode: write every evaluation's full measurement vector as JSON lines.
//!
//! This is the investigation's stage 4 ("the cheap win"): run the filters over **history** with
//! this sink attached and you get a sweepable dataset of every measurement for every candidate.
//! Thresholds can then be set from distributions instead of guesses, before a backtester exists.
//!
//! One line per record:
//!
//! ```text
//! {"type":"eval","filter":"spike_up","symbol":"XYZ","at_ms":1781531460000,"at_ny":"2026-06-15 09:51:00","result":"F","fired":false,"m":[["last",41.2],["atr",null]]}
//! {"type":"signal", ...same fields...}
//! {"type":"error","symbol":"XYZ","error":"..."}
//! ```
//!
//! `m` is an ordered list of `[label, value]` pairs, with `null` for unknown. Size is roughly
//! filters × symbols × bars, so point it at history or a watchlist, not the live full market.

use crate::engine::{EngineError, Evaluation, Signal, Sink};
use serde_json::{json, Value};
use std::io::{self, Write};
use std::sync::Arc;
use tp_marketdata::time::Ts;

pub struct JsonlSink<W: Write> {
    out: W,
    evaluations: bool,
    lines: u64,
    first_io_error: Option<io::Error>,
}

impl<W: Write> JsonlSink<W> {
    /// `evaluations: true` turns on instrumentation mode (every evaluation, not just signals).
    pub fn new(out: W, evaluations: bool) -> Self {
        JsonlSink { out, evaluations, lines: 0, first_io_error: None }
    }

    pub fn lines(&self) -> u64 {
        self.lines
    }

    /// Flush and return the writer, or the first write error. Writes can't fail loudly
    /// mid-scan without stopping it, so they fail loudly here.
    pub fn finish(mut self) -> io::Result<W> {
        if let Some(e) = self.first_io_error.take() {
            return Err(e);
        }
        self.out.flush()?;
        Ok(self.out)
    }

    fn write(&mut self, v: Value) {
        if self.first_io_error.is_some() {
            return;
        }
        let r = serde_json::to_writer(&mut self.out, &v).map_err(io::Error::from).and_then(|_| self.out.write_all(b"\n"));
        match r {
            Ok(()) => self.lines += 1,
            Err(e) => self.first_io_error = Some(e),
        }
    }
}

fn m(ms: &[(Arc<str>, Option<f64>)]) -> Value {
    Value::Array(ms.iter().map(|(l, v)| json!([&**l, v])).collect())
}

fn ny(t: Ts) -> String {
    t.ny().format("%Y-%m-%d %H:%M:%S").to_string()
}

impl<W: Write> Sink for JsonlSink<W> {
    fn signal(&mut self, s: &Signal) {
        let v = json!({
            "type": "signal", "filter": &*s.filter, "symbol": &*s.symbol,
            "at_ms": s.at.millis(), "at_ny": ny(s.at), "m": m(&s.measurements),
        });
        self.write(v);
    }

    fn evaluation(&mut self, e: &Evaluation<'_>) {
        let v = json!({
            "type": "eval", "filter": &**e.filter, "symbol": &**e.symbol,
            "at_ms": e.at.millis(), "at_ny": ny(e.at), "result": e.result.symbol(), "fired": e.fired,
            "m": m(e.measurements),
        });
        self.write(v);
    }

    fn wants_evaluations(&self) -> bool {
        self.evaluations
    }

    fn error(&mut self, symbol: &str, e: &EngineError) {
        self.write(json!({"type": "error", "symbol": symbol, "error": e.to_string()}));
    }
}
