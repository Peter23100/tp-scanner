//! Massive REST: whole-market daily bars, splits, float, holidays, and one ticker's minute history.
//!
//! Blocking on purpose: every call here is reference data fetched at startup or on a click, never
//! on the scan path. The key travels in the `Authorization` header, never in a URL, so it can't
//! leak into an error message or a log.

use crate::{MassiveError, Secret};
use chrono::NaiveDate;
use serde_json::Value;
use std::time::Duration;
use tp_marketdata::bar::Bar;
use tp_marketdata::price::{AdjFactor, Price, Unadjusted};
use tp_marketdata::time::Ts;

pub const API_BASE: &str = "https://api.massive.com";

/// One symbol's bar for one day, as printed (unadjusted).
#[derive(Debug, Clone, PartialEq)]
pub struct DayRow {
    pub symbol: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// A split (or stock dividend): on `execution_date`, prices before it scale by `factor`.
#[derive(Debug, Clone, PartialEq)]
pub struct Split {
    pub symbol: String,
    pub execution_date: NaiveDate,
    pub factor: AdjFactor,
}

/// A day the market is closed or closes early.
#[derive(Debug, Clone, PartialEq)]
pub struct Holiday {
    pub date: NaiveDate,
    pub name: String,
    /// `None` when closed all day; the early close instant otherwise.
    pub early_close: Option<Ts>,
}

pub struct Rest {
    http: reqwest::blocking::Client,
    key: Secret,
    base: String,
}

impl Rest {
    pub fn new(key: Secret) -> Result<Rest, MassiveError> {
        Rest::with_base(key, API_BASE)
    }

    pub fn with_base(key: Secret, base: &str) -> Result<Rest, MassiveError> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| MassiveError::Http(e.to_string()))?;
        Ok(Rest { http, key, base: base.trim_end_matches('/').to_string() })
    }

    fn get(&self, url: &str) -> Result<Value, MassiveError> {
        let r = self
            .http
            .get(url)
            .bearer_auth(self.key.expose())
            .send()
            .map_err(|e| MassiveError::Http(e.without_url().to_string()))?;
        let status = r.status();
        let text = r.text().map_err(|e| MassiveError::Http(e.without_url().to_string()))?;
        if !status.is_success() {
            let body: String = text.chars().take(300).collect();
            return Err(MassiveError::Status { status: status.as_u16(), url: strip_query(url), body });
        }
        serde_json::from_str(&text).map_err(|e| MassiveError::Parse(format!("{}: {e}", strip_query(url))))
    }

    /// Follow `next_url` pages, collecting every `results` entry.
    fn get_all(&self, first: &str) -> Result<Vec<Value>, MassiveError> {
        let mut out = vec![];
        let mut url = Some(first.to_string());
        while let Some(u) = url.take() {
            let v = self.get(&u)?;
            if let Some(rs) = v.get("results").and_then(Value::as_array) {
                out.extend(rs.iter().cloned());
            }
            url = v.get("next_url").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        }
        Ok(out)
    }

    /// Every US stock's bar for `date`, unadjusted. Empty on a day the market was closed.
    pub fn grouped_daily(&self, date: NaiveDate) -> Result<Vec<DayRow>, MassiveError> {
        let url = format!("{}/v2/aggs/grouped/locale/us/market/stocks/{date}?adjusted=false", self.base);
        parse_grouped(&self.get(&url)?)
    }

    /// Every split executing on or after `from`.
    pub fn splits_since(&self, from: NaiveDate) -> Result<Vec<Split>, MassiveError> {
        let url = format!("{}/stocks/v1/splits?execution_date.gte={from}&limit=5000", self.base);
        Ok(self.get_all(&url)?.iter().filter_map(parse_split).collect())
    }

    /// Free float in shares for every symbol Massive has one for.
    pub fn floats(&self) -> Result<Vec<(String, f64)>, MassiveError> {
        let url = format!("{}/stocks/vX/float?limit=5000", self.base);
        Ok(self
            .get_all(&url)?
            .iter()
            .filter_map(|r| {
                let t = r.get("ticker")?.as_str()?;
                let f = r.get("free_float")?.as_f64().filter(|x| x.is_finite() && *x >= 0.0)?;
                Some((t.to_string(), f))
            })
            .collect())
    }

    /// Upcoming closures and early closes.
    pub fn upcoming_holidays(&self) -> Result<Vec<Holiday>, MassiveError> {
        let v = self.get(&format!("{}/v1/marketstatus/upcoming", self.base))?;
        Ok(parse_holidays(&v))
    }

    /// One ticker's 1-minute bars from `from` through `to` (inclusive dates, New York), unadjusted,
    /// oldest first. Includes extended hours.
    pub fn minute_bars(&self, ticker: &str, from: NaiveDate, to: NaiveDate) -> Result<Vec<Bar<Unadjusted>>, MassiveError> {
        if ticker.is_empty() || !ticker.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
            return Err(MassiveError::Parse(format!("`{ticker}` is not a ticker")));
        }
        let url = format!(
            "{}/v2/aggs/ticker/{ticker}/range/1/minute/{from}/{to}?adjusted=false&sort=asc&limit=50000",
            self.base
        );
        let v = self.get(&url)?;
        Ok(parse_aggs(&v))
    }
}

fn strip_query(url: &str) -> String {
    url.split('?').next().unwrap_or(url).to_string()
}

fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k)?.as_f64().filter(|x| x.is_finite())
}

pub(crate) fn parse_grouped(v: &Value) -> Result<Vec<DayRow>, MassiveError> {
    let Some(rs) = v.get("results").and_then(Value::as_array) else {
        // A closed day answers with no `results` (resultsCount 0).
        return match v.get("resultsCount").and_then(Value::as_u64) {
            Some(0) | None if v.get("status").is_some() => Ok(vec![]),
            _ => Err(MassiveError::Parse("grouped daily: no results".into())),
        };
    };
    Ok(rs
        .iter()
        .filter_map(|r| {
            let row = DayRow {
                symbol: r.get("T")?.as_str()?.to_string(),
                open: num(r, "o")?,
                high: num(r, "h")?,
                low: num(r, "l")?,
                close: num(r, "c")?,
                volume: num(r, "v")?,
            };
            // A bar that isn't sane is dropped here rather than poisoning a reference.
            (row.low <= row.open.min(row.close) && row.open.max(row.close) <= row.high && row.volume >= 0.0).then_some(row)
        })
        .collect())
}

pub(crate) fn parse_split(r: &Value) -> Option<Split> {
    let symbol = r.get("ticker")?.as_str()?.to_string();
    let execution_date = NaiveDate::parse_from_str(r.get("execution_date")?.as_str()?, "%Y-%m-%d").ok()?;
    let factor = AdjFactor::from_split(num(r, "split_to")?, num(r, "split_from")?)?;
    Some(Split { symbol, execution_date, factor })
}

pub(crate) fn parse_holidays(v: &Value) -> Vec<Holiday> {
    let Some(rs) = v.as_array() else { return vec![] };
    let mut out: Vec<Holiday> = vec![];
    for r in rs {
        // NYSE and NASDAQ list the same days; one entry per date is enough.
        let Some(date) = r.get("date").and_then(Value::as_str).and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        else {
            continue;
        };
        if out.iter().any(|h| h.date == date) {
            continue;
        }
        let early_close = match r.get("status").and_then(Value::as_str) {
            Some("early-close") => r
                .get("close")
                .and_then(Value::as_str)
                .and_then(|c| chrono::DateTime::parse_from_rfc3339(c).ok())
                .and_then(|t| Ts::from_millis(t.timestamp_millis())),
            _ => None,
        };
        let name = r.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        out.push(Holiday { date, name, early_close });
    }
    out
}

pub(crate) fn parse_aggs(v: &Value) -> Vec<Bar<Unadjusted>> {
    let Some(rs) = v.get("results").and_then(Value::as_array) else { return vec![] };
    rs.iter()
        .filter_map(|r| {
            let start = Ts::from_millis(r.get("t")?.as_i64()?)?;
            let (o, h, l, c, vol) = (num(r, "o")?, num(r, "h")?, num(r, "l")?, num(r, "c")?, num(r, "v")?);
            Some(Bar { start, open: Price::new(o), high: Price::new(h), low: Price::new(l), close: Price::new(c), volume: vol })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn grouped_daily_parses_and_drops_insane_rows() {
        let v = json!({"status":"OK","resultsCount":3,"results":[
            {"T":"AAPL","o":200.0,"h":202.5,"l":199.0,"c":201.0,"v":5.1e7,"vw":200.8,"n":1,"t":1},
            {"T":"BAD","o":5.0,"h":4.0,"l":3.0,"c":3.5,"v":10.0},
            {"T":"NOVOL","o":5.0,"h":6.0,"l":4.0,"c":5.5}
        ]});
        let rows = parse_grouped(&v).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "AAPL");
        assert_eq!(rows[0].close, 201.0);
    }

    #[test]
    fn a_closed_day_is_empty_not_an_error() {
        assert!(parse_grouped(&json!({"status":"OK","resultsCount":0,"adjusted":false})).unwrap().is_empty());
        assert!(parse_grouped(&json!({"weird":1})).is_err());
    }

    #[test]
    fn splits_give_the_pre_split_price_factor() {
        let fwd = parse_split(&json!({"ticker":"NVDA","execution_date":"2024-06-10","split_from":1,"split_to":10})).unwrap();
        assert_eq!(fwd.factor.get(), 0.1, "10-for-1: old prices / 10");
        let rev = parse_split(&json!({"ticker":"PFSA","execution_date":"2026-09-01","split_from":4,"split_to":1})).unwrap();
        assert_eq!(rev.factor.get(), 4.0, "1-for-4 reverse: old prices x 4");
        assert!(parse_split(&json!({"ticker":"X","execution_date":"2026-09-01","split_from":0,"split_to":1})).is_none());
    }

    #[test]
    fn holidays_dedupe_exchanges_and_keep_early_closes() {
        let v = json!([
            {"date":"2026-11-26","exchange":"NYSE","name":"Thanksgiving","status":"closed"},
            {"date":"2026-11-26","exchange":"NASDAQ","name":"Thanksgiving","status":"closed"},
            {"date":"2026-11-27","exchange":"NYSE","name":"Thanksgiving","status":"early-close",
             "open":"2026-11-27T14:30:00.000Z","close":"2026-11-27T18:00:00.000Z"}
        ]);
        let h = parse_holidays(&v);
        assert_eq!(h.len(), 2);
        assert!(h[0].early_close.is_none());
        assert_eq!(h[1].early_close.unwrap().ny().format("%H:%M").to_string(), "13:00");
    }

    #[test]
    fn minute_aggs_parse_in_order() {
        let v = json!({"results":[{"t":1760000000000i64,"o":1.0,"h":1.2,"l":0.9,"c":1.1,"v":1000.0}]});
        let b = parse_aggs(&v);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].start.millis(), 1_760_000_000_000);
    }

    #[test]
    fn errors_never_carry_the_query_string() {
        assert_eq!(strip_query("https://api.massive.com/v2/x?apiKey=SECRET"), "https://api.massive.com/v2/x");
    }
}
