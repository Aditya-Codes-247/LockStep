mod browser;
mod commands;
mod config;
mod extract;
mod logger;
mod opencode;
mod whitelist;

use tauri::{Emitter, Manager};

use config::ConfigState;
use logger::Logger;
use opencode::OpencodeState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let cfg =
                ConfigState::load(&app.handle()).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            app.manage(cfg);
            let logger =
                Logger::init(&app.handle()).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            app.manage(logger);
            app.manage(browser::BrowserState::default());
            app.manage(browser::ExtractionPanel::default());
            app.manage(extract::ExtractionState::default());
            app.manage(extract::VisualState::default());
            app.manage(OpencodeState::default());
            browser::init_tabs(&app.handle()).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let _ = app.emit("app-ready", true);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::Resized(_) = event {
                    browser::relayout(&window.app_handle());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::check_url,
            commands::get_config,
            commands::add_domain,
            commands::remove_domain,
            commands::add_bookmark,
            commands::remove_bookmark,
            commands::update_settings,
            commands::open_url,
            commands::navigate_tab,
            commands::close_tab,
            commands::set_active,
            commands::set_extraction_panel,
            commands::go_back,
            commands::go_forward,
            commands::reload_tab,
            commands::stop_tab,
            commands::home_tab,
            commands::get_tabs,
            extract::extract_dom,
            extract::get_extraction,
            extract::list_extractions,
            opencode::capture_and_analyze_chart,
            opencode::get_visual_analysis,
            opencode::list_visual_analyses,
            opencode::opencode_login,
            opencode::opencode_login_status,
            commands::trigger_chart_capture,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}