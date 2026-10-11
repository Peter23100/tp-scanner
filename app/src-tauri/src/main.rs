//! TP Scanner: the Primus replacement's desktop app.
//!
//! Windows: a launcher bar, one window per screen (`screen-<name>`, laid out by
//! `filters/display.json`) and any number of chart windows (`chart-<n>`). Each chart window and
//! each chart panel has a link colour; clicking a symbol in a panel sends it to every chart of
//! that panel's colour.
//!
//! **Hard rule:** this process holds Massive market-data keys only. It never holds broker
//! credentials and has no path to an order.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod layout;
mod scanner;

use scanner::{Cmd, Shared, Snapshot};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// Which windows were open, and each chart's colour. Saved so a restart reopens the same desk.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct UiState {
    windows: Vec<WinRec>,
    /// Panel key (`screen/row/col/item`) → link colour, when changed from display.json's.
    #[serde(default)]
    panel_links: HashMap<String, String>,
    /// Link colour → the symbol it shows.
    #[serde(default)]
    links: HashMap<String, String>,
    /// The data source running at shutdown, restarted at launch.
    #[serde(default)]
    resume: Option<Resume>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
enum Resume {
    Live { realtime: bool },
    Replay { date: String, speed: f64, start_at: Option<String> },
}

fn resume_cmd(r: &Resume) -> Result<Cmd, String> {
    match r {
        Resume::Live { realtime } => Ok(Cmd::Live { realtime: *realtime }),
        Resume::Replay { date, speed, start_at } => {
            let date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|e| format!("date `{date}`: {e}"))?;
            let start_at = match start_at.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(s) => Some(chrono::NaiveTime::parse_from_str(s, "%H:%M").map_err(|e| format!("start `{s}`: {e}"))?),
                None => None,
            };
            Ok(Cmd::Replay { date, speed: *speed, start_at })
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct WinRec {
    label: String,
    /// "screen" or "chart".
    kind: String,
    /// The screen name, or the chart's link colour.
    arg: String,
}

struct App {
    cmd: Mutex<Sender<Cmd>>,
    shared: Arc<Mutex<Shared>>,
    ui: Mutex<UiState>,
    ui_path: PathBuf,
    exiting: AtomicBool,
    rest: Option<Arc<tp_massive::rest::Rest>>,
    chart_cache: Mutex<HashMap<String, Arc<Vec<[f64; 6]>>>>,
}

impl App {
    fn save_ui(&self) {
        let ui = self.ui.lock().expect("ui").clone();
        if let Ok(t) = serde_json::to_string_pretty(&ui) {
            let tmp = self.ui_path.with_extension("json.tmp");
            if std::fs::write(&tmp, t).is_ok() {
                let _ = std::fs::rename(&tmp, &self.ui_path);
            }
        }
    }

    fn send(&self, c: Cmd) -> Result<(), String> {
        self.cmd.lock().expect("cmd").send(c).map_err(|_| "the scan engine has stopped; restart the app".to_string())
    }
}

#[derive(Serialize)]
struct LayoutReply {
    layout: Option<layout::Layout>,
    error: Option<String>,
    panel_links: HashMap<String, String>,
}

#[tauri::command]
fn get_layout(app: State<'_, App>) -> LayoutReply {
    let g = app.shared.lock().expect("shared");
    LayoutReply { layout: g.layout.clone(), error: g.layout_error.clone(), panel_links: app.ui.lock().expect("ui").panel_links.clone() }
}

#[tauri::command]
fn get_snapshot(app: State<'_, App>) -> Snapshot {
    app.shared.lock().expect("shared").snapshot.clone()
}

#[tauri::command]
async fn reload_filters(app: State<'_, App>) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    app.send(Cmd::Reload(tx))?;
    tauri::async_runtime::spawn_blocking(move || rx.recv().map_err(|_| "no answer from the engine".to_string())?)
        .await
        .map_err(|e| e.to_string())?
}

fn start(app: &App, r: Resume) -> Result<(), String> {
    app.send(resume_cmd(&r)?)?;
    app.ui.lock().expect("ui").resume = Some(r);
    app.save_ui();
    Ok(())
}

#[tauri::command]
fn start_live(app: State<'_, App>, realtime: bool) -> Result<(), String> {
    start(&app, Resume::Live { realtime })
}

#[tauri::command]
fn start_replay(app: State<'_, App>, date: String, speed: f64, start_at: Option<String>) -> Result<(), String> {
    start(&app, Resume::Replay { date, speed, start_at })
}

#[tauri::command]
fn set_speed(app: State<'_, App>, speed: f64) -> Result<(), String> {
    if let Some(Resume::Replay { speed: s, .. }) = app.ui.lock().expect("ui").resume.as_mut() {
        *s = speed;
    }
    app.save_ui();
    app.send(Cmd::SetSpeed(speed))
}

#[tauri::command]
fn stop_source(app: State<'_, App>) -> Result<(), String> {
    app.ui.lock().expect("ui").resume = None;
    app.save_ui();
    app.send(Cmd::Stop)
}

#[tauri::command]
async fn explain(app: State<'_, App>, filter: String, symbol: String) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    app.send(Cmd::Explain { filter, symbol, reply: tx })?;
    tauri::async_runtime::spawn_blocking(move || rx.recv().map_err(|_| "no answer from the engine".to_string())?)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn link_set(handle: AppHandle, app: State<'_, App>, color: String, symbol: String) -> Result<(), String> {
    if !layout::LINK_COLORS.contains(&color.as_str()) {
        return Err(format!("no link colour `{color}`"));
    }
    let symbol = symbol.trim().to_uppercase();
    app.ui.lock().expect("ui").links.insert(color.clone(), symbol.clone());
    app.save_ui();
    handle.emit("link-changed", serde_json::json!({"color": color, "symbol": symbol})).map_err(|e| e.to_string())
}

#[tauri::command]
fn link_get(app: State<'_, App>) -> HashMap<String, String> {
    app.ui.lock().expect("ui").links.clone()
}

#[tauri::command]
fn set_panel_link(app: State<'_, App>, key: String, color: String) -> Result<(), String> {
    if !layout::LINK_COLORS.contains(&color.as_str()) {
        return Err(format!("no link colour `{color}`"));
    }
    app.ui.lock().expect("ui").panel_links.insert(key, color);
    app.save_ui();
    Ok(())
}

/// Tell the engine which symbol a chart shows, so its live bars are forwarded.
#[tauri::command]
fn chart_watch(app: State<'_, App>, id: String, symbol: Option<String>) {
    let mut g = app.shared.lock().expect("shared");
    match symbol.filter(|s| !s.is_empty()) {
        Some(s) => g.chart_symbols.insert(id, s.to_uppercase()),
        None => g.chart_symbols.remove(&id),
    };
}

/// 1-minute bars `[t_ms, o, h, l, c, v]` for the `days` calendar days ending `until` (default
/// today), as printed. Past ranges are cached for the session.
#[tauri::command]
async fn chart_history(app: State<'_, App>, symbol: String, until: Option<String>, days: i64) -> Result<Arc<Vec<[f64; 6]>>, String> {
    let rest = app.rest.clone().ok_or_else(|| format!("{} is not set: charts need the Massive API key", tp_massive::API_KEY_VAR))?;
    let today = tp_marketdata::time::Ts::from_millis(scanner::now_ms()).expect("now").ny().date_naive();
    let to = match until {
        Some(u) => chrono::NaiveDate::parse_from_str(&u, "%Y-%m-%d").map_err(|e| e.to_string())?,
        None => today,
    };
    let from = to - chrono::Duration::days(days.clamp(1, 30));
    let symbol = symbol.trim().to_uppercase();
    let key = format!("{symbol}/{from}/{to}");
    if let Some(v) = app.chart_cache.lock().expect("cache").get(&key) {
        return Ok(v.clone());
    }
    let sym = symbol.clone();
    let bars = tauri::async_runtime::spawn_blocking(move || rest.minute_bars(&sym, from, to))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let out: Arc<Vec<[f64; 6]>> = Arc::new(
        bars.iter()
            .map(|b| [b.start.millis() as f64, b.open.value(), b.high.value(), b.low.value(), b.close.value(), b.volume])
            .collect(),
    );
    if to < today {
        app.chart_cache.lock().expect("cache").insert(key, out.clone());
    }
    Ok(out)
}

fn open_window(handle: &AppHandle, rec: &WinRec) -> Result<(), String> {
    if let Some(w) = handle.get_webview_window(&rec.label) {
        let _ = w.unminimize();
        return w.set_focus().map_err(|e| e.to_string());
    }
    let (route, title, w, h) = match rec.kind.as_str() {
        "screen" => (format!("#/screen/{}", rec.arg), format!("Scanner: {}", rec.arg), 1400.0, 900.0),
        "chart" => (format!("#/chart/{}/{}", rec.label, rec.arg), "Chart".to_string(), 900.0, 600.0),
        k => return Err(format!("unknown window kind `{k}`")),
    };
    WebviewWindowBuilder::new(handle, &rec.label, WebviewUrl::App(route.into()))
        .title(title)
        .inner_size(w, h)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn remember(app: &App, rec: WinRec) {
    let mut ui = app.ui.lock().expect("ui");
    ui.windows.retain(|r| r.label != rec.label);
    ui.windows.push(rec);
    drop(ui);
    app.save_ui();
}

#[tauri::command]
fn open_screen(handle: AppHandle, app: State<'_, App>, name: String) -> Result<(), String> {
    let rec = WinRec { label: format!("screen-{name}"), kind: "screen".into(), arg: name };
    open_window(&handle, &rec)?;
    remember(&app, rec);
    Ok(())
}

#[tauri::command]
fn open_chart(handle: AppHandle, app: State<'_, App>, color: String) -> Result<(), String> {
    let n = (1..).find(|n| handle.get_webview_window(&format!("chart-{n}")).is_none()).expect("a free number");
    let rec = WinRec { label: format!("chart-{n}"), kind: "chart".into(), arg: color };
    open_window(&handle, &rec)?;
    remember(&app, rec);
    Ok(())
}

#[tauri::command]
fn set_chart_color(app: State<'_, App>, label: String, color: String) -> Result<(), String> {
    if !layout::LINK_COLORS.contains(&color.as_str()) {
        return Err(format!("no link colour `{color}`"));
    }
    let mut ui = app.ui.lock().expect("ui");
    if let Some(r) = ui.windows.iter_mut().find(|r| r.label == label) {
        r.arg = color;
    }
    drop(ui);
    app.save_ui();
    Ok(())
}

fn filters_dir() -> PathBuf {
    // TP_SCANNER_FILTERS overrides; otherwise the repo's filters/ (the app runs from the repo).
    if let Ok(d) = std::env::var("TP_SCANNER_FILTERS") {
        return PathBuf::from(d);
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../filters");
    dev.canonicalize().unwrap_or(dev)
}

fn main() {
    let data_dir = tp_massive::data_dir().unwrap_or_else(|_| std::env::temp_dir().join("tp-scanner"));
    let _ = std::fs::create_dir_all(&data_dir);
    let ui_path = data_dir.join("ui.json");
    let ui: UiState = std::fs::read_to_string(&ui_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let rest = tp_massive::Secret::from_env(tp_massive::API_KEY_VAR).ok().and_then(|k| tp_massive::rest::Rest::new(k).ok()).map(Arc::new);
    let (tx, rx) = mpsc::channel();
    let shared = Arc::new(Mutex::new(Shared::default()));
    let reopen = ui.windows.clone();
    let resume = ui.resume.as_ref().and_then(|r| resume_cmd(r).ok());
    if let Some(c) = resume {
        // Queued now; the engine thread reads it once its filters and references are loaded.
        let _ = tx.send(c);
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("launcher") {
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(App {
            cmd: Mutex::new(tx),
            shared: shared.clone(),
            ui: Mutex::new(ui),
            ui_path,
            exiting: AtomicBool::new(false),
            rest,
            chart_cache: Mutex::new(HashMap::new()),
        })
        .invoke_handler(tauri::generate_handler![
            get_layout,
            get_snapshot,
            reload_filters,
            start_live,
            start_replay,
            set_speed,
            stop_source,
            explain,
            link_set,
            link_get,
            set_panel_link,
            chart_watch,
            chart_history,
            open_screen,
            open_chart,
            set_chart_color
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let filters = filters_dir();
            std::thread::Builder::new()
                .name("scan-engine".into())
                .spawn(move || {
                    let runner = scanner::Runner::new(handle.clone(), shared, filters, data_dir, rx);
                    let _ = handle.emit("layout-changed", ());
                    runner.run();
                })
                .expect("spawn engine thread");
            for rec in &reopen {
                if let Err(e) = open_window(app.handle(), rec) {
                    eprintln!("reopen {}: {e}", rec.label);
                }
            }
            Ok(())
        })
        .on_window_event(|w, ev| {
            let label = w.label().to_string();
            match ev {
                WindowEvent::CloseRequested { .. } if label == "launcher" => {
                    // Closing the launcher closes the app; the open windows are remembered.
                    let app = w.app_handle().state::<App>();
                    app.exiting.store(true, Ordering::SeqCst);
                    w.app_handle().exit(0);
                }
                WindowEvent::Destroyed => {
                    let app = w.app_handle().state::<App>();
                    if !app.exiting.load(Ordering::SeqCst) && label != "launcher" {
                        app.ui.lock().expect("ui").windows.retain(|r| r.label != label);
                        app.save_ui();
                        app.shared.lock().expect("shared").chart_symbols.retain(|k, _| !k.starts_with(&label));
                    }
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())
        .expect("build the app");
    app.run(|handle, ev| {
        if let RunEvent::ExitRequested { .. } = ev {
            handle.state::<App>().exiting.store(true, Ordering::SeqCst);
        }
    });
}
