use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize,
    Webview, WebviewBuilder, WebviewUrl, Window, Wry,
};
use tauri::webview::{NewWindowFeatures, NewWindowResponse, PageLoadEvent, PageLoadPayload};
use url::Url;

use crate::config::ConfigState;
use crate::logger::Logger;
use crate::whitelist::{block_reason, normalize_input, parse_rules};

/// Layout constants (logical pixels). Keep in sync with `src/lib/app.css`.
pub const SIDEBAR_W: f64 = 216.0;
pub const CHROME_H: f64 = 96.0;
/// Width of the toggleable extracted-data panel on the right edge.
pub const EXTRACT_W: f64 = 360.0;

/// Number of child webviews pre-created at startup.
///
/// Windows (wry#583) deadlocks when creating WebViews from running commands,
/// so tabs are drawn from a fixed pool created in `setup`; the pool is the
/// concurrency ceiling for live tabs.
pub const MAX_TABS: usize = 16;

const EVT_TAB: &str = "tab-event";
const EVT_BLOCKED: &str = "nav-blocked";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TabState {
    Home,
    Loading,
    Loaded,
    Blocked,
    Error,
}

impl TabState {
    pub fn as_str(&self) -> &'static str {
        match self {
            TabState::Home => "home",
            TabState::Loading => "loading",
            TabState::Loaded => "loaded",
            TabState::Blocked => "blocked",
            TabState::Error => "error",
        }
    }
}

/// Frontend-facing snapshot of one browser tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabModel {
    pub label: String,
    pub url: String,
    pub title: String,
    pub state: TabState,
    pub is_home: bool,
}

impl TabModel {
    pub fn home() -> Self {
        Self {
            label: "home".into(),
            url: String::new(),
            title: "Home".into(),
            state: TabState::Home,
            is_home: true,
        }
    }
}

/// Backend bookkeeping for one live (non-home) tab.
#[derive(Clone)]
pub(crate) struct TabEntry {
    label: String,
    url: String,
    title: String,
    state: TabState,
}

pub struct BrowserState(pub(crate) Mutex<HashMap<String, TabEntry>>);

impl Default for BrowserState {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

/// Whether the extracted-data right panel is open. Child webviews must shrink
/// so the panel is not covered by the active site.
pub struct ExtractionPanel(pub(crate) Mutex<bool>);

impl Default for ExtractionPanel {
    fn default() -> Self {
        Self(Mutex::new(false))
    }
}

fn rules(app: &AppHandle) -> Vec<crate::whitelist::HostRule> {
    let cfg = app.state::<ConfigState>().get();
    parse_rules(&cfg.whitelist)
}

fn emit_model(app: &AppHandle, model: &TabModel) {
    let _ = app.emit(EVT_TAB, model.clone());
}

fn model_from(entry: &TabEntry) -> TabModel {
    TabModel {
        label: entry.label.clone(),
        url: entry.url.clone(),
        title: entry.title.clone(),
        state: entry.state,
        is_home: false,
    }
}

fn set_entry_state(app: &AppHandle, label: &str, state: TabState, url: Option<&str>) {
    let state_guard = app.state::<BrowserState>();
    let mut map = state_guard.0.lock().unwrap();
    if let Some(entry) = map.get_mut(label) {
        entry.state = state;
        if let Some(u) = url {
            entry.url = u.to_string();
        }
        emit_model(app, &model_from(entry));
    }
}

/// Content-region rect (logical position + physical size) for child webviews.
fn content_rect(window: &Window) -> Option<(f64, f64, u32, u32)> {
    let physical = window.inner_size().ok()?;
    let scale = window.scale_factor().ok()?;
    if scale <= 0.0 {
        return None;
    }
    let panel_open = window
        .app_handle()
        .state::<ExtractionPanel>()
        .0
        .lock()
        .map(|g| g.to_owned())
        .unwrap_or(false);
    let right_w = if panel_open { EXTRACT_W } else { 0.0 };
    let w_logical = physical.width as f64 / scale;
    let h_logical = physical.height as f64 / scale;
    if w_logical <= SIDEBAR_W + right_w || h_logical <= CHROME_H {
        return None;
    }
    let w = ((w_logical - SIDEBAR_W - right_w) * scale) as u32;
    let h = ((h_logical - CHROME_H) * scale) as u32;
    Some((SIDEBAR_W, CHROME_H, w, h))
}

fn position_webview(app: &AppHandle, label: &str) {
    let Some(window) = app.get_window("main") else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let Some((_, _, w, h)) = content_rect(&window) else {
        return;
    };
    let pos = PhysicalPosition::new((SIDEBAR_W * scale) as i32, (CHROME_H * scale) as i32);
    let size = PhysicalSize::new(w, h);
    if let Some(webview) = app.get_webview(label) {
        let _ = webview.set_position(pos);
        let _ = webview.set_size(size);
    }
}

/// Reposition every live tab after the window is resized.
pub fn relayout(app: &AppHandle) {
    let state_guard = app.state::<BrowserState>();
    let labels: Vec<String> = state_guard.0.lock().unwrap().keys().cloned().collect();
    for label in labels {
        position_webview(app, &label);
    }
}

/// Open/close the extracted-data panel; child webviews shrink to make room.
pub fn set_extraction_panel(app: &AppHandle, open: bool) {
    {
        let g = app.state::<ExtractionPanel>();
        let mut guard = g.0.lock().unwrap();
        *guard = open;
    }
    relayout(app);
}

fn nav_handler(app: AppHandle, label: String) -> Box<dyn Fn(&Url) -> bool + Send> {
    Box::new(move |url: &Url| {
        if let Some(reason) = block_reason(url, &rules(&app)) {
            app.state::<Logger>().blocked(url.as_str(), &reason, &label);
            set_entry_state(&app, &label, TabState::Blocked, Some(url.as_str()));
            let payload = serde_json::json!({ "tab": label, "url": url.as_str(), "reason": reason });
            let _ = app.emit(EVT_BLOCKED, payload.clone());
            false
        } else {
            app.state::<Logger>().navigation(url.as_str(), &label);
            true
        }
    })
}

fn new_window_handler(
    app: AppHandle,
    label: String,
) -> Box<dyn Fn(Url, NewWindowFeatures) -> NewWindowResponse<Wry> + Send> {
    Box::new(move |url, _feat| {
        if let Some(reason) = block_reason(&url, &rules(&app)) {
            app.state::<Logger>().popup_blocked(url.as_str(), &label);
            let _ = app.emit(
                EVT_BLOCKED,
                serde_json::json!({ "tab": label, "url": url.as_str(), "reason": reason }),
            );
        } else {
            // No external windows, ever: route the popup into the same tab.
            if let Some(webview) = app.get_webview(&label) {
                let _ = webview.navigate(url);
            }
        }
        NewWindowResponse::Deny
    })
}

fn page_load_handler(
    app: AppHandle,
    label: String,
) -> impl Fn(Webview, PageLoadPayload) + Send + Sync {
    move |_w, payload| {
        let url = payload.url().to_string();
        let state = match payload.event() {
            PageLoadEvent::Started => TabState::Loading,
            PageLoadEvent::Finished => TabState::Loaded,
        };
        app.state::<Logger>().tab_event(&label, &url, state.as_str());
        set_entry_state(&app, &label, state, Some(&url));
    }
}

fn title_handler(app: AppHandle, label: String) -> impl Fn(Webview, String) + Send {
    move |_w, title| {
        let state_guard = app.state::<BrowserState>();
        let mut map = state_guard.0.lock().unwrap();
        if let Some(entry) = map.get_mut(&label) {
            if !title.trim().is_empty() {
                entry.title = title;
            }
            emit_model(&app, &model_from(entry));
        }
    }
}

fn update_runtime_states(app: &AppHandle, label: &str) {
    if let Some(webview) = app.get_webview(label) {
        if let Ok(url) = webview.url() {
            set_entry_state(app, label, TabState::Loaded, Some(url.as_str()));
        }
    }
}

fn host_title(url: &Url) -> String {
    url.host_str()
        .map(|h| h.to_string())
        .unwrap_or_else(|| url.to_string())
}

fn get_entry(app: &AppHandle, label: &str) -> Option<TabEntry> {
    let state_guard = app.state::<BrowserState>();
    let guard = state_guard.0.lock().unwrap();
    guard.get(label).cloned()
}

/// Pre-create the pool of child webviews. Must run from `setup` (before the
/// event loop pumps) — creating WebViews later deadlocks on Windows (wry#583).
///
/// Every pool webview gets the broker extraction content script injected as an
/// initialization script, so it runs on each top-level document load
/// (including every whitelisted navigation / reload).
pub fn init_tabs(app: &AppHandle) -> Result<(), String> {
    let Some(window) = app.get_window("main") else {
        return Err("Main window is missing.".into());
    };
    let extraction_script = include_str!("../../content/extractor.js");
    let (x, y, w, h) = content_rect(&window).unwrap_or((SIDEBAR_W, CHROME_H, 800, 600));
    for i in 1..=MAX_TABS {
        let label = slot_label(i);
        let handler_app = app.clone();
        let builder =
            WebviewBuilder::new(label.clone(), WebviewUrl::External(blank_url()))
                .on_navigation(nav_handler(handler_app.clone(), label.clone()))
                .on_new_window(new_window_handler(handler_app.clone(), label.clone()))
                .on_page_load(page_load_handler(handler_app.clone(), label.clone()))
                .on_document_title_changed(title_handler(handler_app.clone(), label.clone()))
                .initialization_script(extraction_script);
        let webview = window
            .add_child(
                builder,
                LogicalPosition::new(x, y),
                LogicalSize::new(w as f64, h as f64),
            )
            .map_err(|e| format!("Failed to spawn browser view {label}: {e}"))?;
        let _ = webview.hide();
    }
    Ok(())
}

fn blank_url() -> url::Url {
    url::Url::parse("about:blank").expect("about:blank is a valid URL")
}

fn slot_label(i: usize) -> String {
    format!("tab-{i}")
}

/// First free slot label, or `None` when every webview is in use.
fn reserve_slot(app: &AppHandle) -> Option<String> {
    let state_guard = app.state::<BrowserState>();
    let map = state_guard.0.lock().unwrap();
    (1..=MAX_TABS)
        .map(slot_label)
        .find(|label| !map.contains_key(label))
}

/// Open a whitelisted URL in a free pool webview. Returns an error (block
/// reason) when the URL is not approved, so the frontend never creates a tab.
pub fn create_tab(app: &AppHandle, raw: &str) -> Result<TabModel, String> {
    let url = normalize_input(raw)?;
    if let Some(reason) = block_reason(&url, &rules(app)) {
        return Err(reason);
    }

    let label = reserve_slot(app).ok_or_else(|| {
        format!("All {MAX_TABS} browser tabs are in use. Close a tab and try again.")
    })?;
    {
        let state_guard = app.state::<BrowserState>();
        let mut map = state_guard.0.lock().unwrap();
        map.insert(
            label.clone(),
            TabEntry {
                label: label.clone(),
                url: url.to_string(),
                title: host_title(&url),
                state: TabState::Loading,
            },
        );
    }

    let Some(webview) = app.get_webview(&label) else {
        app.state::<BrowserState>().0.lock().unwrap().remove(&label);
        return Err("Browser view is missing.".into());
    };
    webview
        .navigate(url.clone())
        .map_err(|e| format!("Navigation failed: {e}"))?;
    position_webview(app, &label);
    let _ = webview.show();
    let _ = webview.set_focus();

    app.state::<Logger>().navigation(url.as_str(), &label);

    Ok(TabModel {
        label,
        url: url.to_string(),
        title: host_title(&url),
        state: TabState::Loading,
        is_home: false,
    })
}

/// Navigate an existing tab to a whitelisted URL.
pub fn navigate(app: &AppHandle, label: &str, raw: &str) -> Result<TabModel, String> {
    let url = normalize_input(raw)?;
    if let Some(reason) = block_reason(&url, &rules(app)) {
        return Err(reason);
    }
    {
        let state_guard = app.state::<BrowserState>();
        let mut map = state_guard.0.lock().unwrap();
        if let Some(entry) = map.get_mut(label) {
            entry.url = url.to_string();
            entry.state = TabState::Loading;
            entry.title = host_title(&url);
            let model = model_from(entry);
            drop(map);
            emit_model(app, &model);
        }
    }
    let Some(webview) = app.get_webview(label) else {
        return Err("Tab no longer exists.".into());
    };
    webview.navigate(url.clone()).map_err(|e| format!("Navigation failed: {e}"))?;

    app.state::<Logger>().navigation(url.as_str(), label);
    Ok(get_entry(app, label).map(|e| model_from(&e)).unwrap_or(TabModel {
        label: label.into(),
        url: url.to_string(),
        title: host_title(&url),
        state: TabState::Loading,
        is_home: false,
    }))
}

/// Close a tab webview and free its pool slot for reuse.
pub fn close_tab(app: &AppHandle, label: &str) {
    let state_guard = app.state::<BrowserState>();
    state_guard.0.lock().unwrap().remove(label);
    if let Some(webview) = app.get_webview(label) {
        let _ = webview.hide();
    }
}

/// Show exactly one tab's webview, or hide them all for home/UI tabs.
pub fn set_active(app: &AppHandle, label: &str, is_home: bool) {
    // Hide every other live webview first.
    let state_guard = app.state::<BrowserState>();
    let labels: Vec<String> = state_guard.0.lock().unwrap().keys().cloned().collect();
    for existing in &labels {
        if existing == label {
            continue;
        }
        if let Some(w) = app.get_webview(existing) {
            let _ = w.hide();
        }
    }
    if is_home {
        if let Some(w) = app.get_webview(label) {
            let _ = w.hide();
        }
        return;
    }
    if let Some(w) = app.get_webview(label) {
        position_webview(app, label);
        let _ = w.show();
        let _ = w.set_focus();
    }
    update_runtime_states(app, label);
}

pub fn go_back(app: &AppHandle, label: &str) {
    eval(app, label, "window.history.back()");
}

pub fn go_forward(app: &AppHandle, label: &str) {
    eval(app, label, "window.history.forward()");
}

pub fn reload(app: &AppHandle, label: &str) {
    set_entry_state(app, label, TabState::Loading, None);
    eval(app, label, "location.reload()");
}

pub fn stop_load(app: &AppHandle, label: &str) {
    eval(app, label, "window.stop(); stop()");
}

fn eval(app: &AppHandle, label: &str, js: &str) {
    if let Some(w) = app.get_webview(label) {
        let _ = w.eval(js);
    }
}

/// Tell the broker tab currently showing `broker` to capture + analyze its chart.
///
/// The content script (`content/extractor.js`) exposes `window.analyzeChart`,
/// which runs `captureChartCanvas()` and pushes the bytes to Rust via
/// `capture_and_analyze_chart`. Returns the tab label that was triggered, or an
/// error when no live tab is showing that broker.
pub fn trigger_chart_capture(app: &AppHandle, broker: &str) -> Result<String, String> {
    let slug = broker.trim().to_ascii_lowercase();
    let keyword = match slug.as_str() {
        "angel" => "angelone",
        "coinswitch" => "coinswitch",
        other => other,
    };
    let state_guard = app.state::<BrowserState>();
    let labels: Vec<String> = state_guard
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, e)| e.url.to_ascii_lowercase().contains(keyword))
        .map(|(label, _)| label.clone())
        .collect();
    drop(state_guard);
    let Some(label) = labels.first() else {
        return Err(format!(
            "No live tab is showing '{broker}' — open the broker site and log in first."
        ));
    };
    eval(
        app,
        label,
        "if (typeof window.analyzeChart === 'function') window.analyzeChart();",
    );
    Ok(label.clone())
}

/// All live tab models, for the frontend to rebuild the tab strip.
pub fn all_tabs(app: &AppHandle) -> Vec<TabModel> {
    let state_guard = app.state::<BrowserState>();
    let guard = state_guard.0.lock().unwrap();
    guard.values().map(model_from).collect()
}

/// Re-validate live tabs after whitelist edits and refresh frontend state.
pub fn reload_policy(app: &AppHandle) {
    let _ = rules(app);
    for m in all_tabs(app) {
        emit_model(app, &m);
    }
}