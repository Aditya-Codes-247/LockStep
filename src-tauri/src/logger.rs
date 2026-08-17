use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, Runtime};

pub const LOG_FILE_NAME: &str = "activity.log";

/// Minimal JSON-lines logger. One JSON object per line, appended atomically.
pub struct Logger {
    path: PathBuf,
    inner: Mutex<()>,
}

impl Logger {
    pub fn init<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<Self, String> {
        let dir = app
            .path()
            .app_log_dir()
            .map_err(|e| format!("Failed to resolve log dir: {e}"))?;
        fs::create_dir_all(&dir).map_err(|e| format!("Failed to create log dir: {e}"))?;
        Ok(Self {
            path: dir.join(LOG_FILE_NAME),
            inner: Mutex::new(()),
        })
    }

    pub fn log(&self, kind: &str, detail: Value) {
        let _guard = self.inner.lock().unwrap();
        let line = json!({
            "ts": chrono_like_ts(),
            "event": kind,
            "detail": detail,
        });
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(f, "{line}");
        }
    }

    pub fn blocked(&self, url: &str, reason: &str, tab: &str) {
        self.log("nav_blocked", json!({ "tab": tab, "url": url, "reason": reason }));
    }

    pub fn navigation(&self, url: &str, tab: &str) {
        self.log("nav_allowed", json!({ "tab": tab, "url": url }));
    }

    pub fn popup_blocked(&self, url: &str, tab: &str) {
        self.log("popup_blocked", json!({ "tab": tab, "url": url }));
    }

    pub fn tab_event(&self, tab: &str, url: &str, state: &str) {
        self.log("tab_event", json!({ "tab": tab, "url": url, "state": state }));
    }
}

/// Second-precision local timestamp without pulling in chrono.
fn chrono_like_ts() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let days = secs / 86400;
    let (y, m, d) = civil_from_days(days.try_into().unwrap_or(i64::MAX));
    let (hh, mm, ss) = ((secs % 86400) / 3600, (secs % 3600) / 60, secs % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Convert days since the Unix epoch to a (year, month, day) civil date.
/// Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}