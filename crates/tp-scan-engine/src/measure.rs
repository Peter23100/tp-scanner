//! The measurement catalog: the named quantities a filter can read.
//!
//! Each variant has one exact meaning, stated here and tested. The list was taken from the
//! primitives the exported Primus filters actually use (`spec/primus/`), with the deliberate
//! semantic changes from the investigation: windows are rolling unless marked clock-aligned, and
//! daily references are adjusted onto today's basis.
//!
//! Where Primus behaviour is undocumented, the choice made here is stated on the variant rather
//! than guessed silently. None of them claims to match Primus to the penny.

use crate::state::{SymbolState, Win};
use std::fmt;
use tp_marketdata::indicators::series::{bollinger_last, ema_last, rsi_last, sma_last};

/// An indicator read on the symbol's 5-minute candles (Primus `EMAStock(..., MINUTES_5, ...)`).
/// Bollinger's `k` is stored in hundredths so the enum stays hashable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CandleInd {
    Ema(u16),
    Sma(u16),
    /// Wilder's RSI.
    Rsi(u16),
    /// Bollinger upper band: SMA(len) + k × population sd.
    BbUpper { len: u16, k_centi: u16 },
    BbLower { len: u16, k_centi: u16 },
}

/// Which listing venue's imbalance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Venue {
    Nyse,
    Nasdaq,
    /// Whichever venue published one; the sum if both did. Primus's `ANY` is undocumented (it may
    /// or may not equal `ALL_VENUES`); a symbol has one primary listing, so in practice one side.
    Any,
}

/// A numeric measurement. Evaluates to `None` ("unknown") rather than inventing a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Measure {
    /// Close of the newest bar, any session phase.
    Last,
    /// Session volume so far. `extended: false` counts RTH only.
    DayVolume { extended: bool },
    /// Highest high in the window. Unknown if the window has no bars.
    High(Win),
    /// Lowest low in the window. Unknown if the window has no bars.
    Low(Win),
    /// Volume in the window (0 if covered but quiet).
    Volume(Win),
    /// Open of the window's first bar (Primus `MinuteOpen_I(..., N, Pk, ...)`).
    Open(Win),
    /// Close of the window's last bar (Primus `MinuteClose_I(..., N, Pk, ...)`).
    Close(Win),
    /// RTH high so far today, including the newest bar. Unknown before 09:30.
    DayHigh,
    /// RTH low so far today.
    DayLow,
    /// The prior session's close, adjusted onto today's basis. Unknown without a daily reference
    /// built for today. (Primus `ClosePrice(PRIMARY, P1)` and `AdjustedClosePrice(P1)` both map
    /// here: there is one consolidated close, and it is always adjusted.)
    PrevClose,
    /// The prior session's high / low (Primus `DayHigh(ALL_VENUES, 1, P1, NO)`).
    PrevHigh,
    PrevLow,
    /// High / low of the prior completed calendar month (Primus `MonthHigh_I(P1)`).
    PrevMonthHigh,
    PrevMonthLow,
    /// Wilder ATR over the daily history, adjusted onto today's basis. Same condition. Covers
    /// completed days only; whether Primus's `CURRENT` folds in today's range is unknown.
    Atr,
    /// Mean daily volume over the daily history. Same condition.
    AvgDailyVolume,
    /// EMA of daily closes with today's last price as the provisional close for today (a chart's
    /// daily EMA on the forming bar). Unknown without today's daily reference.
    DailyEma(u16),
    /// Session VWAP (RTH-anchored). Unknown before the first RTH volume.
    Vwap,
    /// VWAP ± k standard deviations, the TradierPro chart's bands. `k` in hundredths. ⚠ Primus's
    /// `VWAP_UpperBand`/`LowerBand` are documented as broken with an unknown formula; these are
    /// NOT a reproduction of them.
    VwapBand { upper: bool, k_centi: u16 },
    /// VWAP over every phase from the session's first print, premarket included
    /// (Primus `VWAP(CURRENT, YES, ...)`).
    VwapExt,
    /// Open of the first RTH bar (Primus `DayBar_Open(..., '9:30')`). Unknown before 09:30.
    SessionOpen,
    PremarketHigh,
    PremarketLow,
    PremarketVolume,
    /// Minutes since New York midnight of the newest print (Primus `TickTime`; 570 = 09:30).
    Time,
    /// An indicator on clock-aligned candles of `tf_min` minutes. Only 5 is kept; any other width
    /// is unknown.
    Candle { tf_min: u16, ind: CandleInd },
    /// Float in shares.
    Float,
    /// Closing/opening auction imbalance, shares.
    Imbalance { buy: bool, venue: Venue },
    /// NYSE indicative bid / ask.
    IndBid,
    IndAsk,
    /// Auction clearing price and near (indicative) price on the primary listing.
    ClearPrice,
    NearPrice,
}

fn k(centi: u16) -> f64 {
    centi as f64 / 100.0
}

impl Measure {
    pub fn eval(&self, s: &SymbolState) -> Option<f64> {
        let px = |p: Option<tp_marketdata::price::Price<tp_marketdata::price::Unadjusted>>| p.map(|p| p.value());
        match *self {
            Measure::Last => s.last_bar().map(|b| b.close.value()),
            Measure::DayVolume { extended } => s.day_volume(extended),
            Measure::High(w) => s.window(w)?.high.map(|p| p.value()),
            Measure::Low(w) => s.window(w)?.low.map(|p| p.value()),
            Measure::Volume(w) => s.window(w).map(|a| a.volume),
            Measure::Open(w) => s.window(w)?.open.map(|p| p.value()),
            Measure::Close(w) => s.window(w)?.close.map(|p| p.value()),
            Measure::DayHigh => px(s.day_high()),
            Measure::DayLow => px(s.day_low()),
            Measure::PrevClose => s.daily_for_today().map(|d| d.prev_close.value()),
            Measure::PrevHigh => s.daily_for_today()?.prev_high.map(|p| p.value()),
            Measure::PrevLow => s.daily_for_today()?.prev_low.map(|p| p.value()),
            Measure::PrevMonthHigh => s.daily_for_today()?.prev_month_high.map(|p| p.value()),
            Measure::PrevMonthLow => s.daily_for_today()?.prev_month_low.map(|p| p.value()),
            Measure::Atr => s.daily_for_today()?.atr.map(|a| a.value()),
            Measure::AvgDailyVolume => s.daily_for_today()?.adv,
            Measure::DailyEma(n) => {
                let d = s.daily_for_today()?;
                let last = s.last_bar()?.close.value();
                let mut xs = d.closes.to_vec();
                xs.push(last);
                ema_last(&xs, n as usize)
            }
            Measure::Vwap => s.vwap().map(|v| v.vwap.value()),
            Measure::VwapBand { upper, k_centi } => {
                let v = s.vwap()?;
                Some(if upper { v.upper(k(k_centi)) } else { v.lower(k(k_centi)) }.value())
            }
            Measure::VwapExt => s.vwap_extended(),
            Measure::SessionOpen => px(s.session_open()),
            Measure::PremarketHigh => px(s.premarket_high()),
            Measure::PremarketLow => px(s.premarket_low()),
            Measure::PremarketVolume => s.premarket_volume(),
            Measure::Time => s.tick_minutes(),
            Measure::Candle { tf_min, ind } => {
                if tf_min as i64 * 60_000 != crate::state::CANDLE_MS {
                    return None;
                }
                let xs = s.candle_closes();
                match ind {
                    CandleInd::Ema(n) => ema_last(&xs, n as usize),
                    CandleInd::Sma(n) => sma_last(&xs, n as usize),
                    CandleInd::Rsi(n) => rsi_last(&xs, n as usize),
                    CandleInd::BbUpper { len, k_centi } => bollinger_last(&xs, len as usize, k(k_centi)).map(|b| b.upper),
                    CandleInd::BbLower { len, k_centi } => bollinger_last(&xs, len as usize, k(k_centi)).map(|b| b.lower),
                }
            }
            Measure::Float => s.reference().float_shares,
            Measure::Imbalance { buy, venue } => {
                let a = s.auction_today()?;
                let (nyse, nasdaq) = if buy {
                    (a.imbalance_buy_nyse, a.imbalance_buy_nasdaq)
                } else {
                    (a.imbalance_sell_nyse, a.imbalance_sell_nasdaq)
                };
                match venue {
                    Venue::Nyse => nyse,
                    Venue::Nasdaq => nasdaq,
                    Venue::Any => match (nyse, nasdaq) {
                        (Some(a), Some(b)) => Some(a + b),
                        (a, b) => a.or(b),
                    },
                }
            }
            Measure::IndBid => px(s.auction_today()?.ind_bid),
            Measure::IndAsk => px(s.auction_today()?.ind_ask),
            Measure::ClearPrice => px(s.auction_today()?.clear_price),
            Measure::NearPrice => px(s.auction_today()?.near_price),
        }
    }
}

fn fmt_k(centi: u16) -> String {
    let v = k(centi);
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

impl fmt::Display for Measure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Measure::Last => write!(f, "last"),
            Measure::DayVolume { extended: false } => write!(f, "day_volume"),
            Measure::DayVolume { extended: true } => write!(f, "day_volume_ext"),
            Measure::High(w) => write!(f, "high({w})"),
            Measure::Low(w) => write!(f, "low({w})"),
            Measure::Volume(w) => write!(f, "volume({w})"),
            Measure::Open(w) => write!(f, "open({w})"),
            Measure::Close(w) => write!(f, "close({w})"),
            Measure::DayHigh => write!(f, "day_high"),
            Measure::DayLow => write!(f, "day_low"),
            Measure::PrevClose => write!(f, "prev_close"),
            Measure::PrevHigh => write!(f, "prev_high"),
            Measure::PrevLow => write!(f, "prev_low"),
            Measure::PrevMonthHigh => write!(f, "prev_month_high"),
            Measure::PrevMonthLow => write!(f, "prev_month_low"),
            Measure::Atr => write!(f, "atr"),
            Measure::AvgDailyVolume => write!(f, "adv"),
            Measure::DailyEma(n) => write!(f, "daily_ema({n})"),
            Measure::Vwap => write!(f, "vwap"),
            Measure::VwapBand { upper: true, k_centi } => write!(f, "vwap_upper({})", fmt_k(*k_centi)),
            Measure::VwapBand { upper: false, k_centi } => write!(f, "vwap_lower({})", fmt_k(*k_centi)),
            Measure::VwapExt => write!(f, "vwap_ext"),
            Measure::SessionOpen => write!(f, "open"),
            Measure::PremarketHigh => write!(f, "pm_high"),
            Measure::PremarketLow => write!(f, "pm_low"),
            Measure::PremarketVolume => write!(f, "pm_volume"),
            Measure::Time => write!(f, "time"),
            Measure::Candle { tf_min, ind } => match ind {
                CandleInd::Ema(n) => write!(f, "ema({tf_min}m, {n})"),
                CandleInd::Sma(n) => write!(f, "sma({tf_min}m, {n})"),
                CandleInd::Rsi(n) => write!(f, "rsi({tf_min}m, {n})"),
                CandleInd::BbUpper { len, k_centi } => write!(f, "bb_upper({tf_min}m, {len}, {})", fmt_k(*k_centi)),
                CandleInd::BbLower { len, k_centi } => write!(f, "bb_lower({tf_min}m, {len}, {})", fmt_k(*k_centi)),
            },
            Measure::Float => write!(f, "float"),
            Measure::Imbalance { buy, venue } => {
                let side = if *buy { "imb_buy" } else { "imb_sell" };
                let v = match venue {
                    Venue::Nyse => "nyse",
                    Venue::Nasdaq => "nasdaq",
                    Venue::Any => "any",
                };
                write!(f, "{side}({v})")
            }
            Measure::IndBid => write!(f, "ind_bid"),
            Measure::IndAsk => write!(f, "ind_ask"),
            Measure::ClearPrice => write!(f, "clear_price"),
            Measure::NearPrice => write!(f, "near_price"),
        }
    }
}

/// A true/false fact about the symbol now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// The newest bar's high beats every high in the preceding `span_ms` (Primus `NewMinuteHigh`).
    NewHigh { span_ms: i64 },
    /// The newest bar's low undercuts every low in the preceding `span_ms`.
    NewLow { span_ms: i64 },
    /// The symbol is halted (Primus `IsHalt`). Unknown unless a halt state was set for today.
    Halted,
}

impl Event {
    pub fn eval(&self, s: &SymbolState) -> Option<bool> {
        match *self {
            Event::NewHigh { span_ms } => s.new_extreme(span_ms, true),
            Event::NewLow { span_ms } => s.new_extreme(span_ms, false),
            Event::Halted => s.halted_today(),
        }
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = |ms: &i64| ms / 60_000;
        match self {
            Event::NewHigh { span_ms } => write!(f, "new_high({}m)", m(span_ms)),
            Event::NewLow { span_ms } => write!(f, "new_low({}m)", m(span_ms)),
            Event::Halted => write!(f, "halted"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tests::{b, minutes, t};
    use crate::state::{AuctionSnapshot, DailyRef, Reference, SymbolState};
    use chrono::NaiveDate;
    use tp_marketdata::bar::Bar;
    use tp_marketdata::price::{Adjusted, Price};
    use tp_marketdata::source::Resolution;
    use tp_marketdata::time::{SessionDate, Ts};

    fn today() -> SessionDate {
        SessionDate(NaiveDate::from_ymd_opt(2026, 6, 15).unwrap())
    }

    #[test]
    fn catalog_reads_state() {
        let s = minutes(10);
        assert_eq!(Measure::Last.eval(&s), Some(18.5));
        assert_eq!(Measure::High(Win::rolling_min(5)).eval(&s), Some(19.0));
        assert_eq!(Measure::Volume(Win::rolling_min_back(5, 1)).eval(&s), Some(1500.0));
        // 10 bars 09:30..09:39, open = close = 9.5 + i: the last 5 minutes are bars 5..9.
        assert_eq!(Measure::Open(Win::rolling_min(5)).eval(&s), Some(14.5));
        assert_eq!(Measure::Close(Win::rolling_min(5)).eval(&s), Some(18.5));
        assert_eq!(Measure::Close(Win::rolling_min_back(5, 1)).eval(&s), Some(13.5));
        assert_eq!(Measure::SessionOpen.eval(&s), Some(9.5));
        assert_eq!(Measure::DayHigh.eval(&s), Some(19.0));
        assert_eq!(Measure::DayLow.eval(&s), Some(9.0));
        assert_eq!(Measure::PremarketHigh.eval(&s), None);
        assert_eq!(Measure::PremarketVolume.eval(&s), Some(0.0));
        // Newest bar 09:39 -> its last print is 09:39:59 -> TickTime 579.
        assert_eq!(Measure::Time.eval(&s), Some(579.0));
        assert!(Measure::VwapExt.eval(&s).is_some());
        let up = Measure::VwapBand { upper: true, k_centi: 200 }.eval(&s).unwrap();
        let lo = Measure::VwapBand { upper: false, k_centi: 200 }.eval(&s).unwrap();
        let v = Measure::Vwap.eval(&s).unwrap();
        assert!(up > v && lo < v && ((up - v) - (v - lo)).abs() < 1e-9);
    }

    #[test]
    fn daily_measures_need_todays_reference() {
        let mut s = minutes(2);
        assert_eq!(Measure::Atr.eval(&s), None);
        s.set_daily(DailyRef {
            atr: Some(Price::new(0.8)),
            prev_high: Some(Price::new(9.5)),
            ..DailyRef::bare(s.session_date().unwrap(), Price::new(9.0))
        });
        assert_eq!(Measure::Atr.eval(&s), Some(0.8));
        assert_eq!(Measure::PrevClose.eval(&s), Some(9.0));
        assert_eq!(Measure::PrevHigh.eval(&s), Some(9.5));
        assert_eq!(Measure::AvgDailyVolume.eval(&s), None, "short history stays unknown");
        assert_eq!(Measure::PrevMonthHigh.eval(&s), None);
    }

    fn daily(y: i32, m: u32, d: u32, h: f64, l: f64, c: f64) -> Bar<Adjusted> {
        let start = Ts::from_ny_local(NaiveDate::from_ymd_opt(y, m, d).unwrap(), chrono::NaiveTime::MIN).unwrap();
        Bar { start, open: Price::new(c), high: Price::new(h), low: Price::new(l), close: Price::new(c), volume: 1e6 }
    }

    #[test]
    fn prior_month_needs_the_whole_month_in_view() {
        // Session 2026-06-15: the prior month is May.
        let hist = [daily(2026, 4, 30, 50.0, 1.0, 10.0), daily(2026, 5, 4, 12.0, 9.0, 11.0), daily(2026, 5, 29, 14.0, 8.0, 13.0), daily(2026, 6, 12, 99.0, 0.5, 20.0)];
        let d = DailyRef::from_history(today(), &hist, 2, 2).unwrap();
        assert_eq!(d.prev_month_high.map(|p| p.value()), Some(14.0), "April's 50 and June's 99 excluded");
        assert_eq!(d.prev_month_low.map(|p| p.value()), Some(8.0));
        assert_eq!(d.prev_high.map(|p| p.value()), Some(99.0));
        let partial = DailyRef::from_history(today(), &hist[1..], 2, 2).unwrap();
        assert_eq!(partial.prev_month_high, None, "history starts inside May: May isn't known whole");
    }

    #[test]
    fn daily_ema_folds_in_todays_last() {
        let mut s = minutes(1); // last 9.5
        let hist = [daily(2026, 6, 10, 1.0, 1.0, 1.0), daily(2026, 6, 11, 2.0, 2.0, 2.0), daily(2026, 6, 12, 3.0, 3.0, 3.0)];
        s.set_daily(DailyRef::from_history(today(), &hist, 2, 2).unwrap());
        // EMA3 over [1, 2, 3, 9.5]: seed 2, then 2 + 0.5 × (9.5 − 2) = 5.75.
        assert_eq!(Measure::DailyEma(3).eval(&s), Some(5.75));
        assert_eq!(Measure::DailyEma(5).eval(&s), None);
    }

    #[test]
    fn candle_indicators_use_rth_five_minute_closes() {
        let mut s = SymbolState::new();
        // Premarket bar: must not enter the candles.
        s.ingest(Resolution::Minute, b(t(9, 0, 0), 500.0, 500.0, 500.0, 500.0, 1.0)).unwrap();
        // 09:30..09:44 closes 1..15 -> candles close at 5, 10, 15 (the last is still forming).
        for i in 0..15u32 {
            let c = (i + 1) as f64;
            s.ingest(Resolution::Minute, b(t(9, 30 + i, 0), c, c, c, c, 1.0)).unwrap();
        }
        assert_eq!(s.candle_closes(), vec![5.0, 10.0, 15.0]);
        let ema = |n| Measure::Candle { tf_min: 5, ind: CandleInd::Ema(n) }.eval(&s);
        assert_eq!(ema(3), Some(10.0), "SMA seed of 5, 10, 15");
        assert_eq!(ema(4), None);
        assert_eq!(Measure::Candle { tf_min: 5, ind: CandleInd::Rsi(2) }.eval(&s), Some(100.0));
        assert_eq!(Measure::Candle { tf_min: 15, ind: CandleInd::Ema(1) }.eval(&s), None, "only 5m kept");
        let bb = Measure::Candle { tf_min: 5, ind: CandleInd::BbLower { len: 3, k_centi: 0 } }.eval(&s);
        assert_eq!(bb, Some(10.0), "k = 0 is the SMA");
    }

    #[test]
    fn reference_and_auction_data() {
        let mut s = minutes(2);
        assert_eq!(Measure::Float.eval(&s), None);
        s.set_reference(Reference { float_shares: Some(12e6), ..Default::default() });
        assert_eq!(Measure::Float.eval(&s), Some(12e6));

        assert_eq!(Measure::IndBid.eval(&s), None);
        assert_eq!(Event::Halted.eval(&s), None);
        let mut a = AuctionSnapshot::empty(today());
        a.imbalance_buy_nyse = Some(5000.0);
        a.ind_bid = Some(Price::new(10.25));
        s.set_auction(a.clone());
        s.set_halted(today(), true);
        assert_eq!(Measure::IndBid.eval(&s), Some(10.25));
        assert_eq!(Measure::ClearPrice.eval(&s), None, "unpublished is unknown, not 0");
        assert_eq!(Measure::Imbalance { buy: true, venue: Venue::Any }.eval(&s), Some(5000.0));
        assert_eq!(Measure::Imbalance { buy: true, venue: Venue::Nasdaq }.eval(&s), None);
        assert_eq!(Event::Halted.eval(&s), Some(true));

        let yesterday = SessionDate(NaiveDate::from_ymd_opt(2026, 6, 12).unwrap());
        s.set_auction(AuctionSnapshot { session: yesterday, ..a });
        s.set_halted(yesterday, true);
        assert_eq!(Measure::IndBid.eval(&s), None, "another session's auction is refused");
        assert_eq!(Event::Halted.eval(&s), None);
    }

    #[test]
    fn labels_are_readable() {
        assert_eq!(Measure::High(Win::rolling_min(15)).to_string(), "high(15m)");
        assert_eq!(Measure::Volume(Win::rolling_min_back(5, 1)).to_string(), "volume(5m, 5m back)");
        assert_eq!(Measure::Low(Win::clock_min(30, 0)).to_string(), "low(30m clock)");
        assert_eq!(Measure::Candle { tf_min: 5, ind: CandleInd::BbUpper { len: 20, k_centi: 250 } }.to_string(), "bb_upper(5m, 20, 2.5)");
        assert_eq!(Measure::VwapBand { upper: false, k_centi: 200 }.to_string(), "vwap_lower(2)");
        assert_eq!(Measure::Imbalance { buy: false, venue: Venue::Nyse }.to_string(), "imb_sell(nyse)");
    }
}
