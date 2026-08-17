# Broker Extraction — Integration Guide

This guide explains how the whitelist browser extracts live market data out of
logged-in third-party broker webviews and how to add or tune a broker.

## Architecture

1. Every pool webview (`tab-1` … `tab-16`) is created with the extractor as an
   **initialization script** (`WebviewBuilder::initialization_script` in
   `src-tauri/src/browser.rs`). WebView2 re-runs it for every top-level document,
   so it fires again after any navigation / reload / login redirect.
2. The script polls the page DOM every **500 ms**, detects the broker, reads
   symbols through fallback selectors, grades the snapshot and `invoke`s it.
3. `extract_dom` (Rust) validates + sanitises the payload, caches it per broker
   (`ExtractionState` in `extract.rs`, `Mutex<HashMap>`, 50+ concurrent reads
   safe), logs it, and emits `extraction-status`.
4. The scanner/UI pulls snapshots with `get_extraction(broker)` /
   `list_extractions()`.

## Why remote tabs can only reach `extract_dom`

Tauri v2 denies IPC to non-local origins unless a **capability** grants a
specific command to that webview **and** lists the remote origins. The build is
configured so the application has an explicit ACL manifest
(`src-tauri/build.rs` → `AppManifest::commands(&[…])`), which turns ACL
enforcement on for every command. The lock matrix is therefore:

| Webview      | Origin                       | Commands reachable                     |
|--------------|------------------------------|----------------------------------------|
| `main`       | `tauri://localhost` (UI)     | the 16 UI commands + `get_extraction`, `list_extractions`, `get_visual_analysis`, `list_visual_analyses`, `trigger_chart_capture` |
| `tab-1..16`  | whitelisted broker domains   | `extract_dom` + `capture_and_analyze_chart` |
| `tab-1..16`  | any other origin             | none (random popups, ad frames, etc.)   |

`allow-open-url`, `allow-run`-style APIs are *not* granted to child webviews, so
compromised broker pages cannot call any other Tauri API.

## On-demand visual chart analysis (OpenCode CLI)

Beyond DOM text, the panel can capture the **rendered chart canvas** and ask an
AI vision model to analyze it. It never touches disk:

1. The UI "Analyze chart" button calls `trigger_chart_capture(broker)` (main
   webview → Rust), which evals `window.analyzeChart()` in the live broker tab.
2. `extractor.js` captures the chart `<canvas>` via `toBlob("image/png")` —
   **in memory only** — and invokes `capture_and_analyze_chart` with the bytes.
   A cross-origin/tainted canvas yields `status:"canvas_tainted"` (never a
   throw); no canvas yields `status:"no_chart"`.
3. Rust (`opencode.rs`) rate-limits per symbol (default 10 s), then lazily
   spawns a local `opencode serve` (or reuses `OPENCODE_SERVER_URL`), picks the
   first vision-capable free model at runtime (`/provider` capabilities), sends
   the image as a `data:` URI part, and caches only the JSON summary via
   `ExtractState`-style `VisualState` (`extract.rs`).
4. The finished `VisualAnalysis` arrives on the `visual-analysis` event and the
   panel renders it (pattern / trend / signal / support / resistance /
   indicators). Any failure (error, rate limit, taint) is surfaced as an
   explicit `status` + `error` string.

Repeat captures of the same symbol within the cooldown return
`status:"rate_limited"` without hitting the model.

## Files that make this work

| File                                      | Role                                                        |
|-------------------------------------------|-------------------------------------------------------------|
| `content/extractor.ts` → `extractor.js`   | The injected script (keep bytes in sync — `.js` is a copy of `.ts`) |
| `src-tauri/src/browser.rs`                | `.initialization_script(include_str!("../../content/extractor.js"))` |
| `src-tauri/src/commands.rs`                | UI commands incl. `trigger_chart_capture` |
| `src-tauri/src/opencode.rs`                | Local `opencode serve` client, model pick, session/message, cooldown |
| `src-tauri/src/extract.rs`                | Schema, validation, cache, `extract_dom`/`get_extraction`/`list_extractions` + `VisualState` |
| `src-tauri/build.rs`                      | `AppManifest::commands(&[…])` — declares the ACL manifest    |
| `src-tauri/capabilities/default.json`     | Grants the 16 UI commands + read commands to `main`          |
| `src-tauri/capabilities/extractor.json`   | Grants `extract-dom` to `tab-1..16` for the broker origins   |
| `src-tauri/src/lib.rs`                    | Manages `ExtractionState`; registers the three commands      |
| `docs/extraction-schema.md`               | Payload reference + example payloads per broker              |

## Adding a new broker

1. **Whitelist** the domain (UI → Settings, or `add_domain`).
2. **Remote capability**: add its origins to `remote.urls` in
   `src-tauri/capabilities/extractor.json`, e.g. `https://*.mybroker.in/*`.
3. **Detection**: add a block to `BROKERS` in `content/extractor.ts`
   (`hosts`, `titleMarkers`, `domMarkers`).
4. **Extraction**: add `FIELD_SEL[broker]` and `ROW_SEL[broker]`; if the site
   renders a plain `<table>`, the header-aware fallback reads full rows
   (ticker, company name, LTP, change %, absolute change, volume) with zero
   configuration regardless of column order.
5. **Charts (optional)**: the chart reader prefers an ECharts instance on the
   page (`window.echarts.getInstanceByDom(...).getOption()`), otherwise falls
   back to the OHLC summary text the broker paints next to the chart. If a
   broker renders charts some other way, extend `findChartCanvas()` /
   `collectEChartsCandles()` in `extractor.ts`.

Rebuild (`cargo build --release`), re-verify the schema doc example, and run the
harness below.

## Tuning selectors against a live page

Selectors are best-effort heuristics: page layouts change. To adapt:

1. Open the broker tab in-device (`open_url` on `kite.zerodha.com` etc.),
   enable DevTools for the app (or run the extraction page in a normal browser).
2. Inspect the rows that hold a symbol.
3. Update the selector arrays in `extractor.ts`: each array is tried in order
   and the first hit wins, so append new selectors instead of replacing.

## Testing

### In a plain browser (no Tauri host) — headless

```html
<!-- content/extractor.test.html renders a fake Zerodha-like watchlist and
     loads content/extractor.js standalone. -->
```

```bash
msedge --headless=new --disable-gpu \
  --user-data-dir="$env:TEMP\lsb-edge" \
  --virtual-time-budget=2500 \
  --dump-dom "file://%CD%/content/extractor.test.html?lsb=test"
```

The dumped DOM carries `data-lsb-extract` on `<body>` with
`{ quality, durationMs, symbols, first, chart }`. Verified output for the fixture
(rows with company name + change %, and a fake ECharts candle series):

```json
{"quality":"good","durationMs":0,"symbols":3,"first":{"ticker":"RELIANCE","name":"Reliance Industries","price":2900.45,"change":null,"bid":null,"ask":null,"volume":1234567,"changePercent":1.23,"ohlc1min":null,"ohlc5min":null,"indicators":[],"orderBook":[]},"chart":{"instrument":"RELIANCE","timeframe":"5m","candles":[{"time":1786811980549,"open":2900,"close":2901,"high":2902,"low":2899,"volume":null}]}}
```

> **Field names matter:** the extractor emits **camelCase** only
> (`changePercent`, `ohlc1min`, `orderBook`, …). Rust deserializes camelCase,
> so keep every emit site camelCase — snake-case keys are silently dropped.

(In `?lsb=test` mode the script also pushes every capture to
`window.__EXTRACTIONS__` and fires an `extraction-ready` CustomEvent.)

### In the running app

1. `npm run tauri dev`, open a whitelisted broker tab and log in.
2. Watch `activity.log` — each accepted snapshot writes
   `{"event":"extraction_accepted",...}`; rejected payloads write
   `extraction_rejected` with the reason.
3. From the UI or DevTools:
   `await window.__TAURI__?.core.invoke('get_extraction', { broker: 'zerodha' })`
   (or via the app's typed `invoke()`).
4. Subscribe to the `extraction-status` event to see freshness ticks:
   `{ broker, quality, symbolCount, durationMs, receivedAtMs }`.

## Validating live prices

Cross-check `price`/`changePercent`/`volume` for a few symbols against the
broker's own page or a public quote; mismatch usually means a selector is
grabbing the wrong column — tune `FIELD_SEL` for that broker.

## Latency under load

- `extract.rs::latest()` is a single lock + clone; benchmark with 50 concurrent
  `get_extraction` calls (each `ExtractionSnapshot` clone is shallow). Observed
  budget: well under the 200 ms read budget.
- `extractionDurationMs` (set by the script) is the honest DOM-read cost; keep
  selectors scoped so the poll stays < 200 ms.

## Offline capability

The extractor never makes network calls and takes no screenshots; it reads only
the already-loaded DOM. If a broker page serves no live data without connection,
`dataQuality` degrades to `empty`/`degraded` naturally.