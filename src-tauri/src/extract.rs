//! Broker-agnostic DOM extraction subsystem.
//!
//! The content script embedded in every broker webview (`content/extractor.js`)
//! reads the trading page DOM and posts a snapshot to [`extract_dom`]. This
//! module validates the payload against the extraction schema, stores the
//! latest snapshot per broker in a small concurrent cache, and serves it to
//! the scanner via [`get_extraction`].
//!
//! Cache reads are lock-and-clone of a shallow struct, so 50+ concurrent
//! scanner reads stay well under the 200&nbsp;ms budget.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::logger::Logger;

/// Event emitted to the UI whenever a broker snapshot is accepted.
pub const EVT_EXTRACTION_STATUS: &str = "extraction-status";

/// Event emitted to the UI whenever a chart visual-analysis completes.
pub const EVT_VISUAL: &str = "visual-analysis";

/// Maximum number of brokers we keep snapshots for before evicting the oldest.
const CACHE_CAPACITY: usize = 64;

/// Known `data_quality` values produced by the content script.
///
/// - `good`: at least one symbol carried a numeric quote (price/bid/ask).
/// - `degraded`: symbols were found but none had a usable quote.
/// - `empty`: the DOM yielded no symbols at all.
pub const QUALITY_GOOD: &str = "good";
pub const QUALITY_DEGRADED: &str = "degraded";
pub const QUALITY_EMPTY: &str = "empty";

/// Maximum number of symbols accepted from a single DOM snapshot.
const MAX_SYMBOLS: usize = 500;

/// Maximum number of chart candles accepted from a single DOM snapshot.
const MAX_CANDLES: usize = 500;

// ---------------------------------------------------------------------------
// Schema (mirrors docs/extraction-schema.md and content/extractor.js)
// ---------------------------------------------------------------------------

/// One level of the order book.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderBookLevel {
    /// `"bid"` or `"ask"`.
    pub side: String,
    pub price: f64,
    pub qty: f64,
}

/// Open / high / low / close for a symbol over a window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ohlc {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

/// A computed indicator on a symbol (e.g. EMA-200).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Indicator {
    pub kind: String,
    #[serde(default)]
    pub period: Option<u32>,
    pub value: f64,
}

/// One market symbol extracted from the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolExtract {
    pub ticker: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub price: Option<f64>,
    #[serde(default)]
    pub change: Option<f64>,
    #[serde(default)]
    pub bid: Option<f64>,
    #[serde(default)]
    pub ask: Option<f64>,
    #[serde(default)]
    pub volume: Option<f64>,
    #[serde(default)]
    pub change_percent: Option<f64>,
    #[serde(default)]
    pub ohlc_1min: Option<Ohlc>,
    #[serde(default)]
    pub ohlc_5min: Option<Ohlc>,
    #[serde(default)]
    pub indicators: Vec<Indicator>,
    #[serde(default)]
    pub order_book: Vec<OrderBookLevel>,
}

/// One OHLCV candle as rendered on the broker's price chart.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candle {
    /// Epoch ms of the candle's open time.
    pub time: u64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    #[serde(default)]
    pub volume: Option<f64>,
}

/// Best-effort read of the chart currently rendered by the broker page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartExtract {
    /// Instrument the chart is showing, when detectable.
    #[serde(default)]
    pub instrument: Option<String>,
    /// Timeframe label the broker displays (e.g. "5m", "15m", "1D").
    #[serde(default)]
    pub timeframe: Option<String>,
    /// Candle series read from the chart library / DOM.
    #[serde(default)]
    pub candles: Vec<Candle>,
}

/// Payload sent by the content script (unvalidated).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawExtraction {
    /// One of `angel`, `coinswitch`, `dhan`, or any future broker slug.
    pub broker_type: String,
    /// Epoch milliseconds when the DOM capture started.
    pub timestamp: u64,
    /// How long the DOM read took, in milliseconds.
    pub extraction_duration_ms: u64,
    #[serde(default)]
    pub url: String,
    #[serde(default = "default_quality")]
    pub data_quality: String,
    #[serde(default)]
    pub symbols: Vec<SymbolExtract>,
    /// Chart currently rendered by the page (best-effort), when found.
    #[serde(default)]
    pub chart: Option<ChartExtract>,
}

fn default_quality() -> String {
    QUALITY_EMPTY.into()
}

/// Snapshot served to the scanner, with receive-time freshness metadata.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionSnapshot {
    pub broker: String,
    pub data: RawExtraction,
    /// Epoch ms when Rust accepted the snapshot into the cache.
    pub received_at_ms: u64,
    /// Milliseconds since `received_at_ms` (negative if clock-skewed).
    pub age_ms: i64,
    /// Number of symbols in the snapshot.
    pub symbol_count: usize,
}

/// Acknowledgment returned to the content script.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractAck {
    pub accepted: bool,
    pub broker: String,
    pub symbol_count: usize,
    pub quality: String,
}

// ---------------------------------------------------------------------------
// Visual analysis (chart bitmap → AI, see opencode.rs)
// ---------------------------------------------------------------------------

/// One named observation the vision model attaches to the chart.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualIndicator {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub signal: String,
}

/// One indicator the vision model reports seeing rendered in the chart image.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedIndicator {
    pub name: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub visible: bool,
}

/// The `observedImage` block of the model's reply — a faithful list of what
/// is actually visible in the captured screenshot (used to verify the broker
/// really renders the asked-for indicators).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedImage {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub timeframe: Option<String>,
    #[serde(default)]
    pub price_scale_visible: Option<String>,
    #[serde(default)]
    pub ohlc_legend: Option<String>,
    #[serde(default)]
    pub indicators: Vec<ObservedIndicator>,
    #[serde(default)]
    pub overlays: Vec<String>,
    #[serde(default)]
    pub drawings: Vec<String>,
    #[serde(default)]
    pub crosshair_values: Option<String>,
}

/// Lightweight result of an on-demand chart analysis, cached per broker+symbol.
///
/// Only the JSON summary is retained (never the image).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualAnalysis {
    pub broker: String,
    pub symbol: String,
    #[serde(default)]
    pub model: Option<String>,
    /// `ok` | `canvas_tainted` | `no_chart` | `error` | `rate_limited`.
    pub status: String,
    /// Epoch ms when the analysis finished (or failed) on the Rust side.
    pub timestamp_ms: u64,
    #[serde(default)]
    pub latency_ms: Option<u64>,
    /// (status == "ok") model's chart-pattern read, e.g. "bullish engulfing".
    #[serde(default)]
    pub pattern: Option<String>,
    /// (status == "ok") `up` | `down` | `sideways`.
    #[serde(default)]
    pub trend: Option<String>,
    /// (status == "ok") `buy` | `sell` | `neutral`.
    #[serde(default)]
    pub signal: Option<String>,
    #[serde(default)]
    pub support: Option<f64>,
    #[serde(default)]
    pub resistance: Option<f64>,
    #[serde(default)]
    pub indicators: Vec<VisualIndicator>,
    /// What the vision model reports seeing in the chart image (indicators,
    /// overlay names, legend text) — the `observedImage` block of its reply.
    #[serde(default)]
    pub observed: Option<ObservedImage>,
    #[serde(default)]
    pub summary: Option<String>,
    /// Full raw model text (fallback when the reply isn't parseable JSON).
    #[serde(default)]
    pub raw: Option<String>,
    /// (status != "ok") human-readable failure reason.
    #[serde(default)]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

pub(crate) struct CacheEntry {
    snapshot: RawExtraction,
    updated_at_ms: u64,
}

/// Latest accepted snapshot per broker, safe for many concurrent readers.
pub struct ExtractionState(pub(crate) Mutex<HashMap<String, CacheEntry>>);

impl Default for ExtractionState {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

/// Latest visual analysis per key, safe for many concurrent readers.
pub struct VisualState(pub(crate) Mutex<HashMap<String, VisualAnalysis>>);

impl Default for VisualState {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

/// Maximum number of visual analyses we keep before evicting the oldest.
const VISUAL_CACHE_CAPACITY: usize = 64;

fn visual_key(broker: &str, symbol: &str) -> String {
    format!("{broker}\u{0}{symbol}")
}

/// Cache a completed visual analysis (keyed broker+symbol, evict oldest).
pub fn store_visual(app: &AppHandle, analysis: VisualAnalysis) {
    let key = visual_key(&analysis.broker, &analysis.symbol);
    let state = app.state::<VisualState>();
    let mut map = state.0.lock().unwrap();
    if map.len() >= VISUAL_CACHE_CAPACITY {
        if let Some(oldest) = map
            .iter()
            .min_by_key(|(_, a)| a.timestamp_ms)
            .map(|(k, _)| k.clone())
        {
            map.remove(&oldest);
        }
    }
    map.insert(key, analysis);
}

/// Latest cached visual analysis for a broker+symbol.
pub fn latest_visual(app: &AppHandle, broker: &str, symbol: &str) -> Option<VisualAnalysis> {
    let key = visual_key(broker, symbol);
    let state = app.state::<VisualState>();
    let map = state.0.lock().unwrap();
    map.get(&key).cloned()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Normalize a broker slug: lowercase, ASCII, alphanumerics plus `_`/`-`.
pub(crate) fn normalize_broker(raw: &str) -> Option<String> {
    let slug = raw.trim().to_ascii_lowercase();
    if slug.is_empty() || slug.chars().count() > 32 {
        return None;
    }
    if slug
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Some(slug)
    } else {
        None
    }
}

/// Validate and normalize a raw payload from the content script.
fn validate_payload(raw: &mut RawExtraction) -> Result<String, String> {
    let broker = normalize_broker(&raw.broker_type)
        .ok_or_else(|| "extraction payload missing a valid broker type".to_string())?;

    if raw.timestamp == 0 {
        return Err("extraction payload missing timestamp".into());
    }
    if raw.data_quality.is_empty() {
        raw.data_quality = QUALITY_EMPTY.into();
    }
    if !matches!(
        raw.data_quality.as_str(),
        QUALITY_GOOD | QUALITY_DEGRADED | QUALITY_EMPTY
    ) {
        return Err(format!("unknown data_quality '{}'", raw.data_quality));
    }
    if raw.extraction_duration_ms > 60_000 {
        return Err("extraction_duration_ms out of range".into());
    }

    // Drop malformed symbols and cap the list.
    let mut kept: Vec<SymbolExtract> = Vec::new();
    for mut sym in raw.symbols.drain(..) {
        if sym.ticker.trim().is_empty() {
            continue;
        }
        sym.indicators.truncate(16);
        sym.order_book.truncate(8);
        kept.push(sym);
        if kept.len() >= MAX_SYMBOLS {
            break;
        }
    }
    raw.symbols = kept;

    // Cap chart candles; drop candles with unusable OHLC.
    if let Some(chart) = &mut raw.chart {
        chart.candles.retain(|c| {
            c.open.is_finite() && c.high.is_finite() && c.low.is_finite() && c.close.is_finite()
        });
        chart.candles.truncate(MAX_CANDLES);
    }

    Ok(broker)
}

/// Accept a DOM snapshot from a broker webview into the cache.
///
/// Runs fast (<1&nbsp;ms in the common case): parse + a short Mutex lock.
pub fn ingest(app: &AppHandle, mut payload: RawExtraction) -> ExtractAck {
    let received = now_ms();
    match validate_payload(&mut payload) {
        Ok(broker) => {
            {
                let state = app.state::<ExtractionState>();
                let mut map = state.0.lock().unwrap();
                if map.len() >= CACHE_CAPACITY {
                    if let Some(oldest) = map
                        .iter()
                        .min_by_key(|(_, e)| e.updated_at_ms)
                        .map(|(k, _)| k.clone())
                    {
                        map.remove(&oldest);
                    }
                }
                map.insert(
                    broker.clone(),
                    CacheEntry {
                        snapshot: payload.clone(),
                        updated_at_ms: received,
                    },
                );
            }

            app.state::<Logger>().log(
                "extraction_accepted",
                json!({
                    "broker": broker,
                    "quality": payload.data_quality,
                    "symbols": payload.symbols.len(),
                    "duration_ms": payload.extraction_duration_ms,
                    "url": payload.url,
                }),
            );
            let _ = app.emit(
                EVT_EXTRACTION_STATUS,
                json!({
                    "broker": broker,
                    "quality": payload.data_quality,
                    "symbolCount": payload.symbols.len(),
                    "durationMs": payload.extraction_duration_ms,
                    "receivedAtMs": received,
                }),
            );

            ExtractAck {
                accepted: true,
                broker,
                symbol_count: payload.symbols.len(),
                quality: payload.data_quality,
            }
        }
        Err(reason) => {
            app.state::<Logger>().log("extraction_rejected", json!({ "reason": reason }));
            ExtractAck {
                accepted: false,
                broker: payload.broker_type,
                symbol_count: 0,
                quality: payload.data_quality,
            }
        }
    }
}

/// Latest snapshot for a broker, or an error when none is cached yet.
pub fn latest(app: &AppHandle, broker: &str) -> Result<ExtractionSnapshot, String> {
    let slug = normalize_broker(broker).ok_or_else(|| "invalid broker identifier".to_string())?;
    let state = app.state::<ExtractionState>();
    let map = state.0.lock().unwrap();
    let entry = map.get(&slug).ok_or_else(|| {
        format!("no extraction snapshot cached for broker '{slug}' yet")
    })?;
    let now = now_ms();
    Ok(ExtractionSnapshot {
        broker: slug,
        data: entry.snapshot.clone(),
        received_at_ms: entry.updated_at_ms,
        age_ms: now as i64 - entry.updated_at_ms as i64,
        symbol_count: entry.snapshot.symbols.len(),
    })
}

/// All brokers that currently have a cached snapshot.
///
/// Lets the scanner enumerate what it can poll without a fixed config.
pub fn list_cached(app: &AppHandle) -> Vec<ExtractionSnapshot> {
    let state = app.state::<ExtractionState>();
    let map = state.0.lock().unwrap();
    map.iter()
        .map(|(broker, e)| {
            let now = now_ms();
            ExtractionSnapshot {
                broker: broker.clone(),
                data: e.snapshot.clone(),
                received_at_ms: e.updated_at_ms,
                age_ms: now as i64 - e.updated_at_ms as i64,
                symbol_count: e.snapshot.symbols.len(),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tauri commands (wired in commands.rs / lib.rs)
// ---------------------------------------------------------------------------

/// Entry point for the content script: validate + cache + ack.
#[tauri::command]
pub fn extract_dom(app: AppHandle, payload: RawExtraction) -> ExtractAck {
    ingest(&app, payload)
}

/// Pull the latest snapshot for a broker. Targets the need of a fast scanner:
/// O(1) cache read, returns a full deep-ish clone of a small struct.
#[tauri::command]
pub fn get_extraction(app: AppHandle, broker: String) -> Result<ExtractionSnapshot, String> {
    latest(&app, &broker)
}

/// Enumerate every broker snapshot currently cached.
#[tauri::command]
pub fn list_extractions(app: AppHandle) -> Vec<ExtractionSnapshot> {
    list_cached(&app)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RawExtraction {
        RawExtraction {
            broker_type: " Dhan ".into(),
            timestamp: 1_700_000_000_000,
            extraction_duration_ms: 42,
            url: "https://dhan.co/".into(),
            data_quality: "good".into(),
            symbols: vec![SymbolExtract {
                ticker: "NIFTY 50".into(),
                name: Some("Nifty 50 Spot".into()),
                price: Some(22100.5),
                change: Some(92.4),
                bid: Some(22100.0),
                ask: Some(22101.0),
                volume: Some(123_456.0),
                change_percent: Some(0.42),
                ohlc_1min: None,
                ohlc_5min: None,
                indicators: vec![Indicator { kind: "EMA".into(), period: Some(200), value: 21_234.5 }],
                order_book: vec![OrderBookLevel { side: "bid".into(), price: 22100.0, qty: 150.0 }],
            }],
            chart: Some(ChartExtract {
                instrument: Some("NIFTY 50".into()),
                timeframe: Some("5m".into()),
                candles: vec![Candle {
                    time: 1_700_000_000_000,
                    open: 22_080.0,
                    high: 22_105.0,
                    low: 22_070.0,
                    close: 22_100.0,
                    volume: Some(1_000.0),
                }],
            }),
        }
    }

    #[test]
    fn normalize_broker_slug() {
        assert_eq!(normalize_broker(" Dhan ").as_deref(), Some("dhan"));
        assert_eq!(normalize_broker("ANGEL").as_deref(), Some("angel"));
        assert_eq!(normalize_broker("coin-switch_").as_deref(), Some("coin-switch_"));
        assert_eq!(normalize_broker(""), None);
        assert_eq!(normalize_broker("a b"), None);
        assert_eq!(normalize_broker("日本語"), None);
    }

    #[test]
    fn validate_accepts_good_payload() {
        let mut payload = sample();
        assert_eq!(validate_payload(&mut payload).as_deref(), Ok("dhan"));
        assert_eq!(payload.quality_for_test(), QUALITY_GOOD);
        assert_eq!(payload.symbols.len(), 1);
    }

    #[test]
    fn validate_rejects_bad_broker() {
        let mut payload = sample();
        payload.broker_type = "".into();
        assert!(validate_payload(&mut payload).is_err());
    }

    #[test]
    fn validate_rejects_bad_quality() {
        let mut payload = sample();
        payload.data_quality = "jam".into();
        assert!(validate_payload(&mut payload).is_err());
    }

    #[test]
    fn validate_caps_and_keeps_chart() {
        let mut payload = sample();
        payload.chart = Some(ChartExtract {
            instrument: Some("NIFTY 50".into()),
            timeframe: Some("1m".into()),
            candles: (0..MAX_CANDLES)
                .map(|i| Candle {
                    time: 1_700_000_000_000 + i as u64,
                    open: 100.0,
                    high: 100.0,
                    low: 100.0,
                    close: 100.0,
                    volume: None,
                })
                .collect(),
        });
        payload.chart.as_mut().unwrap().candles[0].close = f64::NAN;
        assert!(validate_payload(&mut payload).is_ok());
        let candles = payload.chart.as_ref().unwrap().candles.len();
        assert_eq!(candles, MAX_CANDLES - 1, "non-finite candle dropped");
        payload.chart = Some(ChartExtract {
            instrument: Some("NIFTY 50".into()),
            timeframe: Some("1m".into()),
            candles: (0..(MAX_CANDLES + 50))
                .map(|i| Candle {
                    time: 1_700_000_000_000 + i as u64,
                    open: 100.0,
                    high: 100.0,
                    low: 100.0,
                    close: 100.0,
                    volume: None,
                })
                .collect(),
        });
        assert!(validate_payload(&mut payload).is_ok());
        assert_eq!(payload.chart.as_ref().unwrap().candles.len(), MAX_CANDLES, "capped");
    }

    #[test]
    fn validate_drops_blank_tickers_and_caps() {
        let mut payload = sample();
        payload.symbols.push(SymbolExtract {
            ticker: "   ".into(),
            name: None,
            price: None,
            change: None,
            bid: None,
            ask: None,
            volume: None,
            change_percent: None,
            ohlc_1min: None,
            ohlc_5min: None,
            indicators: vec![],
            order_book: vec![],
        });
        assert!(validate_payload(&mut payload).is_ok());
        assert_eq!(payload.symbols.len(), 1);
    }

    trait QualityForTest {
        fn quality_for_test(&self) -> &str;
    }
    impl QualityForTest for RawExtraction {
        fn quality_for_test(&self) -> &str {
            &self.data_quality
        }
    }
}