//! How fast the engine takes per-second bars, the live feed's shape (`A.*`). Ignored by default: it
//! reads the local Massive cache (`tp-scan sync`, and a downloaded day). Run with
//! `cargo test --release -p tp-scan-cli --test per_second_bench -- --ignored --nocapture`.
//!
//! It takes one real day's 09:30–09:40 minute bars for every symbol and splits each into 60
//! one-second bars (price walking open → close, the minute's volume spread evenly), then runs all
//! the filters over them with the real daily references. Massive sends a second only when the
//! stock traded, so live is lighter than this: it's an upper bound.

use std::io::BufReader;
use tp_marketdata::bar::Bar;
use tp_marketdata::price::Price;
use tp_marketdata::source::{MarketEvent, MarketSource, Resolution};
use tp_scan_engine::engine::{Engine, EngineError, ListChange, Signal, Sink};

#[derive(Default)]
struct Count(u64);
impl Sink for Count {
    fn signal(&mut self, _s: &Signal) {
        self.0 += 1;
    }
    fn list_change(&mut self, _c: &ListChange<'_>) {}
    fn error(&mut self, _s: &str, _e: &EngineError) {}
}

#[test]
#[ignore]
fn per_second_bars_through_every_filter() {
    let data = tp_massive::data_dir().unwrap();
    let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
    let path = tp_massive::flatfiles::local_path(&data, day);
    let rd = BufReader::new(flate2::read::GzDecoder::new(std::fs::File::open(&path).expect("the 2026-10-09 flat file (fetch-day)")));
    let mut src = tp_marketdata::flatfile::read_minute_aggs(rd).unwrap();
    let open = tp_marketdata::time::Ts::from_ny_local(day, chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap()).unwrap().millis();
    let mut minutes = vec![];
    while let Some(Ok(MarketEvent::Bar { symbol, bar, .. })) = src.next_event() {
        let t = bar.start.millis();
        if (open..open + 10 * 60_000).contains(&t) {
            minutes.push((symbol, bar));
        }
    }
    // Split each minute into 60 seconds, all symbols interleaved in time order.
    let mut events = Vec::with_capacity(minutes.len() * 60);
    for s in 0..600i64 {
        let minute = open + (s / 60) * 60_000;
        let k = (s % 60) as f64;
        for (sym, b) in minutes.iter().filter(|(_, b)| b.start.millis() == minute) {
            let (o, c) = (b.open.value(), b.close.value());
            let p = o + (c - o) * (k + 1.0) / 60.0;
            let start = tp_marketdata::time::Ts::from_millis(open + s * 1000).unwrap();
            let bar = Bar { start, open: Price::new(p), high: Price::new(p.max(o.min(c)).min(b.high.value())), low: Price::new(p.min(o.max(c)).max(b.low.value())), close: Price::new(p), volume: b.volume / 60.0 };
            let bar = Bar { high: Price::new(bar.high.value().max(p)), low: Price::new(bar.low.value().min(p)), ..bar };
            events.push(MarketEvent::Bar { symbol: sym.clone(), resolution: Resolution::Second, bar });
        }
    }
    let symbols = minutes.iter().map(|(s, _)| s.clone()).collect::<std::collections::HashSet<_>>().len();

    let program = tp_scan_engine::lang::load_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../filters")).unwrap();
    let mut engine = Engine::new(&program).unwrap();
    let raw = tp_massive::daily_cache::RawDaily::load(&data.join("daily"), day - chrono::Duration::days(400)).unwrap();
    engine.set_daily_provider(tp_scan_engine::daily::provider_from(move |sym, session| raw.adjusted_history(sym, session, tp_scan_engine::daily::HISTORY_DAYS)));
    let mut sink = Count::default();
    let t0 = std::time::Instant::now();
    for e in &events {
        engine.on_event(e, &mut sink);
    }
    let secs = t0.elapsed().as_secs_f64();
    let rate = events.len() as f64 / secs;
    println!(
        "{} per-second bars ({symbols} symbols, 10 minutes) through {} filters in {secs:.2}s: {rate:.0} bars/s, {} signals. 10 minutes of market = 600 s, so the engine ran {:.1}x faster than the feed at this load.",
        events.len(),
        program.filters.len(),
        sink.0,
        600.0 / secs
    );
    assert!(600.0 / secs > 2.0, "the engine must keep up with the feed with room to spare");
}
