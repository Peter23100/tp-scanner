//! The translated Primus filters (`filters/*.scan`): they parse, compile, and behave.
//!
//! This is the language's conformance test: every verbatim filter from the master has to be
//! expressible. The scenario tests then build market shapes by hand and check the filters that
//! hunt them fire, and the look-alikes that shouldn't, don't. They prove the translation runs end
//! to end; they say nothing about whether the thresholds are good (that needs real data).

use chrono::{NaiveDate, NaiveTime};
use std::path::Path;
use std::sync::Arc;
use tp_marketdata::bar::Bar;
use tp_marketdata::price::Price;
use tp_marketdata::source::{MarketEvent, Resolution};
use tp_marketdata::time::{SessionDate, Ts};
use tp_scan_engine::compile::{Kind, Side, Tri};
use tp_scan_engine::engine::{Engine, VecSink};
use tp_scan_engine::lang::load_dir;
use tp_scan_engine::state::{DailyRef, Reference};

fn filters_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../filters")
}

fn engine() -> Engine {
    let p = load_dir(&filters_dir()).unwrap_or_else(|e| panic!("{e}"));
    Engine::new(&p).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn every_translated_filter_parses_and_compiles() {
    let p = load_dir(&filters_dir()).unwrap_or_else(|e| panic!("{e}"));
    let e = Engine::new(&p).unwrap();
    let names: Vec<&str> = e.compiled().filters().iter().map(|f| &*f.name).collect();
    let expected = [
        // left
        "M_Spike_Up", "M_Spike_Down", "M_Pop_Sensitive_Up", "M_Drop_Sensitive_Down", "M_Pop_Medium_EXT_Up",
        "M_Drop_Medium_EXT_Down", "M_Drive_Up", "M_Drive_Down", "High_ATR_Pop", "High_ATR_Drop",
        // right
        "M_Spike_Hold_Up", "M_Spike_Hold_Down", "M_Run_Hold_Up", "M_Run_Hold_Down", "M3_Breakout_Long",
        "M4_Breakdown_Short", "M5_Pullback_Long", "M6_Bounce_Short", "M7_GrindUp", "M8_GrindDown",
        "LowFloat_HOD_Break_Up", "HOD_Base_Break_LQ", "Base_Break_Hold_Up", "Base_Break_Hold_Down",
        // top
        "M_Open_Flush_Up", "M_OpenFlush_Down", "Blowoff_Reversal_Up", "Blowoff_Reversal_Down", "Quite_Reversal_Up",
        "Quiet_Reversal_Down",
        // statics
        "Movers_Up_Alist", "Movers_Down_Alist", "LowFloat_Movers_List", "PM_Coil_Up", "PM_Coil_Down",
        "Imbalance_Nasdaq_Buy", "Imbalance_Nasdaq_Sell", "Imbalance_NYSE_Buy", "Imbalance_NYSE_Sell",
        "Halted_Static_New_Minute", "NYSE_IND_UP", "NYSE_IND_DOWN",
    ];
    let mut got = names.clone();
    got.sort();
    let mut want = expected.to_vec();
    want.sort();
    assert_eq!(got, want);

    // The 21 custom fields from master §2 are all defined.
    let fields: Vec<&str> = p.num_defs.iter().map(|(n, _)| &**n).collect();
    for f in [
        "RVOL20DAYTIMEADJUSTED", "PM_RVOL", "VolROC3", "VolROC_1m_3", "VolFade_1m_Smooth", "Spike_Vol_Pct", "ATR_POS",
        "ATR_NEG", "Spike_Range_Pct", "PM_RangePos", "VWAP_Stretch", "Retrace_Pct_Up", "Retrace_Pct_Down",
        "Retrace_1m_Up", "Retrace_1m_Down", "RNK_UP", "RNK_DWN", "Clear_ATR", "Near_ATR", "Imb_Buy_Ratio",
        "Imb_Sell_Ratio",
    ] {
        assert!(fields.contains(&f), "field {f} missing");
    }

    // Every up/long filter has a side, and lists are lists.
    for f in e.compiled().filters() {
        let n = &*f.name;
        if n.contains("_Up") || n.contains("Long") || n.ends_with("_Pop") || n.contains("GrindUp") || n.ends_with("_UP") {
            assert_eq!(f.side, Side::Long, "{n}");
        }
        if n.contains("Down") || n.contains("Short") || n.ends_with("_Drop") || n.ends_with("_DOWN") || n.ends_with("_Sell") {
            assert_eq!(f.side, Side::Short, "{n}");
        }
    }
    let lists: Vec<&str> = e.compiled().filters().iter().filter(|f| f.kind == Kind::List).map(|f| &*f.name).collect();
    assert_eq!(lists.len(), 10, "{lists:?}");
}

// ---------------------------------------------------------------------------------------------
// Scenario harness

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 6, 15).unwrap()
}

fn at(h: u32, m: u32) -> Ts {
    Ts::from_ny_local(day(), NaiveTime::from_hms_opt(h, m, 0).unwrap()).unwrap()
}

#[allow(clippy::too_many_arguments)] // an OHLCV bar reads best as one call
fn bar(sym: &str, h: u32, m: u32, o: f64, hi: f64, lo: f64, c: f64, v: f64) -> MarketEvent {
    MarketEvent::Bar {
        symbol: Arc::from(sym),
        resolution: Resolution::Minute,
        bar: Bar { start: at(h, m), open: Price::new(o), high: Price::new(hi), low: Price::new(lo), close: Price::new(c), volume: v },
    }
}

/// prev close 100, ATR 4 (4% of price: the "volatile" band), ADV 2M.
fn reference(sym: &str, e: &mut Engine) {
    let d = DailyRef {
        atr: Some(Price::new(4.0)),
        adv: Some(2_000_000.0),
        prev_high: Some(Price::new(101.5)),
        prev_low: Some(Price::new(98.0)),
        ..DailyRef::bare(SessionDate(day()), Price::new(100.0))
    };
    e.set_daily(sym, d);
}

/// 09:30–09:59: quiet at 101, 20k shares a minute (RVOL 2 by 10:00).
fn quiet_open(sym: &str) -> Vec<MarketEvent> {
    (0..30).map(|i| bar(sym, 9, 30 + i, 101.0, 101.2, 100.8, 101.0, 20_000.0)).collect()
}

/// 10:00–10:04: a clean ramp 101 -> 106, closing on each high, 100k a minute.
fn spike(sym: &str) -> Vec<MarketEvent> {
    (0..5)
        .map(|i| {
            let lo = 101.0 + i as f64;
            bar(sym, 10, i, lo, lo + 1.0, lo, lo + 1.0, 100_000.0)
        })
        .collect()
}

fn run(evs: Vec<MarketEvent>, sym: &str) -> (Engine, VecSink) {
    let mut e = engine();
    reference(sym, &mut e);
    let mut sink = VecSink::default();
    for ev in &evs {
        e.on_event(ev, &mut sink);
    }
    assert!(sink.errors.is_empty(), "{:?}", sink.errors);
    (e, sink)
}

fn fired<'a>(s: &'a VecSink, filter: &str) -> Vec<&'a tp_scan_engine::engine::Signal> {
    s.signals.iter().filter(|x| &*x.filter == filter).collect()
}

#[test]
fn a_spike_fires_m_spike_up_and_the_hold_fires_m_spike_hold_up() {
    let mut evs = quiet_open("SNDK");
    evs.extend(spike("SNDK"));
    // 10:05–10:12: holds in the top quarter of the spike bar (104.75+), never dipping far.
    for i in 5..13 {
        evs.push(bar("SNDK", 10, i, 105.6, 105.9, 105.3, 105.6, 30_000.0));
    }
    let (_, s) = run(evs, "SNDK");

    let spikes = fired(&s, "M_Spike_Up");
    assert!(!spikes.is_empty(), "M_Spike_Up should fire during the ramp");
    assert!(spikes[0].at <= at(10, 5), "on the ramp, not after it: {:?}", spikes[0].at.ny());
    let atr_pos = spikes[0].columns.iter().find(|(l, _)| &**l == "ATR_POS").unwrap().1.unwrap();
    assert!(atr_pos >= 50.0, "ATR_POS {atr_pos} should clear the 0.50 ATR band");

    let holds = fired(&s, "M_Spike_Hold_Up");
    assert!(!holds.is_empty(), "M_Spike_Hold_Up should fire once the hold is established");
    assert!(holds[0].at > at(10, 5), "not before the spike bar completes: {:?}", holds[0].at.ny());
    // Every short-side filter stays quiet on a stock ramping up.
    for sig in &s.signals {
        assert_ne!(sig.side, Side::Short, "{} fired short", sig.filter);
    }
}

#[test]
fn a_spike_that_gives_it_back_does_not_fire_the_hold() {
    let mut evs = quiet_open("AMAT");
    evs.extend(spike("AMAT"));
    // 10:05–10:12: round-trips to 102, deep inside the spike bar.
    for i in 5..13 {
        evs.push(bar("AMAT", 10, i, 103.0, 103.5, 102.0, 102.2, 30_000.0));
    }
    let (e, s) = run(evs, "AMAT");
    assert!(fired(&s, "M_Spike_Hold_Up").is_empty());
    // And the explanation names the retention clauses.
    let ex = e.explain("M_Spike_Hold_Up", "AMAT").unwrap();
    assert_eq!(ex.result, Tri::False);
    let blocking: Vec<String> = ex.blocking().iter().map(|n| n.label.clone()).collect();
    assert!(blocking.iter().any(|l| l.contains("spike_range * 0.25")), "{blocking:?}");
}

#[test]
fn a_quiet_stock_fires_nothing() {
    let (_, s) = run(quiet_open("KO"), "KO");
    assert!(s.signals.is_empty(), "{:?}", s.signals.iter().map(|x| &*x.filter).collect::<Vec<_>>());
}

#[test]
fn the_movers_list_ranks_by_rnk_up() {
    let mut e = engine();
    reference("AAA", &mut e);
    reference("BBB", &mut e);
    let mut sink = VecSink::default();
    // Both up on the day with RVOL > 1.5: AAA +2 ATR, BBB +1 ATR.
    for i in 0..10 {
        e.on_event(&bar("AAA", 9, 30 + i, 108.0, 108.2, 107.8, 108.0, 100_000.0), &mut sink);
        e.on_event(&bar("BBB", 9, 30 + i, 104.0, 104.2, 103.8, 104.0, 100_000.0), &mut sink);
    }
    let rows: Vec<(String, f64)> =
        e.list("Movers_Up_Alist").unwrap().iter().map(|r| (r.symbol.to_string(), r.rank.unwrap())).collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "AAA");
    assert!((rows[0].1 - 200.0).abs() < 1e-9 && (rows[1].1 - 100.0).abs() < 1e-9, "{rows:?}");
    assert!(e.list("Movers_Down_Alist").unwrap().is_empty());
}

#[test]
fn rvol_follows_the_curve() {
    // 600k by 10:00 on a 2M ADV: curve 0.15 -> RVOL 2.0 at the 09:59 bar (TickTime 599 -> 0.15).
    let (e, _) = run(quiet_open("SNDK"), "SNDK");
    let text = e.explain("M_Spike_Up", "SNDK").unwrap().to_string();
    assert!(text.contains("RVOL20DAYTIMEADJUSTED = 2"), "{text}");
}

/// The MSGY shape (master §4.21, rebuilt 2026-09-25): a dormant $2 low-float with a 0.12 ATR wakes
/// up and runs. Every bar closes 3 cents under its high, so the old `day_high - 0.15 * atr`
/// tolerance (under 2 cents) blocked it; the rebuilt `0.15 * day_range` tolerance doesn't.
fn low_float_runner(sym: &str, float_shares: f64) -> (Engine, VecSink) {
    let mut e = engine();
    let d = DailyRef {
        atr: Some(Price::new(0.12)),
        adv: Some(300_000.0),
        ..DailyRef::bare(SessionDate(day()), Price::new(2.0))
    };
    e.set_daily(sym, d);
    e.set_reference(sym, Reference { float_shares: Some(float_shares) });
    let mut sink = VecSink::default();
    // One 07:00 premarket print (so the feed covers the 120-minute window), then 09:30–09:59
    // drifts 2.00 -> 2.87 on 30k a minute, and 10:00–10:09 runs 2.90 -> 3.80 on 60k then 100k.
    let mut evs = vec![bar(sym, 7, 0, 2.0, 2.0, 2.0, 2.0, 5_000.0)];
    for i in 0..30 {
        let c = 2.0 + 0.03 * i as f64;
        evs.push(bar(sym, 9, 30 + i, c - 0.01, c + 0.03, c - 0.02, c, 30_000.0));
    }
    for i in 0..10 {
        let c = 2.9 + 0.1 * i as f64;
        let v = if i < 5 { 60_000.0 } else { 100_000.0 };
        evs.push(bar(sym, 10, i, c - 0.08, c + 0.03, c - 0.09, c, v));
    }
    for ev in &evs {
        e.on_event(ev, &mut sink);
    }
    assert!(sink.errors.is_empty(), "{:?}", sink.errors);
    (e, sink)
}

#[test]
fn the_rebuilt_low_float_runner_fires_on_a_tiny_atr_name() {
    let (_, s) = low_float_runner("MSGY", 1_500_000.0);
    let fires = fired(&s, "LowFloat_HOD_Break_Up");
    // The 09:30 drift (+1.5% a minute) already qualifies by ~09:47; every bar closes 3 cents off
    // its high, more than the old 0.15 x ATR (1.8 cents) allowed.
    assert!(!fires.is_empty(), "should fire during the run");
}

#[test]
fn the_low_float_runner_ignores_a_big_float() {
    let (e, s) = low_float_runner("BIGF", 50_000_000.0);
    assert!(fired(&s, "LowFloat_HOD_Break_Up").is_empty());
    let ex = e.explain("LowFloat_HOD_Break_Up", "BIGF").unwrap();
    let blocking: Vec<String> = ex.blocking().iter().map(|n| n.label.clone()).collect();
    assert!(blocking.iter().any(|l| l.contains("float")), "{blocking:?}");
}
