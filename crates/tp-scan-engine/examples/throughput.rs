//! Measures the investigation's scale claim: 40 filters x 5,000 symbols x 1 Hz ~ 20% of one core.
//!
//!   cargo run --release -p tp-scan-engine --example throughput
//!   cargo run --release -p tp-scan-engine --example throughput -- filters   # the real Primus set
//!
//! Synthetic data: 5,000 symbols, one-minute bars from 09:30, 40 filters shaped like the Primus
//! sharp-move family (windows, an ATR ratio, a volume ratio, a new-high event, a shared
//! tradeable predicate), each with different thresholds. Reports evaluations per second on one
//! thread, measured over the second hour (warm rings, full windows).

use std::sync::Arc;
use std::time::Instant;
use tp_marketdata::bar::Bar;
use tp_marketdata::price::Price;
use tp_marketdata::source::{MarketEvent, Resolution};
use tp_marketdata::time::{SessionDate, Ts};
use tp_scan_engine::compile::{FilterDef, Program};
use tp_scan_engine::engine::{Engine, Signal, Sink};
use tp_scan_engine::expr::{def, m, n, Cond};
use tp_scan_engine::measure::{Event, Measure};
use tp_scan_engine::state::{DailyRef, Win};

struct Count(u64);
impl Sink for Count {
    fn signal(&mut self, _: &Signal) {
        self.0 += 1;
    }
}

fn main() {
    const SYMBOLS: usize = 5_000;
    const FILTERS: usize = 40;
    const MINUTES: i64 = 120;
    let windows = [5i64, 15, 30];
    let mut p = Program::default().define_cond(
        "tradeable",
        Cond::all([m(Measure::Last).ge(n(2.0)), m(Measure::DayVolume { extended: false }).ge(n(250_000.0))]),
    );
    for w in windows {
        p = p.define_num(&format!("lift{w}"), m(Measure::Last) - m(Measure::Low(Win::rolling_min(w))));
    }
    for i in 0..FILTERS {
        let w = windows[i % 3];
        let k = 0.2 + 0.01 * i as f64;
        p = p.filter(FilterDef::new(
            &format!("f{i}"),
            Cond::all([
                Cond::named("tradeable"),
                (def(&format!("lift{w}")) / m(Measure::Atr)).ge(n(k)),
                def(&format!("lift{w}")).ge(n(0.5)),
                m(Measure::Volume(Win::rolling_min(w))).ge(m(Measure::Volume(Win::rolling_min_back(w, 1))) * n(1.5)),
                Cond::event(Event::NewHigh { span_ms: w * 60_000 }),
            ]),
            60_000,
        ));
    }
    // With a directory argument, measure the translated Primus filters instead.
    if let Some(dir) = std::env::args().nth(1) {
        p = tp_scan_engine::lang::load_dir(std::path::Path::new(&dir)).unwrap_or_else(|e| panic!("{e}"));
    }
    let mut engine = Engine::new(&p).unwrap();
    let filters = engine.compiled().filters().len();
    let open = Ts::from_ny_local(
        chrono::NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
        chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
    )
    .unwrap();
    let session = SessionDate::of(open);
    let syms: Vec<Arc<str>> = (0..SYMBOLS).map(|i| Arc::from(format!("S{i:04}"))).collect();
    for s in &syms {
        engine.set_daily(
            s,
            DailyRef {
                atr: Some(Price::new(1.0)),
                adv: Some(1e6),
                prev_high: Some(Price::new(21.0)),
                prev_low: Some(Price::new(19.0)),
                prev_month_high: Some(Price::new(25.0)),
                prev_month_low: Some(Price::new(15.0)),
                closes: Arc::from(vec![20.0; 260]),
                ..DailyRef::bare(session, Price::new(20.0))
            },
        );
    }
    let mut px: Vec<f64> = (0..SYMBOLS).map(|i| 5.0 + (i % 200) as f64).collect();
    let mut rng: u64 = 0x1234_5678_9abc_def0;
    let mut sink = Count(0);
    let mut timed_evals = 0u64;
    let mut timed = std::time::Duration::ZERO;
    for minute in 0..MINUTES {
        let start = open.plus_millis(minute * 60_000).unwrap();
        let evs: Vec<MarketEvent> = syms
            .iter()
            .enumerate()
            .map(|(i, s)| {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                let d = ((rng % 1000) as f64 / 1000.0 - 0.5) * 0.02 * px[i];
                let o = px[i];
                let c = (o + d).max(0.5);
                px[i] = c;
                let bar = Bar {
                    start,
                    open: Price::new(o),
                    high: Price::new(o.max(c) * 1.002),
                    low: Price::new(o.min(c) * 0.998),
                    close: Price::new(c),
                    volume: (5_000 + rng % 50_000) as f64,
                };
                MarketEvent::Bar { symbol: s.clone(), resolution: Resolution::Minute, bar }
            })
            .collect();
        let before = engine.stats().evaluations;
        let t = Instant::now();
        for ev in &evs {
            engine.on_event(ev, &mut sink);
        }
        if minute >= 60 {
            timed += t.elapsed();
            timed_evals += engine.stats().evaluations - before;
        }
    }
    let per_sec = timed_evals as f64 / timed.as_secs_f64();
    let cycle = timed.as_secs_f64() / (MINUTES - 60) as f64;
    println!("symbols {SYMBOLS}, filters {filters}, measured over {} one-minute cycles", MINUTES - 60);
    println!("evaluations/sec (one thread): {per_sec:.0}");
    println!("one full-market cycle (all symbols x all filters): {:.1} ms", cycle * 1e3);
    println!("=> at a 1 Hz cycle, {:.1}% of one core", cycle * 100.0);
    println!("signals: {}, unknown evaluations: {}, errors: {}", sink.0, engine.stats().unknown, engine.stats().errors);
}
