//! The Rust port against TradierPro's own chart math.
//!
//! `tools/parity/gen.mjs` ran a verbatim copy of the TradierPro functions over the same fixture
//! and wrote `tools/parity/expected/parity_two_sessions.json`. This test replays the fixture
//! through [`ReplaySource`] (so the input door is exercised too) and requires agreement to 1e-9.

use serde_json::Value;
use std::collections::BTreeMap;
use tp_marketdata::bar::Bar;
use tp_marketdata::indicators::profile::{poc, value_area};
use tp_marketdata::indicators::vwap::vwap_bands;
use tp_marketdata::price::Unadjusted;
use tp_marketdata::source::{MarketEvent, MarketSource, ReplaySource, Resolution};
use tp_marketdata::time::Phase;

const FIXTURE: &str = include_str!("fixtures/parity_two_sessions.csv");
const EXPECTED: &str = include_str!("../../../tools/parity/expected/parity_two_sessions.json");
const TOL: f64 = 1e-9;

fn bars() -> Vec<Bar<Unadjusted>> {
    let mut src = ReplaySource::from_csv(FIXTURE, Resolution::Minute).expect("fixture loads");
    let mut out = vec![];
    while let Some(ev) = src.next_event() {
        match ev.expect("replay event") {
            MarketEvent::Bar { bar, .. } => out.push(bar),
            _ => unreachable!("fixture holds only bars"),
        }
    }
    out
}

fn close(a: f64, b: &Value, what: &str) {
    let b = b.as_f64().unwrap_or_else(|| panic!("{what}: expected a number, got {b}"));
    assert!((a - b).abs() <= TOL, "{what}: rust {a} vs ts {b}");
}

#[test]
fn vwap_bands_match_tradierpro() {
    let expected: Value = serde_json::from_str(EXPECTED).unwrap();
    let bars = bars();
    assert_eq!(bars.len() as u64, expected["bars"].as_u64().unwrap());
    let want = expected["vwap"].as_array().unwrap();
    let got = vwap_bands(&bars);
    assert_eq!(got.len(), want.len());
    let mut compared = 0;
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        match (g, w.is_null()) {
            (None, true) => {}
            (Some(g), false) => {
                close(g.vwap.value(), &w["vwap"], &format!("bar {i} vwap"));
                close(g.upper(1.0).value(), &w["d1up"], &format!("bar {i} +1σ"));
                close(g.lower(1.0).value(), &w["d1dn"], &format!("bar {i} -1σ"));
                close(g.upper(2.0).value(), &w["d2up"], &format!("bar {i} +2σ"));
                close(g.lower(2.0).value(), &w["d2dn"], &format!("bar {i} -2σ"));
                compared += 1;
            }
            (g, _) => panic!("bar {i}: rust {g:?} vs ts {w}"),
        }
    }
    assert!(compared > 150, "only {compared} bars had a VWAP; fixture lost its RTH bars?");
}

#[test]
fn poc_and_value_area_match_tradierpro() {
    let expected: Value = serde_json::from_str(EXPECTED).unwrap();
    let mut by_session: BTreeMap<String, Vec<Bar<Unadjusted>>> = BTreeMap::new();
    for b in bars() {
        if b.phase() == Phase::Rth {
            by_session.entry(b.session_date().to_string()).or_default().push(b);
        }
    }
    let sessions = expected["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), by_session.len());
    for s in sessions {
        let date = s["date"].as_str().unwrap();
        let rth = &by_session[date];
        close(poc(rth).unwrap().value(), &s["poc"], &format!("{date} POC"));
        let va = value_area(rth).unwrap();
        close(va.vah.value(), &s["vah"], &format!("{date} VAH"));
        close(va.val.value(), &s["val"], &format!("{date} VAL"));
    }
}
