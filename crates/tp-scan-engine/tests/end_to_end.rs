//! The engine over a replay: determinism, no lookahead, fault isolation, instrumentation.
//!
//! The filter is a hand translation of the Primus `M_Spike_Up` **structure**. Its thresholds are
//! placeholders sized to the synthetic fixture, not the user's numbers (thresholds don't port;
//! see the investigation §7b).

use chrono::NaiveDate;
use tp_marketdata::price::Price;
use tp_marketdata::source::{MarketEvent, MarketSource, ReplaySource, Resolution, SourceError};
use tp_marketdata::time::SessionDate;
use tp_scan_engine::compile::{FilterDef, Program, Tri};
use tp_scan_engine::engine::{Engine, Signal, VecSink};
use tp_scan_engine::expr::{m, n, Cond};
use tp_scan_engine::instrument::JsonlSink;
use tp_scan_engine::measure::{Event, Measure};
use tp_scan_engine::state::{DailyRef, Win};

const FIXTURE: &str = include_str!("../../tp-marketdata/tests/fixtures/parity_two_sessions.csv");

fn events() -> Vec<MarketEvent> {
    let mut src = ReplaySource::from_csv(FIXTURE, Resolution::Minute).unwrap();
    let mut v = vec![];
    while let Some(e) = src.next_event() {
        v.push(e.unwrap());
    }
    v
}

struct VecSource(std::vec::IntoIter<MarketEvent>);
impl MarketSource for VecSource {
    fn next_event(&mut self) -> Option<Result<MarketEvent, SourceError>> {
        self.0.next().map(Ok)
    }
}

fn program() -> Program {
    let last = || m(Measure::Last);
    let low5 = || m(Measure::Low(Win::rolling_min(5)));
    Program::default()
        .define_num("lift", last() - low5())
        .define_cond("tradeable", Cond::all([last().ge(n(2.0)), m(Measure::DayVolume { extended: false }).ge(n(50_000.0))]))
        .filter(FilterDef::new(
            "spike_up",
            Cond::all([
                Cond::named("tradeable"),
                (tp_scan_engine::expr::def("lift") / m(Measure::Atr)).ge(n(0.30)),
                tp_scan_engine::expr::def("lift").ge(n(0.30)),
                m(Measure::Volume(Win::rolling_min(5))).ge(m(Measure::Volume(Win::rolling_min_back(5, 1))) * n(1.2)),
                Cond::event(Event::NewHigh { span_ms: 5 * 60_000 }),
            ]),
            60_000,
        ))
        // Needs no daily reference, so it can fire on both days.
        .filter(FilterDef::new("vwap_reclaim", Cond::all([m(Measure::Last).gt(m(Measure::Vwap)), m(Measure::Low(Win::rolling_min(1))).lt(m(Measure::Vwap))]), 5 * 60_000))
}

fn day1() -> DailyRef {
    DailyRef {
        session: SessionDate(NaiveDate::from_ymd_opt(2026, 6, 15).unwrap()),
        prev_close: Price::new(41.0),
        atr: Some(Price::new(1.0)),
        atr_len: 14,
        adv: Some(2_000_000.0),
        adv_len: 20,
    }
}

fn run(evs: Vec<MarketEvent>) -> (Engine, VecSink) {
    let mut e = Engine::new(&program()).unwrap();
    e.set_daily("XYZ", day1());
    let mut sink = VecSink::default();
    e.run(&mut VecSource(evs.into_iter()), &mut sink);
    (e, sink)
}

#[test]
fn fires_and_is_deterministic() {
    let (e1, s1) = run(events());
    let (_, s2) = run(events());
    assert!(!s1.signals.is_empty());
    assert_eq!(s1.signals, s2.signals);
    assert!(s1.errors.is_empty(), "{:?}", s1.errors);
    let st = e1.stats();
    assert_eq!(st.evaluations, st.events * 2);
    let spikes: Vec<&Signal> = s1.signals.iter().filter(|s| &*s.filter == "spike_up").collect();
    assert!(!spikes.is_empty(), "the spike filter should fire on the fixture");
    // Every spike carries its full measurement vector, with the ATR known.
    let atr = spikes[0].measurements.iter().find(|(l, _)| &**l == "atr").unwrap();
    assert_eq!(atr.1, Some(1.0));
}

#[test]
fn a_reference_for_another_session_is_refused_not_reused() {
    let (_, s) = run(events());
    let day2 = NaiveDate::from_ymd_opt(2026, 6, 16).unwrap();
    let spikes_day2 = s.signals.iter().filter(|x| &*x.filter == "spike_up" && x.at.ny().date_naive() == day2).count();
    assert_eq!(spikes_day2, 0, "day 2 has no daily reference, so ATR is unknown and spike_up can't fire");
    let reclaims_day2 = s.signals.iter().filter(|x| &*x.filter == "vwap_reclaim" && x.at.ny().date_naive() == day2).count();
    assert!(reclaims_day2 > 0, "filters that don't need the reference keep working");
}

#[test]
fn every_signal_is_reproducible_from_data_up_to_its_timestamp() {
    // The no-lookahead property: cut the replay at a signal's time and the signal is still there,
    // identical. A filter that peeked at a later bar would change or vanish.
    let all = events();
    let (_, full) = run(all.clone());
    for sig in full.signals.iter().take(25) {
        let prefix: Vec<MarketEvent> = all.iter().filter(|e| e.available_at() <= sig.at).cloned().collect();
        let (_, cut) = run(prefix);
        assert_eq!(cut.signals.last(), Some(sig), "signal at {:?} changed when later data was removed", sig.at);
    }
}

#[test]
fn a_broken_symbol_does_not_stop_the_scan() {
    let mut evs = events();
    // Inject a second symbol that goes backwards in time halfway through.
    let MarketEvent::Bar { bar, .. } = evs[40].clone() else { unreachable!() };
    let bad = std::sync::Arc::<str>::from("BAD");
    let good_bar = MarketEvent::Bar { symbol: bad.clone(), resolution: Resolution::Minute, bar };
    let mut back = bar;
    back.start = evs[10].available_at();
    let backwards = MarketEvent::Bar { symbol: bad, resolution: Resolution::Minute, bar: back };
    evs.insert(41, good_bar);
    evs.insert(42, backwards);
    let (e, s) = run(evs);
    let (_, clean) = run(events());
    assert_eq!(e.stats().errors, 1);
    assert_eq!(s.errors.len(), 1);
    assert_eq!(s.errors[0].0, "BAD");
    let xyz: Vec<&Signal> = s.signals.iter().filter(|x| &*x.symbol == "XYZ").collect();
    assert_eq!(xyz, clean.signals.iter().collect::<Vec<_>>(), "XYZ is unaffected");
}

#[test]
fn instrumentation_writes_every_evaluation() {
    let mut e = Engine::new(&program()).unwrap();
    e.set_daily("XYZ", day1());
    let mut sink = JsonlSink::new(Vec::new(), true);
    let stats = e.run(&mut VecSource(events().into_iter()), &mut sink);
    let lines = sink.lines();
    let out = String::from_utf8(sink.finish().unwrap()).unwrap();
    assert_eq!(lines, stats.evaluations + stats.signals);
    let mut evals = 0;
    for l in out.lines() {
        let v: serde_json::Value = serde_json::from_str(l).unwrap();
        if v["type"] == "eval" {
            evals += 1;
            assert!(v["m"].as_array().unwrap().len() >= 2);
        }
    }
    assert_eq!(evals, stats.evaluations);
}

#[test]
fn explain_after_a_run() {
    let (e, _) = run(events());
    let ex = e.explain("spike_up", "XYZ").unwrap();
    // Last bar is day 2, no reference: the ATR clause is unknown, and it's named.
    assert_ne!(ex.result, Tri::True);
    let text = ex.to_string();
    assert!(text.contains("atr = ?"), "{text}");
    assert!(e.explain("nope", "XYZ").is_none());
    assert!(e.explain("spike_up", "NOPE").is_none());
}
