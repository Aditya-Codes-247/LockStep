fn main() {
    let attributes = tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            // Main (UI) webview commands.
            "check_url",
            "get_config",
            "add_domain",
            "remove_domain",
            "add_bookmark",
            "remove_bookmark",
            "update_settings",
            "open_url",
            "navigate_tab",
            "close_tab",
            "set_active",
            "set_extraction_panel",
            "go_back",
            "go_forward",
            "reload_tab",
            "stop_tab",
            "home_tab",
            "get_tabs",
            // Extraction subsystem.
            "extract_dom",
            "get_extraction",
            "list_extractions",
            // On-demand visual chart analysis (OpenCode CLI).
            "capture_and_analyze_chart",
            "get_visual_analysis",
            "list_visual_analyses",
            "opencode_login",
            "opencode_login_status",
            "trigger_chart_capture",
        ]),
    );
    tauri_build::try_build(attributes).expect("tauri build script failed");
}