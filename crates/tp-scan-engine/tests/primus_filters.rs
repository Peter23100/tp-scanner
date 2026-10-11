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
        "Breakout_Hold_Up", "Breakout_Hold_Down",
        // top
        "M_Open_Flush_Up", "M_OpenFlush_Down", "Blowoff_Reversal_Up", "Blowoff_Reversal_Down", "Quite_Reversal_Up",
        "Quiet_Reversal_Down",
        // statics
        "Movers_Up_Alist", "Movers_Down_Alist", "LowFloat_Movers_List", "PM_Coil_Up", "PM_Coil_Down",
        "Imbalance_Nasdaq_Buy", "Imbalance_Nasdaq_Sell", "Imbalance_NYSE_Buy", "Imbalance_NYSE_Sell",
        "Halted_Static_New_Minute", "NYSE_IND_UP", "NYSE_IND_DOWN", "PM_LowFloat_Coil_Up",
        // contra
        "Contra_Short_A_FAST", "Contra_Short_B_Extended", "Contra_Short_C_Breakdown", "Contra_Cap_A_Fast",
        "Contra_Cap_B_Extended", "Contra_Reclaim_C", "Contra_Long", "Contra_Short", "Contra_Long_V5", "Contra_Short_V5",
        "Contra_Long_V8", "Contra_Short_V8", "Contra_Long_Parabolic", "Contra_Short_Parabolic",
        "Momo_VWAP_Divergence_Long", "Contra_Long_VWAP_Divergence",
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
    // Every short-side filter stays quiet on a stock ramping up, except the Contra group, whose
    // job is fading exactly this (Contra_Short_A_FAST: an accelerating move).
    for sig in &s.signals {
        if !sig.filter.starts_with("Contra_") {
            assert_ne!(sig.side, Side::Short, "{} fired short", sig.filter);
        }
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
    // 600k by 10:00 on a 2M ADV: at the 09:59 bar (time 599) the curve is 0.1466 -> RVOL 2.05.
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
    e.set_reference(sym, Reference { float_shares: Some(float_shares), ..Default::default() });
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

/// RVOL_CURVE and PM_CURVE are piecewise linear between the master's points: the PYPL figure (0.92
/// at 15:44) is the documented check. Read through a probe list's columns at one print per time.
#[test]
fn the_volume_curves_interpolate() {
    let mut srcs: Vec<(String, String)> = std::fs::read_dir(filters_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "scan"))
        .map(|p| (p.display().to_string(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    srcs.push(("probe".into(), "list probe { hours any  show RVOL_CURVE, PM_CURVE  when 1 < 2 }".into()));
    let refs: Vec<(&str, &str)> = srcs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let curve_at = |h: u32, m: u32| -> (f64, f64) {
        let mut e = Engine::new(&tp_scan_engine::lang::parse_sources(&refs).unwrap()).unwrap();
        let mut sink = VecSink::default();
        e.on_event(&bar("X", h, m, 10.0, 10.0, 10.0, 10.0, 100.0), &mut sink);
        let rows = e.list("probe").unwrap();
        let col = |i: usize| rows[0].columns[i].1.unwrap();
        (col(0), col(1))
    };
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    for (h, m, want) in [(9, 30, 0.01), (9, 31, 0.016), (9, 32, 0.022), (10, 0, 0.15), (10, 15, 0.195), (15, 44, 0.92), (16, 0, 1.0), (17, 0, 1.0)] {
        let got = curve_at(h, m).0;
        assert!(close(got, want), "RVOL_CURVE at {h}:{m:02} = {got}, want {want}");
    }
    for (h, m, want) in [(4, 30, 0.03), (5, 30, 0.065), (8, 15, 0.425), (9, 30, 1.0)] {
        let got = curve_at(h, m).1;
        assert!(close(got, want), "PM_CURVE at {h}:{m:02} = {got}, want {want}");
    }
}

/// RVOL_NOW (common.scan) reads the pace of the last half hour; RVOL20DAYTIMEADJUSTED reads the
/// whole day. The PYPL shape: dead all session, then a burst at 15:30. Cumulative RVOL stays under
/// its 1.25 gate; RVOL_NOW shows the burst.
#[test]
fn rvol_now_sees_a_late_burst_that_cumulative_rvol_misses() {
    let mut srcs: Vec<(String, String)> = std::fs::read_dir(filters_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "scan"))
        .map(|p| (p.display().to_string(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    srcs.push(("probe".into(), "list probe { hours any  show RVOL_NOW, RVOL20DAYTIMEADJUSTED  when 1 < 2 }".into()));
    let refs: Vec<(&str, &str)> = srcs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut e = Engine::new(&tp_scan_engine::lang::parse_sources(&refs).unwrap()).unwrap();
    reference("PYPL", &mut e); // ADV 2M
    let mut sink = VecSink::default();
    let read = |e: &Engine| {
        let rows = e.list("probe").unwrap();
        (rows[0].columns[0].1, rows[0].columns[1].1)
    };
    // Normal pace until 15:00. 13:00-14:30 the curve adds 0.17 of the day over 90 minutes, so a
    // normal minute is 2M * 0.17 / 90 = 3,778 shares; hold that pace from 9:30 at the same rate
    // (well under the curve's morning share, so cumulative RVOL ends up low).
    let quiet = 3_778.0;
    let mut now_14 = None;
    for i in 0..(6 * 60) {
        let (h, m) = (9 + (30 + i) / 60, (30 + i) % 60);
        e.on_event(&bar("PYPL", h, m, 100.0, 100.1, 99.9, 100.0, quiet), &mut sink);
        if (h, m) == (14, 0) {
            // The last half hour ran at its normal pace: RVOL_NOW ~ 1.
            now_14 = read(&e).0;
        }
    }
    // 15:30-15:44: the burst, 60k a minute.
    for m in 30..45 {
        e.on_event(&bar("PYPL", 15, m, 100.0, 101.0, 99.9, 100.8, 60_000.0), &mut sink);
    }
    let (now_1544, cum_1544) = read(&e);
    assert!(sink.errors.is_empty(), "{:?}", sink.errors);
    let now_14 = now_14.unwrap_or_else(|| panic!("RVOL_NOW unknown at 14:00"));
    // The 14:00 window is the last 30 bars of the 13:00-14:30 segment: expected 30 * 3,778.
    assert!((now_14 - 1.0).abs() < 0.05, "RVOL_NOW at normal pace = {now_14}");
    let (now_1544, cum_1544) = (now_1544.unwrap(), cum_1544.unwrap());
    assert!(cum_1544 < 1.25, "cumulative RVOL {cum_1544} should stay under the 1.25 gate");
    assert!(now_1544 > 3.0, "RVOL_NOW {now_1544} should show the burst");
}

#[test]
fn rvol_now_is_unknown_outside_10_to_16() {
    let mut srcs: Vec<(String, String)> = std::fs::read_dir(filters_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "scan"))
        .map(|p| (p.display().to_string(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    srcs.push(("probe".into(), "list probe { hours any  show RVOL_NOW  when 1 < 2 }".into()));
    let refs: Vec<(&str, &str)> = srcs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut e = Engine::new(&tp_scan_engine::lang::parse_sources(&refs).unwrap()).unwrap();
    reference("X", &mut e);
    let mut sink = VecSink::default();
    for m in 0..50 {
        e.on_event(&bar("X", 9, m + 5, 100.0, 100.0, 100.0, 100.0, 50_000.0), &mut sink);
    }
    assert_eq!(e.list("probe").unwrap()[0].columns[0].1, None, "09:54 reaches into the premarket");
}

/// fixtures.json "unsatisfiable_at_load": the three Primus filters that ran "healthy" and could never
/// fire (or filtered nothing) are refused as written. Their translations carry the corrections.
#[test]
fn the_dead_primus_originals_are_refused_at_load() {
    use tp_scan_engine::lang::parse;
    let momo = "trigger Momo { when day_volume >= 500_000 and last - vwap <= -0.75 and last / vwap <= 0.97 and last - vwap >= 0.75 }";
    assert!(parse(momo).unwrap_err().msg.contains("can never be true"));
    let div = "trigger Div { when day_volume >= 500_000 and last - vwap = -1.25 }";
    assert!(parse(div).unwrap_err().msg.contains("no `=`"));
    let coil = "trigger Coil { when float <= 10_000_000_000_000 and pm_volume >= 100_000 }";
    assert!(parse(coil).unwrap_err().msg.contains("restricts nothing"));
}

/// A capitulation: quiet at 100 from 09:30 to 10:45 (15 RTH candles, so the 5-minute RSI is
/// known), then 30 minutes of red bars down to ~85 on rising volume. Contra_Long (four red 5-minute
/// stretches, RSI under 25, 75c+ under VWAP, a new low) fires; the Contra short side doesn't.
#[test]
fn a_capitulation_fires_contra_long() {
    let mut evs = vec![bar("CAP", 7, 0, 100.0, 100.0, 100.0, 100.0, 1_000.0)];
    for i in 0..75 {
        let (h, m) = (9 + (30 + i) / 60, (30 + i) % 60);
        evs.push(bar("CAP", h, m, 100.0, 100.1, 99.9, 100.0, 20_000.0));
    }
    for i in 0..30 {
        let (h, m) = (10 + (45 + i) / 60, (45 + i) % 60);
        let o = 100.0 - 0.5 * i as f64;
        evs.push(bar("CAP", h, m, o, o + 0.05, o - 0.55, o - 0.5, 60_000.0 + 2_000.0 * i as f64));
    }
    let (e, s) = run(evs, "CAP");
    let fires = fired(&s, "Contra_Long");
    if fires.is_empty() {
        panic!("{}", e.explain("Contra_Long", "CAP").unwrap());
    }
    for sig in &s.signals {
        assert!(!(sig.filter.starts_with("Contra_Short") && sig.side == Side::Short), "{} fired", sig.filter);
    }
}
