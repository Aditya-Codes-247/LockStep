<script lang="ts">
  import { app } from "$lib/states.svelte";
  import type {
    Candle,
    ExtractionSnapshot,
    SymbolExtract,
    DataQuality,
    VisualAnalysis,
  } from "$lib/types";

  let openBroker: string | null = $state(null);
  const expanded = $state<Record<string, boolean>>({});

  const active = $derived.by<ExtractionSnapshot | undefined>(() => {
    const list = app.extractionSnapshots;
    if (openBroker) return list.find((s) => s.broker === openBroker);
    return list[0];
  });

  const visual = $derived.by<VisualAnalysis | undefined>(() => {
    const snap = active;
    if (!snap || !app.visualAnalyses[snap.broker]) return undefined;
    return app.visualAnalyses[snap.broker];
  });

  const symbols = $derived.by(() => {
    const snap = active;
    if (!snap) return [];
    return [...snap.data.symbols].sort((a, b) => a.ticker.localeCompare(b.ticker));
  });

  function qualityMeta(q: DataQuality) {
    switch (q) {
      case "good":
        return { label: "Good", cls: "bg-ok-soft text-ok" };
      case "degraded":
        return { label: "Degraded", cls: "bg-danger-soft text-warn" };
      default:
        return { label: "Empty", cls: "bg-surface-3 text-faint" };
    }
  }

  function fmtNum(v: number | null | undefined, digits = 2): string {
    if (v === null || v === undefined || Number.isNaN(v)) return "—";
    return v.toLocaleString("en-IN", {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    });
  }

  function fmtVol(v: number | null | undefined): string {
    if (v === null || v === undefined || Number.isNaN(v)) return "—";
    const abs = Math.abs(v);
    if (abs >= 1e7) return `${(v / 1e7).toFixed(2)}Cr`;
    if (abs >= 1e5) return `${(v / 1e5).toFixed(2)}L`;
    if (abs >= 1e3) return `${(v / 1e3).toFixed(1)}K`;
    return v.toFixed(0);
  }

  function chgCls(v: number | null | undefined): string {
    if (v === null || v === undefined || Number.isNaN(v)) return "text-faint";
    if (v > 0) return "text-ok";
    if (v < 0) return "text-danger";
    return "text-muted";
  }

  function fmtChg(v: number | null | undefined): string {
    if (v === null || v === undefined || Number.isNaN(v)) return "—";
    return `${v > 0 ? "+" : ""}${v.toFixed(2)}%`;
  }

  function ageLabel(ms: number): string {
    if (ms < 1_000) return "now";
    if (ms < 60_000) return `${Math.floor(ms / 1_000)}s ago`;
    if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m ago`;
    return `${Math.floor(ms / 3_600_000)}h ago`;
  }

  function timeLabel(ms: number): string {
    return new Date(ms).toLocaleTimeString("en-IN", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  }

  function brokerName(broker: string): string {
    return broker.replace(/[-_]/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
  }

  function fmtUrl(url: string | undefined): string {
    if (!url) return "—";
    try {
      return new URL(url).host;
    } catch {
      return url;
    }
  }

  function fmtChange(sym: SymbolExtract): string {
    const pct = sym.changePercent ?? null;
    const abs = sym.change ?? null;
    if (pct === null && abs === null) return "—";
    const parts: string[] = [];
    if (pct !== null) parts.push(fmtChg(pct));
    if (abs !== null) parts.push(fmtNum(abs));
    return parts.join(" ");
  }

  /** SVG sparkline of candle closes. */
  function sparkline(candles: Candle[]): string | null {
    const closes = candles.map((c) => c.close).filter((v) => Number.isFinite(v));
    if (closes.length < 2) return null;
    const w = 300;
    const h = 48;
    const min = Math.min(...closes);
    const max = Math.max(...closes);
    const span = max - min || 1;
    const pts = closes
      .map((v, i) => {
        const x = (i / (closes.length - 1)) * w;
        const y = h - ((v - min) / span) * (h - 6) - 3;
        return `${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(" ");
    return `<svg viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" class="h-12 w-full"><polyline points="${pts}" fill="none" stroke="var(--accent)" stroke-width="1.5" /></svg>`;
  }

  function trendLabel(t: VisualAnalysis["trend"]): string {
    const map = { up: "Bullish", down: "Bearish", sideways: "Sideways" };
    return t ? map[t] ?? t : "—";
  }

  function trendCls(t: VisualAnalysis["trend"]): string {
    if (t === "up") return "text-ok";
    if (t === "down") return "text-danger";
    return "text-muted";
  }

  function sigCls(s: VisualAnalysis["signal"]): string {
    if (s === "buy") return "text-ok";
    if (s === "sell") return "text-danger";
    return "text-muted";
  }

  function statusLabel(a: VisualAnalysis): string {
    switch (a.status) {
      case "ok":
        return "Analyzed";
      case "rate_limited":
        return "Rate limited";
      case "canvas_tainted":
        return "Tainted canvas";
      case "no_chart":
        return "No chart";
      case "error":
        return "Analysis failed";
    }
  }
</script>

<aside
  class="flex w-[360px] shrink-0 flex-col border-l border-line bg-surface"
  style="width: 360px"
>
  <div class="flex h-10 shrink-0 items-center gap-2 border-b border-line px-3">
    <span class="text-sm font-semibold tracking-tight">Extracted data</span>
    <span class="ml-auto text-[10px] uppercase tracking-wide text-faint"
      >{app.extractionSnapshots.length} broker{app.extractionSnapshots.length === 1 ? "" : "s"}</span>
    <button
      class="flex h-6 w-6 items-center justify-center rounded-md text-lg leading-none text-muted transition-colors hover:bg-surface-2 hover:text-ink"
      title="Close panel"
      onclick={() => app.closeExtraction()}
    >×</button>
  </div>

  {#if app.extractionSnapshots.length > 1}
    <div class="flex shrink-0 items-center gap-1 overflow-x-auto border-b border-line px-2 py-1.5 [scrollbar-width:none]">
      {#each app.extractionSnapshots as snap (snap.broker)}
        <button
          class="shrink-0 rounded-full px-2.5 py-0.5 text-xs capitalize transition-colors
                 {active?.broker === snap.broker ? 'bg-surface-3 text-ink' : 'text-muted hover:bg-surface-2 hover:text-ink'}"
          onclick={() => (openBroker = snap.broker)}
        >{snap.broker}</button>
      {/each}
    </div>
  {/if}

  {#if !active}
    <div class="flex flex-1 flex-col items-center justify-center gap-2 px-6 text-center">
      <p class="text-sm font-medium text-ink">No extracted data yet</p>
      <p class="text-xs leading-relaxed text-faint">
        Open a whitelisted broker site and log in — the scanner will read the
        watchlist and show the live snapshot here.
      </p>
      <button
        class="mt-2 rounded-md border border-line px-3 py-1 text-xs text-muted transition-colors hover:bg-surface-2 hover:text-ink"
        onclick={() => app.refreshExtractions()}
      >Refresh</button>
    </div>
  {:else}
    {@const snap = active}
    {@const meta = qualityMeta(snap.data.dataQuality)}
    {@const vis = visual}
    <!-- summary -->
    <div class="shrink-0 border-b border-line px-3 py-2.5">
      <div class="flex items-center gap-2">
        <span class="text-sm font-semibold capitalize">{brokerName(snap.broker)}</span>
        <span class="rounded-full px-2 py-0.5 text-[10px] font-medium {meta.cls}">{meta.label}</span>
        <span class="ml-auto font-mono text-xs text-muted">{snap.symbolCount} symbols</span>
      </div>
      <div class="mt-1.5 flex items-center gap-3 text-[10px] text-faint">
        <span>Updated {ageLabel(snap.ageMs)}</span>
        <span>·</span>
        <span>{timeLabel(snap.receivedAtMs)}</span>
        <span>·</span>
        <span>{snap.data.extractionDurationMs}ms scan</span>
      </div>
      <div class="mt-0.5 truncate text-[10px] text-faint" title={snap.data.url}>{fmtUrl(snap.data.url)}</div>

      {#if snap.data.chart}
        <div class="mt-2 rounded-md border border-line bg-surface-2 px-2.5 py-2">
          <div class="flex items-center gap-2 text-[10px] text-faint">
            <span class="font-semibold uppercase">Chart</span>
            {#if snap.data.chart.instrument}
              <span class="text-ink">{snap.data.chart.instrument}</span>
            {/if}
            {#if snap.data.chart.timeframe}
              <span class="rounded bg-surface-3 px-1.5 py-0.5 font-mono text-ink">{snap.data.chart.timeframe}</span>
            {/if}
            <span class="ml-auto">{snap.data.chart.candles.length} candles</span>
          </div>
          {#if snap.data.chart.candles.length > 1}
            {@html sparkline(snap.data.chart.candles)}
          {/if}
        </div>
      {/if}

      <!-- on-demand visual analysis (OpenCode CLI) -->
      <div class="mt-2 rounded-md border border-line bg-surface-2 px-2.5 py-2">
        <div class="flex items-center gap-2 text-[10px] text-faint">
          <span class="font-semibold uppercase">AI analysis</span>
          <span class="ml-auto">
            {#if app.visualPending[snap.broker]}
              Analyzing…
            {:else if vis}
              {statusLabel(vis)}
              {#if vis.model}<span class="font-mono text-faint">· {vis.model}</span>{/if}
            {:else}
              Not run yet
            {/if}
          </span>
        </div>

        {#if vis && vis.status === "ok"}
          <div class="mt-1.5 grid grid-cols-[auto_auto] gap-x-3 gap-y-0.5 text-[10px]">
            <span class="text-faint">Symbol</span>
            <span class="font-mono text-ink">{vis.symbol}</span>
            {#if vis.pattern}
              <span class="text-faint">Pattern</span>
              <span class="text-ink">{vis.pattern}</span>
            {/if}
            <span class="text-faint">Trend</span>
            <span class="{trendCls(vis.trend ?? null)}">{trendLabel(vis.trend ?? null)}</span>
            {#if vis.signal}
              <span class="text-faint">Signal</span>
              <span class="{sigCls(vis.signal ?? null)}">{vis.signal}</span>
            {/if}
            {#if vis.support !== null && vis.support !== undefined}
              <span class="text-faint">Support</span>
              <span class="font-mono text-ink">{fmtNum(vis.support)}</span>
            {/if}
            {#if vis.resistance !== null && vis.resistance !== undefined}
              <span class="text-faint">Resistance</span>
              <span class="font-mono text-ink">{fmtNum(vis.resistance)}</span>
            {/if}
          </div>
          {#if vis.summary}
            <p class="mt-1.5 text-[10px] leading-relaxed text-muted">{vis.summary}</p>
          {/if}
          {#if vis.indicators.length > 0}
            <div class="mt-1.5 space-y-0.5 text-[10px]">
              {#each vis.indicators as ind, i (i)}
                <div class="flex justify-between gap-2">
                  <span class="text-muted">{ind.name}</span>
                  <span class="truncate font-mono text-ink">{ind.value}</span>
                  {#if ind.signal}
                    <span class="{sigCls(ind.signal === "bullish" ? "buy" : ind.signal === "bearish" ? "sell" : "neutral")}">{ind.signal}</span>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
          {#if vis.observed}
            <div class="mt-2 rounded-md border border-line bg-surface-2/60 px-2 py-1.5 text-[10px]">
              <div class="mb-1 text-[9px] uppercase tracking-wide text-faint">Seen in image</div>
              {#if vis.observed.symbol || vis.observed.timeframe}
                <div class="flex flex-wrap gap-x-3 text-faint">
                  {#if vis.observed.symbol}
                    <span>Sym <span class="font-mono text-ink">{vis.observed.symbol}</span></span>
                  {/if}
                  {#if vis.observed.timeframe}
                    <span>TF <span class="font-mono text-ink">{vis.observed.timeframe}</span></span>
                  {/if}
                </div>
              {/if}
              {#if vis.observed.ohlcLegend}
                <div class="font-mono text-faint" title="OHLC legend read off the image">{vis.observed.ohlcLegend}</div>
              {/if}
              {#if vis.observed.indicators.length > 0}
                <div class="mt-1 space-y-0.5">
                  {#each vis.observed.indicators as ind (ind.name)}
                    <div class="flex justify-between gap-2">
                      <span class="text-muted">{ind.name}</span>
                      {#if ind.value}
                        <span class="truncate font-mono text-ink">{ind.value}</span>
                      {/if}
                      <span class="text-ok">{ind.visible ? "visible" : "hidden"}</span>
                    </div>
                  {/each}
                </div>
              {/if}
              {#if vis.observed.overlays.length > 0}
                <div class="mt-1 text-faint">Overlays: <span class="text-ink">{vis.observed.overlays.join(", ")}</span></div>
              {/if}
              {#if vis.observed.crosshairValues}
                <div class="mt-0.5 font-mono text-faint">Crosshair {vis.observed.crosshairValues}</div>
              {/if}
            </div>
          {/if}
          {#if vis.latencyMs !== null && vis.latencyMs !== undefined}
            <p class="mt-1 text-[9px] text-faint">Took {vis.latencyMs}ms</p>
          {/if}
        {:else if vis && vis.error}
          <p class="mt-1.5 text-[10px] leading-relaxed text-danger">{vis.error}</p>
        {/if}

        <button
          class="mt-2 w-full rounded-md border border-line px-2 py-1 text-center text-[11px] text-muted transition-colors hover:bg-surface-3 hover:text-ink"
          disabled={app.visualPending[snap.broker] || !snap.data.chart}
          title={snap.data.chart
            ? "Capture the broker's chart canvas and ask the AI to analyze it"
            : "No chart detected on this broker page yet"}
          onclick={() => app.analyzeChart(snap.broker)}
        >
          {app.visualPending[snap.broker] ? "Analyzing…" : "Analyze chart"}
        </button>
      </div>
    </div>

    <!-- symbols table -->
    <div class="min-h-0 flex-1 overflow-y-auto">
      {#if symbols.length === 0}
        <p class="px-3 py-4 text-xs leading-relaxed text-faint">
          The broker page yielded no watchlist symbols yet (quality: {snap.data.dataQuality}).
          Make sure a watchlist is visible on the page; snapshots refresh automatically.
        </p>
      {:else}
        <div class="grid grid-cols-[1fr_auto_auto] items-center gap-x-2 border-b border-line bg-surface-2 px-3 py-1 text-[10px] uppercase tracking-wide text-faint">
          <span>Symbol</span>
          <span class="text-right">Last</span>
          <span class="w-20 text-right">Chg</span>
        </div>
        {#each symbols as sym (sym.ticker)}
          {@const chg = sym.changePercent ?? sym.change ?? null}
          {@const open = expanded[sym.ticker]}
          <button
            class="grid w-full grid-cols-[1fr_auto_auto] items-center gap-x-2 border-b border-line/50 px-3 py-1.5 text-left transition-colors hover:bg-surface-2"
            onclick={() => (expanded[sym.ticker] = !expanded[sym.ticker])}
          >
            <span class="min-w-0">
              <span class="block truncate text-xs font-medium text-ink">{sym.ticker}</span>
              {#if sym.name}
                <span class="block truncate text-[10px] text-faint">{sym.name}</span>
              {/if}
            </span>
            <span class="text-right font-mono text-xs text-ink">{fmtNum(sym.price ?? sym.bid ?? sym.ask)}</span>
            <span class="w-20 text-right font-mono text-xs {chgCls(chg)}">{fmtChange(sym)}</span>
          </button>
          {#if open}
            <div class="border-b border-line bg-surface px-3 py-2">
              {#if sym.bid !== null || sym.ask !== null || sym.volume !== null}
                <div class="mb-1.5 grid grid-cols-3 gap-2 text-[10px]">
                  <div><span class="text-faint">Bid</span> <span class="float-right font-mono text-ink">{fmtNum(sym.bid)}</span></div>
                  <div><span class="text-faint">Ask</span> <span class="float-right font-mono text-ink">{fmtNum(sym.ask)}</span></div>
                  <div><span class="text-faint">Vol</span> <span class="float-right font-mono text-ink">{fmtVol(sym.volume)}</span></div>
                </div>
              {/if}
              {#if sym.ohlc1min}
                {@const o = sym.ohlc1min}
                <div class="mb-1 text-[10px] text-faint">OHLC 1m</div>
                <div class="mb-1.5 grid grid-cols-4 gap-2 text-[10px]">
                  <div class="text-faint">O <span class="float-right font-mono text-ink">{fmtNum(o.open)}</span></div>
                  <div class="text-faint">H <span class="float-right font-mono text-ok">{fmtNum(o.high)}</span></div>
                  <div class="text-faint">L <span class="float-right font-mono text-danger">{fmtNum(o.low)}</span></div>
                  <div class="text-faint">C <span class="float-right font-mono text-ink">{fmtNum(o.close)}</span></div>
                </div>
              {/if}
              {#if sym.ohlc5min}
                {@const o = sym.ohlc5min}
                <div class="mb-1 text-[10px] text-faint">OHLC 5m</div>
                <div class="mb-1.5 grid grid-cols-4 gap-2 text-[10px]">
                  <div class="text-faint">O <span class="float-right font-mono text-ink">{fmtNum(o.open)}</span></div>
                  <div class="text-faint">H <span class="float-right font-mono text-ok">{fmtNum(o.high)}</span></div>
                  <div class="text-faint">L <span class="float-right font-mono text-danger">{fmtNum(o.low)}</span></div>
                  <div class="text-faint">C <span class="float-right font-mono text-ink">{fmtNum(o.close)}</span></div>
                </div>
              {/if}
              {#if sym.indicators.length > 0}
                <div class="mb-1 text-[10px] text-faint">Indicators</div>
                <div class="space-y-0.5">
                  {#each sym.indicators as ind, i (i)}
                    <div class="flex justify-between text-[10px]">
                      <span class="text-muted">{ind.kind}{ind.period ? `-${ind.period}` : ""}</span>
                      <span class="font-mono text-ink">{fmtNum(ind.value)}</span>
                    </div>
                  {/each}
                </div>
              {/if}
              {#if sym.orderBook.length > 0}
                <div class="mb-1 mt-2 text-[10px] text-faint">Order book</div>
                <div class="grid grid-cols-[1fr_1fr] gap-2">
                  <div>
                    {#each sym.orderBook.filter((l) => l.side === "bid") as lvl, i (i)}
                      <div class="flex justify-between text-[10px]">
                        <span class="text-ok">{fmtNum(lvl.price)}</span>
                        <span class="font-mono text-muted">{fmtVol(lvl.qty)}</span>
                      </div>
                    {/each}
                  </div>
                  <div>
                    {#each sym.orderBook.filter((l) => l.side === "ask") as lvl, i (i)}
                      <div class="flex justify-between text-[10px]">
                        <span class="text-danger">{fmtNum(lvl.price)}</span>
                        <span class="font-mono text-muted">{fmtVol(lvl.qty)}</span>
                      </div>
                    {/each}
                  </div>
                </div>
              {/if}
            </div>
          {/if}
        {/each}
      {/if}
    </div>
  {/if}
</aside>