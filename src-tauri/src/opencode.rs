//! On-demand visual chart analysis through the OpenCode CLI.
//
// The content script (`content/extractor.js`) captures the broker's chart
// canvas as a base64 in-memory PNG and hands it to `capture_and_analyze_chart`.
// This module writes those bytes to a transient temp file, runs
// `opencode run --format json -f <png> -m <vision model> "<prompt>"`, and
// returns the model's structured JSON reply on stdout. The temp file is a
// Drop-guarded RAII pair and is deleted on every exit path.
//
// Only `opencode/mimo-v2.5-free` accepts image input of the available free
// models (verified by probing each with a real PNG), so it is hardcoded here
// for the chart-analysis call path.
//
// All network work runs on a background `std::thread` so the UI thread
// (and every live broker webview) is never blocked by the LLM round‑trip.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use serde_json::json;

use crate::extract::{
    latest as latest_extraction, normalize_broker, store_visual, ObservedImage, VisualAnalysis,
    VisualIndicator, EVT_VISUAL,
};
use crate::logger::Logger;

/// Default minimum gap between two analyses of the same symbol.
const COOLDOWN_MS_DEFAULT: u64 = 10_000;

/// How long to wait for the OpenCode CLI to produce a reply (seconds).
const OPENCODE_TIMEOUT_S: u64 = 180;

/// Model used for the text-only prompt path. Live chart analysis always uses
/// `VISION_MODEL`; this constant is kept for the end-to-end spawn test.
#[cfg(test)]
const DEFAULT_MODEL: &str = "opencode/deepseek-v4-flash-free";

/// Vision-capable model for image (chart screenshot) analysis. The only free
/// model of those available that accepts image input — verified by probing
/// each one with a real PNG.
const VISION_MODEL: &str = "opencode/mimo-v2.5-free";

// ---------------------------------------------------------------------------
// IPC types (camelCase — mirrors the content-script capture payload)
// ---------------------------------------------------------------------------

/// Payload produced by `captureChartCanvas()` in the content script.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePayload {
    pub broker_type: String,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub timeframe: Option<String>,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// Epoch ms when the canvas was captured.
    #[serde(default)]
    pub timestamp: u64,
    #[serde(default)]
    pub mime: String,
    /// `ok` | `no_chart`.
    #[serde(default)]
    pub status: String,
    /// Base64 PNG — no longer included in the stripped version.
    #[serde(default)]
    pub image: Option<String>,
    /// Number of chart-pane canvases folded into `image` (>=1 on success).
    #[serde(default)]
    pub panes: u32,
}

/// Immediate acknowledgment returned to the calling webview. The finished
/// analysis arrives later on the `visual-analysis` event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureAck {
    pub accepted: bool,
    pub broker: String,
    /// `started` | `rate_limited` | `no_chart`.
    pub status: String,
    /// Epoch ms for the next allowed capture of the same symbol.
    pub retry_after_ms: u64,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct ServerInfo {
    base: String,
    user: String,
    pass: String,
}

#[derive(Default)]
struct OpencodeInner {
    /// (broker, symbol) → last analysis start (rate limiting per symbol).
    cooldowns: HashMap<(String, String), Instant>,
}

/// Managed state for the OpenCode CLI integration.
pub struct OpencodeState {
    inner: Mutex<OpencodeInner>,
}

impl Default for OpencodeState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(OpencodeInner::default()),
        }
    }
}

impl Drop for OpencodeState {
    fn drop(&mut self) {
        // No child process to kill; the CLI was spawned per‑call.
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn cooldown_ms() -> u64 {
    std::env::var("LOCKSTEP_COOLDOWN_MS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(COOLDOWN_MS_DEFAULT)
    // Cooldowns stay in-memory: reads of `get_extraction` are never rate-limited.
}

// ---------------------------------------------------------------------------
// Tauri commands (registered in lib.rs / build.rs)
// ---------------------------------------------------------------------------

/// Entry point called by the content script after a manual chart capture.
#[tauri::command]
pub fn capture_and_analyze_chart(
    app: AppHandle,
    payload: CapturePayload,
) -> CaptureAck {
    let broker = normalize_broker(&payload.broker_type).unwrap_or_else(|| "unknown".into());
    let symbol = payload
        .symbol
        .clone()
        .unwrap_or_else(|| "unknown".into());
    let ack_broker = broker.clone();

    // TEMP DEBUG — persisted capture telemetry (removed after verification).
    app.state::<Logger>().log(
        "capture_received",
        json!({
            "broker": broker,
            "symbol": symbol,
            "status": payload.status,
            "width": payload.width,
            "height": payload.height,
            "panes": payload.panes,
            "image_b64_len": payload.image.as_ref().map(|s| s.len()).unwrap_or(0),
        }),
    );

    // Failures that never reach a model are reported immediately as events so
    // the extraction panel can render the explicit reason (no-chart/tainted).
    if payload.status != "ok" {
        let status = payload.status.clone();
        let error = match status.as_str() {
            "canvas_tainted" => Some(
                "The chart canvas is tainted (cross-origin content) — the bitmap cannot be read. Use the DOM-extracted candles instead.".into(),
            ),
            _ => Some("No chart canvas was found to capture.".into()),
        };
        let analysis = VisualAnalysis {
            broker: broker.clone(),
            symbol: symbol.clone(),
            model: None,
            timestamp_ms: payload.timestamp,
            latency_ms: None,
            status,
            pattern: None,
            trend: None,
            signal: None,
            support: None,
            resistance: None,
            indicators: Vec::new(),
            observed: None,
            summary: None,
            raw: None,
            error,
        };
        store_visual(&app, analysis.clone());
        emit_visual(&app, &broker, &symbol, &analysis);
        return CaptureAck {
            accepted: false,
            broker: ack_broker,
            status: analysis.status,
            retry_after_ms: 0,
        };
    }

    // Rate limit: max once per symbol within the configured interval.
    {
        let state = app.state::<OpencodeState>();
        let mut inner = state.inner.lock().unwrap();
        let now = Instant::now();
        if let Some(last) = inner.cooldowns.get(&(broker.clone(), symbol.clone())) {
            let waited = now.duration_since(*last).as_millis() as u64;
            if waited < cooldown_ms() {
                let retry = cooldown_ms() - waited;
                // Surface the cooldown to the panel so the button can grey out.
                let analysis = VisualAnalysis {
                    broker: broker.clone(),
                    symbol: symbol.clone(),
                    model: None,
                    timestamp_ms: now_ms(),
                    latency_ms: None,
                    status: "rate_limited".into(),
                    pattern: None,
                    trend: None,
                    signal: None,
                    support: None,
                    resistance: None,
                    indicators: Vec::new(),
                    observed: None,
                    summary: None,
                    raw: None,
                    error: Some(format!(
                        "Analysis rate limit: {symbol} can be analyzed once every {}s. Retry in {:.0}s.",
                        cooldown_ms() / 1000,
                        retry as f64 / 1000.0
                    )),
                };
                emit_visual(&app, &broker, &symbol, &analysis);
                return CaptureAck {
                    accepted: false,
                    broker: ack_broker,
                    status: "rate_limited".into(),
                    retry_after_ms: retry,
                };
            }
        }
        inner.cooldowns.insert((broker.clone(), symbol.clone()), now);
    }

    let thread_app = app.clone();
    std::thread::spawn(move || analyze_visual(&thread_app, payload));

    CaptureAck {
        accepted: true,
        broker: ack_broker,
        status: "started".into(),
        retry_after_ms: cooldown_ms(),
    }
}

/// Latest cached visual analysis for a broker+symbol (for the panel on open).
#[tauri::command]
pub fn get_visual_analysis(
    app: AppHandle,
    broker: String,
    symbol: String,
) -> Result<VisualAnalysis, String> {
    crate::extract::latest_visual(&app, &broker, &symbol)
        .ok_or_else(|| "no visual analysis cached for this symbol yet".into())
}

/// Every cached visual analysis (the panel groups by broker).
#[tauri::command]
pub fn list_visual_analyses(app: AppHandle) -> Vec<VisualAnalysis> {
    let state = app.state::<crate::extract::VisualState>();
    let map = state.0.lock().unwrap();
    map.values().cloned().collect()
}

fn emit_visual(app: &AppHandle, broker: &str, symbol: &str, analysis: &VisualAnalysis) {
    let _ = app.emit(
        EVT_VISUAL,
        json!({ "broker": broker, "symbol": symbol, "analysis": analysis }),
    );
}

// ---------------------------------------------------------------------------
// Analysis pipeline (runs on a background thread)
// ---------------------------------------------------------------------------

fn analyze_visual(app: &AppHandle, payload: CapturePayload) {
    let started = Instant::now();
    let log = app.state::<Logger>();
    let outcome = try_analyze_visual(app, &payload, started);
    let latency_ms = started.elapsed().as_millis() as u64;

    match outcome {
        Ok(analysis) => {
            let mut analysis = analysis;
            analysis.latency_ms = Some(latency_ms);
            analysis.timestamp_ms = now_ms();
            store_visual(app, analysis.clone());
            emit_visual(app, &analysis.broker, &analysis.symbol, &analysis);
            log.log(
                "opencode_visual",
                json!({
                    "broker": analysis.broker,
                    "symbol": analysis.symbol,
                    "model": analysis.model,
                    "status": "ok", "latency_ms": latency_ms, "ok": true,
                }),
            );
        }
        Err(reason) => {
            let analysis = VisualAnalysis {
                broker: payload.broker_type.clone(),
                symbol: payload.symbol.clone().unwrap_or_else(|| "unknown".into()),
                model: None,
                timestamp_ms: now_ms(),
                latency_ms: Some(latency_ms),
                status: "error".into(),
                pattern: None,
                trend: None,
                signal: None,
                support: None,
                resistance: None,
                indicators: Vec::new(),
                observed: None,
                summary: None,
                raw: None,
                error: Some(reason.clone()),
            };
            store_visual(app, analysis.clone());
            emit_visual(app, &analysis.broker, &analysis.symbol, &analysis);
            log.log(
                "opencode_visual",
                json!({
                    "broker": analysis.broker,
                    "symbol": analysis.symbol,
                    "status": "error", "latency_ms": latency_ms, "ok": false,
                    "error": reason,
                }),
            );
        }
    }
}

fn try_analyze_visual(
    app: &AppHandle,
    payload: &CapturePayload,
    started: Instant,
) -> Result<VisualAnalysis, String> {
    let log = app.state::<Logger>();
    let model = std::env::var("OPENCODE_MODEL").unwrap_or_else(|_| VISION_MODEL.into());
    let symbol = payload.symbol.clone().unwrap_or_else(|| "unknown".into());

    // Decode the chart PNG (base64 from the content script) and stage it as a
    // transient temp file that the CLI attaches with `-f`. The `ChartTempFile`
    // guard deletes the file on every exit path (success, spawn failure,
    // timeout, any early return) — it never outlives this call. It lives for
    // the whole function, including across the parse retry, so the retried
    // invocation still sees the intact image file.
    let png_bytes = match payload.image.as_deref() {
        Some(b64) if !b64.trim().is_empty() => {
            base64_decode_std(b64.trim()).map_err(|e| format!("Chart image decode failed: {e}"))?
        }
        _ => return Err("No chart image captured — the canvas couldn't be read, please retry.".into()),
    };
    let tmp = chart_temp_path();
    fs::write(&tmp, &png_bytes)
        .map_err(|e| format!("Failed to write chart temp file: {e}"))?;
    let _guard = ChartTempFile {
        path: tmp.clone(),
        log: Some(&*log),
    };
    log.log(
        "chart_tmp_created",
        json!({
            "path": tmp.to_string_lossy(),
            "bytes": png_bytes.len(),
        }),
    );
    log.log(
        "image_included",
        json!({
            "included": true,
            "image_bytes_sent": png_bytes.len(),
        }),
    );
    // TEMP DEBUG — hard evidence: the temp file exactly as it sits on disk
    // when the CLI spawns (on-disk size ≠ written len, PNG magic check).
    // Removed once the root cause is confirmed.
    if let Ok(meta) = fs::metadata(&tmp) {
        let head = &png_bytes[..png_bytes.len().min(8)];
        let is_png = head == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        log.log(
            "chart_tmp_disk",
            json!({
                "path": tmp.to_string_lossy(),
                "disk_bytes": meta.len(),
                "written_bytes": png_bytes.len(),
                "png_signature": is_png,
            }),
        );
    }

    // Resolve real OHLC ground truth from the extraction cache (the same
    // source `collectChart()` / `collectHeaderOhlc()` populate). When the
    // cache has nothing usable we intentionally fall back to the weaker,
    // context-free prompt and log `ohlc_context_available: false`.
    let ohlc = resolve_ohlc_context(app, payload);
    let ohlc_available = ohlc.as_ref().map(|c| c.is_usable()).unwrap_or(false);
    log.log(
        "opencode_ohlc_context",
        json!({
            "broker": payload.broker_type,
            "symbol": symbol,
            "ohlc_context_available": ohlc_available,
            "open": ohlc.as_ref().map(|c| c.open),
            "high": ohlc.as_ref().map(|c| c.high),
            "low": ohlc.as_ref().map(|c| c.low),
            "close": ohlc.as_ref().map(|c| c.close),
            "range_low": ohlc.as_ref().map(|c| c.range_low),
            "range_high": ohlc.as_ref().map(|c| c.range_high),
        }),
    );

    let chart_prompt = build_chart_prompt(payload, ohlc.as_ref());

    // Run the CLI at most twice: a second, identical invocation fires only
    // when the first reply isn't valid STRICT-JSON in the expected shape.
    // Spawn/timeout failures are handled inside the runner and never retried.
    let run_attempt = |attempt: u32| -> Result<AttemptOutput, String> {
        run_opencode_attempt(
            &*log,
            &chart_prompt,
            &model,
            &tmp,
            &symbol,
            &payload.broker_type,
            attempt,
            started,
        )
    };
    analyze_with_retry(Some(&*log), payload, &model, run_attempt)
}

/// Output of one successful CLI spawn: exit status + captured stdout.
type AttemptOutput = (ExitStatus, Vec<u8>);

/// Spawn `opencode run --format json -f <png>`, wait for it with the timeout,
/// and write the per-attempt telemetry — including the full prompt text
/// verbatim so any future model-behavior anomaly is captured. Err only on
/// spawn failure; a non-zero/timed-out exit is reflected in the status.
fn run_opencode_attempt(
    log: &Logger,
    chart_prompt: &str,
    model: &str,
    tmp: &PathBuf,
    symbol: &str,
    broker: &str,
    attempt: u32,
    started: Instant,
) -> Result<AttemptOutput, String> {
    // TEMP DEBUG — exact command + args we are about to run, plus the full
    // prompt text verbatim (not just its length).
    log.log(
        "opencode_spawn",
        json!({
            "broker": broker,
            "symbol": symbol,
            "attempt": attempt,
            "command": "opencode",
            "args": ["run", "--format", "json", "-m", model, "-f", "<chart.png>", "<prompt>"],
            "prompt": chart_prompt,
            "prompt_len": chart_prompt.len(),
            "model": model,
            "open_code_model_env": std::env::var("OPENCODE_MODEL").ok(),
            "timeout_s": OPENCODE_TIMEOUT_S,
        }),
    );

    let mut child = match spawn_opencode_with_file(chart_prompt, model, Some(tmp)) {
        Ok(c) => {
            log.log("opencode_spawn_ok", json!({ "pid": c.id(), "attempt": attempt }));
            c
        }
        Err(e) => {
            log.log("opencode_spawn_err", json!({ "error": e, "attempt": attempt }));
            return Err(format!("OpenCode CLI failed: {e}"));
        }
    };

    let (status, stdout_bytes, stderr_bytes, timed_out) =
        wait_for_child_with_timeout(&mut child, OPENCODE_TIMEOUT_S * 1000);

    // TEMP DEBUG — what actually came back from the subprocess.
    let stdout_tail = String::from_utf8_lossy(
        &stdout_bytes[stdout_bytes.len().saturating_sub(4000)..],
    )
    .to_string();
    let stderr_tail = String::from_utf8_lossy(
        &stderr_bytes[stderr_bytes.len().saturating_sub(4000)..],
    )
    .to_string();
    log.log(
        "opencode_exit",
        json!({
            "attempt": attempt,
            "success": status.success(),
            "code": status.code(),
            "timed_out": timed_out,
            "elapsed_ms": started.elapsed().as_millis(),
            "stdout_len": stdout_bytes.len(),
            "stderr_len": stderr_bytes.len(),
            "stdout_tail": stdout_tail,
            "stderr_tail": stderr_tail,
        }),
    );

    Ok((status, stdout_bytes))
}

/// Run the CLI at most twice via `run_attempt`. Attempt 1 always runs; if its
/// reply isn't valid STRICT-JSON in the expected shape, attempt 2 runs with
/// the identical invocation (same prompt, same temp file, same model). A
/// second malformed reply surfaces the error normally through the Err branch
/// (the caller still carries broker/symbol, so the UI un-hangs). Spawn
/// failures and non-zero exits are not retried.
fn analyze_with_retry<F>(
    log: Option<&Logger>,
    payload: &CapturePayload,
    model: &str,
    mut run_attempt: F,
) -> Result<VisualAnalysis, String>
where
    F: FnMut(u32) -> Result<AttemptOutput, String>,
{
    let symbol = payload.symbol.clone().unwrap_or_default();
    let parse_failed = |attempt: u32, reply: &str| {
        if let Some(log) = log {
            log.log(
                "opencode_parse_failed",
                json!({
                    "attempt": attempt,
                    "reason": "response did not match expected JSON shape",
                    "broker": payload.broker_type,
                    "symbol": &symbol,
                    "model": model,
                    "raw": clip(reply, 4000),
                }),
            );
        }
    };

    let (status, stdout) = run_attempt(1)?;
    if !status.success() {
        return Err("OpenCode CLI exited with error".into());
    }
    let reply = collect_opencode_reply(&stdout).unwrap_or_default();
    if is_valid_structured_reply(&reply) {
        return Ok(finish_analysis(payload, &reply));
    }

    // Attempt 1 produced no usable structured reply — retry exactly once.
    parse_failed(1, &reply);
    let (status, stdout) = run_attempt(2)?;
    if !status.success() {
        return Err("OpenCode CLI exited with error".into());
    }
    let reply = collect_opencode_reply(&stdout).unwrap_or_default();
    if is_valid_structured_reply(&reply) {
        return Ok(finish_analysis(payload, &reply));
    }

    parse_failed(2, &reply);
    Err("OpenCode reply was not valid JSON in the expected shape after one retry".into())
}

/// Build the final `VisualAnalysis` from a validated structured reply.
fn finish_analysis(payload: &CapturePayload, reply: &str) -> VisualAnalysis {
    let mut analysis = parse_reply(reply);
    analysis.broker = payload.broker_type.clone();
    analysis.symbol = payload.symbol.clone().unwrap_or_default();
    analysis.status = "ok".into();
    // Store a clipped snippet of the raw model text.
    analysis.raw = Some(clip(reply, 4000));
    analysis
}

/// True when the reply strips to a single JSON object carrying a non-empty
/// `summary` string and an `observedImage` object whose `indicators` array
/// lists at least one named indicator — i.e. the model both did the task and
/// reported what it saw in the image. A reply that omits `observedImage` (or
/// returns it empty) is treated as unusable so the retry fires.
fn is_valid_structured_reply(text: &str) -> bool {
    let bare = strip_fences(text);
    match serde_json::from_str::<Value>(bare) {
        Ok(Value::Object(map)) => {
            let has_summary = matches!(
                map.get("summary"),
                Some(Value::String(s)) if !s.trim().is_empty()
            );
            let observed_ok = match map.get("observedImage") {
                Some(Value::Object(o)) => {
                    match o.get("indicators") {
                        Some(Value::Array(arr)) => arr.iter().any(|i| {
                            matches!(
                                i.get("name"),
                                Some(Value::String(n)) if !n.trim().is_empty()
                            )
                        }),
                        _ => false,
                    }
                }
                _ => false,
            };
            has_summary && observed_ok
        }
        _ => false,
    }
}

/// Real price ground truth pulled from the extraction cache and injected into
/// the chart prompt. `None` when the cache has nothing usable (the caller then
/// falls back to the weaker, context-free prompt on purpose).
#[derive(Debug, Clone, Default)]
struct OhlcContext {
    open: Option<f64>,
    high: Option<f64>,
    low: Option<f64>,
    close: Option<f64>,
    /// Visible 24h-ish range (min low / max high over the cached candle stack).
    range_low: Option<f64>,
    range_high: Option<f64>,
}

impl OhlcContext {
    /// True when at least one numeric anchor reached the prompt.
    fn is_usable(&self) -> bool {
        self.open.is_some() || self.high.is_some() || self.low.is_some() || self.close.is_some()
    }
}

/// Pull the latest cached chart/symbol OHLC for the payload's broker, matching
/// the extraction cache the content script already populates (`extract::latest`).
///
/// Preferred source is the chart's own candle series (the same window the
/// vision model sees): take the latest candle for O/H/L/C and fold the stack
/// into a visible high/low range. Falls back to a symbol's 5 min/1 min OHLC
/// windows when the chart series isn't cached yet.
fn resolve_ohlc_context(app: &AppHandle, payload: &CapturePayload) -> Option<OhlcContext> {
    let snapshot = latest_extraction(app, &payload.broker_type).ok()?;
    let data = &snapshot.data;

    // 1) Chart candle series: latest candle + full-stack visible range.
    if let Some(chart) = &data.chart {
        let candles = &chart.candles;
        if let Some(latest) = candles.iter().max_by_key(|c| c.time) {
            let range_low = candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
            let range_high = candles.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
            return Some(OhlcContext {
                open: Some(latest.open),
                high: Some(latest.high),
                low: Some(latest.low),
                close: Some(latest.close),
                range_low: Some(range_low),
                range_high: Some(range_high),
            });
        }
    }

    // 2) Symbol-level OHLC windows (5 min, then 1 min) for the matching symbol.
    let symbol = payload.symbol.as_deref()?.trim();
    if symbol.is_empty() {
        return None;
    }
    let sym = data
        .symbols
        .iter()
        .find(|s| ticker_matches(&s.ticker, symbol))?;
    let win = sym.ohlc_5min.as_ref().or(sym.ohlc_1min.as_ref())?;
    Some(OhlcContext {
        open: Some(win.open),
        high: Some(win.high),
        low: Some(win.low),
        close: Some(win.close),
        range_low: None,
        range_high: None,
    })
}

/// Substring-insensitive ticker match: true when either name contains the
/// other after normalizing separators, or when their base coins agree after
/// stripping the quote currency (e.g. "BTCINR" vs "BTC/USDT Futures").
fn ticker_matches(cached: &str, wanted: &str) -> bool {
    let norm = |s: &str| s.to_ascii_lowercase().replace(['/', ' ', '-', '_'], "");
    let a = norm(cached);
    let b = norm(wanted);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a.contains(&b) || b.contains(&a) {
        return true;
    }
    // Strip known quote currencies / suffixes to compare base coins: e.g.
    // "btcinr" -> "btc", "solusdtfutures" -> "sol".
    let strips = ["usdt", "usdc", "usd", "inr", "eur", "btc", "eth", "sol", "futures", "perpetual", "spot", "swap"];
    let base = |s: &str| {
        let mut out = s.to_string();
        for suf in &strips {
            if out.ends_with(suf) {
                out.truncate(out.len() - suf.len());
                break;
            }
        }
        out
    };
    let ab = base(&a);
    let bb = base(&b);
    !ab.is_empty() && !bb.is_empty() && (ab.contains(&bb) || bb.contains(&ab))
}

/// Build the chart-analysis prompt for the vision model. The actual chart is
/// attached as a PNG (`-f`); the prompt supplies the instrument, timeframe,
/// the required structured response shape, and — when real OHLC is available —
/// an anchor the model must treat as ground truth for the visible price scale.
///
/// When `ohlc` is `None` the prompt falls back to the original context-free
/// wording (the caller must log `ohlc_context_available: false` so the weaker
/// mode is visible in telemetry).
fn build_chart_prompt(payload: &CapturePayload, ohlc: Option<&OhlcContext>) -> String {
    let symbol = payload.symbol.as_deref().unwrap_or("unknown");
    let timeframe = payload.timeframe.as_deref().unwrap_or("?");

    let mut prompt = format!(
        "You are a professional technical analyst. Analyze the attached chart image for {symbol} (timeframe {timeframe}). "
    );

    if let Some(ctx) = ohlc.filter(|c| c.is_usable()) {
        // Emit whichever anchors are present (partial OHLC still grounds the
        // model to the real numbers; never invent a missing field).
        let mut data = String::new();
        if let (Some(open), Some(high), Some(low), Some(close)) = (ctx.open, ctx.high, ctx.low, ctx.close) {
            data.push_str(&format!("Open {open}, High {high}, Low {low}, Close {close}"));
        } else {
            if let Some(open) = ctx.open {
                data.push_str(&format!("Open {open}"));
            }
            if let Some(high) = ctx.high {
                if !data.is_empty() { data.push_str(", "); }
                data.push_str(&format!("High {high}"));
            }
            if let Some(low) = ctx.low {
                if !data.is_empty() { data.push_str(", "); }
                data.push_str(&format!("Low {low}"));
            }
            if let Some(close) = ctx.close {
                if !data.is_empty() { data.push_str(", "); }
                data.push_str(&format!("Close {close}"));
            }
        }
        if let (Some(lo), Some(hi)) = (ctx.range_low, ctx.range_high) {
            if !data.is_empty() {
                data.push_str(", ");
            }
            data.push_str(&format!("24h range {lo}–{hi}"));
        }
        if !data.is_empty() {
            prompt.push_str(&format!(
                "The chart's current price data is: {data}. \
                 Use these as ground truth for the visible price scale — do not estimate price levels outside this context. "
            ));
        }
        prompt.push_str(
            "Support and resistance must be derived from the chart pattern in relation to the \
             provided OHLC context. If you cannot determine a level with reasonable confidence \
             from the visible chart shape, return null for that field rather than fabricate a number. ",
        );
    }

    prompt.push_str(
        " Additionally, report exactly what you can see in the image itself in the required \
         \"observedImage\" field: the symbol, timeframe, price scale and OHLC legend text visible \
         on the chart, every indicator, overlay, drawing and crosshair value you can actually see \
         rendered in the screenshot. This field is used to verify which indicators are really \
         visible, so it must never be omitted. List only what is genuinely visible; for anything \
         not present in the image return null or an empty list rather than guessing. \
         Respond with STRICT JSON only and no markdown fences, exactly matching this shape:\n\
         {{\"pattern\": string | null, \"trend\": \"up\"|\"down\"|\"sideways\"|null, \
         \"signal\": \"buy\"|\"sell\"|\"neutral\", \"support\": number | null, \
         \"resistance\": number | null, \
         \"indicators\": [{{\"name\": string, \"value\": string, \"signal\": string}}], \
         \"observedImage\": {{\"symbol\": string | null, \"timeframe\": string | null, \
         \"priceScaleVisible\": string | null, \"ohlcLegend\": string | null, \
         \"indicators\": [{{\"name\": string, \"value\": string | null, \"visible\": true}}], \
         \"overlays\": [string], \"drawings\": [string], \"crosshairValues\": string | null}}, \
         \"summary\": string}}",
    );
    prompt
}

/// Resolve how to launch the OpenCode CLI on this platform.
///
/// Returns `(program, shell)` where `shell == Some("/C")` means `program`
/// must be invoked as `cmd.exe /C opencode …` (Windows `.cmd` shim fallback).
fn opencode_command() -> Result<(String, Option<&'static str>), String> {
    #[cfg(windows)]
    {
        if let Some(exe) = resolve_opencode_exe() {
            return Ok((exe, None));
        }
        // Last resort: the npm `.cmd`/`.ps1` shims only work when run through
        // the real shell, so hand the whole invocation to cmd.exe.
        Ok(("cmd.exe".into(), Some("/C")))
    }
    #[cfg(not(windows))]
    {
        // Non-Windows PATH entries resolve to real binaries (or shell scripts
        // with a shebang), so the plain name is fine there.
        Ok(("opencode".into(), None))
    }
}

/// Locate a real `opencode.exe`, preferring the npm global install, then any
/// `where`-resolvable `.exe` (skipping `.cmd`/`.ps1` shims). Windows only.
#[cfg(windows)]
fn resolve_opencode_exe() -> Option<String> {
    // 1) `npm root -g` → <root>/opencode-ai/bin/opencode.exe.
    //    npm is itself a `.cmd` shim on Windows, so it must run via cmd.exe
    //    (the same CreateProcess limitation that affects `opencode`).
    if let Some(root) = run_capture("cmd.exe", &["/C", "npm", "root", "-g"]) {
        let mut cand = PathBuf::from(root.trim());
        cand.push("opencode-ai");
        cand.push("bin");
        cand.push("opencode.exe");
        if cand.is_file() {
            return Some(cand.to_string_lossy().into_owned());
        }
    }
    // 2) `where.exe opencode` / `where opencode`, keep the first real .exe.
    for (prog, args) in [("where.exe", &["opencode"][..]), ("where", &["opencode"][..])] {
        if let Some(out) = run_capture(prog, args) {
            for line in out.lines() {
                let t = line.trim();
                if t.is_empty() {
                    continue;
                }
                if t.to_ascii_lowercase().ends_with(".exe") && PathBuf::from(t).is_file() {
                    return Some(t.to_string());
                }
            }
        }
    }
    None
}

/// Run a short-lived child and return its trimmed stdout (None when the
/// program cannot be launched at all).
#[cfg(windows)]
fn run_capture(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Spawn `opencode run --format json [-f <image>] -m <model> "<prompt>"` and
/// return the child `Child` (or an error).
fn spawn_opencode_with_file(
    prompt: &str,
    model: &str,
    image_file: Option<&PathBuf>,
) -> Result<Child, String> {
    let (program, shell) = opencode_command()?;
    let mut args: Vec<String> = vec![
        "run".into(),
        "--format".into(),
        "json".into(),
        "-m".into(),
        model.into(),
    ];
    args.push(prompt.into());
    // Empirically the CLI only binds `-f <path>` when the attachment comes
    // after the positional prompt; `-f <path>` *before* the prompt makes it
    // treat the prompt text as the file path ("File not found: <prompt>").
    if let Some(img) = image_file {
        args.push("-f".into());
        args.push(img.to_string_lossy().into_owned());
    }

    let mut command = Command::new(&program);
    if let Some(flag) = shell {
        // Windows shim fallback: cmd.exe /C opencode …
        command.arg(flag).arg("opencode");
    }
    command.args(&args);
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn '{program}': {e}"))
}

/// Spawn with no attached image — exercised by the text-only spawn test.
#[cfg(test)]
fn spawn_opencode(prompt: &str, model: &str) -> Result<Child, String> {
    spawn_opencode_with_file(prompt, model, None)
}

/// Wait for the OpenCode CLI child to finish, with a timeout.
/// Drains stdout and stderr on reader threads so a chatty child can never
/// deadlock on a full pipe; returns the child's exit status, whatever it
/// printed, and whether the timeout killed it.
fn wait_for_child_with_timeout(
    child: &mut Child,
    timeout_ms: u64,
) -> (std::process::ExitStatus, Vec<u8>, Vec<u8>, bool) {
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut s) = stdout {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });
    let err = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut s) = stderr {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                return (
                    s,
                    out.join().unwrap_or_default(),
                    err.join().unwrap_or_default(),
                    false,
                );
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let s = child
                        .wait()
                        .unwrap_or_else(|_| std::process::ExitStatus::default());
                    return (
                        s,
                        out.join().unwrap_or_default(),
                        err.join().unwrap_or_default(),
                        true,
                    );
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                return (
                    std::process::ExitStatus::default(),
                    out.join().unwrap_or_default(),
                    err.join().unwrap_or_default(),
                    false,
                );
            }
        }
    }
}

/// Collect the JSON‑lines reply from the OpenCode CLI and stitch together the
/// model's text reply.
fn collect_opencode_reply(stdout: &[u8]) -> Result<String, String> {
    let stdout_str = String::from_utf8_lossy(stdout);
    let lines: Vec<&str> = stdout_str.lines().collect();

    // Collect text parts from "type":"text" events.
    let mut collected = String::new();
    for line in &lines {
        // Try to parse as JSON Value.
        let val: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // skip non‑JSON lines
        };

        if let Some(t) = val.get("type").and_then(|t| t.as_str()) {
            if t == "text" {
                if let Some(text) = val.get("part").and_then(|p| p.get("text")).and_then(|t| t.as_str()) {
                    collected.push_str(text);
                    collected.push('\n');
                }
            }
        }
    }

    if collected.trim().is_empty() {
        // Fallback: return the raw first line if no text events found.
        collected = lines.first().copied().unwrap_or("").to_string();
    }

    Ok(collected.trim().to_string())
}

/// Parse the model's JSON reply into a `VisualAnalysis`.
fn parse_reply(text: &str) -> VisualAnalysis {
    let bare = strip_fences(text);
    match serde_json::from_str::<Value>(bare) {
        Ok(v) => {
            let s = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
            let n = |k: &str| v.get(k).and_then(Value::as_f64);
            let indicators = v
                .get("indicators")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|o| {
                    let name = o.get("name").and_then(Value::as_str).unwrap_or("");
                    if name.is_empty() {
                        return None;
                    }
                    Some(VisualIndicator {
                        name: name.to_string(),
                        value: o.get("value").and_then(Value::as_str).unwrap_or("").to_string(),
                        signal: o.get("signal").and_then(Value::as_str).unwrap_or("").to_string(),
                    })
                })
                .take(12)
                .collect();
            let observed = v
                .get("observedImage")
                .filter(|o| o.is_object())
                .and_then(|o| serde_json::from_value::<ObservedImage>(o.clone()).ok());
            VisualAnalysis {
                broker: String::new(),
                symbol: String::new(),
                model: None,
                timestamp_ms: now_ms(),
                latency_ms: None,
                status: "ok".into(),
                pattern: s("pattern"),
                trend: s("trend"),
                signal: s("signal"),
                support: n("support"),
                resistance: n("resistance"),
                indicators,
                observed,
                summary: s("summary"),
                raw: None,
                error: None,
            }
        }
        Err(_) => {
            // Raw-text fallback: the model replied but not as expected JSON.
            VisualAnalysis {
                broker: String::new(),
                symbol: String::new(),
                model: None,
                timestamp_ms: now_ms(),
                latency_ms: None,
                status: "ok".into(),
                pattern: None,
                trend: None,
                signal: None,
                support: None,
                resistance: None,
                indicators: Vec::new(),
                observed: None,
                summary: Some(clip(text, 1500)),
                raw: None,
                error: None,
            }
        }
    }
}

fn strip_fences(text: &str) -> &str {
    let t = text.trim();
    if t.starts_with("```") {
        if let Some(stripped) = t.strip_prefix("```json").or_else(|| t.strip_prefix("````")) {
            return stripped.trim_end_matches("```").trim();
        }
        if t.ends_with("```") {
            return t.trim_start_matches("```").trim_end_matches("```").trim();
        }
    }
    t
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

// ---------------------------------------------------------------------------
// Chart temp-file handling
// ---------------------------------------------------------------------------

/// RAII guard for the transient chart-image temp file. Deleting it from
/// `Drop` guarantees the file is removed on every exit path — success, spawn
/// failure, timeout, or any early return — so it can never be skippable or
/// left behind.
struct ChartTempFile<'a> {
    path: PathBuf,
    log: Option<&'a Logger>,
}

impl Drop for ChartTempFile<'_> {
    fn drop(&mut self) {
        let deleted = fs::remove_file(&self.path).is_ok();
        if let Some(log) = self.log {
            log.log(
                "chart_tmp_deleted",
                json!({ "path": self.path.to_string_lossy(), "deleted": deleted }),
            );
        }
    }
}

/// Random-enough temp filename for one chart capture. Built from
/// std-only pieces (nanosecond clock + PID) since Cargo.toml cannot grow a
/// uuid/rand dependency in this scope.
fn chart_temp_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0) as u64;
    let pid = std::process::id();
    std::env::temp_dir().join(format!("lsb-chart-{pid}-{nonce:016x}.png"))
}

/// Std-only standard-alphabet (RFC 4648) base64 decoder. The content script
/// ships `payload.image` as `btoa()` output (padded, no whitespace), but both
/// whitespace and unpadded tails are tolerated here.
fn base64_decode_std(input: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let mut bytes: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let rem = bytes.len() % 4;
    if rem == 1 {
        return Err("invalid base64 length".into());
    }
    // Pad the tail group with '=' so decoding can run in strict 4-char groups.
    for _ in 0..(4 - rem) % 4 {
        bytes.push(b'=');
    }

    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut i = 0;
    while i < bytes.len() {
        let a = val(bytes[i]).ok_or_else(|| "invalid base64 alphabet character".to_string())?;
        let b = val(bytes[i + 1]).ok_or_else(|| "invalid base64 alphabet character".to_string())?;
        let c1 = bytes[i + 2];
        let d1 = bytes[i + 3];
        let c = if c1 == b'=' { 0 } else { val(c1).ok_or_else(|| "invalid base64 alphabet character".to_string())? };
        let d = if d1 == b'=' { 0 } else { val(d1).ok_or_else(|| "invalid base64 alphabet character".to_string())? };

        out.push(((a << 2) | (b >> 4)) as u8);
        if c1 != b'=' {
            out.push((((b & 0x0F) << 4) | (c >> 2)) as u8);
        }
        if d1 != b'=' {
            out.push((((c & 0x03) << 6) | d) as u8);
        }
        i += 4;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_json_fences() {
        assert_eq!(strip_fences("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_fences("\n{\"a\":1}\n"), "{\"a\":1}");
    }

    #[test]
    fn parses_structured_reply() {
        let reply = "```json\n{\"pattern\":\"bullish engulfing\",\"trend\":\"up\",\
                     \"signal\":\"buy\",\"support\":2890.5,\"resistance\":2915,\
                     \"indicators\":[{\"name\":\"EMA\",\"value\":\"2905\",\"signal\":\"bullish\"}],\
                     \"observedImage\":{\"symbol\":\"RELIANCE\",\"timeframe\":\"15m\",\
                     \"priceScaleVisible\":\"2890–2920\",\"ohlcLegend\":\"O 2890 H 2915 L 2885 C 2905\",\
                     \"indicators\":[{\"name\":\"EMA\",\"value\":\"2905\",\"visible\":true},\
                     {\"name\":\"RSI\",\"value\":\"62\",\"visible\":true}],\
                     \"overlays\":[\"VWAP\"],\"drawings\":[],\"crosshairValues\":\"2905.10 @ 14:35\"},\
                     \"summary\":\"Momentum looks positive.\"}\n```";
        let a = parse_reply(reply);
        assert_eq!(a.status, "ok");
        assert_eq!(a.trend.as_deref(), Some("up"));
        assert_eq!(a.pattern.as_deref(), Some("bullish engulfing"));
        assert_eq!(a.signal.as_deref(), Some("buy"));
        assert_eq!(a.support, Some(2890.5));
        assert_eq!(a.resistance, Some(2915.0));
        assert_eq!(a.indicators.len(), 1);
        assert_eq!(a.summary.as_deref(), Some("Momentum looks positive."));
        let observed = a.observed.expect("observedImage must be parsed");
        assert_eq!(observed.symbol.as_deref(), Some("RELIANCE"));
        assert_eq!(observed.timeframe.as_deref(), Some("15m"));
        assert_eq!(observed.indicators.len(), 2);
        assert_eq!(observed.indicators[0].name, "EMA");
        assert_eq!(observed.indicators[1].value.as_deref(), Some("62"));
        assert_eq!(observed.overlays, vec!["VWAP".to_string()]);
        assert_eq!(observed.crosshair_values.as_deref(), Some("2905.10 @ 14:35"));
    }

    #[test]
    fn falls_back_to_raw_when_not_json() {
        let a = parse_reply("I see a rising channel with lower volumes.");
        assert_eq!(a.status, "ok");
        assert!(a.pattern.is_none());
        assert!(a.summary.is_some());
    }

    #[test]
    fn clip_keeps_boundaries() {
        let s = "abc€def";
        let c = clip(s, 4);
        // Result stays a (possibly truncated) prefix of the input and is
        // always valid UTF-8 (ellipsis appended).
        let base = c.trim_end_matches('…');
        assert!(s.starts_with(base), "clip must drop a suffix, got '{c}'");
        assert!(c.is_char_boundary(c.len()));
        assert!(c.len() <= s.len() + 3);
    }

    #[test]
    fn decodes_standard_base64() {
        assert_eq!(
            base64_decode_std("SGVsbG8gV29ybGQ=").unwrap(),
            b"Hello World"
        );
        assert_eq!(base64_decode_std("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(base64_decode_std("Zg==").unwrap().as_slice(), b"f");
        assert_eq!(base64_decode_std("Zg").unwrap().as_slice(), b"f");
        assert_eq!(base64_decode_std("Yg==").unwrap().as_slice(), b"b");
        assert!(base64_decode_std("a").is_err(), "mod-4 == 1 must be rejected");
        assert!(
            base64_decode_std("a*b=").is_err(),
            "non-alphabet characters must be rejected"
        );
    }

    #[test]
    fn structured_reply_validation() {
        let with_observed = |summary: &str, indicators: &str| {
            format!(
                "{{\"pattern\": null, \"summary\": {summary}, \
                 \"observedImage\": {{\"indicators\": {indicators}}}}}"
            )
        };
        assert!(is_valid_structured_reply(&with_observed(
            "\"ok\"",
            "[{\"name\": \"RSI\", \"value\": \"62\", \"visible\": true}]"
        )));
        assert!(is_valid_structured_reply(&format!(
            "```json\n{}\n```",
            with_observed("\"ok\"", "[{\"name\": \"EMA\"}]")
        )));
        assert!(
            !is_valid_structured_reply(&with_observed("\"ok\"", "[]")),
            "an empty observedImage.indicators must be rejected"
        );
        assert!(
            !is_valid_structured_reply("{\"summary\": \"ok\"}"),
            "a missing observedImage block must be rejected"
        );
        assert!(!is_valid_structured_reply("I'm a software engineering assistant, not a financial analyst."));
        assert!(!is_valid_structured_reply("{\"foo\": 1}"));
        assert!(!is_valid_structured_reply(&with_observed(
            "\"\"",
            "[{\"name\": \"EMA\"}]"
        )));
        assert!(!is_valid_structured_reply("[1, 2, 3]"));
    }

    fn payload(symbol: &str) -> CapturePayload {
        CapturePayload {
            broker_type: "coinswitch".into(),
            symbol: Some(symbol.into()),
            timeframe: Some("15m".into()),
            width: 780,
            height: 512,
            timestamp: 0,
            mime: "image/png".into(),
            status: "ok".into(),
            image: Some(String::new()),
            panes: 1,
        }
    }

    /// The grounded prompt must embed the real OHLC numbers verbatim, label
    /// them as ground truth, and forbid fabricating levels outside them.
    #[test]
    fn grounded_prompt_includes_real_ohlc_and_grounding_clause() {
        let p = payload("BTC/USDT Futures");
        let ctx = OhlcContext {
            open: Some(100.0),
            high: Some(105.0),
            low: Some(95.0),
            close: Some(102.0),
            range_low: None,
            range_high: None,
        };
        let prompt = build_chart_prompt(&p, Some(&ctx));
        assert!(prompt.contains("Open 100, High 105, Low 95, Close 102"), "prompt: {prompt}");
        assert!(
            prompt.contains("Use these as ground truth for the visible price scale"),
            "prompt: {prompt}"
        );
        assert!(
            prompt.contains("do not estimate price levels outside this context"),
            "prompt: {prompt}"
        );
        assert!(
            prompt.contains("return null for that field rather than fabricate a number"),
            "prompt: {prompt}"
        );
    }

    /// The grounded prompt with a candle-derived range must include the range
    /// anchor between the em-dash and treat it as context too.
    #[test]
    fn grounded_prompt_includes_visible_range() {
        let p = payload("ETH/USDT Futures");
        let ctx = OhlcContext {
            open: Some(1880.0),
            high: Some(1885.6),
            low: Some(1875.2),
            close: Some(1880.3),
            range_low: Some(1874.0),
            range_high: Some(1886.0),
        };
        let prompt = build_chart_prompt(&p, Some(&ctx));
        assert!(prompt.contains("24h range 1874–1886"), "prompt: {prompt}");
        assert!(prompt.contains("1880.3"), "prompt: {prompt}");
    }

    /// Without cached OHLC the prompt falls back to the original context-free
    /// wording — no ground-truth anchor, no grounding clause — so the weaker
    /// mode stays explicitly distinguishable (telemetry carries the flag).
    #[test]
    fn no_ohlc_falls_back_to_context_free_prompt() {
        let p = payload("SOL/USDT Futures");
        let prompt = build_chart_prompt(&p, None);
        assert!(!prompt.contains("ground truth"), "fallback must stay context-free: {prompt}");
        assert!(!prompt.contains("fabricate a number"), "fallback must stay context-free: {prompt}");
        assert!(
            prompt.contains("Respond with STRICT JSON only"),
            "schema shape must always be present: {prompt}"
        );
    }

    /// The prompt always demands an `observedImage` report of everything the
    /// model sees in the image (indicators, overlays, drawings, legend) as
    /// part of the required schema — present both with and without the OHLC
    /// grounding context.
    #[test]
    fn prompt_asks_for_observed_image_json() {
        let p = payload("BTC/USDT Futures");
        let ungrounded = build_chart_prompt(&p, None);
        let grounded = build_chart_prompt(&p, Some(&OhlcContext {
            open: Some(100.0),
            high: Some(105.0),
            low: Some(95.0),
            close: Some(102.0),
            range_low: None,
            range_high: None,
        }));
        for prompt in [&ungrounded, &grounded] {
            assert!(
                prompt.contains("\"observedImage\""),
                "prompt must demand the observed-image JSON block: {prompt}"
            );
            assert!(
                prompt.contains("must never be omitted"),
                "prompt must require the field unconditionally: {prompt}"
            );
            assert!(
                prompt.contains("return null or an empty list rather than guessing"),
                "prompt must forbid fabricating image contents: {prompt}"
            );
            assert!(
                prompt.contains("\"visible\": true"),
                "prompt must spell out the observed-image schema: {prompt}"
            );
        }
    }

    /// The OHLC-lookup fallback to a partially-filled context still carries
    /// every anchor that is present and refuses to emit an ungrounded one.
    #[test]
    fn partial_ohlc_uses_what_is_available() {
        let p = payload("XRP/USDT Futures");
        let ctx = OhlcContext {
            open: None,
            high: None,
            low: None,
            close: Some(1.0011),
            range_low: Some(0.9985),
            range_high: Some(1.0061),
        };
        let prompt = build_chart_prompt(&p, Some(&ctx));
        // Only the close + range anchors are present, so only they can appear.
        assert!(prompt.contains("Close 1.0011"), "prompt: {prompt}");
        assert!(prompt.contains("24h range 0.9985–1.0061"), "prompt: {prompt}");
        // The full O/H/L/C quartet must not be invented.
        assert!(!prompt.contains("Open "), "must not fabricate open: {prompt}");
        assert!(prompt.contains("ground truth"), "prompt: {prompt}");
    }

    /// Loose ticker matching used by the cache lookup handles slash/space
    /// differences between the page header and the capture payload.
    #[test]
    fn ticker_match_is_separator_insensitive() {
        assert!(ticker_matches("BTCINR", "BTC/USDT Futures"));
        assert!(ticker_matches("btc inr", "BTC/USDT"));
        assert!(ticker_matches("ETH/USDT", "ETHUSDT"));
        assert!(!ticker_matches("", "BTC/USDT"));
        assert!(!ticker_matches("SOL", ""));
        assert!(!ticker_matches("ABC", "XYZ"));
    }

    /// Drives the real retry orchestration with an injected runner: the first
    /// reply is persona-style prose (not JSON), the retried reply is valid
    /// structured JSON. Proves the retry fires exactly once and recovers.
    #[test]
    fn retry_recovers_on_second_attempt() {
        let malformed = json!({
            "type": "text",
            "part": { "type": "text", "text": "I'm a software engineering assistant, not a financial analyst." },
        })
        .to_string()
        .into_bytes();
        let payload_json = r#"{"pattern": "rising channel", "trend": "up", "signal": "buy", "support": null, "resistance": null, "indicators": [], "observedImage": {"indicators": [{"name": "EMA", "value": "2905", "visible": true}]}, "summary": "Ascending channel."}"#;
        let valid = json!({
            "type": "text",
            "part": { "type": "text", "text": payload_json },
        })
        .to_string()
        .into_bytes();
        let payload = CapturePayload {
            broker_type: "coinswitch".into(),
            symbol: Some("MAGMA/USDT Futures".into()),
            timeframe: None,
            width: 1458,
            height: 1058,
            timestamp: 0,
            mime: String::new(),
            status: "ok".into(),
            image: Some(String::new()),
            panes: 1,
        };
        let mut calls = 0;
        let run_attempt = |_attempt: u32| -> Result<AttemptOutput, String> {
            calls += 1;
            let bytes = if calls == 1 {
                malformed.clone()
            } else {
                valid.clone()
            };
            Ok((ExitStatus::default(), bytes))
        };
        let analysis = analyze_with_retry(None, &payload, VISION_MODEL, run_attempt)
            .expect("retry must recover");
        assert_eq!(calls, 2, "a malformed first reply must trigger exactly one retry");
        assert_eq!(analysis.status, "ok");
        assert_eq!(analysis.pattern.as_deref(), Some("rising channel"));
        assert_eq!(analysis.symbol, "MAGMA/USDT Futures");
    }

    /// Same orchestration with a runner that always returns malformed replies:
    /// the retry runs (attempt 2) and the error surfaces normally, never a
    /// third attempt.
    #[test]
    fn retry_fails_after_two_malformed_attempts() {
        let malformed = json!({
            "type": "text",
            "part": { "type": "text", "text": "I can't do financial analysis." },
        })
        .to_string()
        .into_bytes();
        let payload = CapturePayload {
            broker_type: "dhan".into(),
            symbol: Some("RELIANCE".into()),
            timeframe: None,
            width: 100,
            height: 100,
            timestamp: 0,
            mime: String::new(),
            status: "ok".into(),
            image: Some(String::new()),
            panes: 1,
        };
        let mut calls = 0;
        let run_attempt = |_attempt: u32| -> Result<AttemptOutput, String> {
            calls += 1;
            Ok((ExitStatus::default(), malformed.clone()))
        };
        let err = analyze_with_retry(None, &payload, VISION_MODEL, run_attempt).unwrap_err();
        assert_eq!(calls, 2, "must stop after a single retry");
        assert!(err.contains("not valid JSON"), "unexpected error: {err}");
    }

    /// End-to-end reality check of the spawn path the app uses: resolve the
    /// launcher, actually spawn the OpenCode CLI, and read its JSON-lines
    /// reply. Skipped when no real CLI can be found.
    #[test]
    fn real_open_code_spawn_produces_json_reply() {
        let (program, shell) =
            opencode_command().expect("opencode_command() must resolve a launcher");
        println!("opencode_resolved program={program} shell={shell:?}");
        #[cfg(windows)]
        {
            assert!(
                program.to_ascii_lowercase().ends_with(".exe"),
                "must resolve a real .exe through the resolver, got {program} (cmd fallback means resolve_opencode_exe() failed)"
            );
        }
        let prompt = r#"You are a professional technical analyst. Reply with STRICT JSON only and no markdown fences, exactly matching this shape: {"pattern": null, "trend": null, "signal": "neutral", "support": null, "resistance": null, "indicators": [], "observedImage": {"symbol": null, "timeframe": null, "priceScaleVisible": null, "ohlcLegend": null, "indicators": [{"name": "none-visible", "value": null, "visible": false}], "overlays": [], "drawings": [], "crosshairValues": null}, "summary": "ok"} "#;

        let mut child =
            spawn_opencode(prompt, DEFAULT_MODEL).expect("spawn_opencode() must succeed");
        println!("opencode_spawn_ok pid={}", child.id());

        let (status, stdout, stderr, timed_out) =
            wait_for_child_with_timeout(&mut child, 120_000);
        let tail = String::from_utf8_lossy(
            &stdout[stdout.len().saturating_sub(2000)..],
        )
        .to_string();
        println!(
            "opencode_exit success={} code={:?} timed_out={} elapsed_stdout_len={} stderr_len={}",
            status.success(), status.code(), timed_out, stdout.len(), stderr.len(),
        );
        println!("opencode_exit stderr_tail={:?}", String::from_utf8_lossy(&stderr));
        println!("opencode_exit stdout_tail={tail}");

        assert!(status.success(), "opencode CLI must exit 0");
        assert!(!timed_out, "must not hit the timeout");
        assert!(
            tail.contains("\"type\":\"text\""),
            "expected a JSON-lines text event on stdout, got: {tail}"
        );
        let reply = collect_opencode_reply(&stdout).expect("reply must be collectable");
        println!("opencode_reply_collected={reply}");
    }
}