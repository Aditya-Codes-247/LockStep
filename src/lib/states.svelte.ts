import { api, onAppReady, onBlockedEvent, onExtractionStatus, onTabEvent, onVisualAnalysis } from "./api";
import type {
  AppConfig,
  BlockedInfo,
  BrowserSettings,
  ExtractionSnapshot,
  Tab,
  VisualAnalysis,
} from "./types";

type ToastKind = "ok" | "error" | "warn";

export interface Toast {
  message: string;
  kind: ToastKind;
}

/**
 * Single reactive source of truth for the whole UI.
 * All browser-chrome state is derived from `tabs` + `activeId`.
 */
class AppState {
  config: AppConfig | null = $state(null);
  tabs: Tab[] = $state([]);
  activeId: string = $state("");
  settingsOpen = $state(false);
  blocked: BlockedInfo | null = $state(null);
  addressFocused = $state(false);
  toast: Toast | null = $state(null);
  ready = $state(false);
  extractionOpen = $state(false);
  extractionSnapshots: ExtractionSnapshot[] = $state([]);
  /** Latest visual analysis per broker key (broker -> analysis). */
  visualAnalyses: Record<string, VisualAnalysis> = $state({});
  /** Which brokers are currently running (or awaiting) an analysis. */
  visualPending: Record<string, boolean> = $state({});

  async init() {
    const config = await api.getConfig();
    this.config = config;
    this.applyTheme(config.settings.theme);

    // Reuse the backend home-tab model as the canonical first tab.
    const home = await api.homeTab();
    this.tabs = [home];
    this.activeId = home.label;
    await api.setActive(home.label, true);

    this.ready = true;
    this.wireEvents();
  }

  private wireEvents() {
    onTabEvent((tab) => this.upsertTab(tab));
    onBlockedEvent((info) => this.handleBlocked(info));
    onAppReady(() => this.refreshConfig());
    onExtractionStatus(() => this.pollExtractions());
    onVisualAnalysis((payload) => this.handleVisualAnalysis(payload.broker, payload.analysis));
  }

  private pollTimer: number | undefined;
  private pollRunning = false;

  /**
   * Keep the panel's snapshot list fresh while it is open. The backend emits
   * `extraction-status` on every accepted snapshot; we listen and refresh the
   * cached list (cheap `list_extractions`) so the panel tracks live changes.
   */
  private async pollExtractions() {
    if (!this.extractionOpen || this.pollRunning) return;
    this.pollRunning = true;
    try {
      this.extractionSnapshots = await api.listExtractions();
    } catch {
      /* non-fatal: next status event will retry */
    } finally {
      this.pollRunning = false;
    }
  }

  private async refreshConfig() {
    try {
      this.config = await api.getConfig();
    } catch {
      /* non-fatal */
    }
  }

  async applyTheme(theme: "dark" | "light") {
    document.documentElement.dataset.theme = theme;
  }

  private upsertTab(tab: Tab) {
    const idx = this.tabs.findIndex((t) => t.label === tab.label);
    if (idx === -1) {
      this.tabs = [...this.tabs, tab];
    } else {
      this.tabs = this.tabs.map((t, i) => (i === idx ? { ...t, ...tab } : t));
    }
  }

  private async handleBlocked(info: BlockedInfo) {
    this.blocked = info;
    this.tabs = this.tabs.map((t) =>
      t.label === info.tab ? { ...t, state: "blocked" } : t,
    );
    // The tab's webview must hide so the UI block overlay is visible.
    await api.setActive(info.tab, true);
  }

  /** Open a whitelisted site in a brand-new tab. */
  async openSite(raw: string): Promise<boolean> {
    try {
      const tab = await api.openUrl(raw);
      this.tabs = [...this.tabs, tab];
      this.activeId = tab.label;
      this.blocked = null;
      await api.setActive(tab.label, false);
      return true;
    } catch (reason) {
      await this.showBlock(String(reason ?? "This address cannot be opened."), raw);
      return false;
    }
  }

  /** Navigate the active tab; if it is the home tab, promote it to a real tab. */
  async navigateActive(raw: string): Promise<boolean> {
    const active = this.getActive();
    if (!active) return this.openSite(raw);
    if (active.isHome) return this.openSite(raw);
    try {
      const tab = await api.navigateTab(active.label, raw);
      this.upsertTab(tab);
      this.activeId = tab.label;
      this.blocked = null;
      return true;
    } catch (reason) {
      await this.showBlock(String(reason ?? "This address cannot be opened."), raw, active.label);
      return false;
    }
  }

  private async showBlock(reason: string, target: string, label?: string) {
    const block = {
      tab: label ?? this.activeId,
      url: target,
      reason,
    };
    this.blocked = block;
    const tabId = label ?? this.activeId;
    this.tabs = this.tabs.map((t) => (t.label === tabId ? { ...t, state: "blocked" } : t));
    await api.setActive(tabId, true);
  }

  async dismissBlock() {
    const info = this.blocked;
    this.blocked = null;
    if (!info) return;
    const tab = this.tabs.find((t) => t.label === info.tab);
    const nextState = tab && !tab.isHome ? "loaded" : "home";
    this.tabs = this.tabs.map((t) =>
      t.label === info.tab ? { ...t, state: nextState } : t,
    );
    await api.setActive(info.tab, tab?.isHome ?? false);
  }

  async goHome() {
    await this.newTab();
  }

  /** Create a home tab (no webview) and make it active. */
  async newTab() {
    const home = await api.homeTab();
    // Rename collides with a closed "home" label? backend returns stable model.
    home.label = `home-${Date.now()}`;
    this.tabs = [...this.tabs, home];
    this.activeId = home.label;
    this.blocked = null;
    await api.setActive(home.label, true);
  }

  async activate(label: string) {
    const tab = this.tabs.find((t) => t.label === label);
    if (!tab) return;
    this.activeId = label;
    this.blocked = null;
    if (tab.state === "blocked") {
      // Re-activating a blocked tab shows the block overlay (webview stays hidden).
      await api.setActive(label, true);
    } else {
      await api.setActive(label, tab.isHome);
    }
  }

  async closeTab(label: string) {
    const tab = this.tabs.find((t) => t.label === label);
    if (!tab) return;
    const wasActive = this.activeId === label;
    const remaining = this.tabs.filter((t) => t.label !== label);
    this.tabs = remaining;
    if (tab.isHome) {
      // Home tabs have no webview; nothing more to do.
    } else {
      await api.closeTab(label);
    }
    if (wasActive) {
      if (remaining.length === 0) {
        await this.newTab();
      } else {
        const next = remaining[Math.min(this.activeIndex, remaining.length - 1)] ?? remaining[remaining.length - 1];
        this.activeId = next.label;
        await api.setActive(next.label, next.isHome);
      }
    }
  }

  get activeIndex(): number {
    return this.tabs.findIndex((t) => t.label === this.activeId);
  }

  getActive(): Tab | undefined {
    return this.tabs.find((t) => t.label === this.activeId);
  }

  /** Hide the tab webview while the address autocomplete is open. */
  async focusAddress(open: boolean) {
    this.addressFocused = open;
    const active = this.getActive();
    if (!active || active.isHome) return;
    if (open) {
      await api.setActive(active.label, true);
    } else if (active.state === "loaded" || active.state === "loading") {
      await api.setActive(active.label, false);
    }
  }

  async back() {
    const a = this.getActive();
    if (a && !a.isHome) await api.goBack(a.label);
  }
  async forward() {
    const a = this.getActive();
    if (a && !a.isHome) await api.goForward(a.label);
  }
  async reload() {
    const a = this.getActive();
    if (a && !a.isHome) await api.reloadTab(a.label);
  }
  async stop() {
    const a = this.getActive();
    if (a && !a.isHome) await api.stopTab(a.label);
  }

  async openSettings() {
    this.settingsOpen = true;
    // Ensure no site webview covers the panel.
    const a = this.getActive();
    if (a && !a.isHome) await api.setActive(a.label, true);
  }

  async closeSettings() {
    this.settingsOpen = false;
    const a = this.getActive();
    if (a && !a.isHome && a.state === "loaded") await api.setActive(a.label, false);
  }

  // ---- extracted-data panel ----------------------------------------------

  async toggleExtraction() {
    if (this.extractionOpen) {
      await this.closeExtraction();
    } else {
      await this.openExtraction();
    }
  }

  async openExtraction() {
    this.extractionOpen = true;
    await this.pollExtractions();
    await api.setExtractionPanel(true);
  }

  async closeExtraction() {
    this.extractionOpen = false;
    await api.setExtractionPanel(false);
  }

  async refreshExtractions() {
    await this.pollExtractions();
  }

  // ---- on-demand chart visual analysis -----------------------------------

  /** Store the analysis for a broker and clear its pending flag. */
  private handleVisualAnalysis(broker: string, analysis: VisualAnalysis) {
    this.visualAnalyses[broker] = analysis;
    this.visualPending[broker] = false;
  }

  /**
   * Trigger a visual analysis of the chart currently shown by the given broker
   * (capture bytes stay in the broker webview; only the JSON result returns).
   * Returns false when no live tab is showing the broker or when rate-limited.
   */
  async analyzeChart(broker: string): Promise<boolean> {
    if (this.visualPending[broker]) return false;
    this.visualPending[broker] = true;
    try {
      await api.triggerChartCapture(broker);
      return true;
    } catch (e) {
      this.visualPending[broker] = false;
      this.flash(String(e), "error");
      return false;
    }
  }

  async clearVisual(broker: string) {
    delete this.visualAnalyses[broker];
    delete this.visualPending[broker];
  }

  // ---- config mutations -------------------------------------------------

  async setTheme(theme: "dark" | "light") {
    if (!this.config) return;
    const settings: BrowserSettings = { ...this.config.settings, theme };
    await this.saveSettings(settings);
  }

  async setHomepage(homepage: "landing" | "custom") {
    if (!this.config) return;
    await this.saveSettings({ ...this.config.settings, homepage });
  }

  async setHomeUrl(homeUrl: string) {
    if (!this.config) return;
    await this.saveSettings({ ...this.config.settings, homeUrl });
  }

  private async saveSettings(settings: BrowserSettings) {
    try {
      this.config = await api.updateSettings(settings);
      this.applyTheme(settings.theme);
      this.flash("Settings saved", "ok");
    } catch (e) {
      this.flash(String(e), "error");
    }
  }

  async addDomain(domain: string): Promise<boolean> {
    try {
      this.config = await api.addDomain(domain.trim());
      this.flash(`Added ${domain.trim()} to the approved list`, "ok");
      return true;
    } catch (e) {
      this.flash(String(e), "error");
      return false;
    }
  }

  async removeDomain(domain: string) {
    try {
      this.config = await api.removeDomain(domain);
      this.flash(`Removed ${domain} from the approved list`, "ok");
    } catch (e) {
      this.flash(String(e), "error");
    }
  }

  async addBookmark(name: string, url: string, color?: string): Promise<boolean> {
    try {
      this.config = await api.addBookmark(name, url, color);
      this.flash(`Added shortcut "${name}"`, "ok");
      return true;
    } catch (e) {
      this.flash(String(e), "error");
      return false;
    }
  }

  async removeBookmark(name: string) {
    try {
      this.config = await api.removeBookmark(name);
      this.flash(`Removed "${name}"`, "ok");
    } catch (e) {
      this.flash(String(e), "error");
    }
  }

  flash(message: string, kind: ToastKind = "ok") {
    this.toast = { message, kind };
    window.setTimeout(() => {
      if (this.toast?.message === message) this.toast = null;
    }, 3200);
  }
}

export const app = new AppState();