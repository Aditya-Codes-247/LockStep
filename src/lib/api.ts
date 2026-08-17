import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AppConfig,
  BlockedInfo,
  BrowserSettings,
  CaptureAck,
  ExtractionSnapshot,
  ExtractionStatus,
  Tab,
  UrlCheck,
  VisualAnalysis,
  VisualAnalysisEvent,
} from "./types";

/** Thin wrappers around the Rust commands (see src-tauri/src/commands.rs). */
export const api = {
  checkUrl: (raw: string) => invoke<UrlCheck>("check_url", { raw }),
  getConfig: () => invoke<AppConfig>("get_config"),
  addDomain: (domain: string) => invoke<AppConfig>("add_domain", { domain }),
  removeDomain: (domain: string) => invoke<AppConfig>("remove_domain", { domain }),
  addBookmark: (name: string, url: string, color?: string) =>
    invoke<AppConfig>("add_bookmark", { name, url, color: color ?? null }),
  removeBookmark: (name: string) => invoke<AppConfig>("remove_bookmark", { name }),
  updateSettings: (settings: BrowserSettings) =>
    invoke<AppConfig>("update_settings", { settings }),

  openUrl: (raw: string) => invoke<Tab>("open_url", { raw }),
  navigateTab: (label: string, raw: string) => invoke<Tab>("navigate_tab", { label, raw }),
  closeTab: (label: string) => invoke<void>("close_tab", { label }),
  setActive: (label: string, isHome: boolean) => invoke<void>("set_active", { label, isHome }),
  goBack: (label: string) => invoke<void>("go_back", { label }),
  goForward: (label: string) => invoke<void>("go_forward", { label }),
  reloadTab: (label: string) => invoke<void>("reload_tab", { label }),
  stopTab: (label: string) => invoke<void>("stop_tab", { label }),
  homeTab: () => invoke<Tab>("home_tab"),
  getTabs: () => invoke<Tab[]>("get_tabs"),

  getExtraction: (broker: string) => invoke<ExtractionSnapshot>("get_extraction", { broker }),
  listExtractions: () => invoke<ExtractionSnapshot[]>("list_extractions"),
  setExtractionPanel: (open: boolean) => invoke<void>("set_extraction_panel", { open }),

  getVisualAnalysis: (broker: string, symbol: string) =>
    invoke<VisualAnalysis>("get_visual_analysis", { broker, symbol }),
  listVisualAnalyses: () => invoke<VisualAnalysis[]>("list_visual_analyses"),
  triggerChartCapture: (broker: string) =>
    invoke<string>("trigger_chart_capture", { broker }),
  captureAndAnalyzeChart: (payload: unknown) =>
    invoke<CaptureAck>("capture_and_analyze_chart", { payload }),
};

/** Subscribe to backend → frontend events, typed per event. */
export function onTabEvent(handler: (payload: Tab) => void) {
  return listen<Tab>("tab-event", (e) => handler(e.payload));
}

export function onBlockedEvent(handler: (payload: BlockedInfo) => void) {
  return listen<BlockedInfo>("nav-blocked", (e) => handler(e.payload));
}

export function onAppReady(handler: () => void) {
  return listen<boolean>("app-ready", () => handler());
}

export function onExtractionStatus(handler: (payload: ExtractionStatus) => void) {
  return listen<ExtractionStatus>("extraction-status", (e) => handler(e.payload));
}

export function onVisualAnalysis(handler: (payload: VisualAnalysisEvent) => void) {
  return listen<VisualAnalysisEvent>("visual-analysis", (e) => handler(e.payload));
}