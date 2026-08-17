use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

pub const CONFIG_FILE_NAME: &str = "config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub theme: String,
    pub homepage: String,
    pub home_url: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            homepage: "landing".into(),
            home_url: "https://coinswitch.co".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    /// Approved hostnames. Matching is exact-host or subdomain, case-insensitive.
    pub whitelist: Vec<String>,
    /// Shortcuts / favorites shown on the landing page and the sidebar.
    pub bookmarks: Vec<Bookmark>,
    pub settings: Settings,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            whitelist: vec!["coinswitch.co".into(), "dhan.co".into()],
            bookmarks: vec![
                Bookmark {
                    name: "Coinswitch".into(),
                    url: "https://coinswitch.co".into(),
                    color: Some("#8b5cf6".into()),
                },
                Bookmark {
                    name: "Dhan".into(),
                    url: "https://dhan.co".into(),
                    color: Some("#3f8cff".into()),
                },
            ],
            settings: Settings::default(),
        }
    }
}

/// Shared, mutex-backed place for the application configuration.
pub struct ConfigState {
    pub path: PathBuf,
    pub inner: Mutex<AppConfig>,
}

impl ConfigState {
    pub fn load<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<Self, String> {
        let dir = app
            .path()
            .app_config_dir()
            .map_err(|e| format!("Failed to resolve config dir: {e}"))?;
        fs::create_dir_all(&dir).map_err(|e| format!("Failed to create config dir: {e}"))?;
        let path = dir.join(CONFIG_FILE_NAME);
        let config = if path.exists() {
            let raw = fs::read_to_string(&path).map_err(|e| format!("Failed to read config: {e}"))?;
            serde_json::from_str(&raw).unwrap_or_else(|e| {
                let _ = eprintln!("config.json parse error ({e}); falling back to defaults");
                AppConfig::default()
            })
        } else {
            let config = AppConfig::default();
            let raw = serde_json::to_string_pretty(&config).map_err(|e| format!("Serialize config: {e}"))?;
            fs::write(&path, raw).map_err(|e| format!("Failed to write default config: {e}"))?;
            config
        };
        Ok(Self {
            path,
            inner: Mutex::new(config),
        })
    }

    pub fn get(&self) -> AppConfig {
        self.inner.lock().unwrap().clone()
    }

    pub fn save(&self, config: AppConfig) -> Result<(), String> {
        let raw =
            serde_json::to_string_pretty(&config).map_err(|e| format!("Serialize config: {e}"))?;
        fs::write(&self.path, raw).map_err(|e| format!("Failed to write config: {e}"))?;
        *self.inner.lock().unwrap() = config;
        Ok(())
    }
}

use tauri::Runtime;