//! Everything the scanner knows about one symbol: recent bars in two tiers, running session
//! statistics, and the daily reference values (prior close, ATR, ADV).
//!
//! ## Two tiers
//!
//! | tier   | resolution | keeps          | used for                       |
//! |--------|------------|----------------|--------------------------------|
//! | fine   | 1 second   | [`FINE_SPAN_MS`]   | windows that fit inside it     |
//! | coarse | 1 minute   | [`COARSE_SPAN_MS`] | everything longer              |
//!
//! A symbol is fed at ONE resolution for its whole life. Fed 1-second bars, it fills the fine tier
//! and rolls them up into the coarse tier. Fed 1-minute bars (the historical corpus), it fills the
//! coarse tier directly and the fine tier stays empty. Mixing resolutions for one symbol would count
//! the same trades twice, so it is refused.
//!
//! ## Unknown is not zero
//!
//! Queries return `None` when the answer is not knowable from what has been seen: a window
//! reaching back before this symbol's first bar, a daily reference for a different session, no
//! RTH bar yet. `None` becomes `Unknown` in the evaluator. A window that IS covered but saw no
//! trades has volume 0, which is a known answer.
//!
//! ## Beyond bars
//!
//! - **5-minute candles** ([`CANDLE_MS`]), clock-aligned and built from RTH bars only, kept across
//!   sessions so a 5-minute EMA20 is warm at the open, the way a chart's is. ⚠ They are
//!   unadjusted: a split between sessions breaks the series until it refills.
//! - **Reference data** ([`Reference`]): float. Slow-changing, set from outside.
//! - **Auction data** ([`AuctionSnapshot`]) and the halt flag: set from outside for one session,
//!   refused on any other, like [`DailyRef`].

use chrono::{Datelike, NaiveDate, Timelike};
use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use tp_marketdata::bar::{Aggregator, Bar};
use tp_marketdata::indicators::atr::Atr;
use tp_marketdata::indicators::ma::Sma;
use tp_marketdata::indicators::vwap::{SessionVwap, VwapBands};
use tp_marketdata::price::{AdjFactor, Adjusted, Price, Unadjusted};
use tp_marketdata::source::Resolution;
use tp_marketdata::time::{Phase, SessionDate, Ts};

/// How much 1-second history is kept.
pub const FINE_SPAN_MS: i64 = 5 * 60_000;
/// How much 1-minute history is kept: the longest window (120 min) plus room for offset windows.
pub const COARSE_SPAN_MS: i64 = 240 * 60_000;
/// The candle width intraday indicators (EMA, SMA, RSI, Bollinger) are computed on. The only one
/// the Primus filters use.
pub const CANDLE_MS: i64 = 5 * 60_000;
/// How many completed candles are kept: enough that a 20-period EMA or Wilder average started from
/// the kept series matches one started from the beginning to ~1e-8.
pub const CANDLES_KEPT: usize = 250;

/// Bars at one resolution, keeping those that start within `span_ms` of the newest.
#[derive(Debug, Clone)]
struct Ring {
    res_ms: i64,
    span_ms: i64,
    bars: VecDeque<Bar<Unadjusted>>,
}

impl Ring {
    fn new(res_ms: i64, span_ms: i64) -> Self {
        Ring { res_ms, span_ms, bars: VecDeque::new() }
    }

    fn push(&mut self, b: Bar<Unadjusted>) {
        let cut = b.start.millis() - self.span_ms;
        self.bars.push_back(b);
        while self.bars.front().is_some_and(|f| f.start.millis() <= cut) {
            self.bars.pop_front();
        }
    }

    /// The earliest instant this ring can still answer for: the newest start minus its span.
    fn coverage_from(&self) -> Option<i64> {
        self.bars.back().map(|b| b.start.millis() - self.span_ms)
    }
}

/// Where a window sits in time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Align {
    /// The trailing `span` ending `offset` before now. **The default, and the right one**: a
    /// "15-minute high" at 10:07 covers 09:52–10:07.
    Rolling,
    /// A clock period: `[k·span, (k+1)·span)` on the New York clock. Offset `0` is the period in
    /// progress, offset `span` the one before it. The offset must be a whole number of spans and
    /// the span must divide one hour. At 10:07 the current 15-minute clock window is 10:00–10:07,
    /// only 7 minutes of data. That trap is why this has to be asked for by name.
    Clock,
}

/// A window specification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Win {
    pub span_ms: i64,
    pub offset_ms: i64,
    pub align: Align,
}

impl Win {
    /// The trailing `minutes` ending now.
    pub const fn rolling_min(minutes: i64) -> Win {
        Win { span_ms: minutes * 60_000, offset_ms: 0, align: Align::Rolling }
    }

    /// The `minutes` window immediately before [`Win::rolling_min`] of the same size (Primus
    /// `P1`), or `n` windows back (`P{n}`).
    pub const fn rolling_min_back(minutes: i64, n: i64) -> Win {
        Win { span_ms: minutes * 60_000, offset_ms: n * minutes * 60_000, align: Align::Rolling }
    }

    pub const fn clock_min(minutes: i64, periods_back: i64) -> Win {
        Win { span_ms: minutes * 60_000, offset_ms: periods_back * minutes * 60_000, align: Align::Clock }
    }
}

impl fmt::Display for Win {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dur = |ms: i64| {
            if ms % 60_000 == 0 {
                format!("{}m", ms / 60_000)
            } else {
                format!("{}s", ms / 1000)
            }
        };
        let kind = match self.align {
            Align::Rolling => "",
            Align::Clock => " clock",
        };
        if self.offset_ms == 0 {
            write!(f, "{}{kind}", dur(self.span_ms))
        } else {
            write!(f, "{}{kind}, {} back", dur(self.span_ms), dur(self.offset_ms))
        }
    }
}

/// What a window saw. Prices are `None` when it's covered but had no bars. `open` is the first
/// bar's open and `close` the last bar's close (bar order is time order).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowAgg {
    pub high: Option<Price<Unadjusted>>,
    pub low: Option<Price<Unadjusted>>,
    pub open: Option<Price<Unadjusted>>,
    pub close: Option<Price<Unadjusted>>,
    pub volume: f64,
    pub bars: usize,
}

/// Running statistics for the session in progress.
#[derive(Debug, Clone)]
struct SessionStats {
    date: SessionDate,
    open: Option<Price<Unadjusted>>,
    rth_high: Option<Price<Unadjusted>>,
    rth_low: Option<Price<Unadjusted>>,
    vol_all: f64,
    vol_rth: f64,
    /// Extended-hours VWAP accumulators: every phase from the session's first print.
    ext_tpv: f64,
    ext_vol: f64,
    pre_high: Option<Price<Unadjusted>>,
    pre_low: Option<Price<Unadjusted>>,
    pre_vol: f64,
    vwap: SessionVwap,
    last_vwap: Option<VwapBands<Unadjusted>>,
}

impl SessionStats {
    fn new(date: SessionDate) -> Self {
        SessionStats {
            date,
            open: None,
            rth_high: None,
            rth_low: None,
            vol_all: 0.0,
            vol_rth: 0.0,
            ext_tpv: 0.0,
            ext_vol: 0.0,
            pre_high: None,
            pre_low: None,
            pre_vol: 0.0,
            vwap: SessionVwap::new(),
            last_vwap: None,
        }
    }

    fn update(&mut self, b: &Bar<Unadjusted>) {
        let phase = b.phase();
        self.vol_all += b.volume;
        if b.volume > 0.0 {
            self.ext_tpv += b.typical_price().value() * b.volume;
            self.ext_vol += b.volume;
        }
        match phase {
            Phase::Rth => {
                self.vol_rth += b.volume;
                self.open.get_or_insert(b.open);
                self.rth_high = Some(self.rth_high.map_or(b.high, |h| h.max(b.high)));
                self.rth_low = Some(self.rth_low.map_or(b.low, |l| l.min(b.low)));
            }
            Phase::Pre => {
                self.pre_vol += b.volume;
                self.pre_high = Some(self.pre_high.map_or(b.high, |h| h.max(b.high)));
                self.pre_low = Some(self.pre_low.map_or(b.low, |l| l.min(b.low)));
            }
            Phase::Post | Phase::Closed => {}
        }
        if let Some(v) = self.vwap.update(b) {
            self.last_vwap = Some(v);
        }
    }
}

/// Reference values computed from completed daily bars **before** `session`.
///
/// Prices are adjusted onto `session`'s basis. That is what makes them comparable with the live
/// prints of `session`, and ONLY with those. Used on any other session they are refused (see
/// [`SymbolState::daily_for_today`]). A split overnight would otherwise put yesterday's
/// reference 4× off today's prices: the incumbent's $53-ATR bug.
#[derive(Debug, Clone, PartialEq)]
pub struct DailyRef {
    pub session: SessionDate,
    pub prev_close: Price<Adjusted>,
    /// The prior session's high and low.
    pub prev_high: Option<Price<Adjusted>>,
    pub prev_low: Option<Price<Adjusted>>,
    /// High and low of the prior **completed calendar month** (Primus `MonthHigh_I(P1)`). `None`
    /// unless the history reaches back before that month's first day, so a month seen only in
    /// part is never passed off as the whole month.
    pub prev_month_high: Option<Price<Adjusted>>,
    pub prev_month_low: Option<Price<Adjusted>>,
    pub atr: Option<Price<Adjusted>>,
    pub atr_len: usize,
    pub adv: Option<f64>,
    pub adv_len: usize,
    /// The daily closes the reference was built from, oldest first, for daily moving averages.
    pub closes: Arc<[f64]>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DailyRefError {
    NoHistory,
    /// A bar dated on or after `session` was passed in: using it would look ahead.
    Lookahead { bar: SessionDate, session: SessionDate },
    OutOfOrder,
}

impl fmt::Display for DailyRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DailyRefError::NoHistory => write!(f, "no daily history"),
            DailyRefError::Lookahead { bar, session } => {
                write!(f, "daily bar {bar} is not before session {session} (lookahead)")
            }
            DailyRefError::OutOfOrder => write!(f, "daily bars are not in date order"),
        }
    }
}

impl std::error::Error for DailyRefError {}

impl DailyRef {
    /// Only a prior close; every other value unknown. For tests and hand-built references.
    pub fn bare(session: SessionDate, prev_close: Price<Adjusted>) -> DailyRef {
        DailyRef {
            session,
            prev_close,
            prev_high: None,
            prev_low: None,
            prev_month_high: None,
            prev_month_low: None,
            atr: None,
            atr_len: 14,
            adv: None,
            adv_len: 20,
            closes: Arc::from(Vec::new()),
        }
    }

    /// Build from completed daily bars, oldest first, all dated before `session` and all adjusted
    /// to `session`'s basis. ATR is Wilder's over `atr_len` bars; ADV is the simple mean volume of
    /// the last `adv_len` bars. Either is `None` if the history is too short.
    pub fn from_history(
        session: SessionDate,
        history: &[Bar<Adjusted>],
        atr_len: usize,
        adv_len: usize,
    ) -> Result<DailyRef, DailyRefError> {
        let last = history.last().ok_or(DailyRefError::NoHistory)?;
        let mut prev: Option<SessionDate> = None;
        let mut atr = Atr::new(atr_len);
        let mut adv = Sma::<f64>::new(adv_len);
        let (mut atr_v, mut adv_v) = (None, None);
        let this_month = NaiveDate::from_ymd_opt(session.0.year(), session.0.month(), 1).expect("day 1 exists");
        let prior_month = this_month.checked_sub_months(chrono::Months::new(1)).expect("in range");
        let mut month: Option<(Price<Adjusted>, Price<Adjusted>)> = None;
        let mut seen_before_prior_month = false;
        let mut closes = Vec::with_capacity(history.len());
        for b in history {
            let d = b.session_date();
            if d >= session {
                return Err(DailyRefError::Lookahead { bar: d, session });
            }
            if prev.is_some_and(|p| d <= p) {
                return Err(DailyRefError::OutOfOrder);
            }
            prev = Some(d);
            atr_v = atr.update(b);
            adv_v = adv.update(b.volume);
            closes.push(b.close.value());
            if d.0 < prior_month {
                seen_before_prior_month = true;
            } else if d.0 < this_month {
                month = Some(month.map_or((b.high, b.low), |(h, l)| (h.max(b.high), l.min(b.low))));
            }
        }
        let month = month.filter(|_| seen_before_prior_month);
        Ok(DailyRef {
            session,
            prev_close: last.close,
            prev_high: Some(last.high),
            prev_low: Some(last.low),
            prev_month_high: month.map(|m| m.0),
            prev_month_low: month.map(|m| m.1),
            atr: atr_v,
            atr_len,
            adv: adv_v,
            adv_len,
            closes: Arc::from(closes),
        })
    }
}

/// Slow-changing facts about a symbol, set from outside.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reference {
    /// Float in **shares** (Primus `FloatShares` is in millions; the translation multiplies).
    pub float_shares: Option<f64>,
}

/// Auction and imbalance data for one session, set from outside (no feed is wired yet).
///
/// Every field is optional: "not published" is unknown, never 0. That is exactly the guard the
/// user's `Clear_ATR` / `Near_ATR` fields lack in Primus, where an unpublished clearing price
/// reads 0 and becomes a large false move.
#[derive(Debug, Clone, PartialEq)]
pub struct AuctionSnapshot {
    pub session: SessionDate,
    pub imbalance_buy_nyse: Option<f64>,
    pub imbalance_sell_nyse: Option<f64>,
    pub imbalance_buy_nasdaq: Option<f64>,
    pub imbalance_sell_nasdaq: Option<f64>,
    pub ind_bid: Option<Price<Unadjusted>>,
    pub ind_ask: Option<Price<Unadjusted>>,
    pub clear_price: Option<Price<Unadjusted>>,
    pub near_price: Option<Price<Unadjusted>>,
}

impl AuctionSnapshot {
    pub fn empty(session: SessionDate) -> Self {
        AuctionSnapshot {
            session,
            imbalance_buy_nyse: None,
            imbalance_sell_nyse: None,
            imbalance_buy_nasdaq: None,
            imbalance_sell_nasdaq: None,
            ind_bid: None,
            ind_ask: None,
            clear_price: None,
            near_price: None,
        }
    }
}

/// Clock-aligned candles from RTH bars, kept across sessions.
#[derive(Debug, Clone)]
struct Candles {
    agg: Aggregator<Unadjusted>,
    closes: VecDeque<f64>,
}

impl Candles {
    fn new() -> Self {
        Candles { agg: Aggregator::new(CANDLE_MS).expect("5 minutes divides an hour"), closes: VecDeque::new() }
    }

    fn push(&mut self, b: Bar<Unadjusted>) {
        if b.phase() != Phase::Rth {
            return;
        }
        // Order was checked by the caller, so the aggregator can't refuse this bar.
        if let Ok(Some(done)) = self.agg.push(b) {
            self.closes.push_back(done.close.value());
            if self.closes.len() > CANDLES_KEPT {
                self.closes.pop_front();
            }
        }
    }

    /// Completed closes, oldest first, then the forming candle's close (Primus `CURRENT`).
    fn series(&self) -> Vec<f64> {
        let mut v: Vec<f64> = self.closes.iter().copied().collect();
        if let Some(c) = self.agg.current() {
            v.push(c.close.value());
        }
        v
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StateError {
    OutOfOrder { prev: Ts, got: Ts },
    MixedResolution { first: Resolution, got: Resolution },
    UnsupportedResolution(Resolution),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateError::OutOfOrder { prev, got } => {
                write!(f, "bar at {}ms arrived after {}ms", got.millis(), prev.millis())
            }
            StateError::MixedResolution { first, got } => {
                write!(f, "symbol is fed {first:?} bars; refusing {got:?} (would double count)")
            }
            StateError::UnsupportedResolution(r) => write!(f, "{r:?} bars are not intraday input"),
        }
    }
}

impl std::error::Error for StateError {}

/// One symbol's scanner state.
#[derive(Debug, Clone)]
pub struct SymbolState {
    res: Option<Resolution>,
    fine: Ring,
    coarse: Ring,
    agg: Aggregator<Unadjusted>,
    first_start: Option<Ts>,
    /// Set by the engine: the feed has been delivering since this instant, so a window reaching
    /// back to it is covered even if this symbol hadn't printed yet (it had no trades).
    quiet_since: Option<Ts>,
    last: Option<Bar<Unadjusted>>,
    now: Option<Ts>,
    session: Option<SessionStats>,
    daily: Option<DailyRef>,
    candles: Candles,
    reference: Reference,
    auction: Option<AuctionSnapshot>,
    halted: Option<(SessionDate, bool)>,
}

impl Default for SymbolState {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolState {
    pub fn new() -> Self {
        SymbolState {
            res: None,
            fine: Ring::new(1_000, FINE_SPAN_MS),
            coarse: Ring::new(60_000, COARSE_SPAN_MS),
            agg: Aggregator::new(60_000).expect("one minute divides an hour"),
            first_start: None,
            quiet_since: None,
            last: None,
            now: None,
            session: None,
            daily: None,
            candles: Candles::new(),
            reference: Reference::default(),
            auction: None,
            halted: None,
        }
    }

    pub fn set_daily(&mut self, d: DailyRef) {
        self.daily = Some(d);
    }

    /// Declare that the feed has carried every trade since `t`, so this symbol having no bars
    /// before its first print means it was quiet, not unseen. Windows reaching back as far as `t`
    /// then count as covered. Only the first call has any effect.
    pub fn assume_quiet_since(&mut self, t: Ts) {
        self.quiet_since.get_or_insert(t);
    }

    /// The earliest instant this state can speak for.
    fn seen_from(&self) -> Option<i64> {
        let first = self.first_start?.millis();
        Some(self.quiet_since.map_or(first, |q| q.millis().min(first)))
    }

    pub fn set_reference(&mut self, r: Reference) {
        self.reference = r;
    }

    pub fn set_auction(&mut self, a: AuctionSnapshot) {
        self.auction = Some(a);
    }

    pub fn set_halted(&mut self, session: SessionDate, halted: bool) {
        self.halted = Some((session, halted));
    }

    pub fn reference(&self) -> &Reference {
        &self.reference
    }

    /// Today's auction data, only if it was set for the session in progress.
    pub fn auction_today(&self) -> Option<&AuctionSnapshot> {
        let today = self.session_date()?;
        self.auction.as_ref().filter(|a| a.session == today)
    }

    /// Whether the symbol is halted, only if a halt state was set for the session in progress.
    pub fn halted_today(&self) -> Option<bool> {
        let today = self.session_date()?;
        self.halted.filter(|h| h.0 == today).map(|h| h.1)
    }

    /// Add a completed intraday bar. `now` becomes the bar's end.
    pub fn ingest(&mut self, res: Resolution, bar: Bar<Unadjusted>) -> Result<(), StateError> {
        let res_ms = res.millis().ok_or(StateError::UnsupportedResolution(res))?;
        match self.res {
            Some(first) if first != res => return Err(StateError::MixedResolution { first, got: res }),
            _ => {}
        }
        if let Some(prev) = self.last.map(|b| b.start) {
            if bar.start <= prev {
                return Err(StateError::OutOfOrder { prev, got: bar.start });
            }
        }
        self.res = Some(res);
        self.first_start.get_or_insert(bar.start);
        let date = bar.session_date();
        if self.session.as_ref().is_none_or(|s| s.date != date) {
            self.session = Some(SessionStats::new(date));
        }
        if let Some(s) = self.session.as_mut() {
            s.update(&bar);
        }
        self.candles.push(bar);
        match res {
            Resolution::Second => {
                self.fine.push(bar);
                if let Some(done) = self.agg.push(bar).map_err(|_| StateError::OutOfOrder {
                    prev: self.last.map_or(bar.start, |b| b.start),
                    got: bar.start,
                })? {
                    self.coarse.push(done);
                }
            }
            Resolution::Minute => self.coarse.push(bar),
            Resolution::Day => unreachable!("rejected above"),
        }
        self.last = Some(bar);
        self.now = bar.start.plus_millis(res_ms);
        Ok(())
    }

    /// The end of the newest bar: the instant everything here is "as of".
    pub fn now(&self) -> Option<Ts> {
        self.now
    }

    pub fn last_bar(&self) -> Option<&Bar<Unadjusted>> {
        self.last.as_ref()
    }

    pub fn session_date(&self) -> Option<SessionDate> {
        self.session.as_ref().map(|s| s.date)
    }

    /// The daily reference, only if it was built for the session in progress.
    pub fn daily_for_today(&self) -> Option<&DailyRef> {
        let today = self.session_date()?;
        self.daily.as_ref().filter(|d| d.session == today)
    }

    /// A print from today's session, restated on the adjusted basis. By definition today's
    /// prints ARE the current basis, so the factor is 1. The check that makes this safe is that a
    /// [`DailyRef`] for today exists, meaning the adjusted history was brought up to date through
    /// today. Without it this returns `None`.
    pub fn today_on_adjusted_basis(&self, p: Price<Unadjusted>) -> Option<Price<Adjusted>> {
        self.daily_for_today().map(|_| p.adjust(AdjFactor::IDENTITY))
    }

    pub fn session_open(&self) -> Option<Price<Unadjusted>> {
        self.session.as_ref()?.open
    }

    /// The RTH high so far today, including the newest bar (Primus `DayHigh(CURRENT, NO)`).
    pub fn day_high(&self) -> Option<Price<Unadjusted>> {
        self.session.as_ref()?.rth_high
    }

    pub fn day_low(&self) -> Option<Price<Unadjusted>> {
        self.session.as_ref()?.rth_low
    }

    /// VWAP over every phase from the session's first print (Primus `VWAP(CURRENT, YES, ...)`).
    /// Unknown before any volume.
    pub fn vwap_extended(&self) -> Option<f64> {
        let s = self.session.as_ref()?;
        (s.ext_vol > 0.0).then(|| s.ext_tpv / s.ext_vol)
    }

    /// Minutes since New York midnight of the newest print (Primus `TickTime`: 570 = 09:30). The
    /// newest print is the last millisecond of the newest bar, so the 09:30 minute bar reads 570.
    pub fn tick_minutes(&self) -> Option<f64> {
        let t = Ts::from_millis(self.now?.millis() - 1)?.ny().time();
        Some((t.hour() * 60 + t.minute()) as f64)
    }

    /// 5-minute candle closes (see [`CANDLE_MS`]), oldest first, ending with the forming candle.
    pub fn candle_closes(&self) -> Vec<f64> {
        self.candles.series()
    }


    /// Session volume so far: all phases, or RTH only.
    pub fn day_volume(&self, include_extended: bool) -> Option<f64> {
        let s = self.session.as_ref()?;
        Some(if include_extended { s.vol_all } else { s.vol_rth })
    }

    pub fn premarket_high(&self) -> Option<Price<Unadjusted>> {
        self.session.as_ref()?.pre_high
    }

    pub fn premarket_low(&self) -> Option<Price<Unadjusted>> {
        self.session.as_ref()?.pre_low
    }

    pub fn premarket_volume(&self) -> Option<f64> {
        self.session.as_ref().map(|s| s.pre_vol)
    }

    /// The session VWAP bands as of the newest bar (RTH-anchored; see `tp_marketdata`'s vwap).
    pub fn vwap(&self) -> Option<VwapBands<Unadjusted>> {
        self.session.as_ref()?.last_vwap
    }

    /// Aggregate over a window ending relative to [`SymbolState::now`].
    ///
    /// **Rolling** `(a, b]` with `b = now − offset` and `a = b − span` includes every bar that
    /// overlaps it (`end > a` and `start < b`), so it is accurate to one bar at each edge.
    /// **Clock** includes the bars that start inside the clock period.
    ///
    /// Tier: the fine tier answers when the symbol is fed seconds and `span + offset` fits in
    /// [`FINE_SPAN_MS`]; otherwise the coarse tier does, plus the minute still in progress.
    ///
    /// `None` when the window reaches before what this state can see (before the first bar, or
    /// past the ring's retention), or for an invalid clock window.
    pub fn window(&self, w: Win) -> Option<WindowAgg> {
        let now = self.now?.millis();
        let (a, b) = match w.align {
            Align::Rolling => {
                let b = now - w.offset_ms;
                (b - w.span_ms, b)
            }
            Align::Clock => {
                if w.span_ms <= 0 || 3_600_000 % w.span_ms != 0 || w.offset_ms % w.span_ms != 0 {
                    return None;
                }
                // The period containing the instant just before `now` (the newest bar's last ms).
                let cur = (now - 1).div_euclid(w.span_ms) * w.span_ms;
                let s = cur - w.offset_ms;
                (s, s + w.span_ms)
            }
        };
        let use_fine = self.res == Some(Resolution::Second) && w.span_ms + w.offset_ms <= FINE_SPAN_MS;
        let ring = if use_fine { &self.fine } else { &self.coarse };
        let first = self.seen_from()?;
        let coverage = ring.coverage_from().map_or(first, |c| c.max(first));
        if a < coverage {
            return None;
        }
        let partial = (!use_fine && self.res == Some(Resolution::Second)).then(|| self.agg.current()).flatten();
        let mut agg = WindowAgg { high: None, low: None, open: None, close: None, volume: 0.0, bars: 0 };
        let rolling = w.align == Align::Rolling;
        for bar in ring.bars.iter().chain(partial) {
            let s = bar.start.millis();
            let inside = if rolling { s + ring.res_ms > a && s < b } else { s >= a && s < b };
            if inside {
                agg.high = Some(agg.high.map_or(bar.high, |h| h.max(bar.high)));
                agg.low = Some(agg.low.map_or(bar.low, |l| l.min(bar.low)));
                agg.open = agg.open.or(Some(bar.open));
                agg.close = Some(bar.close);
                agg.volume += bar.volume;
                agg.bars += 1;
            }
        }
        Some(agg)
    }

    /// Did the newest bar make a new high over the preceding `span_ms`? Compares the newest
    /// bar's high with the highest high of the bars before it in `(start − span, start)`.
    /// `None` if that preceding window isn't covered. A covered window with no bars counts as a
    /// new high.
    pub fn new_extreme(&self, span_ms: i64, high: bool) -> Option<bool> {
        let last = self.last?;
        let first = self.seen_from()?;
        let b = last.start.millis();
        let a = b - span_ms;
        if a < first {
            return None;
        }
        let use_fine = self.res == Some(Resolution::Second) && span_ms <= FINE_SPAN_MS;
        let ring = if use_fine { &self.fine } else { &self.coarse };
        if ring.coverage_from().is_some_and(|c| a < c) {
            return None;
        }
        let mut ext: Option<Price<Unadjusted>> = None;
        for bar in ring.bars.iter() {
            let s = bar.start.millis();
            if s + ring.res_ms > a && s < b && bar.start != last.start {
                let v = if high { bar.high } else { bar.low };
                ext = Some(match ext {
                    None => v,
                    Some(e) if high => e.max(v),
                    Some(e) => e.min(v),
                });
            }
        }
        Some(match ext {
            None => true,
            Some(e) if high => last.high > e,
            Some(e) => last.low < e,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tp_marketdata::price::Price;
    use tp_marketdata::time::Ts;

    pub(crate) fn t(h: u32, m: u32, s: u32) -> Ts {
        use chrono::{NaiveDate, NaiveTime};
        Ts::from_ny_local(NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(), NaiveTime::from_hms_opt(h, m, s).unwrap())
            .unwrap()
    }

    pub(crate) fn b(start: Ts, o: f64, h: f64, l: f64, c: f64, v: f64) -> Bar<Unadjusted> {
        Bar { start, open: Price::new(o), high: Price::new(h), low: Price::new(l), close: Price::new(c), volume: v }
    }

    /// Minute bars 09:30.. with high = 10 + i, low = 9 + i, volume = 100·(i+1).
    pub(crate) fn minutes(n: u32) -> SymbolState {
        let mut s = SymbolState::new();
        for i in 0..n {
            let x = i as f64;
            s.ingest(Resolution::Minute, b(t(9, 30 + i, 0), 9.5 + x, 10.0 + x, 9.0 + x, 9.5 + x, 100.0 * (x + 1.0)))
                .unwrap();
        }
        s
    }

    #[test]
    fn rolling_window_and_its_predecessor() {
        let s = minutes(10); // bars 09:30..09:39, now = 09:40
        let cur = s.window(Win::rolling_min(5)).unwrap(); // 09:35..09:39
        assert_eq!((cur.bars, cur.high.unwrap().value(), cur.low.unwrap().value()), (5, 19.0, 14.0));
        let p1 = s.window(Win::rolling_min_back(5, 1)).unwrap(); // 09:30..09:34
        assert_eq!((p1.bars, p1.high.unwrap().value()), (5, 14.0));
        assert_eq!(p1.volume, 100.0 + 200.0 + 300.0 + 400.0 + 500.0);
    }

    #[test]
    fn windows_before_the_first_bar_are_unknown_not_empty() {
        let s = minutes(10);
        assert!(s.window(Win::rolling_min(10)).is_some());
        assert!(s.window(Win::rolling_min(11)).is_none());
        assert!(s.window(Win::rolling_min_back(5, 2)).is_none());
    }

    #[test]
    fn a_feed_start_covers_a_symbol_that_was_quiet_until_now() {
        let mut s = SymbolState::new();
        s.assume_quiet_since(t(4, 0, 0));
        s.ingest(Resolution::Minute, b(t(9, 30, 0), 10.0, 11.0, 9.0, 10.5, 500.0)).unwrap();
        let w = s.window(Win::rolling_min(120)).expect("covered back to 04:00");
        assert_eq!((w.volume, w.bars), (500.0, 1));
        assert_eq!(s.new_extreme(15 * 60_000, true), Some(true));
        assert!(s.window(Win::rolling_min(400)).is_none(), "before the feed started is still unknown");
        assert!(minutes(1).window(Win::rolling_min(120)).is_none(), "without it, unknown");
    }

    #[test]
    fn clock_window_is_only_the_period_in_progress() {

        let s = minutes(10); // now 09:40 -> current 15m clock period is 09:30-09:45
        let w = s.window(Win::clock_min(15, 0)).unwrap();
        assert_eq!(w.bars, 10);
        let s = minutes(17); // now 09:47 -> period 09:45-10:00 holds only 2 bars
        assert_eq!(s.window(Win::clock_min(15, 0)).unwrap().bars, 2);
        assert!(s.window(Win::clock_min(7, 0)).is_none(), "7 does not divide an hour");
    }

    #[test]
    fn quiet_covered_window_has_zero_volume() {
        let mut s = minutes(3);
        s.ingest(Resolution::Minute, b(t(9, 50, 0), 1.0, 1.0, 1.0, 1.0, 5.0)).unwrap();
        // now 09:51; 09:41..09:49 had no bars.
        let w = s.window(Win::rolling_min_back(5, 1)).unwrap(); // (09:41, 09:46]
        assert_eq!((w.bars, w.volume, w.high), (0, 0.0, None));
    }

    #[test]
    fn seconds_feed_both_tiers_and_long_windows_include_the_current_minute() {
        let mut s = SymbolState::new();
        let mut start = t(9, 30, 0);
        for i in 0..150 {
            s.ingest(Resolution::Second, b(start, 10.0, 10.0 + i as f64 * 0.01, 10.0, 10.0, 1.0)).unwrap();
            start = start.plus_millis(1_000).unwrap();
        }
        // 150 s of bars (09:30:00..09:32:29): now 09:32:30.
        let short = s.window(Win { span_ms: 60_000, offset_ms: 0, align: Align::Rolling }).unwrap();
        assert_eq!((short.bars, short.volume), (60, 60.0), "fine tier, exact to the second");
        let all = s.window(Win { span_ms: 150_000, offset_ms: 0, align: Align::Rolling }).unwrap();
        assert_eq!(all.volume, 150.0);
        assert!(s.window(Win { span_ms: 60_000, offset_ms: 120_000, align: Align::Rolling }).is_none());
        // Run to 400 s so a 6-minute window has to come from the coarse tier.
        for _ in 150..400 {
            s.ingest(Resolution::Second, b(start, 10.0, 10.0, 10.0, 10.0, 1.0)).unwrap();
            start = start.plus_millis(1_000).unwrap();
        }
        // now 09:36:40; coarse holds 09:30..09:35 complete, 09:36 is 40 s in progress.
        let six = s.window(Win::rolling_min(6)).unwrap();
        assert_eq!((six.bars, six.volume), (7, 400.0), "6 complete minutes + the partial one");
    }

    #[test]
    fn mixed_resolution_and_disorder_are_refused() {
        let mut s = minutes(2);
        assert!(matches!(
            s.ingest(Resolution::Second, b(t(9, 40, 0), 1.0, 1.0, 1.0, 1.0, 1.0)),
            Err(StateError::MixedResolution { .. })
        ));
        assert!(matches!(
            s.ingest(Resolution::Minute, b(t(9, 30, 0), 1.0, 1.0, 1.0, 1.0, 1.0)),
            Err(StateError::OutOfOrder { .. })
        ));
    }

    #[test]
    fn session_stats_split_premarket_from_rth() {
        let mut s = SymbolState::new();
        s.ingest(Resolution::Minute, b(t(8, 0, 0), 5.0, 6.0, 4.5, 5.5, 1000.0)).unwrap();
        s.ingest(Resolution::Minute, b(t(9, 0, 0), 5.5, 7.0, 5.0, 6.5, 500.0)).unwrap();
        s.ingest(Resolution::Minute, b(t(9, 30, 0), 6.6, 6.8, 6.4, 6.7, 200.0)).unwrap();
        assert_eq!(s.premarket_high().unwrap().value(), 7.0);
        assert_eq!(s.premarket_low().unwrap().value(), 4.5);
        assert_eq!(s.premarket_volume(), Some(1500.0));
        assert_eq!(s.day_volume(false), Some(200.0));
        assert_eq!(s.day_volume(true), Some(1700.0));
        assert_eq!(s.session_open().unwrap().value(), 6.6);
        assert!(s.vwap().is_some());
    }

    #[test]
    fn new_high_compares_against_the_bars_before() {
        let s = minutes(6); // highs rise every bar
        assert_eq!(s.new_extreme(5 * 60_000, true), Some(true));
        assert_eq!(s.new_extreme(5 * 60_000, false), Some(false));
        let mut s = minutes(6);
        s.ingest(Resolution::Minute, b(t(9, 36, 0), 12.0, 12.0, 11.0, 11.5, 1.0)).unwrap();
        assert_eq!(s.new_extreme(5 * 60_000, true), Some(false), "12 < prior high 15");
        assert_eq!(minutes(3).new_extreme(5 * 60_000, true), None, "window before first bar");
    }

    #[test]
    fn daily_ref_is_refused_on_any_other_session() {
        use chrono::NaiveDate;
        let today = SessionDate(NaiveDate::from_ymd_opt(2026, 6, 15).unwrap());
        let yesterday = SessionDate(NaiveDate::from_ymd_opt(2026, 6, 12).unwrap());
        let mut s = minutes(1);
        let r = DailyRef::bare(yesterday, Price::new(1.0));
        s.set_daily(r.clone());
        assert!(s.daily_for_today().is_none());
        assert!(s.today_on_adjusted_basis(Price::new(1.0)).is_none());
        s.set_daily(DailyRef { session: today, ..r });
        assert!(s.today_on_adjusted_basis(Price::new(1.0)).is_some());
    }

    #[test]
    fn daily_ref_refuses_lookahead_and_disorder() {
        use chrono::{NaiveDate, NaiveTime};
        let day = |d| {
            let start = Ts::from_ny_local(NaiveDate::from_ymd_opt(2026, 6, d).unwrap(), NaiveTime::MIN).unwrap();
            Bar::<Adjusted> { start, open: Price::new(10.0), high: Price::new(11.0), low: Price::new(9.0), close: Price::new(10.5), volume: 1e6 }
        };
        let session = SessionDate(NaiveDate::from_ymd_opt(2026, 6, 15).unwrap());
        let ok = DailyRef::from_history(session, &[day(10), day(11), day(12)], 2, 3).unwrap();
        assert_eq!(ok.prev_close.value(), 10.5);
        assert_eq!(ok.adv, Some(1e6));
        assert!(ok.atr.is_some());
        assert!(matches!(DailyRef::from_history(session, &[day(12), day(15)], 2, 3), Err(DailyRefError::Lookahead { .. })));
        assert!(matches!(DailyRef::from_history(session, &[day(12), day(11)], 2, 3), Err(DailyRefError::OutOfOrder)));
        assert!(matches!(DailyRef::from_history(session, &[], 2, 3), Err(DailyRefError::NoHistory)));
    }
}
