//! The live market-data door: Massive's WebSocket as a [`MarketSource`].
//!
//! A background thread owns the socket (its own small tokio runtime) and hands bars over a channel,
//! so the engine stays synchronous and deterministic, exactly as with a replay. `AM.*` is every US
//! stock's minute bar, sent once the minute closes. On Starter the feed is the 15-minute delayed
//! one; the code path is identical to real time, only the host differs.
//!
//! The supervisor reconnects with backoff (1 s doubling to 30 s) and publishes a [`FeedStatus`]
//! the UI shows. A wrong key stops it for good ([`FeedState::AuthFailed`]): retrying a rejected key
//! only gets the account throttled.
//!
//! ⚠ **A reconnect gap is not filled.** The engine treats a symbol with no bars as quiet, so bars
//! lost while disconnected make windows spanning the gap read low. The status counts reconnects so
//! the gap is visible; a REST backfill is a later step.

use crate::Secret;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use tp_marketdata::bar::Bar;
use tp_marketdata::price::{Price, Unadjusted};
use tp_marketdata::source::{MarketEvent, MarketSource, Resolution, SourceError, Symbol};
use tp_marketdata::time::Ts;

pub const REALTIME_URL: &str = "wss://socket.massive.com/stocks";
pub const DELAYED_URL: &str = "wss://delayed.massive.com/stocks";

#[derive(Debug, Clone, PartialEq)]
pub enum FeedState {
    Connecting,
    Authenticating,
    /// Authenticated and subscribed.
    Live,
    Reconnecting { attempt: u32, wait_ms: u64 },
    /// The key was refused. Not retried.
    AuthFailed(String),
    Stopped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeedStatus {
    pub url: String,
    pub state: FeedState,
    /// Bars handed to the engine.
    pub bars: u64,
    /// The newest bar's start (market time; ~15 minutes behind the clock on the delayed feed).
    pub last_bar_start: Option<Ts>,
    /// Wall-clock ms when the last message of any kind arrived.
    pub last_message_wall_ms: Option<i64>,
    /// Messages or bars that couldn't be used (malformed, insane OHLC). Counted, never fatal.
    pub bad_messages: u64,
    pub reconnects: u32,
    /// The latest status text from the server ("authenticated", "subscribed to: AM.*", ...).
    pub server_message: Option<String>,
    pub last_error: Option<String>,
}

impl FeedStatus {
    fn new(url: &str) -> FeedStatus {
        FeedStatus {
            url: url.to_string(),
            state: FeedState::Connecting,
            bars: 0,
            last_bar_start: None,
            last_message_wall_ms: None,
            bad_messages: 0,
            reconnects: 0,
            server_message: None,
            last_error: None,
        }
    }
}

/// One decoded WebSocket message.
#[derive(Debug, Clone, PartialEq)]
pub enum WsMsg {
    Bar { symbol: String, bar: Bar<Unadjusted> },
    Status { status: String, message: String },
    /// Well-formed but not for us (another channel, an OTC ticker).
    Ignored,
    Bad(String),
}

/// Decode one text frame: a JSON array of events.
pub fn parse_messages(text: &str) -> Vec<WsMsg> {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return vec![WsMsg::Bad(format!("not JSON: {e}"))],
    };
    let items = match v {
        Value::Array(a) => a,
        other => vec![other],
    };
    items.iter().map(parse_one).collect()
}

fn parse_one(m: &Value) -> WsMsg {
    match m.get("ev").and_then(Value::as_str) {
        Some("status") => WsMsg::Status {
            status: m.get("status").and_then(Value::as_str).unwrap_or("").to_string(),
            message: m.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
        },
        Some("AM") => {
            if m.get("otc").and_then(Value::as_bool) == Some(true) {
                return WsMsg::Ignored;
            }
            let n = |k: &str| m.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
            let (Some(sym), Some(s), Some(o), Some(h), Some(l), Some(c), Some(v)) = (
                m.get("sym").and_then(Value::as_str),
                m.get("s").and_then(Value::as_i64),
                n("o"),
                n("h"),
                n("l"),
                n("c"),
                n("v"),
            ) else {
                return WsMsg::Bad(format!("AM missing a field: {}", truncate(&m.to_string())));
            };
            let Some(start) = Ts::from_millis(s).filter(|t| (946_684_800_000..4_102_444_800_000).contains(&t.millis())) else {
                return WsMsg::Bad(format!("AM {sym}: start {s} is not a ms timestamp"));
            };
            if !(l <= o.min(c) && o.max(c) <= h && v >= 0.0) || sym.is_empty() {
                return WsMsg::Bad(format!("AM {sym}: insane bar o={o} h={h} l={l} c={c} v={v}"));
            }
            WsMsg::Bar {
                symbol: sym.to_string(),
                bar: Bar { start, open: Price::new(o), high: Price::new(h), low: Price::new(l), close: Price::new(c), volume: v },
            }
        }
        _ => WsMsg::Ignored,
    }
}

fn truncate(s: &str) -> String {
    s.chars().take(160).collect()
}

/// What [`LiveSource::next_timeout`] returns.
#[derive(Debug)]
pub enum Poll {
    Event(MarketEvent),
    Timeout,
    /// The feed thread has ended (stopped, or the key was refused).
    Closed,
}

/// Full-market minute bars from Massive.
pub struct LiveSource {
    rx: Receiver<MarketEvent>,
    status: Arc<Mutex<FeedStatus>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LiveSource {
    /// Connect to `url` (normally [`DELAYED_URL`] on Starter) and subscribe to `params`
    /// (`"AM.*"` for the whole market).
    pub fn start(key: Secret, url: &str, params: &str) -> LiveSource {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(FeedStatus::new(url)));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp, url, params) = (status.clone(), stop.clone(), url.to_string(), params.to_string());
        let thread = std::thread::Builder::new()
            .name("massive-ws".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
                rt.block_on(supervise(key, url, params, tx, st.clone(), sp));
                let mut s = st.lock().expect("status");
                if !matches!(s.state, FeedState::AuthFailed(_)) {
                    s.state = FeedState::Stopped;
                }
            })
            .expect("spawn feed thread");
        LiveSource { rx, status, stop, thread: Some(thread) }
    }

    /// A source fed by hand, for tests and replays through the live path.
    pub fn from_channel(rx: Receiver<MarketEvent>) -> LiveSource {
        LiveSource { rx, status: Arc::new(Mutex::new(FeedStatus::new("test"))), stop: Arc::new(AtomicBool::new(false)), thread: None }
    }

    pub fn status(&self) -> FeedStatus {
        self.status.lock().expect("status").clone()
    }

    pub fn status_handle(&self) -> Arc<Mutex<FeedStatus>> {
        self.status.clone()
    }

    /// The next bar, waiting at most `d`. Lets a caller do periodic work (snapshots, commands)
    /// between bars.
    pub fn next_timeout(&mut self, d: Duration) -> Poll {
        match self.rx.recv_timeout(d) {
            Ok(e) => Poll::Event(e),
            Err(RecvTimeoutError::Timeout) => Poll::Timeout,
            Err(RecvTimeoutError::Disconnected) => Poll::Closed,
        }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for LiveSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl MarketSource for LiveSource {
    /// Blocks until a bar arrives; `None` once the feed has ended.
    fn next_event(&mut self) -> Option<Result<MarketEvent, SourceError>> {
        self.rx.recv().ok().map(Ok)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

enum Ended {
    /// Reconnect after backoff.
    Retry(String),
    /// Don't (stop requested, receiver gone, key refused).
    Stop,
}

async fn supervise(key: Secret, url: String, params: String, tx: Sender<MarketEvent>, st: Arc<Mutex<FeedStatus>>, stop: Arc<AtomicBool>) {
    let mut attempt: u32 = 0;
    let mut interned: std::collections::HashMap<String, Symbol> = std::collections::HashMap::new();
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        st.lock().expect("status").state = FeedState::Connecting;
        match session(&key, &url, &params, &tx, &st, &stop, &mut interned, &mut attempt).await {
            Ended::Stop => return,
            Ended::Retry(why) => {
                attempt += 1;
                let wait_ms = (1000u64 << (attempt - 1).min(5)).min(30_000);
                {
                    let mut s = st.lock().expect("status");
                    s.reconnects += 1;
                    s.last_error = Some(why);
                    s.state = FeedState::Reconnecting { attempt, wait_ms };
                }
                let mut waited = 0;
                while waited < wait_ms {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    waited += 250;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn session(
    key: &Secret,
    url: &str,
    params: &str,
    tx: &Sender<MarketEvent>,
    st: &Arc<Mutex<FeedStatus>>,
    stop: &Arc<AtomicBool>,
    interned: &mut std::collections::HashMap<String, Symbol>,
    attempt: &mut u32,
) -> Ended {
    let (mut ws, _) = match tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(url)).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return Ended::Retry(format!("connect: {e}")),
        Err(_) => return Ended::Retry("connect: timed out".into()),
    };
    st.lock().expect("status").state = FeedState::Authenticating;
    let auth = serde_json::json!({"action": "auth", "params": key.expose()}).to_string();
    if let Err(e) = ws.send(Message::Text(auth)).await {
        return Ended::Retry(format!("send auth: {e}"));
    }
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        let msg = tokio::select! {
            m = ws.next() => m,
            _ = tick.tick() => {
                if stop.load(Ordering::SeqCst) {
                    let _ = ws.close(None).await;
                    return Ended::Stop;
                }
                continue;
            }
        };
        let text = match msg {
            None => return Ended::Retry("server closed the connection".into()),
            Some(Err(e)) => return Ended::Retry(format!("read: {e}")),
            Some(Ok(Message::Text(t))) => t,
            Some(Ok(Message::Close(f))) => return Ended::Retry(format!("server closed: {}", f.map(|f| f.reason.to_string()).unwrap_or_default())),
            Some(Ok(_)) => continue,
        };
        st.lock().expect("status").last_message_wall_ms = Some(now_ms());
        for m in parse_messages(&text) {
            match m {
                WsMsg::Bar { symbol, bar } => {
                    let symbol = match interned.get(&symbol) {
                        Some(s) => s.clone(),
                        None => {
                            let s: Symbol = Arc::from(symbol.as_str());
                            interned.insert(symbol, s.clone());
                            s
                        }
                    };
                    {
                        let mut s = st.lock().expect("status");
                        s.bars += 1;
                        if s.last_bar_start.is_none_or(|t| bar.start > t) {
                            s.last_bar_start = Some(bar.start);
                        }
                    }
                    if tx.send(MarketEvent::Bar { symbol, resolution: Resolution::Minute, bar }).is_err() {
                        return Ended::Stop;
                    }
                }
                WsMsg::Status { status, message } => {
                    st.lock().expect("status").server_message = Some(message.clone());
                    match status.as_str() {
                        "auth_success" => {
                            let sub = serde_json::json!({"action": "subscribe", "params": params}).to_string();
                            if let Err(e) = ws.send(Message::Text(sub)).await {
                                return Ended::Retry(format!("send subscribe: {e}"));
                            }
                            let mut s = st.lock().expect("status");
                            s.state = FeedState::Live;
                            s.last_error = None;
                            *attempt = 0;
                        }
                        "auth_failed" => {
                            st.lock().expect("status").state = FeedState::AuthFailed(message);
                            return Ended::Stop;
                        }
                        "max_connections" => return Ended::Retry(format!("max connections: {message}")),
                        _ => {}
                    }
                }
                WsMsg::Bad(why) => {
                    let mut s = st.lock().expect("status");
                    s.bad_messages += 1;
                    s.last_error = Some(why);
                }
                WsMsg::Ignored => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The sample from Massive's AM docs, plus a status frame.
    const FRAME: &str = r#"[
        {"ev":"status","status":"auth_success","message":"authenticated"},
        {"ev":"AM","sym":"GTE","v":4110,"av":9470157,"op":0.4372,"vw":0.4488,"o":0.4488,"c":0.4486,"h":0.4489,"l":0.4486,"a":0.4352,"z":685,"s":1610144640000,"e":1610144700000}
    ]"#;

    #[test]
    fn decodes_the_documented_am_message() {
        let m = parse_messages(FRAME);
        assert_eq!(m.len(), 2);
        assert!(matches!(&m[0], WsMsg::Status { status, .. } if status == "auth_success"));
        let WsMsg::Bar { symbol, bar } = &m[1] else { panic!("{:?}", m[1]) };
        assert_eq!(symbol, "GTE");
        assert_eq!(bar.start.millis(), 1_610_144_640_000, "s is the START, in ms");
        assert_eq!(bar.close.value(), 0.4486);
        assert_eq!(bar.volume, 4110.0, "v is the bar's volume, not the day's (av)");
    }

    #[test]
    fn bad_messages_are_reported_not_fatal() {
        assert!(matches!(parse_messages("{oops").as_slice(), [WsMsg::Bad(_)]));
        let insane = r#"[{"ev":"AM","sym":"X","v":1,"o":5,"c":5,"h":4,"l":3,"s":1610144640000}]"#;
        assert!(matches!(parse_messages(insane).as_slice(), [WsMsg::Bad(_)]));
        let seconds = r#"[{"ev":"AM","sym":"X","v":1,"o":5,"c":5,"h":5,"l":5,"s":1610144640}]"#;
        assert!(matches!(parse_messages(seconds).as_slice(), [WsMsg::Bad(_)]), "a seconds timestamp is caught");
        let missing = r#"[{"ev":"AM","sym":"X","o":5,"c":5,"h":5,"l":5,"s":1610144640000}]"#;
        assert!(matches!(parse_messages(missing).as_slice(), [WsMsg::Bad(_)]));
    }

    #[test]
    fn otc_and_other_channels_are_ignored() {
        let otc = r#"[{"ev":"AM","sym":"OTCX","v":1,"o":5,"c":5,"h":5,"l":5,"s":1610144640000,"otc":true}]"#;
        assert_eq!(parse_messages(otc), vec![WsMsg::Ignored]);
        assert_eq!(parse_messages(r#"[{"ev":"T","sym":"X"}]"#), vec![WsMsg::Ignored]);
    }

    #[test]
    fn a_channel_source_feeds_the_engine_contract() {
        let (tx, rx) = mpsc::channel();
        let mut src = LiveSource::from_channel(rx);
        let WsMsg::Bar { symbol, bar } = parse_messages(FRAME).remove(1) else { panic!() };
        tx.send(MarketEvent::Bar { symbol: Arc::from(symbol.as_str()), resolution: Resolution::Minute, bar }).unwrap();
        assert!(matches!(src.next_timeout(Duration::from_millis(10)), Poll::Event(_)));
        assert!(matches!(src.next_timeout(Duration::from_millis(10)), Poll::Timeout));
        drop(tx);
        assert!(matches!(src.next_timeout(Duration::from_millis(10)), Poll::Closed));
        assert!(src.next_event().is_none());
    }
}
