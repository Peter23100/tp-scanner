//! TP Scanner: the Primus replacement's desktop app.
//!
//! Windows: a launcher bar, one window per filter panel (`panel-<id>`, defined in `panels.json`,
//! seeded from `filters/display.json`) and any number of chart windows (`chart-<n>`, or
//! `chart-<group>-<n>` for the charts that open with a group). Each panel and chart has a link
//! colour; clicking a symbol in a panel sends it to every chart of that panel's colour.
//!
//! Every window can be moved and resized; where it sits is remembered per label (`ui.json`).
//!
//! **Hard rule:** this process holds Massive market-data keys only. It never holds broker
//! credentials and has no path to an order.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod layout;
mod panels;
mod scanner;
mod tools;

use panels::{PanelDef, Panels};
use scanner::{Cmd, Shared, Snapshot};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, RunEvent, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

/// Which windows were open, where every window sits, and each link colour's symbol. Saved so a
/// restart reopens the same desk.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct UiState {
    windows: Vec<WinRec>,
    /// Link colour → the symbol it shows.
    #[serde(default)]
    links: HashMap<String, String>,
    /// The data source running at shutdown, restarted at launch.
    #[serde(default)]
    resume: Option<Resume>,
    /// Window label → outer position and inner size, physical pixels.
    #[serde(default)]
    geom: HashMap<String, Geom>,
    /// 2 once the windows saved by the first app versions (tiled with gaps: two invisible borders
    /// across, a title bar's worth down) have been tidied, once, at launch.
    #[serde(default)]
    geom_version: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
struct Geom {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
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
    /// "panel" or "chart".
    kind: String,
    /// The panel id, or the chart's link colour.
    arg: String,
}

struct App {
    cmd: Mutex<Sender<Cmd>>,
    shared: Arc<Mutex<Shared>>,
    ui: Mutex<UiState>,
    ui_path: PathBuf,
    panels: Mutex<Panels>,
    panels_path: PathBuf,
    filters_dir: PathBuf,
    data_dir: PathBuf,
    /// Window geometry changed and isn't on disk yet (saved by a background timer: a drag sends
    /// hundreds of moves).
    dirty: AtomicBool,
    exiting: AtomicBool,
    rest: Option<Arc<tp_massive::rest::Rest>>,
    chart_cache: Mutex<HashMap<String, Arc<Vec<[f64; 6]>>>>,
    /// Symbol → company name (tickers.csv), read on first use.
    names: Mutex<Option<Arc<HashMap<String, String>>>>,
    /// Window label → when it last moved or resized (by the user). A window still for a moment
    /// after that snaps its edges to its neighbours.
    moved: Mutex<HashMap<String, std::time::Instant>>,
    /// Window label → the place a snap just put it, so the move the snap itself causes isn't
    /// taken for the user's.
    snapped: Mutex<HashMap<String, Geom>>,
}

fn write_json<T: Serialize>(path: &std::path::Path, v: &T) {
    if let Ok(t) = serde_json::to_string_pretty(v) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, t).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

impl App {
    fn save_ui(&self) {
        self.dirty.store(false, Ordering::SeqCst);
        let links = {
            let mut g = self.shared.lock().expect("shared");
            g.links_dirty = false;
            g.links.clone()
        };
        let mut ui = self.ui.lock().expect("ui").clone();
        ui.links = links;
        write_json(&self.ui_path, &ui);
    }

    fn save_panels(&self, handle: &AppHandle) {
        let p = self.panels.lock().expect("panels").clone();
        write_json(&self.panels_path, &p);
        self.shared.lock().expect("shared").follow = follow_map(&p);
        let _ = handle.emit("panels-changed", ());
    }

    fn send(&self, c: Cmd) -> Result<(), String> {
        self.cmd.lock().expect("cmd").send(c).map_err(|_| "the scan engine has stopped; restart the app".to_string())
    }
}

/// `ui.json` if it reads (a byte-order mark from an editor is tolerated). An unreadable file is
/// kept aside as `ui.json.bad` rather than overwritten by the fresh state the app then saves, so a
/// desk's layout is never lost silently.
fn load_ui(path: &std::path::Path) -> UiState {
    let Ok(t) = std::fs::read_to_string(path) else { return UiState::default() };
    match serde_json::from_str::<UiState>(t.trim_start_matches('\u{feff}')) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("ui.json unreadable ({e}); starting fresh, the old file kept as ui.json.bad");
            let _ = std::fs::rename(path, path.with_extension("json.bad"));
            UiState::default()
        }
    }
}

/// `panels.json` if it reads, else the Primus screens from display.json (a broken file is kept
/// aside as `panels.json.bad`, never silently overwritten).
fn load_panels(path: &std::path::Path, filters_dir: &std::path::Path) -> Panels {
    if let Ok(t) = std::fs::read_to_string(path) {
        match serde_json::from_str::<Panels>(t.trim_start_matches('\u{feff}')) {
            Ok(p) => return p,
            Err(e) => {
                eprintln!("panels.json unreadable ({e}); re-seeding from display.json");
                let _ = std::fs::rename(path, path.with_extension("json.bad"));
            }
        }
    }
    seed_panels(filters_dir)
}

/// Filter → the link colours of the panels that show it with Follow on.
fn follow_map(p: &Panels) -> HashMap<String, Vec<String>> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for panel in p.panels.iter().filter(|x| x.follow) {
        for f in &panel.filters {
            let v = m.entry(f.clone()).or_default();
            if !v.contains(&panel.link) {
                v.push(panel.link.clone());
            }
        }
    }
    m
}

fn seed_panels(filters_dir: &std::path::Path) -> Panels {
    std::fs::read_to_string(filters_dir.join("display.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<layout::Display>(&t).ok())
        .map(|d| panels::seed(&d))
        .unwrap_or_default()
}

#[derive(Serialize)]
struct BasketDto {
    name: String,
    types: Vec<String>,
    exchanges: Vec<String>,
    /// `[key, min, max]`.
    ranges: Vec<(String, Option<f64>, Option<f64>)>,
    used_by: Vec<String>,
}

#[derive(Serialize)]
struct FiltersReply {
    filters: Vec<layout::FilterMeta>,
    baskets: Vec<BasketDto>,
    error: Option<String>,
}

#[tauri::command]
fn get_filters(app: State<'_, App>) -> FiltersReply {
    let g = app.shared.lock().expect("shared");
    let baskets = g
        .program
        .baskets
        .iter()
        .map(|b| BasketDto {
            name: b.name.to_string(),
            types: b.types.clone(),
            exchanges: b.exchanges.clone(),
            ranges: b.ranges.clone(),
            used_by: g.program.filters.iter().filter(|f| f.basket.as_deref() == Some(&*b.name)).map(|f| f.name.to_string()).collect(),
        })
        .collect();
    FiltersReply { filters: g.filters.clone(), baskets, error: g.filters_error.clone() }
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
    app.shared.lock().expect("shared").links.insert(color.clone(), symbol.clone());
    app.save_ui();
    handle.emit("link-changed", serde_json::json!({"color": color, "symbol": symbol})).map_err(|e| e.to_string())
}

#[tauri::command]
fn link_get(app: State<'_, App>) -> HashMap<String, String> {
    app.shared.lock().expect("shared").links.clone()
}

#[tauri::command]
fn panels_get(app: State<'_, App>) -> Panels {
    app.panels.lock().expect("panels").clone()
}

/// Replace one panel's definition (filters, title, colour, follow, sound). Its group and first-open
/// tile are kept.
#[tauri::command]
fn panel_update(handle: AppHandle, app: State<'_, App>, panel: PanelDef) -> Result<(), String> {
    panels::validate(&panel)?;
    {
        let mut g = app.panels.lock().expect("panels");
        let slot = g.panels.iter_mut().find(|p| p.id == panel.id).ok_or_else(|| format!("no panel `{}`", panel.id))?;
        *slot = PanelDef { group: slot.group.clone(), tile: slot.tile, ..panel };
    }
    app.save_panels(&handle);
    Ok(())
}

// Window-creating commands must be async: a sync command runs on the main thread, and building a
// webview from there deadlocks on Windows (the window appears frozen and blank).
#[tauri::command]
async fn panel_create(handle: AppHandle, app: State<'_, App>) -> Result<String, String> {
    let id = {
        let mut g = app.panels.lock().expect("panels");
        let id = g.new_id();
        g.panels.push(PanelDef { id: id.clone(), title: Some("New panel".into()), filters: vec![], link: "red".into(), follow: true, sound: false, group: String::new(), tile: None, font: None });
        id
    };
    app.save_panels(&handle);
    open_panel_window(&handle, &app, &id)?;
    Ok(id)
}

#[tauri::command]
async fn panel_delete(handle: AppHandle, app: State<'_, App>, id: String) -> Result<(), String> {
    let label = format!("panel-{id}");
    if let Some(w) = handle.get_webview_window(&label) {
        let _ = w.destroy();
    }
    app.panels.lock().expect("panels").panels.retain(|p| p.id != id);
    {
        let mut ui = app.ui.lock().expect("ui");
        ui.windows.retain(|r| r.label != label);
        ui.geom.remove(&label);
    }
    app.save_ui();
    app.save_panels(&handle);
    Ok(())
}

#[tauri::command]
async fn open_panel(handle: AppHandle, app: State<'_, App>, id: String) -> Result<(), String> {
    open_panel_window(&handle, &app, &id)
}

/// Open every panel and chart of a group (a Primus screen).
#[tauri::command]
async fn open_screen(handle: AppHandle, app: State<'_, App>, name: String) -> Result<(), String> {
    let (ids, charts) = {
        let g = app.panels.lock().expect("panels");
        let ids: Vec<String> = g.panels.iter().filter(|p| p.group == name).map(|p| p.id.clone()).collect();
        let charts: Vec<panels::ChartSeed> = g.charts.iter().filter(|c| c.group == name).cloned().collect();
        (ids, charts)
    };
    if ids.is_empty() && charts.is_empty() {
        return Err(format!("no panels in `{name}`"));
    }
    for id in ids {
        open_panel_window(&handle, &app, &id)?;
    }
    for c in charts {
        let rec = WinRec { label: c.label.clone(), kind: "chart".into(), arg: c.color.clone() };
        open_window(&handle, &app, &rec, c.tile.map(|t| (c.group.clone(), t)))?;
        remember(&app, rec);
    }
    Ok(())
}

/// Back to the Primus screens: every panel window closes, panels.json is re-seeded from
/// display.json, and the panels' remembered positions are forgotten.
#[tauri::command]
async fn reset_layout(handle: AppHandle, app: State<'_, App>) -> Result<(), String> {
    let seeded = seed_panels(&app.filters_dir);
    if seeded.panels.is_empty() {
        return Err(format!("{} has no panels; nothing reset", app.filters_dir.join("display.json").display()));
    }
    let old: Vec<String> = {
        let g = app.panels.lock().expect("panels");
        g.panels.iter().map(|p| format!("panel-{}", p.id)).chain(g.charts.iter().map(|c| c.label.clone())).collect()
    };
    for l in &old {
        if let Some(w) = handle.get_webview_window(l) {
            let _ = w.destroy();
        }
    }
    {
        let mut ui = app.ui.lock().expect("ui");
        ui.windows.retain(|r| !old.contains(&r.label));
        for l in &old {
            ui.geom.remove(l);
        }
    }
    *app.panels.lock().expect("panels") = seeded;
    app.save_ui();
    app.save_panels(&handle);
    Ok(())
}

/// The company name for a chart's watermark. `None` when the ticker list doesn't have it.
#[tauri::command]
fn ticker_name(app: State<'_, App>, symbol: String) -> Option<String> {
    let mut g = app.names.lock().expect("names");
    // Read once; re-read while empty (the list may still be downloading at first start).
    if g.as_ref().is_none_or(|m| m.is_empty()) {
        *g = Some(Arc::new(tp_massive::tickers::names(&app.data_dir)));
    }
    g.as_ref().and_then(|m| m.get(symbol.trim().to_uppercase().as_str()).cloned())
}

// ---------------------------------------------------------------------------------------------
// Named layouts: the whole desk under a name (which windows are open, where every one sits, and
// the panels' definitions: filters, colours, follow, sound, text size). Files in `layouts/`.

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedLayout {
    name: String,
    saved_at: String,
    windows: Vec<WinRec>,
    geom: HashMap<String, Geom>,
    panels: Panels,
}

#[derive(Serialize)]
struct LayoutInfo {
    name: String,
    saved_at: String,
    windows: usize,
}

fn layout_path(app: &App, name: &str) -> Result<PathBuf, String> {
    let n = name.trim();
    if n.is_empty() || n.len() > 40 || !n.chars().all(|c| c.is_ascii_alphanumeric() || " -_".contains(c)) {
        return Err("a layout name is letters, digits, spaces, - or _ (up to 40)".into());
    }
    Ok(app.data_dir.join("layouts").join(format!("{n}.json")))
}

#[tauri::command]
fn layouts_list(app: State<'_, App>) -> Vec<LayoutInfo> {
    let mut out: Vec<LayoutInfo> = std::fs::read_dir(app.data_dir.join("layouts"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<SavedLayout>(t.trim_start_matches('\u{feff}')).ok())
        .map(|l| LayoutInfo { name: l.name, saved_at: l.saved_at, windows: l.windows.len() })
        .collect();
    out.sort_by_key(|l| l.name.to_lowercase());
    out
}

/// Save the desk as it is now (overwrites a layout of the same name).
#[tauri::command]
fn layout_save(app: State<'_, App>, name: String) -> Result<String, String> {
    let path = layout_path(&app, &name)?;
    let ui = app.ui.lock().expect("ui").clone();
    let keep = |l: &str| snappable(l);
    let windows: Vec<WinRec> = ui.windows.iter().filter(|r| keep(&r.label)).cloned().collect();
    let geom: HashMap<String, Geom> = ui.geom.iter().filter(|(l, _)| *l == "launcher" || windows.iter().any(|r| &r.label == *l)).map(|(l, g)| (l.clone(), *g)).collect();
    let layout = SavedLayout {
        name: name.trim().to_string(),
        saved_at: now_stamp().0 + &tp_marketdata::time::Ts::from_millis(scanner::now_ms()).expect("now").ny().format(" %H:%M").to_string(),
        windows,
        geom,
        panels: app.panels.lock().expect("panels").clone(),
    };
    std::fs::create_dir_all(path.parent().expect("dir")).map_err(|e| e.to_string())?;
    write_json(&path, &layout);
    Ok(format!("layout \"{}\" saved ({} windows)", layout.name, layout.windows.len()))
}

/// Switch to a saved layout: its panels, its windows where it had them. Windows it doesn't have
/// close (their panels stay listed under Panels ▾ only if the layout has them).
#[tauri::command]
async fn layout_load(handle: AppHandle, app: State<'_, App>, name: String) -> Result<String, String> {
    let path = layout_path(&app, &name)?;
    let text = std::fs::read_to_string(&path).map_err(|_| format!("no layout \"{}\"", name.trim()))?;
    let layout: SavedLayout = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("{}: {e}", path.display()))?;
    *app.panels.lock().expect("panels") = layout.panels.clone();
    app.save_panels(&handle);
    let wanted: Vec<&str> = layout.windows.iter().map(|r| r.label.as_str()).collect();
    for (label, w) in handle.webview_windows() {
        if snappable(&label) && label != "launcher" && !wanted.contains(&label.as_str()) {
            let _ = w.destroy();
        }
    }
    {
        let mut ui = app.ui.lock().expect("ui");
        ui.geom.extend(layout.geom.iter().map(|(l, g)| (l.clone(), *g)));
        ui.windows.retain(|r| !snappable(&r.label));
        ui.windows.extend(layout.windows.iter().cloned());
    }
    app.save_ui();
    for rec in &layout.windows {
        match handle.get_webview_window(&rec.label) {
            Some(w) => {
                if let Some(g) = layout.geom.get(&rec.label).filter(|g| on_screen(&handle, g)) {
                    let _ = w.unminimize();
                    let _ = w.set_size(PhysicalSize::new(g.w, g.h));
                    let _ = w.set_position(PhysicalPosition::new(g.x, g.y));
                }
            }
            None => {
                if let Err(e) = open_window(&handle, &app, rec, None) {
                    eprintln!("layout {}: {}: {e}", layout.name, rec.label);
                }
            }
        }
    }
    if let (Some(g), Some(w)) = (layout.geom.get("launcher").filter(|g| on_screen(&handle, g)), handle.get_webview_window("launcher")) {
        let _ = w.set_position(PhysicalPosition::new(g.x, g.y));
        let _ = w.set_size(PhysicalSize::new(g.w, g.h));
    }
    Ok(format!("layout \"{}\" loaded", layout.name))
}

#[tauri::command]
fn layout_delete(app: State<'_, App>, name: String) -> Result<String, String> {
    let path = layout_path(&app, &name)?;
    std::fs::remove_file(&path).map_err(|_| format!("no layout \"{}\"", name.trim()))?;
    Ok(format!("layout \"{}\" deleted", name.trim()))
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

#[derive(Serialize)]
struct ChartBars {
    /// `[t_ms, o, h, l, c, v]`, oldest first. Intraday: the bucket start. Daily and longer: UTC
    /// midnight of the New York date, as TradierPro's charts stamp them.
    bars: Vec<[f64; 6]>,
    /// Intraday: the start of the newest 1-minute bar inside `bars`. Live minutes after it extend
    /// the chart; earlier ones are already counted.
    last_minute: Option<f64>,
}

/// One REST answer, cached when its range is wholly in the past (it can't change).
fn rest_bars(app: &App, rest: &tp_massive::rest::Rest, symbol: &str, span: &str, from: chrono::NaiveDate, to: chrono::NaiveDate, today: chrono::NaiveDate) -> Result<Arc<Vec<[f64; 6]>>, String> {
    let key = format!("{symbol}/{span}/{from}/{to}");
    if let Some(v) = app.chart_cache.lock().expect("cache").get(&key) {
        return Ok(v.clone());
    }
    let v = Arc::new(rest.chart_bars(symbol, span, from, to, true).map_err(|e| e.to_string())?);
    if to < today {
        app.chart_cache.lock().expect("cache").insert(key, v.clone());
    }
    Ok(v)
}

/// Calendar days that comfortably hold `bars` bars of `interval` (TradierPro's lookbackDays).
fn lookback_days(bars: i64, interval: &str) -> i64 {
    let up = |a: i64, b: i64| (a + b - 1) / b;
    match interval {
        "1m" => up(bars * 7, 390 * 5) + 3,
        "5m" => up(bars * 7, 78 * 5) + 3,
        "15m" => up(bars * 7, 26 * 5) + 3,
        "daily" => up(bars * 7, 5) + 7,
        "weekly" => bars * 7 + 14,
        _ => bars * 31 + 30,
    }
}

/// Bars for a chart. `until_ms` (replay) cuts history at the replay's clock so the chart never
/// shows what the scan hasn't reached; live charts run to now.
#[tauri::command]
async fn chart_bars(app: State<'_, App>, symbol: String, interval: String, bars: i64, until_ms: Option<i64>) -> Result<ChartBars, String> {
    let rest = app.rest.clone().ok_or_else(|| format!("{} is not set: charts need the Massive API key", tp_massive::API_KEY_VAR))?;
    let symbol = symbol.trim().to_uppercase();
    let ny_date = |ms: i64| tp_marketdata::time::Ts::from_millis(ms).map(|t| t.ny().date_naive());
    let today = ny_date(scanner::now_ms()).expect("now");
    let to = until_ms.and_then(ny_date).unwrap_or(today);
    let bars = bars.clamp(10, 5000);
    let intraday_ms = match interval.as_str() {
        "1m" => Some(60_000i64),
        "5m" => Some(300_000),
        "15m" => Some(900_000),
        "daily" | "weekly" | "monthly" => None,
        i => return Err(format!("unknown interval `{i}`")),
    };
    let app2: &App = &app;
    let cut = |t: f64| until_ms.is_none_or(|u| t as i64 + 60_000 <= u);
    if let Some(ms) = intraday_ms {
        // Indicator warm-up bars ride along (TradierPro's INTRADAY_INDICATOR_WARMUP).
        let from = to - chrono::Duration::days(lookback_days(bars + 50, &interval));
        let mins = rest_bars(app2, &rest, &symbol, "minute", from, to, today)?;
        let mut out: Vec<[f64; 6]> = vec![];
        let mut last_minute = None;
        for m in mins.iter().filter(|m| cut(m[0])) {
            last_minute = Some(m[0]);
            let k = ((m[0] as i64).div_euclid(ms) * ms) as f64;
            match out.last_mut() {
                Some(b) if b[0] == k => {
                    b[2] = b[2].max(m[2]);
                    b[3] = b[3].min(m[3]);
                    b[4] = m[4];
                    b[5] += m[5];
                }
                _ => out.push([k, m[1], m[2], m[3], m[4], m[5]]),
            }
        }
        return Ok(ChartBars { bars: out, last_minute });
    }
    let span = match interval.as_str() {
        "daily" => "day",
        "weekly" => "week",
        _ => "month",
    };
    // In a replay the replay day's own bar would hold the whole day; take history to the day
    // before and build the replay day from its minutes up to the clock.
    let hist_to = if until_ms.is_some() { to - chrono::Duration::days(1) } else { to };
    let from = to - chrono::Duration::days(lookback_days(bars, &interval));
    let midnight = |ms: f64| {
        ny_date(ms as i64).map(|d| d.and_hms_opt(0, 0, 0).expect("midnight").and_utc().timestamp_millis() as f64).unwrap_or(ms)
    };
    let mut out: Vec<[f64; 6]> = rest_bars(app2, &rest, &symbol, span, from, hist_to, today)?.iter().map(|b| [midnight(b[0]), b[1], b[2], b[3], b[4], b[5]]).collect();
    if until_ms.is_some() && span == "day" {
        let mins = rest_bars(app2, &rest, &symbol, "minute", to, to, today)?;
        let mut day: Option<[f64; 6]> = None;
        for m in mins.iter().filter(|m| cut(m[0])) {
            day = Some(match day {
                None => [midnight(m[0]), m[1], m[2], m[3], m[4], m[5]],
                Some(d) => [d[0], d[1], d[2].max(m[2]), d[3].min(m[3]), m[4], d[5] + m[5]],
            });
        }
        out.extend(day);
    }
    Ok(ChartBars { bars: out, last_minute: None })
}

/// The monitor a group tiles onto: groups in order go to monitors left to right; a group with no
/// monitor of its own uses the primary.
fn group_monitor(handle: &AppHandle, app: &App, group: &str) -> Option<tauri::Monitor> {
    let idx = app.panels.lock().expect("panels").groups().iter().position(|g| g == group);
    let mut mons = handle.available_monitors().ok()?;
    mons.sort_by_key(|m| (m.position().x, m.position().y));
    idx.and_then(|i| mons.get(i).cloned()).or_else(|| handle.primary_monitor().ok().flatten()).or_else(|| mons.into_iter().next())
}

fn open_panel_window(handle: &AppHandle, app: &App, id: &str) -> Result<(), String> {
    let tile = {
        let g = app.panels.lock().expect("panels");
        let p = g.get(id).ok_or_else(|| format!("no panel `{id}`"))?;
        p.tile.map(|t| (p.group.clone(), t))
    };
    let rec = WinRec { label: format!("panel-{id}"), kind: "panel".into(), arg: id.to_string() };
    open_window(handle, app, &rec, tile)?;
    remember(app, rec);
    Ok(())
}

/// Open (or focus) a window, placed where it last was; the first time, on its group's tile.
fn open_window(handle: &AppHandle, app: &App, rec: &WinRec, tile: Option<(String, [f64; 4])>) -> Result<(), String> {
    if let Some(w) = handle.get_webview_window(&rec.label) {
        let _ = w.unminimize();
        return w.set_focus().map_err(|e| e.to_string());
    }
    let (route, w, h) = match rec.kind.as_str() {
        "panel" => (format!("#/panel/{}", rec.arg), 420.0, 300.0),
        "chart" => (format!("#/chart/{}/{}", rec.label, rec.arg), 900.0, 600.0),
        k => return Err(format!("unknown window kind `{k}`")),
    };
    // No OS title bar: the panel or chart header is the bar (drag to move, double-click to
    // maximize, its own minimize and close). The edges still resize.
    let win = WebviewWindowBuilder::new(handle, &rec.label, WebviewUrl::App(route.into()))
        .title(if rec.kind == "panel" { "Panel" } else { "Chart" })
        .inner_size(w, h)
        .min_inner_size(160.0, 60.0)
        .decorations(false)
        // No shadow: on Windows 11 the shadow brings rounded corners and a border, which read as a
        // gap between windows that touch.
        .shadow(false)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    let saved = app.ui.lock().expect("ui").geom.get(&rec.label).copied().filter(|g| on_screen(handle, g));
    // Reopened at launch, the caller doesn't pass the tile; a panel or group chart still has one.
    let tile = tile.or_else(|| {
        let g = app.panels.lock().expect("panels");
        match rec.kind.as_str() {
            "panel" => g.get(&rec.arg).and_then(|p| p.tile.map(|t| (p.group.clone(), t))),
            _ => g.charts.iter().find(|c| c.label == rec.label).and_then(|c| c.tile.map(|t| (c.group.clone(), t))),
        }
    });
    let place = |win: &WebviewWindow| {
        if let Some(g) = saved {
            let _ = win.set_size(PhysicalSize::new(g.w, g.h));
            let _ = win.set_position(PhysicalPosition::new(g.x, g.y));
        } else if let Some((group, t)) = &tile {
            if let Some(m) = group_monitor(handle, app, group) {
                place_on_tile(win, &m, *t);
            }
        }
    };
    place(&win);
    win.show().map_err(|e| e.to_string())?;
    // A frameless window measures its frame as if it had a title bar until it's shown (it came
    // back 30px taller): place it again, now that it's visible.
    place(&win);
    Ok(())
}

/// Whether a saved place is still reachable: its title bar's left end is on a connected monitor.
/// A window saved on a monitor that's gone (unplugged, or the desk rearranged) would reopen where
/// nobody can see or grab it; it goes back to its tile or the default spot instead. (TradierPro's
/// off-screen recovery, the same rule.)
fn on_screen(handle: &AppHandle, g: &Geom) -> bool {
    let (x, y) = (g.x + 40, g.y + 12);
    handle.available_monitors().map_or(true, |ms| {
        ms.iter().any(|m| {
            let (p, s) = (m.position(), m.size());
            x >= p.x && y >= p.y && x < p.x + s.width as i32 && y < p.y + s.height as i32
        })
    })
}

/// Put a window's visible rectangle exactly on a fraction of a monitor's work area, so tiled
/// neighbours touch with no gap (their invisible resize borders overlap instead).
fn place_on_tile(win: &WebviewWindow, m: &tauri::Monitor, [fx, fy, fw, fh]: [f64; 4]) {
    let wa = m.work_area();
    let (ax, ay, aw, ah) = (wa.position.x as f64, wa.position.y as f64, wa.size.width as f64, wa.size.height as f64);
    let px = |v: f64| v.round() as i32;
    let rect = (px(ax + fx * aw), px(ay + fy * ah), px(ax + (fx + fw) * aw), px(ay + (fy + fh) * ah));
    let f = frame_of(win).unwrap_or(Frame { l: 0, t: 0, r: 0, b: 0, inset_l: 0, inset_t: 0 });
    let g = set_frame(win, &f, rect);
    if let Some(app) = win.app_handle().try_state::<App>() {
        app.snapped.lock().expect("snapped").insert(win.label().to_string(), g);
    }
}

fn remember(app: &App, rec: WinRec) {
    let mut ui = app.ui.lock().expect("ui");
    ui.windows.retain(|r| r.label != rec.label);
    ui.windows.push(rec);
    drop(ui);
    app.save_ui();
}

#[tauri::command]
async fn open_chart(handle: AppHandle, app: State<'_, App>, color: String) -> Result<(), String> {
    let n = (1..).find(|n| handle.get_webview_window(&format!("chart-{n}")).is_none()).expect("a free number");
    let rec = WinRec { label: format!("chart-{n}"), kind: "chart".into(), arg: color };
    open_window(&handle, &app, &rec, None)?;
    remember(&app, rec);
    Ok(())
}

#[tauri::command]
fn set_chart_color(handle: AppHandle, app: State<'_, App>, label: String, color: String) -> Result<(), String> {
    if !layout::LINK_COLORS.contains(&color.as_str()) {
        return Err(format!("no link colour `{color}`"));
    }
    let mut ui = app.ui.lock().expect("ui");
    if let Some(r) = ui.windows.iter_mut().find(|r| r.label == label) {
        r.arg = color.clone();
    }
    drop(ui);
    app.save_ui();
    // A group chart keeps its colour when the group is reopened.
    let changed = {
        let mut g = app.panels.lock().expect("panels");
        g.charts.iter_mut().find(|c| c.label == label).map(|c| c.color = color).is_some()
    };
    if changed {
        app.save_panels(&handle);
    }
    Ok(())
}

fn tool_context(app: &App) -> tools::Context {
    let g = app.shared.lock().expect("shared");
    tools::Context {
        program: g.program.clone(),
        daily: g.daily.clone(),
        references: g.references.clone(),
        replay_date: g.replay_date,
        feed_start: g.feed_start,
        data_dir: app.data_dir.clone(),
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

/// Why `symbol` does or doesn't fire `filter` over the day (the replay's day, or today live).
#[tauri::command]
async fn diagnose(app: State<'_, App>, filter: String, symbol: String) -> Result<tools::Diagnosis, String> {
    let cx = tool_context(&app);
    let rest = app.rest.clone();
    blocking(move || tools::diagnose(&cx, rest.as_deref(), &filter, &symbol)).await
}

fn item_kind(kind: Option<&str>) -> Result<tools::ItemKind, String> {
    match kind.unwrap_or("filter") {
        "filter" => Ok(tools::ItemKind::Filter),
        "basket" => Ok(tools::ItemKind::Basket),
        k => Err(format!("unknown item kind `{k}`")),
    }
}

#[tauri::command]
fn filter_source(app: State<'_, App>, name: String, kind: Option<String>) -> Result<tools::FilterText, String> {
    tools::source(&app.filters_dir, item_kind(kind.as_deref())?, &name)
}

/// How many symbols a basket (as edited, not yet saved) lets through on the latest daily data.
#[tauri::command]
async fn basket_count(app: State<'_, App>, name: String, block: String) -> Result<tools::BasketCount, String> {
    let cx = tool_context(&app);
    let dir = app.filters_dir.clone();
    blocking(move || {
        let a = tools::apply(&dir, tools::ItemKind::Basket, &name, &block)?;
        let def = a.program.baskets.iter().find(|b| *b.name == *name).ok_or_else(|| format!("no basket `{name}` after the edit"))?;
        tools::basket_count(&cx, def)
    })
    .await
}

/// The edit checked (it must load) and run against the unedited filter over a recorded day.
#[tauri::command]
async fn filter_test(app: State<'_, App>, name: String, block: String) -> Result<tools::TestResult, String> {
    let cx = tool_context(&app);
    let dir = app.filters_dir.clone();
    blocking(move || {
        let a = tools::apply(&dir, tools::ItemKind::Filter, &name, &block)?;
        tools::test(&cx, &name, &a.program)
    })
    .await
}

fn reload_now(app: &App) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    app.send(Cmd::Reload(tx))?;
    rx.recv_timeout(Duration::from_secs(60)).map_err(|_| "no answer from the engine".to_string())?
}

/// Prepend a dated entry to filters/CHANGES.md, under its introduction.
fn log_change(dir: &std::path::Path, entry: &str) {
    let p = dir.join("CHANGES.md");
    let old = std::fs::read_to_string(&p).unwrap_or_else(|_| "# Filter changes\n\n".into());
    let at = old.find("\n## ").map(|i| i + 1).unwrap_or(old.len());
    let rest = if at < old.len() { format!("\n{}", &old[at..]) } else { String::new() };
    let _ = std::fs::write(&p, format!("{}{}\n{}", &old[..at], entry.trim_end(), rest));
}

fn now_stamp() -> (String, String) {
    let t = tp_marketdata::time::Ts::from_millis(scanner::now_ms()).expect("now").ny();
    (t.format("%Y-%m-%d").to_string(), t.format("%Y%m%d-%H%M%S").to_string())
}

/// Save an edit: back the file up, write it, reload the engine (a failed reload puts the old file
/// back), and log the change with the user's reason and the test result.
#[tauri::command]
async fn filter_save(app: State<'_, App>, name: String, block: String, reason: String, tested: Option<String>, kind: Option<String>) -> Result<String, String> {
    let kind = item_kind(kind.as_deref())?;
    let a = tools::apply(&app.filters_dir, kind, &name, &block)?;
    if a.new_text == a.old_text {
        return Err("nothing changed".into());
    }
    let existed = a.path.exists();
    let (day, stamp) = now_stamp();
    let hist = app.data_dir.join("filter-history");
    std::fs::create_dir_all(&hist).map_err(|e| e.to_string())?;
    let file = a.path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let backup = hist.join(format!("{file}.{stamp}"));
    std::fs::write(&backup, &a.old_text).map_err(|e| format!("backup {}: {e}", backup.display()))?;
    std::fs::write(&a.path, &a.new_text).map_err(|e| format!("{}: {e}", a.path.display()))?;
    match reload_now(&app) {
        Ok(msg) => {
            let reason = if reason.trim().is_empty() { "(no reason given)".to_string() } else { reason.trim().to_string() };
            let what = if kind == tools::ItemKind::Basket { format!("basket {name}") } else { name.clone() };
            let verb = if a.old_block.is_empty() { "added" } else { "edited" };
            let mut entry = format!("## {day}: {what} (`{file}`), {verb} in the app\n\n**Why:** {reason}\n\n```diff\n{}\n```\n", tools::line_diff(&a.old_block, block.trim()));
            if let Some(t) = tested.filter(|t| !t.trim().is_empty()) {
                entry.push_str(&format!("\n**Tested:** {}\n", t.trim()));
            }
            entry.push_str(&format!("\nBackup of the previous file: `filter-history/{file}.{stamp}`.\n"));
            log_change(&app.filters_dir, &entry);
            Ok(format!("saved {file}; {msg}"))
        }
        Err(e) => {
            if existed {
                let _ = std::fs::write(&a.path, &a.old_text);
            } else {
                let _ = std::fs::remove_file(&a.path);
            }
            let _ = reload_now(&app);
            Err(format!("not saved (the old file is back): {e}"))
        }
    }
}

/// Put back the file holding `name` as it was before its newest saved edit.
#[tauri::command]
async fn filter_undo(app: State<'_, App>, name: String, kind: Option<String>) -> Result<String, String> {
    let src = tools::source(&app.filters_dir, item_kind(kind.as_deref())?, &name)?;
    let hist = app.data_dir.join("filter-history");
    let prefix = format!("{}.", src.file);
    let newest = std::fs::read_dir(&hist)
        .map_err(|_| "no saved edits to undo".to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|f| f.to_str()).is_some_and(|f| f.starts_with(&prefix) && !f.ends_with(".undone")))
        .max()
        .ok_or_else(|| format!("no saved edits of {} to undo", src.file))?;
    let path = app.filters_dir.join(&src.file);
    let current = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let restore = std::fs::read_to_string(&newest).map_err(|e| e.to_string())?;
    std::fs::write(&path, &restore).map_err(|e| e.to_string())?;
    match reload_now(&app) {
        Ok(msg) => {
            let used = newest.file_name().and_then(|f| f.to_str()).unwrap_or_default().to_string();
            let _ = std::fs::rename(&newest, newest.with_file_name(format!("{used}.undone")));
            let (day, _) = now_stamp();
            log_change(&app.filters_dir, &format!("## {day}: `{}` restored from `filter-history/{used}` (undo in the app)\n", src.file));
            Ok(format!("{} restored; {msg}", src.file))
        }
        Err(e) => {
            let _ = std::fs::write(&path, &current);
            let _ = reload_now(&app);
            Err(format!("not undone (the backup doesn't load): {e}"))
        }
    }
}

/// The panel menu, as its own small window at the mouse pointer: a panel can be too small to
/// hold it. It sits on top, and closes itself when it loses focus.
#[tauri::command]
async fn open_menu(handle: AppHandle, panel: String, symbol: Option<String>, filter: Option<String>) -> Result<(), String> {
    if let Some(w) = handle.get_webview_window("popup-menu") {
        let _ = w.destroy();
    }
    let enc = |s: Option<String>| s.filter(|s| !s.is_empty()).map(|s| s.replace('/', "_")).unwrap_or_else(|| "-".into());
    let route = format!("#/menu/{panel}/{}/{}", enc(symbol), enc(filter));
    let at = handle.cursor_position().map_err(|e| e.to_string())?;
    let w = WebviewWindowBuilder::new(&handle, "popup-menu", WebviewUrl::App(route.into()))
        .title("Menu")
        .inner_size(240.0, 300.0)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    let _ = w.set_position(PhysicalPosition::new(at.x as i32, at.y as i32));
    Ok(())
}

/// The menu window, sized to its content (CSS px) and kept on its monitor, then shown.
#[tauri::command]
async fn popup_fit(window: WebviewWindow, width: f64, height: f64) -> Result<(), String> {
    let scale = window.scale_factor().unwrap_or(1.0);
    let (w, h) = ((width * scale).ceil() as i32, (height * scale).ceil() as i32);
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let (mut x, mut y) = (pos.x, pos.y);
    if let Ok(Some(m)) = window.current_monitor() {
        let a = m.work_area();
        let (x0, y0, x1, y1) = (a.position.x, a.position.y, a.position.x + a.size.width as i32, a.position.y + a.size.height as i32);
        x = x.min(x1 - w).max(x0);
        y = y.min(y1 - h).max(y0);
    }
    let _ = window.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = window.set_position(PhysicalPosition::new(x, y));
    window.show().map_err(|e| e.to_string())?;
    let _ = window.set_size(PhysicalSize::new(w as u32, h as u32));
    window.set_focus().map_err(|e| e.to_string())
}

/// A tool window: `why` (filter + symbol) or `edit` (filter).
#[tauri::command]
async fn open_dialog(handle: AppHandle, app: State<'_, App>, kind: String, filter: String, symbol: Option<String>) -> Result<(), String> {
    let clean = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect::<String>();
    let (label, route, w, h) = match kind.as_str() {
        "why" => {
            let sym = symbol.unwrap_or_default().trim().to_uppercase();
            if sym.is_empty() {
                return Err("which symbol?".into());
            }
            (format!("dialog-why-{}-{}", clean(&filter), clean(&sym)), format!("#/why/{filter}/{sym}"), 640.0, 560.0)
        }
        "edit" => (format!("dialog-edit-{}", clean(&filter)), format!("#/edit/{filter}"), 760.0, 620.0),
        "basket" => (format!("dialog-basket-{}", clean(&filter)), format!("#/basket/{filter}"), 640.0, 640.0),
        "panel" => (format!("dialog-panel-{}", clean(&filter)), format!("#/panelset/{filter}"), 420.0, 620.0),
        k => return Err(format!("unknown dialog `{k}`")),
    };
    if let Some(w) = handle.get_webview_window(&label) {
        let _ = w.unminimize();
        return w.set_focus().map_err(|e| e.to_string());
    }
    let win = WebviewWindowBuilder::new(&handle, &label, WebviewUrl::App(route.into()))
        .title(match kind.as_str() {
            "why" => "Why",
            "basket" => "Basket",
            "panel" => "Panel settings",
            _ => "Adjust filter",
        })
        .inner_size(w, h)
        .decorations(false)
        // No shadow: on Windows 11 the shadow brings rounded corners and a border, which read as a
        // gap between windows that touch.
        .shadow(false)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    let saved = app.ui.lock().expect("ui").geom.get(&label).copied().filter(|g| on_screen(&handle, g));
    let place = |win: &WebviewWindow| {
        if let Some(g) = saved {
            let _ = win.set_size(PhysicalSize::new(g.w, g.h));
            let _ = win.set_position(PhysicalPosition::new(g.x, g.y));
        }
    };
    place(&win);
    win.show().map_err(|e| e.to_string())?;
    place(&win); // again once visible (see open_window)
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

/// Record a window's place after a move or resize (not while minimized: Windows parks those at
/// -32000; not while maximized: restoring would come back full-screen).
fn note_geometry(w: &tauri::Window) {
    if w.is_minimized().unwrap_or(false) || w.is_maximized().unwrap_or(false) {
        return;
    }
    let (Ok(p), Ok(s)) = (w.outer_position(), w.inner_size()) else { return };
    let app = w.app_handle().state::<App>();
    let g = Geom { x: p.x, y: p.y, w: s.width, h: s.height };
    let mut ui = app.ui.lock().expect("ui");
    if ui.geom.get(w.label()) != Some(&g) {
        ui.geom.insert(w.label().to_string(), g);
        app.dirty.store(true, Ordering::SeqCst);
    }
    drop(ui);
    if snappable(w.label()) && app.snapped.lock().expect("snapped").get(w.label()) != Some(&g) {
        app.moved.lock().expect("moved").insert(w.label().to_string(), std::time::Instant::now());
    }
}

// ---------------------------------------------------------------------------------------------
// Lining windows up. Windows without a title bar still have an invisible resize border outside
// what you see, so all of this works on the visible (client) rectangle: placing by the outer
// rectangle would leave a gap of two borders between neighbours.

/// Panels, charts and the launcher line up with each other; tool windows (why, adjust, basket)
/// float free.
fn snappable(label: &str) -> bool {
    label == "launcher" || label.starts_with("panel-") || label.starts_with("chart-")
}

/// A window's visible rectangle (left, top, right, bottom) and how far its outer frame reaches
/// beyond it on the left and top, physical pixels.
#[derive(Clone, Copy, Debug)]
struct Frame {
    l: i32,
    t: i32,
    r: i32,
    b: i32,
    inset_l: i32,
    inset_t: i32,
}

fn frame_of(w: &WebviewWindow) -> Option<Frame> {
    let (op, ip, is) = (w.outer_position().ok()?, w.inner_position().ok()?, w.inner_size().ok()?);
    Some(Frame { l: ip.x, t: ip.y, r: ip.x + is.width as i32, b: ip.y + is.height as i32, inset_l: ip.x - op.x, inset_t: ip.y - op.y })
}

/// Put a window's visible rectangle exactly at (l, t, r, b). Returns the geometry it will report.
fn set_frame(w: &WebviewWindow, f: &Frame, (l, t, r, b): (i32, i32, i32, i32)) -> Geom {
    let (wd, ht) = ((r - l).max(60) as u32, (b - t).max(40) as u32);
    let _ = w.set_size(PhysicalSize::new(wd, ht));
    let _ = w.set_position(PhysicalPosition::new(l - f.inset_l, t - f.inset_t));
    Geom { x: l - f.inset_l, y: t - f.inset_t, w: wd, h: ht }
}

/// How close (physical px) an edge must be to another to snap to it when you let go of a window.
const SNAP: i32 = 18;
/// Tidy's reach: wide enough for the gaps the first versions left (17 across, 39 down).
const TIDY: i32 = 45;

/// The nearest of `candidates` to `v`, if within `reach`.
fn nearest(v: i32, candidates: &[i32], reach: i32) -> Option<i32> {
    candidates.iter().copied().filter(|c| (c - v).abs() <= reach).min_by_key(|c| (c - v).abs())
}

/// Every snappable window that's showing, with its visible rectangle, read once (each read is a
/// round trip to the window thread; reading per pair made a tidy of 60 windows take ~10 s).
fn desk(handle: &AppHandle) -> Vec<(String, WebviewWindow, Frame)> {
    handle
        .webview_windows()
        .into_iter()
        .filter(|(l, w)| snappable(l) && w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(true) && !w.is_maximized().unwrap_or(true))
        .filter_map(|(l, w)| frame_of(&w).map(|f| (l, w, f)))
        .collect()
}

/// Every monitor's work area (left, top, right, bottom).
fn work_areas(handle: &AppHandle) -> Vec<(i32, i32, i32, i32)> {
    handle
        .available_monitors()
        .map(|ms| {
            ms.iter()
                .map(|m| {
                    let a = m.work_area();
                    (a.position.x, a.position.y, a.position.x + a.size.width as i32, a.position.y + a.size.height as i32)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Snap window `i` of `desk` to its neighbours' edges and its monitor's work area. An edge snaps
/// only to windows beside it (overlapping on the other axis). One edge snapping moves the window;
/// both snapping fits it to the gap. Updates `desk[i]` and returns whether it moved.
fn snap_in(handle: &AppHandle, desk: &mut [(String, WebviewWindow, Frame)], areas: &[(i32, i32, i32, i32)], i: usize, reach: i32) -> bool {
    let f = desk[i].2;
    let others: Vec<Frame> = desk.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, d)| d.2).collect();
    let (cx, cy) = ((f.l + f.r) / 2, (f.t + f.b) / 2);
    let wa = areas.iter().copied().find(|&(x0, y0, x1, y1)| cx >= x0 && cx < x1 && cy >= y0 && cy < y1);
    let beside_v = |o: &Frame| o.t < f.b + reach && o.b > f.t - reach;
    let beside_h = |o: &Frame| o.l < f.r + reach && o.r > f.l - reach;
    let mut xl: Vec<i32> = others.iter().filter(|o| beside_v(o)).flat_map(|o| [o.r, o.l]).collect();
    let mut xr = xl.clone();
    let mut yt: Vec<i32> = others.iter().filter(|o| beside_h(o)).flat_map(|o| [o.b, o.t]).collect();
    let mut yb = yt.clone();
    if let Some((x0, y0, x1, y1)) = wa {
        xl.push(x0);
        xr.push(x1);
        yt.push(y0);
        yb.push(y1);
    }
    let fit = |a: i32, b: i32, na: Option<i32>, nb: Option<i32>| match (na, nb) {
        (Some(a2), Some(b2)) if b2 - a2 >= 40 => (a2, b2),
        (Some(a2), _) => (a2, b + (a2 - a)),
        (None, Some(b2)) => (a + (b2 - b), b2),
        _ => (a, b),
    };
    let (l, r) = fit(f.l, f.r, nearest(f.l, &xl, reach), nearest(f.r, &xr, reach));
    let (t, b) = fit(f.t, f.b, nearest(f.t, &yt, reach), nearest(f.b, &yb, reach));
    if (l, t, r, b) == (f.l, f.t, f.r, f.b) {
        return false;
    }
    let g = set_frame(&desk[i].1, &f, (l, t, r, b));
    handle.state::<App>().snapped.lock().expect("snapped").insert(desk[i].0.clone(), g);
    desk[i].2 = Frame { l, t, r, b, ..f };
    true
}

/// Snap one window (the one the user just let go of).
fn snap_window(handle: &AppHandle, label: &str, reach: i32) -> bool {
    let mut d = desk(handle);
    let areas = work_areas(handle);
    match d.iter().position(|x| x.0 == label) {
        Some(i) => snap_in(handle, &mut d, &areas, i, reach),
        None => false,
    }
}

/// Close the gaps between every open panel and chart, as they are now arranged.
fn tidy(handle: &AppHandle) -> usize {
    let mut d = desk(handle);
    let areas = work_areas(handle);
    let mut moved = 0;
    for _ in 0..4 {
        let mut any = false;
        for i in 0..d.len() {
            if snap_in(handle, &mut d, &areas, i, TIDY) {
                moved += 1;
                any = true;
            }
        }
        if !any {
            break;
        }
    }
    moved
}

#[tauri::command]
async fn tidy_windows(handle: AppHandle) -> Result<String, String> {
    Ok(format!("{} window edge adjustments", tidy(&handle)))
}

/// Every open panel and group chart back on its Primus grid tile.
#[tauri::command]
async fn arrange_windows(handle: AppHandle, app: State<'_, App>) -> Result<String, String> {
    let tiles: Vec<(String, String, [f64; 4])> = {
        let g = app.panels.lock().expect("panels");
        g.panels
            .iter()
            .filter_map(|p| p.tile.map(|t| (format!("panel-{}", p.id), p.group.clone(), t)))
            .chain(g.charts.iter().filter_map(|c| c.tile.map(|t| (c.label.clone(), c.group.clone(), t))))
            .collect()
    };
    let mut n = 0;
    for (label, group, t) in tiles {
        let (Some(w), Some(m)) = (handle.get_webview_window(&label), group_monitor(&handle, &app, &group)) else { continue };
        let _ = w.unminimize();
        place_on_tile(&w, &m, t);
        n += 1;
    }
    Ok(format!("{n} windows arranged on the Primus grid"))
}

fn main() {
    let data_dir = tp_massive::data_dir().unwrap_or_else(|_| std::env::temp_dir().join("tp-scanner"));
    let _ = std::fs::create_dir_all(&data_dir);
    let ui_path = data_dir.join("ui.json");
    let ui = load_ui(&ui_path);
    let filters = filters_dir();
    let panels_path = data_dir.join("panels.json");
    let panels = load_panels(&panels_path, &filters);
    if !panels_path.exists() {
        write_json(&panels_path, &panels);
    }
    let rest = tp_massive::Secret::from_env(tp_massive::API_KEY_VAR).ok().and_then(|k| tp_massive::rest::Rest::new(k).ok()).map(Arc::new);
    let (tx, rx) = mpsc::channel();
    let shared = Arc::new(Mutex::new(Shared { links: ui.links.clone(), follow: follow_map(&panels), ..Shared::default() }));
    // Windows from before the panel rewrite (`screen-*`) no longer exist; drop them.
    let reopen: Vec<WinRec> = ui.windows.iter().filter(|r| r.kind == "panel" || r.kind == "chart").cloned().collect();
    let launcher_geom = ui.geom.get("launcher").copied();
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
        .manage(App {
            cmd: Mutex::new(tx),
            shared: shared.clone(),
            ui: Mutex::new(ui),
            ui_path,
            panels: Mutex::new(panels),
            panels_path,
            filters_dir: filters.clone(),
            data_dir: data_dir.clone(),
            dirty: AtomicBool::new(false),
            exiting: AtomicBool::new(false),
            rest,
            chart_cache: Mutex::new(HashMap::new()),
            names: Mutex::new(None),
            moved: Mutex::new(HashMap::new()),
            snapped: Mutex::new(HashMap::new()),
        })
        .invoke_handler(tauri::generate_handler![
            get_filters,
            get_snapshot,
            reload_filters,
            start_live,
            start_replay,
            set_speed,
            stop_source,
            explain,
            link_set,
            link_get,
            panels_get,
            panel_update,
            panel_create,
            panel_delete,
            open_panel,
            open_screen,
            reset_layout,
            chart_watch,
            chart_bars,
            ticker_name,
            open_chart,
            set_chart_color,
            diagnose,
            filter_source,
            filter_test,
            filter_save,
            filter_undo,
            basket_count,
            tidy_windows,
            open_menu,
            popup_fit,
            arrange_windows,
            layouts_list,
            layout_save,
            layout_load,
            layout_delete,
            open_dialog
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("scan-engine".into())
                .spawn({
                    let handle = handle.clone();
                    move || {
                        let runner = scanner::Runner::new(handle.clone(), shared, filters, data_dir, rx);
                        let _ = handle.emit("filters-changed", ());
                        runner.run();
                    }
                })
                .expect("spawn engine thread");
            // A window the user moved or resized snaps once it has been still for a moment.
            std::thread::Builder::new()
                .name("snapper".into())
                .spawn({
                    let handle = handle.clone();
                    move || loop {
                        std::thread::sleep(Duration::from_millis(200));
                        let app = handle.state::<App>();
                        let due: Vec<String> = {
                            let mut m = app.moved.lock().expect("moved");
                            let due: Vec<String> = m.iter().filter(|(_, t)| t.elapsed() >= Duration::from_millis(450)).map(|(l, _)| l.clone()).collect();
                            for l in &due {
                                m.remove(l);
                            }
                            due
                        };
                        for l in due {
                            snap_window(&handle, &l, SNAP);
                        }
                    }
                })
                .expect("spawn snapper");
            // Geometry is saved a second after the last move, not on every pixel of a drag.
            std::thread::Builder::new()
                .name("ui-saver".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_secs(1));
                    let app = handle.state::<App>();
                    let links_dirty = app.shared.lock().expect("shared").links_dirty;
                    if app.dirty.load(Ordering::SeqCst) || links_dirty {
                        app.save_ui();
                    }
                })
                .expect("spawn ui saver");
            // The launcher bar stays above the panel and chart windows.
            if let Some(w) = app.get_webview_window("launcher") {
                let _ = w.set_always_on_top(true);
            }
            if let (Some(g), Some(w)) = (launcher_geom.filter(|g| on_screen(app.handle(), g)), app.get_webview_window("launcher")) {
                let _ = w.set_size(PhysicalSize::new(g.w, g.h));
                let _ = w.set_position(PhysicalPosition::new(g.x, g.y));
            }
            let state = app.state::<App>();
            for rec in &reopen {
                if let Err(e) = open_window(app.handle(), &state, rec, None) {
                    eprintln!("reopen {}: {e}", rec.label);
                }
            }
            if state.ui.lock().expect("ui").geom_version < 2 {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(4)); // the reopened windows settle first
                    tidy(&handle);
                    let app = handle.state::<App>();
                    app.ui.lock().expect("ui").geom_version = 2;
                    app.dirty.store(true, Ordering::SeqCst);
                });
            }
            Ok(())
        })
        .on_window_event(|w, ev| {
            let label = w.label().to_string();
            match ev {
                WindowEvent::Moved(_) | WindowEvent::Resized(_) => note_geometry(w),
                WindowEvent::CloseRequested { .. } if label == "launcher" => {
                    // Closing the launcher closes the app; the open windows are remembered.
                    let app = w.app_handle().state::<App>();
                    app.exiting.store(true, Ordering::SeqCst);
                    app.save_ui();
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
            let app = handle.state::<App>();
            app.exiting.store(true, Ordering::SeqCst);
            app.save_ui();
        }
    });
}
