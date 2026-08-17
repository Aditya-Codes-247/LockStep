/**
 * Lockstep DOM extractor — injected verbatim (no bundling) into every broker
 * webview via `WebviewBuilder::initialization_script` (see src-tauri/src/browser.rs).
 *
 * The file is written so the same bytes are both valid TypeScript(*)/JavaScript and
 * run inside WebView2. It is also the source of truth for docs/extraction-schema.md.
 *
 * (*) No TS-only tokens are used, so this doubles as the standalone browser build.
 *     Run content/extractor.test.html in a normal browser to observe snapshots
 *     without a Tauri host (test mode).
 *
 * Behaviour
 *  1. Detect the broker once per document (host / title / DOM markers).
 *  2. Every 500 ms read the trading DOM through fallback selector chains.
 *  3. Grade the snapshot (`good` / `degraded` / `empty`).
 *  4. Push it to Rust via `__TAURI_INTERNALS__.invoke("extract_dom")`.
 *     In test mode (query `?lsb=test` or `window.__LSB_TEST__`), push to
 *     `window.__EXTRACTIONS__` and dispatch `extraction-ready` instead.
 */
(() => {
  'use strict';

  if (window.__LOCKSTEP_EXTRACTER__) {
    return;
  }
  window.__LOCKSTEP_EXTRACTER__ = true;

  const POLL_MS = 500;
  const MAX_ROWS = 200;

  // ------------------------------------------------------------------ setup
  const TEST_MODE =
    window.__LSB_TEST__ === true ||
    /[?&]lsb=test/.test(window.location.search) ||
    (typeof window.localStorage !== 'undefined' &&
      window.localStorage.getItem('lsbDebug') === '1');

  const debug = (...args) => {
    if (TEST_MODE || window.localStorage.getItem('lsbDebug') === '1') {
      try {
        console.log('[lockstep-extract]', ...args);
      } catch {
        /* noop */
      }
    }
  };

  // --------------------------------------------------------- broker config
  /**
   * @type {Array<{broker: string, hosts: string[], titleMarkers: RegExp[], domMarkers: string[]}>}
   * When a broker is added to the whitelist/capability, add its origin to
   *   - src-tauri/capabilities/extractor.json  (remote.urls)
   *   - this BROKERS array
   */
  const BROKERS = [
    {
      broker: 'angel',
      hosts: ['angelone.in'],
      titleMarkers: [/Angel One/i],
      domMarkers: ['[class*="scrip-name"]', '[class*="ltp"]', '[class*="holding"]', '[class*="watchlist"]'],
    },
    {
      broker: 'coinswitch',
      hosts: ['coinswitch.co'],
      titleMarkers: [/CoinSwitch/i],
      domMarkers: ['[data-testid*="coin"]', '[class*="coin-name"]', '[class*="market-row"]'],
    },
    {
      broker: 'dhan',
      hosts: ['dhan.co'],
      titleMarkers: [/Dhan/i],
      domMarkers: [
        '[data-testid*="watchlist"]',
        '[class*="watchlist"]',
        '[class*="scrip"]',
        '[class*="instrument"]',
      ],
    },
  ];

  // ---------------------------------------------------------- heuristics
  /**
   * Extract a number from loose market text: strips ₹ , % () whitespace.
   * Returns null when the text is not a usable quote.
   * @param {string | null | undefined} raw
   * @returns {number | null}
   */
  function parseNum(raw) {
    if (raw == null) return null;
    let s = String(raw).trim();
    if (!s || s === '--' || /(^|\s)(no data|n\/a|—)(\s|$)/i.test(s)) return null;
    s = s.replace(/[₹$€,\s\u{A0}\u202f]/gu, '');
    s = s.replace(/[()]/g, '');
    const m = s.match(/-?\d+(?:\.\d+)?/);
    if (!m) return null;
    const n = Number(m[0]);
    return Number.isFinite(n) ? n : null;
  }

  /**
   * First non-empty trimmed text of the first selector that matches inside el.
   * @param {ParentNode} el
   * @param {string[]} selectors
   * @returns {string | null}
   */
  function firstText(el, selectors) {
    for (const sel of selectors) {
      let q;
      try {
        q = el.querySelector(sel);
      } catch {
        continue;
      }
      if (q) {
        const txt = (q.textContent || '').trim();
        if (txt) return txt;
      }
    }
    return null;
  }

  /**
   * First numeric value found from a selector chain.
   * @param {ParentNode} el
   * @param {string[]} selectors
   * @returns {number | null}
   */
  function firstNum(el, selectors) {
    for (const sel of selectors) {
      let q;
      try {
        q = el.querySelector(sel);
      } catch {
        continue;
      }
      if (!q) continue;
      const num = parseNum(q.textContent);
      if (num != null) return num;
    }
    return null;
  }

  /**
   * First raw text found from a selector chain (keeps `%`/`+`/`-` context so the
   * caller can decide whether a number is a change, a percent, or something else).
   * @param {ParentNode} el
   * @param {string[]} selectors
   * @returns {string | null}
   */
  function firstTextRaw(el, selectors) {
    return firstText(el, selectors);
  }

  /**
   * Classify a market-change cell into absolute change vs change percent.
   * A `%` in the text makes it a percent; otherwise a signed number is the
   * absolute change. Ambiguous text yields both null.
   * @param {string | null} raw
   * @returns {{ change: number | null, changePercent: number | null }}
   */
  function parseChangeText(raw) {
    if (raw == null) return { change: null, changePercent: null };
    const s = String(raw).trim();
    if (!s || s === '--' || /(^|\s)(no data|n\/a|—)(\s|$)/i.test(s)) {
      return { change: null, changePercent: null };
    }
    if (/%/.test(s)) {
      return { change: null, changePercent: parseNum(s) };
    }
    const candidate = s.replace(/[₹$€,\s\u{A0}\u202f()]/gu, '');
    if (/^[+-]?\d/.test(candidate)) {
      return { change: parseNum(s), changePercent: null };
    }
    return { change: null, changePercent: null };
  }

  /**
   * Normalize a header cell into keywords for column detection.
   * @param {string} text
   * @returns {string}
   */
  function normalizeHeader(text) {
    return (text || '').toLowerCase().replace(/[^a-z0-9%]/g, '');
  }

  /**
   * Detect which table column holds each SymbolExtract field.
   * Returns an index map. Falls back to ordinal guesses when no header tells.
   * @param {string[]} headers
   * @returns {Record<string, number | null>}
   */
  function mapColumns(headers) {
    const idx = {
      ticker: null,
      name: null,
      price: null,
      change: null,
      changePercent: null,
      volume: null,
      bid: null,
      ask: null,
    };
    const keyword = (key) => headers.forEach((h, i) => {
      if (idx[key] != null || !h) return;
      if (key === 'ticker' && /(symbol|scrip|trad|instru|company|coin|name|symbol|code|asset)/.test(h)) idx.ticker = i;
      if (key === 'name' && /(company|instru|scrip|coin|fullname)/.test(h)) idx.name = i;
      if (key === 'price' && /(ltp|last|price|rate|close|bid\s|market)/.test(h)) idx.price = i;
      if (key === 'changePercent' && /(chg|change|percent|%|pct|gain|loss|net)/.test(h) && /%/.test(h)) idx.changePercent = i;
      else if (key === 'change' && /(chg|change|diff|net|gain|loss|pts)/.test(h) && !/%/.test(h)) idx.change = i;
      if (key === 'volume' && /(vol|volume|qty|quantity|lots)/.test(h)) idx.volume = i;
      if (key === 'bid' && /^bid$/.test(h)) idx.bid = i;
      if (key === 'ask' && /^ask$/.test(h)) idx.ask = i;
    });
    keyword('ticker');
    keyword('name');
    keyword('changePercent');
    if (idx.changePercent == null) keyword('change');
    keyword('price');
    keyword('volume');
    keyword('bid');
    keyword('ask');
    // Ordinal fallback (no header → first column symbol, second price).
    if (idx.ticker == null) idx.ticker = 0;
    if (idx.price == null && headers.length > 1) idx.price = 1;
    if (idx.changePercent == null && idx.change == null && headers.length > 2) {
      idx.changePercent = 2;
    }
    return idx;
  }

  /**
   * Build a SymbolExtract for one row using a column map.
   * @param {string[]} texts
   * @param {Record<string, number | null>} col
   * @returns {Partial<import("./types").SymbolExtract> | null}
   */
  function symbolFromColumns(texts, col) {
    const ticker = texts[col.ticker] != null ? String(texts[col.ticker]).trim() : '';
    if (!ticker) return null;
    const out = {
      ticker,
      name: col.name != null && texts[col.name] ? String(texts[col.name]).trim() || null : null,
      price: col.price != null ? parseNum(texts[col.price]) : null,
      change: null,
      changePercent: null,
      bid: col.bid != null ? parseNum(texts[col.bid]) : null,
      ask: col.ask != null ? parseNum(texts[col.ask]) : null,
      volume: col.volume != null ? parseNum(texts[col.volume]) : null,
      ohlc1min: null,
      ohlc5min: null,
      indicators: [],
      orderBook: [],
    };
    if (col.change != null && texts[col.change] != null) {
      const c = parseChangeText(String(texts[col.change]));
      out.change = c.change;
      out.changePercent = c.changePercent;
    }
    return out;
  }

  /**
   * Extract symbols using a header-aware table read. Handles rows whose
   * columns (Symbol | LTP | Change% | Volume) are laid out in any order and
   * also keeps change/change-percent together when a single column shows both.
   * @param {string} broker
   * @returns {Array<import("./types").SymbolExtract>}
   */
  function collectFromTables(broker) {
    const seen = new Set();
    const out = [];
    let tables = [];
    try {
      tables = Array.from(document.querySelectorAll('table'));
    } catch {
      tables = [];
    }
    for (const table of tables) {
      const headerRow = table.querySelector('thead tr');
      const rows = Array.from(table.querySelectorAll('tbody tr'));
      if (rows.length === 0) continue;
      let headers;
      if (headerRow) {
        headers = Array.from(headerRow.querySelectorAll('th, td')).map((c) =>
          normalizeHeader((c.textContent || '').trim()),
        );
      } else {
        // No header: guess from the layout the extractor already knows.
        headers = ['symbol', 'price', 'change%', 'volume'];
      }
      const col = mapColumns(headers.length ? headers : ['symbol', 'price']);
      if (col.ticker == null) continue;
      for (const row of rows) {
        if (out.length >= MAX_ROWS) break;
        const cells = Array.from(row.querySelectorAll('td'));
        // Skip obviously-placeholder rows (footer totals, headings).
        if (cells.length === 0) continue;
        const texts = cells.map((c) => (c.textContent || '').trim());
        const sym = symbolFromColumns(texts, col);
        if (!sym || !sym.ticker || seen.has(sym.ticker)) continue;
        // If a single separate change column exists but isn't % and we already
        // have a price, treat it as the absolute change; otherwise as percent.
        seen.add(sym.ticker);
        out.push(sym);
      }
      if (out.length > 0) break;
    }
    return out;
  }

  /**
   * Read the finest pane of a chart container: a `<canvas>` or a library
   * render node. Returns the element or null.
   *
   * Search order:
   *  1. The top-level document (unchanged selector list).
   *  2. Fallback: same-origin chart iframes (e.g. TradingView charting_library
   *     `sameorigin.html`). Hidden / zero-sized iframes are skipped;
   *     within a reachable iframe document the same selector list is tried,
   *     and when several sized canvases exist the largest one wins (the main
   *     price pane is typically the biggest canvas).
   * @returns {Element | null}
   */
  function findChartCanvas() {
    const selectors = [
      '[data-testid*="chart"]',
      '[class*="chart"] canvas',
      '[class*="candle"] canvas',
      '[class*="graph"] canvas',
      '.chart canvas',
      'canvas',
    ];

    for (const sel of selectors) {
      let cands = [];
      try {
        cands = Array.from(document.querySelectorAll(sel));
      } catch {
        continue;
      }
      for (const c of cands) {
        if (chartSized(c)) return c;
      }
    }

    let iframes = [];
    try {
      iframes = Array.from(document.querySelectorAll('iframe'));
    } catch {
      iframes = [];
    }
    let best = null;
    let bestArea = 0;
    for (const f of iframes) {
      if (hiddenFrame(f)) continue;
      let doc = null;
      try {
        doc = f.contentDocument;
      } catch (e) {
        continue;
      }
      if (!doc) continue;
      for (const sel of selectors) {
        let cands = [];
        try {
          cands = Array.from(doc.querySelectorAll(sel));
        } catch {
          continue;
        }
        for (const c of cands) {
          const r = sizedRect(c);
          if (!r) continue;
          const area = r.width * r.height;
          if (area > bestArea) {
            bestArea = area;
            best = c;
          }
        }
      }
    }
    return best;
  }

  /**
   * Whether an element is a chart-sized canvas (>=120x40 logical px).
   * @param {Element | null | undefined} c
   * @returns {boolean}
   */
  function chartSized(c) {
    return sizedRect(c) !== null;
  }

  /**
   * Bounding rect of a canvas that passes the size gate, or null.
   * @param {Element | null | undefined} c
   * @returns {DOMRect | null}
   */
  function sizedRect(c) {
    if (!c || !c.getBoundingClientRect) return null;
    let r = null;
    try {
      r = c.getBoundingClientRect();
    } catch (e) {
      return null;
    }
    if (!r || r.width < 120 || r.height < 40) return null;
    return r;
  }

  /**
   * Whether an iframe should be skipped as a chart container: zero rendered
   * size, or still pointing at a blank/placeholder document.
   * @param {HTMLIFrameElement} f
   * @returns {boolean}
   */
  function hiddenFrame(f) {
    let r = null;
    try {
      r = f.getBoundingClientRect();
    } catch (e) { /* noop */ }
    if (r && (r.width === 0 || r.height === 0)) return true;
    let srcProp = '';
    try {
      srcProp = f.src || '';
    } catch (e) {
      srcProp = '';
    }
    if (srcProp === 'about:blank' || srcProp === 'about:blank#blocked') return true;
    if (srcProp === '') {
      let hasSrcdoc = false;
      try {
        hasSrcdoc = f.hasAttribute('srcdoc');
      } catch (e) {
        hasSrcdoc = false;
      }
      if (!hasSrcdoc) return true;
    }
    return false;
  }

  /**
   * Try to pull candle data out of an ECharts instance rendered on the page.
   * ECharts stores the instance key on the element it attached to, but the
   * option object is only reachable through the exported lib — so we look for
   * a global `echarts` and any element carrying an instance id.
   * @param {Element} canvas
   * @returns {Array<import("./types").Candle> | null}
   */
  function collectEChartsCandles(canvas) {
    // TEMP DEBUG [lsb-candles] — instrumentation only, no logic change.
    const winTop = window;
    const docLocal = (canvas && canvas.ownerDocument) || document;
    const winLocal = docLocal.defaultView || window;
    const inFrame = winLocal !== winTop;
    const frameEl = inFrame ? (winLocal.frameElement || null) : null;
    const hasTop = typeof winTop.echarts !== 'undefined' && typeof winTop.echarts.getInstanceByDom === 'function';
    const hasLocal = typeof winLocal.echarts !== 'undefined' && typeof winLocal.echarts.getInstanceByDom === 'function';
    console.log('[lsb-candles] window.echarts exists on top-level document:', !!hasTop);
    console.log('[lsb-candles] canvas inside chart-bearing iframe:', inFrame, '| frame src:', inFrame && frameEl ? (frameEl.getAttribute('src') || '(srcdoc/blank)') : 'n/a');
    console.log('[lsb-candles] echarts (or equivalent) exists inside the chart iframe:', !!hasLocal);

    const host = canvas.closest('[class*="chart"], [class*="candle"], [class*="graph"]') || canvas.parentElement;
    const probeTargets = [['top', winTop]];
    if (inFrame) probeTargets.push(['iframe', winLocal]);
    for (const [where, winx] of probeTargets) {
      const ec = winx.echarts;
      if (!ec || typeof ec.getInstanceByDom !== 'function') {
        console.log('[lsb-candles]   probe[' + where + ']: no echarts global with getInstanceByDom');
        continue;
      }
      if (!host) {
        console.log('[lsb-candles]   probe[' + where + ']: cannot resolve host element from canvas');
        continue;
      }
      const inst = ec.getInstanceByDom(host);
      if (!inst || typeof inst.getOption !== 'function') {
        console.log('[lsb-candles]   probe[' + where + ']: getInstanceByDom(host) -> ' + (inst ? 'instance without getOption' : 'no instance'));
        continue;
      }
      try {
        const opt = inst.getOption();
        const seriesList = (opt && Array.isArray(opt.series)) ? opt.series : [];
        let rawTotal = 0;
        for (const s of seriesList) {
          if (!s || !Array.isArray(s.data)) continue;
          rawTotal += s.data.length;
          console.log('[lsb-candles]   probe[' + where + ']: series type=' + String(s.type) + ' dataLen=' + s.data.length);
        }
        const xAxis = opt && Array.isArray(opt.xAxis) && opt.xAxis[0] ? opt.xAxis[0] : null;
        const cats = xAxis && Array.isArray(xAxis.data) ? xAxis.data : [];
        console.log('[lsb-candles]   probe[' + where + ']: getOption() raw data points across series =', rawTotal, '| xAxis categories =', cats.length);
      } catch (e) {
        console.log('[lsb-candles]   probe[' + where + ']: getOption() threw', String(e));
      }
    }
    // TEMP DEBUG [lsb-candles] — end instrumentation.

    const echarts = window.echarts;
    if (!echarts || typeof echarts.getInstanceByDom !== 'function') return null;
    if (!host) return null;
    const inst = echarts.getInstanceByDom(host);
    if (!inst || typeof inst.getOption !== 'function') return null;
    try {
      const opt = inst.getOption();
      const seriesList = (opt && Array.isArray(opt.series)) ? opt.series : [];
      const xAxis = opt && Array.isArray(opt.xAxis) && opt.xAxis[0] ? opt.xAxis[0] : null;
      const cats = xAxis && Array.isArray(xAxis.data) ? xAxis.data : [];
      const candles = [];
      for (const s of seriesList) {
        const data = Array.isArray(s.data) ? s.data : [];
        if (s.type === 'candlestick') {
          data.forEach((d, i) => {
            if (!Array.isArray(d) || d.length < 4) return;
            // ECharts candlestick item order: [open, close, lower, upper].
            const candle = {
              time: toEpochMs(cats[i]),
              open: numOr(d[0]),
              close: numOr(d[1]),
              high: numOr(d[3]),
              low: numOr(d[2]),
              volume: null,
            };
            if (candle.time != null && isFinite(candle.open) && isFinite(candle.close)) {
              candles.push(candle);
            }
          });
        } else if (s.type === 'line' || s.type === 'bar') {
          data.forEach((d, i) => {
            if (!Array.isArray(d) || d.length < 2) return;
            const v = numOr(d[1]);
            if (v == null) return;
            candles.push({ time: toEpochMs(d[0] != null ? d[0] : cats[i]), open: v, close: v, high: v, low: v, volume: null });
          });
        }
      }
      return candles.length ? candles : null;
    } catch {
      return null;
    }
  }

  /**
   * Coerce a candle timestamp (number epoch-ms / seconds, or string) to ms.
   * @param {unknown} t
   * @returns {number | null}
   */
  function toEpochMs(t) {
    if (t == null) return null;
    if (typeof t === 'number') {
      // 10-digit values are unix *seconds*.
      return (t < 1e12 ? t * 1000 : t);
    }
    if (typeof t === 'string' && t.trim()) {
      const ms = Date.parse(t);
      return Number.isFinite(ms) ? ms : null;
    }
    return null;
  }

  /**
   * @param {unknown} x
   * @returns {number | null}
   */
  function numOr(x) {
    const n = typeof x === 'number' ? x : parseNum(x);
    return Number.isFinite(n) ? n : null;
  }

  /**
   * Read the trade-quote summary the site paints over/next to the chart
   * (e.g. Kite's quote strip: O H L C, volume, change). Returns the parsed
   * OHLC numbers when at least some are rendered as text.
   * @param {Element} canvas
   * @returns {import("./types").Candle | null}
   */
  function collectHeaderOhlc(canvas) {
    const host = canvas.closest('[class*="chart"], [class*="quote"], [class*="graph"], [class*="instrument"]') ||
      canvas.parentElement;
    if (!host) return null;
    const scope = host.closest('[class*="quote"], [class*="chart"]') || document;
    const text = (scope.textContent || '').slice(0, 4000);
    const grab = (label) => {
      const re = new RegExp('(?:^|\\s|[' + label + ']\\s*[:=]?\\s*)([+-]?\\d[\\d,]*\\.?\\d*)', 'i');
      const m = text.match(re);
      return m ? parseNum(m[1]) : null;
    };
    const open = grab('O');
    const high = grab('H');
    const low = grab('L');
    const close = grab('C');
    if (close == null && high == null && low == null && open == null) return null;
    return { time: Date.now(), open, high, low, close, volume: null };
  }

  /**
   * Detect the instrument the chart is currently showing.
   * @returns {string | null}
   */
  function collectChartInstrument() {
    const title = (document.title || '').trim();
    if (!title) return null;
    // "NIFTY 50 · 5m · Kite" style titles carry the symbol first.
    const m = title.split(/[|·–—-]/)[0];
    const sym = m ? m.trim() : null;
    if (sym && sym.length > 0 && sym.length < 40) return sym;
    return null;
  }

  /**
   * Detect the currently selected timeframe label (e.g. "5m", "1D").
   * @param {Element | null} canvas
   * @returns {string | null}
   */
  function collectTimeframe(canvas) {
    let aligns = [];
    try {
      aligns = Array.from(document.querySelectorAll('button, [role="button"], [class*="interval"], [class*="range"]'));
    } catch {
      aligns = [];
    }
    for (const el of aligns) {
      const t = (el.textContent || '').trim();
      if (!/^\d{1,3}\s*[mhDdWw]?$|^(1D|1W|D|W|1H|1M)$/i.test(t)) continue;
      const active =
        /active|selected|pressed/.test((el.className || '')) ||
        el.getAttribute('aria-selected') === 'true' ||
        el.getAttribute('aria-pressed') === 'true';
      if (active || el === document.activeElement) return t;
    }
    return null;
  }

  /**
   * Best-effort read of the chart currently rendered by the page. Prefers
   * candle series from an ECharts instance, then falls back to the OHLC
   * summary the broker paints as text. Returns `chart` or null.
   * @param {string} broker
   * @returns {import("./types").ChartExtract | null}
   */
  function collectChart(broker) {
    const canvas = findChartCanvas();
    console.log('[lsb-candles] collectChart() findChartCanvas matched:', canvas ? 'yes' : 'no');
    const candles = canvas ? (collectEChartsCandles(canvas) || []) : [];
    console.log('[lsb-candles] collectChart() ECharts path yielded candles:', candles.length);
    let headerCandle = null;
    if (candles.length === 0 && canvas) {
      headerCandle = collectHeaderOhlc(canvas);
      if (headerCandle) candles.push(headerCandle);
      console.log(
        '[lsb-candles] collectChart() ECharts path found none; fallback ' +
        (headerCandle ? 'OK: single header-OHLC candle pushed (reason: no ECharts candles parsed from ' + (window.echarts ? 'existing top-level echarts' : 'missing top-level echarts') + ')' : 'STILL EMPTY: no header-OHLC either')
      );
    }
    const instrument = collectChartInstrument();
    const timeframe = collectTimeframe(canvas);
    if (candles.length === 0 && !instrument && !timeframe) {
      console.log('[lsb-candles] collectChart() returning null (no candles, no instrument, no timeframe)');
      return null;
    }
    const chart = {
      instrument,
      timeframe,
      candles,
    };
    debug('chart', broker, chart);
    return chart;
  }

  /**
   * Per-broker selector chain used to read fields off a row element.
   * Keys are SymbolExtract fields; ticker/price are required-ish, the rest optional.
   * `change` is read as raw text and classified as `change` (absolute) or
   * `changePercent` by whether the cell displays a `%`.
   * @type {Record<string, Record<string, string[]>>}
   */
  const FIELD_SEL = {
    angel: {
      ticker: ['.scrip-name', '[class*="name"]', 'td:first-child'],
      name: ['[class*="company"]', '[class*="scrip-name"]'],
      price: ['.ltp', '[class*="ltp"]', '[class*="last-price"]', 'td:nth-child(2)'],
      change: ['[class*="change"]', '[class*="-%"]', '[class*="chg"]', 'td:nth-child(3)'],
      volume: ['[class*="volume"]', '[class*="vol"]', 'td:nth-child(4)'],
      bid: ['[class*="bid"]'],
      ask: ['[class*="ask"]'],
    },
    coinswitch: {
      ticker: ['[class*="coin-name"]', '[class*="name"]', 'td:first-child'],
      name: ['[class*="coin-full"]', '[class*="coin"]'],
      price: ['[class*="price"]', '[class*="usd"]', 'td:nth-child(2)'],
      change: ['[class*="change"]', '[class*="chg"]', 'td:nth-child(3)'],
      volume: ['[class*="volume"]', '[class*="vol"]', 'td:nth-child(4)'],
      bid: [],
      ask: [],
    },
    dhan: {
      ticker: ['[class*="scrip-name"]', '[class*="trading-symbol"]', '[class*="symbol"]', '[class*="name"]', 'td:first-child'],
      name: ['[class*="company"]', '[class*="scrip"]'],
      price: ['[class*="ltp"]', '[class*="market-price"]', '[class*="last-price"]', '[class*="price"]', 'td:nth-child(2)'],
      change: ['[class*="change"]', '[class*="percent"]', '[class*="chg"]', 'td:nth-child(3)'],
      volume: ['[class*="volume"]', '[class*="vol"]', 'td:nth-child(4)'],
      bid: ['[class*="bid"]'],
      ask: ['[class*="ask"]'],
    },
  };

  /**
   * Container / row selectors per broker. Fallbacks are deliberately broad.
   * @type {Record<string, {rows: string[], headerRow: string[]}>}
   */
  const ROW_SEL = {
    angel: {
      rows: [
        '[class*="wicket"] tr',
        '[class*="holding"] tr',
        '[class*="watchlist"] tr',
        '[class*="position"] tr',
      ],
      headerRow: ['h1', '[class*="instrument"] h2', '[class*="scrip"] h2'],
    },
    coinswitch: {
      rows: [
        '[data-testid*="coin-row"]',
        '[class*="coin-row"]',
        '[class*="market-row"]',
        '[class*="market"] tr',
      ],
      headerRow: ['h1', '[class*="coin"] h1'],
    },
    dhan: {
      rows: [
        '[data-testid*="watchlist"] [role="row"]',
        '[data-testid*="watchlist"] tr',
        '[class*="watchlist"] tr',
        '[class*="scrip-row"]',
        '[class*="instrument"] tr',
      ],
      headerRow: ['h1', '[data-testid*="instrument"] h1', '[class*="instrument"] h1', '[class*="scrip"] h1'],
    },
  };

  // ---------------------------------------------------------- chart capture
  /**
   * Base64-encode raw bytes without hitting per-argument stack limits.
   * @param {Uint8Array} bytes
   * @returns {string}
   */
  function bytesToBase64(bytes) {
    let bin = '';
    const CHUNK = 0x8000;
    for (let i = 0; i < bytes.length; i += CHUNK) {
      bin += String.fromCharCode.apply(null, bytes.subarray(i, i + CHUNK));
    }
    return btoa(bin);
  }

  // TEMP DEBUG — capture diagnosis.
  /**
   * Instrumentation for captureChartCanvas() debugging. Inventories every
   * <canvas> in the light DOM (count + size + parent), reports which current
   * findChartCanvas() selector matched, and scans shadow roots for canvases
   * that document.querySelectorAll() cannot see. Pure logging; no fix logic.
   */
  function debugCanvasInventory() {
    let all = [];
    try {
      all = Array.from(document.querySelectorAll('canvas'));
    } catch (e) {
      console.log('[lsb-capture] document.querySelectorAll("canvas") threw:', e);
    }
    console.log('[lsb-capture] total <canvas> in light DOM =', all.length);
    all.forEach((c, i) => {
      let r = null;
      try {
        r = c.getBoundingClientRect();
      } catch (e) { /* noop */ }
      const p = c.parentElement;
      console.log('[lsb-capture] canvas[' + i + ']', JSON.stringify({
        index: i,
        widthAttr: c.width,
        heightAttr: c.height,
        rectWidth: r ? Math.round(r.width) : null,
        rectHeight: r ? Math.round(r.height) : null,
        parentTag: p ? p.tagName : null,
        parentClass: p ? (p.getAttribute('class') || null) : null,
        parentId: p ? (p.id || null) : null,
        isShadowHost: !!c.shadowRoot,
      }));
    });

    // Step 2: which current findChartCanvas() selector matched (or not).
    const selectors = [
      '[data-testid*="chart"]',
      '[class*="chart"] canvas',
      '[class*="candle"] canvas',
      '[class*="graph"] canvas',
      '.chart canvas',
      'canvas',
    ];
    for (const sel of selectors) {
      let cands = [];
      try {
        cands = Array.from(document.querySelectorAll(sel));
      } catch (e) {
        console.log('[lsb-capture] selector threw:', sel, String(e));
        continue;
      }
      let sized = 0;
      for (const c of cands) {
        let rr = null;
        try {
          rr = c.getBoundingClientRect();
        } catch (e) { /* noop */ }
        if (rr !== null && rr.width >= 120 && rr.height >= 40) sized += 1;
      }
      console.log('[lsb-capture] selector ' + JSON.stringify(sel) + ' => matches ' + cands.length + ', sized(>=120x40) ' + sized);
    }

    // Step 3: canvases inside shadow roots (invisible to querySelectorAll).
    const hidden = [];
    const walkShadow = (host, depth) => {
      if (!host || !host.shadowRoot) return;
      let inner = [];
      try {
        inner = Array.from(host.shadowRoot.querySelectorAll('*'));
      } catch (e) { /* noop */ }
      for (let j = 0; j < inner.length; j += 1) {
        if (inner[j].shadowRoot) walkShadow(inner[j], depth + 1);
      }
      let cv = [];
      try {
        cv = Array.from(host.shadowRoot.querySelectorAll('canvas'));
      } catch (e) { /* noop */ }
      for (const c of cv) {
        let r = null;
        try {
          r = c.getBoundingClientRect();
        } catch (e) { /* noop */ }
        hidden.push({
          depth: depth,
          hostTag: host.tagName,
          hostClass: host.getAttribute('class') || host.id || null,
          canvasWidthAttr: c.width,
          canvasHeightAttr: c.height,
          rectWidth: r ? Math.round(r.width) : null,
          rectHeight: r ? Math.round(r.height) : null,
        });
      }
    };
    let allEls = [];
    try {
      allEls = Array.from(document.querySelectorAll('*'));
    } catch (e) { /* noop */ }
    for (let k = 0; k < allEls.length; k += 1) {
      if (allEls[k].shadowRoot) walkShadow(allEls[k], 1);
    }
    console.log('[lsb-capture] canvases inside shadow roots = ' + hidden.length, hidden);
  }

  // TEMP DEBUG — capture diagnosis.
  /**
   * Instrumentation for captureChartCanvas() debugging. Inventories every
   * <iframe> in the light DOM: src, fallback attribute, rendered rect; then
   * probes same-origin access via contentDocument and counts <canvas> inside
   * each reachable document. Pure logging; no fix logic.
   */
  function debugIframeInventory() {
    let frames = [];
    try {
      frames = Array.from(document.querySelectorAll('iframe'));
    } catch (e) {
      console.log('[lsb-iframe] document.querySelectorAll("iframe") threw:', e);
    }
    console.log('[lsb-iframe] total <iframe> in light DOM =', frames.length);
    frames.forEach((f, i) => {
      let srcProp = '';
      try {
        srcProp = f.src || '';
      } catch (e) {
        srcProp = '';
      }
      const srcAttr = (f.getAttribute('src') || '').trim();
      const srcIsUseful = srcProp.indexOf('://') !== -1;
      const srcUsed = srcIsUseful ? srcProp : (srcAttr || srcProp || null);
      let r = null;
      try {
        r = f.getBoundingClientRect();
      } catch (e) { /* noop */ }
      const hiddenFlag = r !== null && (r.width === 0 || r.height === 0);
      console.log('[lsb-iframe] frame[' + i + ']', JSON.stringify({
        index: i,
        srcProp: srcProp,
        srcAttr: srcAttr,
        srcUsed: srcUsed,
        rectWidth: r ? Math.round(r.width) : null,
        rectHeight: r ? Math.round(r.height) : null,
        rectLeft: r ? Math.round(r.left) : null,
        rectTop: r ? Math.round(r.top) : null,
        rectX: r ? Math.round(r.x) : null,
        rectY: r ? Math.round(r.y) : null,
        hidden: hiddenFlag,
      }));

      let doc = null;
      let access = null;
      let accessDetail = null;
      try {
        doc = f.contentDocument;
        access = doc === null ? 'null' : 'document';
      } catch (e) {
        doc = null;
        const msg = String(e);
        const name = e && e.name ? e.name : '';
        if (name === 'SecurityError' || /security|same-origin|access|permission/i.test(msg)) {
          access = 'SecurityError';
        } else {
          access = 'other';
        }
        accessDetail = msg;
      }
      if (access === 'document') {
        let canvasCount = 0;
        let probe = 'ok';
        try {
          canvasCount = doc.querySelectorAll('canvas').length;
        } catch (e) {
          probe = 'threw: ' + String(e);
        }
        console.log('[lsb-iframe] frame[' + i + '] access=document canvasCount=' + canvasCount + ' (' + probe + ')');
      } else if (access === 'null') {
        console.log('[lsb-iframe] frame[' + i + '] access=null (cross-origin or not yet loaded)');
      } else if (access === 'SecurityError') {
        console.log('[lsb-iframe] frame[' + i + '] access=SecurityError' + (accessDetail ? ' -> ' + accessDetail : ''));
      } else {
        console.log('[lsb-iframe] frame[' + i + '] access=other' + (accessDetail ? ' -> ' + accessDetail : ''));
      }
    });
  }

  /**
   * Capture the currently-rendered chart canvas as an in-memory PNG.
   *
   * Runs ONLY on demand (manual "Analyze chart" trigger). Reads the same chart
   * element as the DOM extracts (`findChartCanvas()`) but produces bitmap
   * bytes instead of structured candles. The chart is captured as a composite
   * of every chart pane canvas (price, volume and any indicator panes such as
   * RSI / MACD); each pane is drawn 1:1 pixel-for-pixel onto an output canvas,
   * so the model sees the whole chart widget exactly as the broker rendered
   * it. Returns a camelCase payload whose `status` splits success from
   * explicit failures:
   *
   *   - `ok`             → `image` carries a base64 PNG (metadata populated).
   *   - `canvas_tainted` → a source canvas could not be read (cross-origin
   *                        draw) and no fallback frame was possible.
   *   - `no_chart`       → `findChartCanvas()` found nothing to capture.
   *
   * The image bytes never touch disk; they are produced in memory, handed to
   * Rust once for the AI analysis, and dropped after the response.
   *
   * @returns {Promise<{
   *   brokerType: string|null, symbol: string|null, timeframe: string|null,
   *   width: number, height: number, timestamp: number, mime: string,
   *   status: string, image: string|null,
   *   panes: number,
   * }>}
   */
  async function captureChartCanvas() {
    // TEMP DEBUG — capture diagnosis.
    debugCanvasInventory();
    debugIframeInventory();
    const canvas = findChartCanvas();
    if (canvas) {
      let r = null;
      try {
        r = canvas.getBoundingClientRect();
      } catch (e) { /* noop */ }
      const p = canvas.parentElement;
      console.log('[lsb-capture] findChartCanvas() matched canvas', JSON.stringify({
        widthAttr: canvas.width,
        heightAttr: canvas.height,
        rectWidth: r ? Math.round(r.width) : null,
        rectHeight: r ? Math.round(r.height) : null,
        parentTag: p ? p.tagName : null,
        parentClass: p ? (p.getAttribute('class') || null) : null,
        parentId: p ? (p.id || null) : null,
      }));
    } else {
      console.log('[lsb-capture] findChartCanvas() matched nothing.');
    }
    // The chart widget is rarely a single canvas: TradingView-style brokers
    // render each pane (price, volume, RSI, MACD, …) as its own canvas stacked
    // vertically inside the chart container. Capture them ALL as one composite
    // frame (plus the legend strip) so the analysis image actually shows the
    // indicators instead of just the price candles.
    const frame = captureChartRegion();
    let rootRect = null;
    try {
      rootRect = canvas ? canvas.getBoundingClientRect() : null;
    } catch (e) { /* noop */ }
    const base = {
      brokerType: detectBroker(),
      symbol: collectChartInstrument(),
      timeframe: canvas ? collectTimeframe(canvas) : null,
      width: (frame && frame.canvas.width) || (canvas && canvas.width) || (rootRect && Math.round(rootRect.width)) || 0,
      height: (frame && frame.canvas.height) || (canvas && canvas.height) || (rootRect && Math.round(rootRect.height)) || 0,
      timestamp: Date.now(),
      mime: 'image/png',
    };
    if (!frame && !canvas) {
      return { ...base, status: 'no_chart', image: null };
    }
    // A chart canvas exists but no composite frame could be built (all panes
    // skipped as tainted/unreadable): fall back to the single price canvas the
    // DOM extractor found.
    const toBlob = frame ? frame.canvas : canvas;
    let blob = null;
    try {
      // The composite canvas is a fresh same-origin bitmap; toBlob resolves to
      // null only when the browser cannot encode it.
      blob = await new Promise((resolve) => toBlob.toBlob(resolve, 'image/png'));
    } catch {
      blob = null;
    }
    console.log('[lsb-capture] composite toBlob ' + (blob ? 'ok (bytes=' + blob.size + ', panes=' + (frame ? frame.panes : 1) + ')' : 'null (tainted/unencodable)'));
    const capture = { ...base, status: blob ? 'ok' : 'canvas_tainted', image: blob ? bytesToBase64(new Uint8Array(await blob.arrayBuffer())) : null, panes: frame ? frame.panes : (canvas ? 1 : 0) };
    console.log('[lsb-capture] result status=' + capture.status + ' imageB64Len=' + (capture.image ? capture.image.length : 0) + ' panes=' + capture.panes);
    return capture;
  }

  /**
   * Composite every same-document chart canvas (price + indicator/volume
   * panes) into a single PNG frame. Returns `{ canvas, panes }`; `panes` is
   * the number of canvases folded in. When only the price pane exists the
   * output is functionally identical to capturing that one canvas.
   *
   * The source canvases are drawn 1:1 pixel-for-pixel (CanvasImageSource →
   * drawImage), so the composite has one pixel per source pixel regardless of
   * devicePixelRatio; axe labels / legends that the broker paints onto the
   * panes are carried over automatically.
   *
   * @returns {{ canvas: HTMLCanvasElement, panes: number } | null}
   */
  function captureChartRegion() {
    const host = findChartCanvas();
    if (!host) return null;
    const doc = host.ownerDocument || document;
    const collectCanvases = (root) => {
      let all = [];
      try {
        all = Array.from(root.querySelectorAll('canvas'));
      } catch (e) {
        return [];
      }
      const usable = [];
      for (const c of all) {
        const r = sizedRect(c);
        if (r && c.width > 0 && c.height > 0 &&
            typeof c.toBlob === 'function' && typeof c.width === 'number') {
          usable.push(c);
        }
      }
      return usable;
    };
    const canvases = collectCanvases(doc);

    // Guard: at minimum the price canvas itself must be capturable.
    let found = false;
    for (const c of canvases) {
      if (c === host) { found = true; break; }
    }
    if (!found && host.toBlob) {
      canvases.unshift(host);
    }

    // Union box of every pane in CSS px.
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const c of canvases) {
      let r = null;
      try {
        r = c.getBoundingClientRect();
      } catch (e) { continue; }
      if (!r) continue;
      if (r.width < 120 && r.height < 40) continue;
      minX = Math.min(minX, r.left);
      minY = Math.min(minY, r.top);
      maxX = Math.max(maxX, r.right);
      maxY = Math.max(maxY, r.bottom);
    }
    if (!Number.isFinite(minX)) return null;
    const cssW = Math.max(1, Math.ceil(maxX - minX));
    const cssH = Math.max(1, Math.ceil(maxY - minY));

    // Output canvas: CSS box × devicePixelRatio keeps the panes sharp on the
    // model side. 1:1 pixel mapping below in a fresh, untainted bitmap.
    const out = doc.createElement('canvas');
    let dpr = 1;
    try {
      dpr = (doc.defaultView && doc.defaultView.devicePixelRatio) || 1;
    } catch (e) { /* noop */ }
    const w = Math.max(1, Math.round(cssW * dpr));
    const h = Math.max(1, Math.round(cssH * dpr));
    if (w > 12000 || h > 12000) {
      out.width = Math.max(1, Math.round(cssW));
      out.height = Math.max(1, Math.round(cssH));
      dpr = 1;
    } else {
      out.width = w;
      out.height = h;
    }
    const ctx = out.getContext('2d');
    if (!ctx) return null;
    let panes = 0;
    for (const c of canvases) {
      let r = null;
      try {
        r = c.getBoundingClientRect();
      } catch (e) { continue; }
      if (!r) continue;
      // Draw the pane's internal pixel buffer into the output at (x,y)*dpr.
      // CanvasImageSource draws without re-sampling when the source and dest
      // sizes match the pane's internal resolution.
      try {
        ctx.drawImage(c, (r.left - minX) * dpr, (r.top - minY) * dpr, c.width, c.height);
        panes += 1;
      } catch (e) {
        // Tainted pane (cross-origin): skip it, keep the remaining panes.
        console.log('[lsb-capture] drawImage skipped tainted pane', e);
      }
    }
    if (panes === 0) return null;
    return { canvas: out, panes };
  }

  /**
   * One-shot entry point used by the manual "Analyze chart" button (Rust evals
   * this via `trigger_chart_capture`). Captures the chart bitmap and hands it
   * to Rust for AI analysis. Never part of the polling loop.
   * @returns {Promise<object>}
   */
  async function analyzeChart() {
    const capture = await captureChartCanvas();
    debug('chart-capture', capture.status, capture.symbol);
    if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) {
      window.__TAURI_INTERNALS__
        .invoke('capture_and_analyze_chart', { payload: capture })
        .catch((err) => debug('capture invoke rejected:', err));
    } else if (TEST_MODE) {
      window.dispatchEvent(new CustomEvent('chart-capture', { detail: capture }));
    }
    return capture;
  }

  // ------------------------------------------------------------- detection
  /**
   * @param {string} host
   * @param {{hosts: string[]}} broker
   */
  function hostMatches(host, broker) {
    return broker.hosts.some((h) => host === h || host.endsWith('.' + h));
  }

  /**
   * @returns {string | null} normalized broker slug, or null when unknown/none.
   */
  function detectBroker() {
    const host = window.location.hostname;
    const title = document.title || '';
    for (const broker of BROKERS) {
      if (hostMatches(host, broker)) return broker.broker;
    }
    for (const broker of BROKERS) {
      if (broker.titleMarkers.some((re) => re.test(title))) return broker.broker;
      if (broker.domMarkers.some((sel) => document.querySelector(sel))) return broker.broker;
    }
    return null;
  }

  // ------------------------------------------------------------ extraction
  /**
   * Read the current instrument from the page header (single-symbol pages).
   * @param {string} broker
   * @returns {import("./types").SymbolExtract | null}
   */
  function currentInstrument(broker) {
    const cfg = ROW_SEL[broker] || ROW_SEL.angel;
    for (const sel of cfg.headerRow || []) {
      const el = document.querySelector(sel);
      if (!el) continue;
      const ticker = (el.textContent || '').trim();
      if (!ticker) continue;
      const fields = FIELD_SEL[broker] || FIELD_SEL.angel;
      // Search the closest trading container for numeric columns.
      const scope = el.closest('[class*="chart"], [class*="instrument"], .container') || document;
      const changeText = firstTextRaw(scope, fields.change);
      const chg = parseChangeText(changeText);
      return {
        ticker,
        name: firstText(scope, fields.name) || null,
        price: firstNum(scope, fields.price),
        change: chg.change,
        bid: firstNum(scope, fields.bid),
        ask: firstNum(scope, fields.ask),
        volume: firstNum(scope, fields.volume),
        changePercent: chg.changePercent,
        ohlc1min: null,
        ohlc5min: null,
        indicators: [],
        orderBook: [],
      };
    }
    return null;
  }

  /**
   * Read one symbol row via the broker's selector chain.
   * @param {Element} row
   * @param {Record<string, string[]>} fields
   * @returns {Partial<import("./types").SymbolExtract> | null}
   */
  function symbolFromSelectors(row, fields) {
    const ticker = firstText(row, fields.ticker);
    if (!ticker) return null;
    const chg = parseChangeText(firstTextRaw(row, fields.change));
    return {
      ticker,
      name: firstText(row, fields.name) || null,
      price: firstNum(row, fields.price),
      change: chg.change,
      bid: firstNum(row, fields.bid),
      ask: firstNum(row, fields.ask),
      volume: firstNum(row, fields.volume),
      changePercent: chg.changePercent,
      ohlc1min: null,
      ohlc5min: null,
      indicators: [],
      orderBook: [],
    };
  }

  /**
   * Extract symbols from every detected row in the page.
   * @param {string} broker
   * @returns {Array<import("./types").SymbolExtract>}
   */
  function collectSymbols(broker) {
    const cfg = ROW_SEL[broker] || ROW_SEL.angel;
    const fields = FIELD_SEL[broker] || FIELD_SEL.angel;
    const seen = new Set();
    const out = [];

    for (const rowsSel of cfg.rows) {
      let rows = [];
      try {
        rows = Array.from(document.querySelectorAll(rowsSel));
      } catch {
        continue;
      }
      for (const row of rows) {
        if (out.length >= MAX_ROWS) break;
        const sym = symbolFromSelectors(row, fields);
        if (!sym || !sym.ticker || seen.has(sym.ticker)) continue;
        seen.add(sym.ticker);
        out.push(sym);
      }
      if (rows.length > 0) break;
    }

    // Generic table fallback: read full rows (price, change %, volume, bid/ask,
    // name) using a header-aware column mapping instead of ordinal positions.
    if (out.length === 0) {
      const de = collectFromTables(broker);
      for (const sym of de) {
        if (out.length >= MAX_ROWS) break;
        if (seen.has(sym.ticker)) continue;
        seen.add(sym.ticker);
        out.push(sym);
      }
    }

    return out;
  }

  /**
   * Compute data_quality from symbols.
   * @param {Array<import("./types").SymbolExtract>} symbols
   */
  function grade(symbols) {
    if (symbols.length === 0) return 'empty';
    if (symbols.some((s) => typeof s.price === 'number')) return 'good';
    return 'degraded';
  }

  // ----------------------------------------------------------------- poll
  function pollOnce() {
    if (document.readyState === 'loading') return;
    // about:/chrome: shells (e.g. the pool's initial about:blank) carry no DOM.
    if (!/^(https?|file):$/.test(window.location.protocol)) return;

    const started = performance.now();
    const broker = detectBroker();
    if (!broker) return;

    const symbols = collectSymbols(broker);
    const instrument = symbols.length === 0 ? currentInstrument(broker) : null;
    if (instrument) symbols.unshift(instrument);

    const chart = collectChart(broker);

    const payload = {
      brokerType: broker,
      timestamp: Date.now(),
      extractionDurationMs: Math.round(performance.now() - started),
      url: window.location.href,
      dataQuality: grade(symbols),
      symbols,
      chart,
    };
    debug(payload);

    if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) {
      window.__TAURI_INTERNALS__
        .invoke('extract_dom', { payload })
        .catch((err) => debug('invoke rejected:', err));
    } else if (TEST_MODE) {
      (window.__EXTRACTIONS__ = window.__EXTRACTIONS__ || []).push(payload);
      window.dispatchEvent(new CustomEvent('extraction-ready', { detail: payload }));
    }
  }

  // Start polling. Guard against leaving a stray timer on document re-inits.
  if (!window.__LOCKSTEP_EXTRACT_TIMER__) {
    window.__LOCKSTEP_EXTRACT_TIMER__ = setInterval(pollOnce, POLL_MS);
    if (document.readyState !== 'loading') {
      pollOnce();
    }
  }

  // On-demand chart capture (no polling involved). Rust triggers this from the
  // UI via `trigger_chart_capture` (browser.rs eval), which calls analyzeChart.
  window.captureChartCanvas = captureChartCanvas;
  window.analyzeChart = analyzeChart;
})();