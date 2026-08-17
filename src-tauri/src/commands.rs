use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::browser;
use crate::config::{AppConfig, Bookmark, ConfigState, Settings};
use crate::logger::Logger;
use crate::whitelist::{normalize_input, parse_rules, HostRule};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlCheck {
    pub allowed: bool,
    pub url: Option<String>,
    pub reason: Option<String>,
}

/// Validate and normalize an address against the current whitelist.
#[tauri::command]
pub fn check_url(app: AppHandle, raw: String) -> UrlCheck {
    let cfg = app.state::<ConfigState>().get();
    let rules = parse_rules(&cfg.whitelist);
    match normalize_input(&raw) {
        Ok(u) => match crate::whitelist::block_reason(&u, &rules) {
            None => UrlCheck { allowed: true, url: Some(u.to_string()), reason: None },
            Some(r) => UrlCheck { allowed: false, url: Some(u.to_string()), reason: Some(r) },
        },
        Err(e) => UrlCheck { allowed: false, url: None, reason: Some(e) },
    }
}

#[tauri::command]
pub fn get_config(app: AppHandle) -> AppConfig {
    app.state::<ConfigState>().get()
}

/// Add a domain to the whitelist (idempotent).
#[tauri::command]
pub fn add_domain(app: AppHandle, domain: String) -> Result<AppConfig, String> {
    let rule = HostRule::parse(&domain).ok_or_else(|| format!("'{domain}' is not a valid domain."))?;
    let mut cfg = app.state::<ConfigState>().get();
    let entry = match rule.port {
        Some(p) => format!("{}:{p}", rule.host),
        None => rule.host,
    };
    if !cfg.whitelist.iter().any(|w| w.eq_ignore_ascii_case(&entry)) {
        cfg.whitelist.push(entry);
        app.state::<ConfigState>().save(cfg.clone())?;
        app.state::<Logger>().log("whitelist_added", serde_json::json!({ "domain": domain }));
        browser::reload_policy(&app);
    }
    Ok(cfg)
}

#[tauri::command]
pub fn remove_domain(app: AppHandle, domain: String) -> Result<AppConfig, String> {
    let mut cfg = app.state::<ConfigState>().get();
    cfg.whitelist.retain(|w| !w.eq_ignore_ascii_case(&domain));
    app.state::<ConfigState>().save(cfg.clone())?;
    app.state::<Logger>().log("whitelist_removed", serde_json::json!({ "domain": domain }));
    browser::reload_policy(&app);
    Ok(cfg)
}

/// Add a bookmark shortcut. The host is auto-added to the whitelist so the
/// shortcut is immediately usable.
#[tauri::command]
pub fn add_bookmark(
    app: AppHandle,
    name: String,
    url: String,
    color: Option<String>,
) -> Result<AppConfig, String> {
    let parsed = normalize_input(&url)?;
    let mut cfg = app.state::<ConfigState>().get();
    let rules = parse_rules(&cfg.whitelist);
    if let Some(reason) = crate::whitelist::block_reason(&parsed, &rules) {
        return Err(reason);
    }
    if let Some(host) = parsed.host_str().map(|h| h.to_lowercase()) {
        if !cfg.whitelist.iter().any(|w| w.eq_ignore_ascii_case(&host)) {
            cfg.whitelist.push(host);
        }
    }
    let pretty_name = if name.trim().is_empty() {
        parsed.host_str().unwrap_or("Bookmark").to_string()
    } else {
        name.clone()
    };
    cfg.bookmarks.retain(|b| !b.name.eq_ignore_ascii_case(&pretty_name));
    cfg.bookmarks.push(Bookmark {
        name: pretty_name,
        url: parsed.to_string(),
        color,
    });
    app.state::<ConfigState>().save(cfg.clone())?;
    app.state::<Logger>().log("bookmark_added", serde_json::json!({ "name": name, "url": parsed.to_string() }));
    browser::reload_policy(&app);
    Ok(cfg)
}

#[tauri::command]
pub fn remove_bookmark(app: AppHandle, name: String) -> Result<AppConfig, String> {
    let mut cfg = app.state::<ConfigState>().get();
    cfg.bookmarks.retain(|b| !b.name.eq_ignore_ascii_case(&name));
    app.state::<ConfigState>().save(cfg.clone())?;
    app.state::<Logger>().log("bookmark_removed", serde_json::json!({ "name": name }));
    Ok(cfg)
}

#[tauri::command]
pub fn update_settings(app: AppHandle, settings: Settings) -> Result<AppConfig, String> {
    let mut cfg = app.state::<ConfigState>().get();
    cfg.settings = settings;
    app.state::<ConfigState>().save(cfg.clone())?;
    let theme = cfg.settings.theme.clone();
    app.state::<Logger>().log("settings_changed", serde_json::json!({ "theme": theme }));
    Ok(cfg)
}

/// Open a whitelisted site in a brand-new tab.
#[tauri::command]
pub fn open_url(app: AppHandle, raw: String) -> Result<browser::TabModel, String> {
    browser::create_tab(&app, &raw)
}

/// Navigate an existing tab to a whitelisted site.
#[tauri::command]
pub fn navigate_tab(app: AppHandle, label: String, raw: String) -> Result<browser::TabModel, String> {
    browser::navigate(&app, &label, &raw)
}

#[tauri::command]
pub fn close_tab(app: AppHandle, label: String) {
    browser::close_tab(&app, &label);
}

#[tauri::command]
pub fn set_active(app: AppHandle, label: String, is_home: bool) {
    browser::set_active(&app, &label, is_home);
}

/// Toggle the extracted-data right panel. Shrinks child webviews so the panel
/// is not covered by the active site.
#[tauri::command]
pub fn set_extraction_panel(app: AppHandle, open: bool) {
    browser::set_extraction_panel(&app, open);
}

#[tauri::command]
pub fn go_back(app: AppHandle, label: String) {
    browser::go_back(&app, &label);
}

#[tauri::command]
pub fn go_forward(app: AppHandle, label: String) {
    browser::go_forward(&app, &label);
}

#[tauri::command]
pub fn reload_tab(app: AppHandle, label: String) {
    browser::reload(&app, &label);
}

#[tauri::command]
pub fn stop_tab(app: AppHandle, label: String) {
    browser::stop_load(&app, &label);
}

/// A canonical home-tab model (the frontend keeps its own copy for bookkeeping).
#[tauri::command]
pub fn home_tab() -> browser::TabModel {
    browser::TabModel::home()
}

#[tauri::command]
pub fn get_tabs(app: AppHandle) -> Vec<browser::TabModel> {
    browser::all_tabs(&app)
}

/// Ask the broker tab currently showing `broker` to capture its chart canvas
/// and kick off an OpenCode visual analysis. Returns the triggered tab label.
#[tauri::command]
pub fn trigger_chart_capture(app: AppHandle, broker: String) -> Result<String, String> {
    browser::trigger_chart_capture(&app, &broker)
}