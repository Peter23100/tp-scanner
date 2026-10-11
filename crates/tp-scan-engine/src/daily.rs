//! Daily references from a store of daily bars, shared by the replay CLI and the live app.

use crate::engine::DailyProvider;
use crate::state::DailyRef;
use std::collections::HashMap;
use tp_marketdata::bar::Bar;
use tp_marketdata::price::Adjusted;
use tp_marketdata::time::SessionDate;

/// How many prior days a reference is built from: a year covers the prior calendar month, a
/// warm daily EMA20 (seed weight ~1e-11) and ATR14.
pub const HISTORY_DAYS: usize = 260;

/// For each (symbol, session): the reference built from that symbol's days strictly before the
/// session, so a replay can never see the day it is replaying. `daily` is split-adjusted bars by
/// symbol, oldest first.
pub fn history_provider(daily: HashMap<String, Vec<Bar<Adjusted>>>) -> DailyProvider {
    Box::new(move |sym, session: SessionDate| {
        let bars = daily.get(sym)?;
        let cut = bars.partition_point(|b| b.session_date() < session);
        let hist = &bars[cut.saturating_sub(HISTORY_DAYS)..cut];
        DailyRef::from_history(session, hist, 14, 20).ok()
    })
}

/// A provider over any store that can produce a symbol's split-adjusted history for a session
/// (oldest first, all before the session, at most [`HISTORY_DAYS`]).
pub fn provider_from<F>(mut history: F) -> DailyProvider
where
    F: FnMut(&str, SessionDate) -> Vec<Bar<Adjusted>> + Send + 'static,
{
    Box::new(move |sym, session| DailyRef::from_history(session, &history(sym, session), 14, 20).ok())
}
