# DOM Extraction — Payload Schema

Source of truth: `content/extractor.ts` (injected verbatim as `content/extractor.js`),
consumed by `src-tauri/src/extract.rs`.

```
broker webview (kite.zerodha.com/…)                 Rust (extract.rs)
┌──────────────────────────────┐     invoke       ┌─────────────────────────────┐
│  content/extractor.ts        │  ───────────────▶ │  extract_dom(payload)       │
│  - detect broker             │    { payload }    │  - validate + sanitise      │
│  - poll DOM every 500 ms     │  ◀─────────────── │  - store in cache (per broker)│
│  - grade data_quality        │   ExtractAck      │  - emit extraction-status   │
└──────────────────────────────┘                   │  get_extraction(broker)  ◀─ scanner/UI
                                                   └─────────────────────────────┘
```

On-demand chart capture (separate from the poll loop):

```
broker webview                                 Rust opencode.rs
┌──────────────────────────────┐ invoke        ┌─────────────────────────────┐
│  analyzeChart() (manual)      │──────────────▶│  capture_and_analyze_chart  │
│  - findChartCanvas()          │  payload      │  - cooldown per symbol      │
│  - canvas.toBlob(PNG, mem)    │               │  - spawn/reuse opencode serve│
│  - status ok/tainted/no_chart │  visual-      │  - pick vision model (/provider)
│  UI: trigger_chart_capture    │  analysis evt │  - session + data:URI image │
└──────────────────────────────┘ ◀────────────── │  - store JSON in VisualState│
                                                 └─────────────────────────────┘
```

## Top-level object

| Field                    | Type              | Required | Notes                                                                 |
|--------------------------|-------------------|----------|-----------------------------------------------------------------------|
| `brokerType`             | `string`          | yes      | Lowercased slug: `zerodha`, `angel`, `coinswitch`, … (≤32 chars)      |
| `timestamp`              | `number` (epoch ms)| yes     | When the DOM capture started (`Date.now()`)                            |
| `extractionDurationMs`   | `number`          | yes      | `performance.now()` delta; must be ≤ 60_000                            |
| `url`                    | `string`          | no       | `location.href` of the snapshot                                        |
| `dataQuality`            | `"good" \| "degraded" \| "empty"` | yes | see below                                     |
| `symbols`                | `SymbolExtract[]` | yes      | Capped at 500 in Rust                                                   |
| `chart`                  | `ChartExtract \| null` | no   | Best-effort read of the rendered price chart, see below                 |

Accepted snapshots are echoed to the UI on the `extraction-status` event:
`{ broker, quality, symbolCount, durationMs, receivedAtMs }`.

> **Field naming:** the extractor emits **camelCase** keys to match the Rust
> serde structs (`changePercent`, `ohlc1min`, `ohlc5min`, `orderBook`, …).
> Snake-case variants are ignored by `extract.rs`, so always emit camelCase.

## SymbolExtract

| Field              | Type                  | Notes                                                     |
|--------------------|-----------------------|-----------------------------------------------------------|
| `ticker`           | `string` (non-empty)  | Rows with a blank ticker are dropped                       |
| `name`             | `string \| null`      | Company / instrument name when the table exposes it        |
| `price`            | `number \| null`      | Last traded price (₹/$ stripped, commas parsed)           |
| `change`           | `number \| null`      | Absolute change (₹/$/pts) vs previous close                |
| `bid`              | `number \| null`      | Best bid                                                   |
| `ask`              | `number \| null`      | Best ask                                                   |
| `volume`           | `number \| null`      | `12,34,567` → `1234567`                                    |
| `changePercent`    | `number \| null`      | e.g. `+1.23%` → `1.23`                                     |
| `ohlc1min` / `ohlc5min` | `Ohlc \| null`   | Reserved                                                                          |
| `indicators`       | `Indicator[]`        | e.g. `{ kind: "EMA", period: 200, value: 21234.5 }`        |
| `orderBook`        | `OrderBookLevel[]`   | `{ side: "bid"\|"ask", price, qty }`                        |

A single change column is classified by its text: text containing `%` lands in
`changePercent`; otherwise a signed number lands in `change`. Column positions
are detected from table headers (Symbol | Company | LTP | Change % | Net Chg |
Volume …), so tables of any column order are read correctly.

| Type             | Shape                                                        |
|------------------|--------------------------------------------------------------|
| `Ohlc`           | `{ open, high, low, close }` — all `number`                 |
| `Indicator`      | `{ kind: string, period?: number, value: number }`           |
| `OrderBookLevel` | `{ side: string, price: number, qty: number }`               |

Rust truncates `indicators` to 16, `orderBook` to 8, `symbols` to 500.

## ChartExtract

| Field        | Type             | Notes                                                            |
|--------------|------------------|------------------------------------------------------------------|
| `instrument` | `string \| null` | Symbol the chart is showing (from the page title when found)     |
| `timeframe`  | `string \| null` | Active interval label (`5m`, `15m`, `1D`, …)                     |
| `candles`    | `Candle[]`       | OHLCV series; capped at 500 in Rust; non-finite candles dropped  |

| Type     | Shape                                              |
|----------|----------------------------------------------------|
| `Candle` | `{ time: number (epoch ms), open, high, low, close, volume?: number \| null }` |

Reading order: the extractor first looks for an ECharts instance on the chart
host (`window.echarts.getInstanceByDom(...).getOption()` → candlestick series).
If none, it falls back to the OHLC summary the broker renders as text next to
the chart (a single candle). Candle values come from the chart library, so
they reflect the selected timeframe / zoom state of the live page.

## `dataQuality`

- `good` — at least one symbol carried a numeric `price`.
- `degraded` — symbols found, but none had a usable quote.
- `empty` — the page DOM yielded no symbols.

## Example payloads

### zerodha — Kite watchlist (verified via `content/extractor.test.html`, headless)

```json
{
  "brokerType": "zerodha",
  "timestamp": 1755292000000,
  "extractionDurationMs": 2,
  "url": "https://kite.zerodha.com/",
  "dataQuality": "good",
  "symbols": [
    {
      "ticker": "RELIANCE",
      "name": "Reliance Industries",
      "price": 2900.45,
      "change": 35.2,
      "bid": null,
      "ask": null,
      "volume": 1234567,
      "changePercent": 1.23,
      "ohlc1min": null,
      "ohlc5min": null,
      "indicators": [],
      "orderBook": []
    }
  ],
  "chart": {
    "instrument": "RELIANCE",
    "timeframe": "5m",
    "candles": [
      { "time": 1786811980000, "open": 2900, "high": 2902, "low": 2899, "close": 2901, "volume": null }
    ]
  }
}
```

### angel — Angel One watchlist (heuristic selectors; tune against the live DOM)

```json
{
  "brokerType": "angel",
  "timestamp": 1755292001000,
  "extractionDurationMs": 3,
  "url": "https://angelone.in/",
  "dataQuality": "good",
  "symbols": [
    {
      "ticker": "TCS",
      "name": "Tata Consultancy",
      "price": 3950.1,
      "change": -17.85,
      "bid": null,
      "ask": null,
      "volume": 98765,
      "changePercent": -0.45,
      "ohlc1min": null,
      "ohlc5min": null,
      "indicators": [],
      "orderBook": []
    }
  ],
  "chart": null
}
```

### coinswitch — crypto market board

```json
{
  "brokerType": "coinswitch",
  "timestamp": 1755292002000,
  "extractionDurationMs": 4,
  "url": "https://www.coinswitch.co/",
  "dataQuality": "good",
  "symbols": [
    {
      "ticker": "BTC",
      "name": "Bitcoin",
      "price": 6543210,
      "change": 134080,
      "bid": null,
      "ask": null,
      "volume": 1234,
      "changePercent": 2.1,
      "ohlc1min": null,
      "ohlc5min": null,
      "indicators": [],
      "orderBook": []
    }
  ],
  "chart": null
}
```

### Empty / not-a-broker page

```json
{
  "brokerType": "zerodha",
  "timestamp": 1755292003000,
  "extractionDurationMs": 0,
  "url": "https://zerodha.com/about",
  "dataQuality": "empty",
  "symbols": [],
  "chart": null
}
```

## Reading snapshots (scanner side)

Tauri commands (main webview only — remote tabs may *only* ever call `extract_dom`):

```ts
// latest snapshot for one broker
const snap = await invoke('get_extraction', { broker: 'zerodha' });
// { broker, symbolCount, ageMs, receivedAtMs, data: { ...payload } }

// all brokers currently cached
const all = await invoke('list_extractions');
```

Cache semantics: one entry per `brokerType`, last-write-wins, evicted oldest-first
when more than 64 brokers are cached. A failed (`Err`) read means no snapshot yet.

## Performance budgets

- Content-script poll: 500 ms.
- Extraction target: < 200 ms per poll (measured via `extractionDurationMs`).
- Rust cache read: shared `Mutex<HashMap>` + clone of a shallow struct — comfortably
  serves 50 concurrent scanner reads well under 200 ms.
- No screenshots, no network calls in the extractor, fully offline-capable.

## On-demand chart capture payload

Triggered only by the manual "Analyze chart" path (never the polling loop).
`canvas.toBlob("image/png")` produces an in-memory PNG; the bytes are base64'd
and handed to Rust once, then freed. Field naming stays camelCase.

| Field        | Type              | Notes                                                        |
|--------------|-------------------|--------------------------------------------------------------|
| `brokerType` | `string`          | Detected broker slug                                         |
| `symbol`     | `string \| null`  | Chart instrument from the page title                         |
| `timeframe`  | `string \| null`  | Active interval label (`5m`, `1D`, …)                        |
| `width`/`height` | `number`      | Canvas bitmap size                                           |
| `timestamp`  | `number` (epoch ms)| `Date.now()` at capture                                      |
| `mime`       | `string`          | `"image/png"`                                                |
| `status`     | `"ok" \| "canvas_tainted" \| "no_chart"` | See below                          |
| `image`      | `string \| null`  | Base64 PNG, only present when `status === "ok"`              |

### `status` semantics

- `ok` — bitmap captured; handoff to `capture_and_analyze_chart` proceeds.
- `canvas_tainted` — `toBlob` threw / returned null (cross-origin draw, e.g.
  a chart composited from an untrusted texture). No throw; reported to the UI.
- `no_chart` — `findChartCanvas()` found nothing to capture.

## Visual analysis result (`VisualAnalysis`)

Cached by `VisualState` (keyed `broker\0symbol`, evicted oldest-first at 64) and
emitted on the `visual-analysis` event after the OpenCode round-trip:

| Field         | Type                                   | Notes                                        |
|---------------|----------------------------------------|----------------------------------------------|
| `broker`      | `string`                               | Broker slug                                  |
| `symbol`      | `string`                               | Symbol analyzed                              |
| `model`       | `string \| null`                       | `provider/model` used (picked at runtime)    |
| `status`      | `"ok" \| "canvas_tainted" \| "no_chart" \| "error" \| "rate_limited"` |      |
| `timestampMs` | `number` (epoch ms)                    | Completion time                              |
| `latencyMs`   | `number \| null`                       | Full round-trip time                         |
| `pattern`     | `string \| null`                       | e.g. `"bullish engulfing"` (from model JSON) |
| `trend`       | `"up" \| "down" \| "sideways" \| null` | Model read                                   |
| `signal`      | `"buy" \| "sell" \| "neutral" \| null` | Model read                                   |
| `support`/`resistance` | `number \| null`               | Price levels the model marked                |
| `indicators`  | `VisualIndicator[]`                    | `{ name, value, signal }`                    |
| `summary`     | `string \| null`                       | Model summary, or raw text when non-JSON reply fell through |
| `raw`         | `string \| null`                       | Full model text (clipped to 4000 chars)      |
| `error`       | `string \| null`                       | Failure reason when `status !== "ok"`        |

The model is asked for STRICT JSON; if the reply doesn't parse, `summary` falls
back to the raw text. Only the lightweight summary is cached — never the image.