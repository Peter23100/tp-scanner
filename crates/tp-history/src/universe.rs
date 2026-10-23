//! Which symbols to download for a day: the day's **major movers** (TradierPro `BACKTEST_PLAN.md`
//! §7.2, rule 1). The other three sources — symbols the user traded, symbols a scanner filter
//! fired on, and a fixed list — are unioned in by the caller.
//!
//! A mover moved enough **and** traded enough:
//! - |close vs previous close| ≥ `min_change_pct`, or |open vs previous close| ≥ `min_gap_pct`;
//! - close ≥ `min_price` and dollar volume ≥ `min_dollar_volume`.
//!
//! The previous close is put on today's basis with the split factor for today (a 1-for-10 reverse
//! split is not a +900% move). A symbol with no previous close — a new listing, a ticker change —
//! is unknown, not a mover: it's reported in [`DayUniverse::no_reference`] so the caller can
//! decide, never silently counted.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoverRules {
    pub min_change_pct: f64,
    pub min_gap_pct: f64,
    pub min_price: f64,
    pub min_dollar_volume: f64,
}

impl Default for MoverRules {
    /// The plan's starting point, to be tuned with a dry run.
    fn default() -> Self {
        MoverRules { min_change_pct: 5.0, min_gap_pct: 4.0, min_price: 2.0, min_dollar_volume: 20_000_000.0 }
    }
}

/// One stock's day, as printed (unadjusted).
#[derive(Clone, Debug, PartialEq)]
pub struct DayBar {
    pub symbol: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl DayBar {
    /// Volume × the bar's typical price. An estimate (no VWAP in a daily bar), good enough for a
    /// liquidity floor.
    pub fn dollar_volume(&self) -> f64 {
        self.volume * (self.high + self.low + self.close) / 3.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mover {
    pub symbol: String,
    pub change_pct: f64,
    pub gap_pct: f64,
    pub dollar_volume: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DayUniverse {
    /// Largest |change| first.
    pub movers: Vec<Mover>,
    /// Liquid symbols with no previous close to measure against.
    pub no_reference: Vec<String>,
}

/// `prev_close`: yesterday's close by symbol, as printed. `split_factor(symbol)`: the factor for a
/// split executing TODAY (prices before it scale by it), 1.0 if none.
pub fn movers(prev_close: &HashMap<String, f64>, today: &[DayBar], split_factor: impl Fn(&str) -> f64, rules: &MoverRules) -> DayUniverse {
    let mut out = DayUniverse::default();
    for b in today {
        let dv = b.dollar_volume();
        if !(b.close >= rules.min_price && dv >= rules.min_dollar_volume) {
            continue;
        }
        let Some(&pc) = prev_close.get(&b.symbol) else {
            out.no_reference.push(b.symbol.clone());
            continue;
        };
        let ref_close = pc * split_factor(&b.symbol);
        if !(ref_close > 0.0) {
            out.no_reference.push(b.symbol.clone());
            continue;
        }
        let change_pct = (b.close / ref_close - 1.0) * 100.0;
        let gap_pct = (b.open / ref_close - 1.0) * 100.0;
        if change_pct.abs() >= rules.min_change_pct || gap_pct.abs() >= rules.min_gap_pct {
            out.movers.push(Mover { symbol: b.symbol.clone(), change_pct, gap_pct, dollar_volume: dv });
        }
    }
    out.movers.sort_by(|a, b| b.change_pct.abs().total_cmp(&a.change_pct.abs()));
    out.no_reference.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(s: &str, open: f64, close: f64, volume: f64) -> DayBar {
        DayBar { symbol: s.into(), open, high: open.max(close), low: open.min(close), close, volume }
    }

    fn prev(v: &[(&str, f64)]) -> HashMap<String, f64> {
        v.iter().map(|(s, c)| (s.to_string(), *c)).collect()
    }

    #[test]
    fn a_big_liquid_move_is_a_mover() {
        let u = movers(&prev(&[("AAA", 10.0)]), &[bar("AAA", 10.1, 11.0, 5_000_000.0)], |_| 1.0, &MoverRules::default());
        assert_eq!(u.movers.len(), 1);
        assert!((u.movers[0].change_pct - 10.0).abs() < 1e-9);
    }

    #[test]
    fn a_gap_that_faded_still_counts() {
        let u = movers(&prev(&[("GAP", 50.0)]), &[bar("GAP", 53.0, 50.5, 1_000_000.0)], |_| 1.0, &MoverRules::default());
        assert_eq!(u.movers.len(), 1, "6% gap, 1% close");
    }

    #[test]
    fn down_moves_count() {
        let u = movers(&prev(&[("DN", 100.0)]), &[bar("DN", 99.0, 90.0, 1_000_000.0)], |_| 1.0, &MoverRules::default());
        assert!(u.movers[0].change_pct < -9.0);
    }

    #[test]
    fn illiquid_or_cheap_names_are_out() {
        let r = MoverRules::default();
        // $5M traded: under the $20M floor.
        assert!(movers(&prev(&[("THIN", 10.0)]), &[bar("THIN", 10.0, 12.0, 400_000.0)], |_| 1.0, &r).movers.is_empty());
        // $1.50 close: under the $2 floor, however much it trades.
        assert!(movers(&prev(&[("PENNY", 1.0)]), &[bar("PENNY", 1.0, 1.5, 900_000_000.0)], |_| 1.0, &r).movers.is_empty());
    }

    #[test]
    fn small_moves_are_out() {
        let u = movers(&prev(&[("FLAT", 100.0)]), &[bar("FLAT", 101.0, 102.0, 9_000_000.0)], |_| 1.0, &MoverRules::default());
        assert!(u.movers.is_empty());
    }

    #[test]
    fn a_reverse_split_is_not_a_move() {
        // 1-for-10: yesterday's $2 close is $20 on today's basis; $20.40 today is +2%.
        let u = movers(&prev(&[("RS", 2.0)]), &[bar("RS", 20.2, 20.4, 3_000_000.0)], |s| if s == "RS" { 10.0 } else { 1.0 }, &MoverRules::default());
        assert!(u.movers.is_empty());
    }

    #[test]
    fn no_previous_close_is_unknown_not_a_mover() {
        let u = movers(&prev(&[]), &[bar("IPO", 20.0, 30.0, 5_000_000.0)], |_| 1.0, &MoverRules::default());
        assert!(u.movers.is_empty());
        assert_eq!(u.no_reference, vec!["IPO".to_string()]);
    }

    #[test]
    fn biggest_moves_come_first() {
        let p = prev(&[("A", 10.0), ("B", 10.0), ("C", 10.0)]);
        let t = [bar("A", 10.0, 10.6, 5e6), bar("B", 10.0, 7.0, 5e6), bar("C", 10.0, 12.0, 5e6)];
        let u = movers(&p, &t, |_| 1.0, &MoverRules::default());
        assert_eq!(u.movers.iter().map(|m| m.symbol.as_str()).collect::<Vec<_>>(), vec!["B", "C", "A"]);
    }
}
