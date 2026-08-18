export type TabState = "home" | "loading" | "loaded" | "blocked" | "error";

export interface Tab {
  label: string;
  url: string;
  title: string;
  state: TabState;
  isHome: boolean;
}

export interface Bookmark {
  name: string;
  url: string;
  color?: string;
}

export interface BrowserSettings {
  theme: "dark" | "light";
  homepage: "landing" | "custom";
  homeUrl: string;
}

export interface AppConfig {
  whitelist: string[];
  bookmarks: Bookmark[];
  settings: BrowserSettings;
}

export interface UrlCheck {
  allowed: boolean;
  url?: string;
  reason?: string;
}

export interface BlockedInfo {
  tab: string;
  url: string;
  reason: string;
}

export interface TabEvent {
  label: string;
  url: string;
  title: string;
  state: TabState;
  isHome: boolean;
}

// ---- DOM extraction subsystem (mirrors src-tauri/src/extract.rs) ---------

export type DataQuality = "good" | "degraded" | "empty";

export interface OrderBookLevel {
  side: "bid" | "ask";
  price: number;
  qty: number;
}

export interface Ohlc {
  open: number;
  high: number;
  low: number;
  close: number;
}

export interface Indicator {
  kind: string;
  period?: number | null;
  value: number;
}

export interface Candle {
  time: number;
  open: number;
  high: number;
  low: number;
  close: number;
  volume?: number | null;
}

export interface ChartExtract {
  instrument?: string | null;
  timeframe?: string | null;
  candles: Candle[];
}

export interface SymbolExtract {
  ticker: string;
  name?: string | null;
  price?: number | null;
  change?: number | null;
  bid?: number | null;
  ask?: number | null;
  volume?: number | null;
  changePercent?: number | null;
  ohlc1min?: Ohlc | null;
  ohlc5min?: Ohlc | null;
  indicators: Indicator[];
  orderBook: OrderBookLevel[];
}

export interface RawExtraction {
  brokerType: string;
  timestamp: number;
  extractionDurationMs: number;
  url?: string;
  dataQuality: DataQuality;
  symbols: SymbolExtract[];
  chart?: ChartExtract | null;
}

export interface ExtractionSnapshot {
  broker: string;
  data: RawExtraction;
  receivedAtMs: number;
  ageMs: number;
  symbolCount: number;
}

export interface ExtractionStatus {
  broker: string;
  quality: DataQuality;
  symbolCount: number;
  durationMs: number;
  receivedAtMs: number;
}

// ---- on-demand chart visual analysis (mirrors src-tauri/src/opencode.rs & extract.rs) --

/** One named observation the vision model attaches to the chart. */
export interface VisualIndicator {
  name: string;
  value: string;
  signal: string;
}

/** One indicator the vision model reports seeing rendered in the chart image. */
export interface ObservedIndicator {
  name: string;
  value?: string | null;
  visible: boolean;
}

/** The `observedImage` block of the model's reply — faithful list of what is
 *  actually visible in the captured screenshot. */
export interface ObservedImage {
  symbol?: string | null;
  timeframe?: string | null;
  priceScaleVisible?: string | null;
  ohlcLegend?: string | null;
  indicators: ObservedIndicator[];
  overlays: string[];
  drawings: string[];
  crosshairValues?: string | null;
}

export type VisualStatus =
  | "ok"
  | "canvas_tainted"
  | "no_chart"
  | "error"
  | "rate_limited";

/** Lightweight result of an on-demand chart analysis (never the image). */
export interface VisualAnalysis {
  broker: string;
  symbol: string;
  model?: string | null;
  status: VisualStatus;
  timestampMs: number;
  latencyMs?: number | null;
  pattern?: string | null;
  trend?: "up" | "down" | "sideways" | null;
  signal?: "buy" | "sell" | "neutral" | null;
  support?: number | null;
  resistance?: number | null;
  indicators: VisualIndicator[];
  observed?: ObservedImage | null;
  summary?: string | null;
  raw?: string | null;
  error?: string | null;
}

/** Immediate acknowledgment for `capture_and_analyze_chart`. */
export interface CaptureAck {
  accepted: boolean;
  broker: string;
  status: "started" | "rate_limited" | "canvas_tainted" | "no_chart";
  retryAfterMs: number;
}

/** Payload emitted on the `visual-analysis` event. */
export interface VisualAnalysisEvent {
  broker: string;
  symbol: string;
  analysis: VisualAnalysis;
}

// ---- bundled opencode CLI authentication -----------------------------------

/** Snapshot of the opencode CLI credential state for the current user. */
export interface OpenCodeLoginStatus {
  loggedIn: boolean;
  providers: string[];
  credsPath?: string | null;
}