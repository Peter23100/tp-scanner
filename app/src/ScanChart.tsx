// TradierPro's chart (C:\tradier-pro\src\App.tsx ChartWindow) without any trading: the same
// klinecharts setup, indicators, overlays, toolbar and bar-count presets, fed from Massive through
// the scanner's backend instead of a broker. The scanner's own signals sit on it as fill-style
// triangles whose tip is exactly the price that fired them.
//
// It follows its link colour: a click in a panel of that colour, or (with Follow on) each new fire,
// sets the symbol. 🔒 holds the current symbol while you study it.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { init, dispose, registerOverlay } from "klinecharts";
import type { LineType, CandleType, PolygonType, TooltipShowRule, LayoutChildType, PanePosition } from "klinecharts";
import { api, LINK_HEX, nextColor } from "./api";
import { useStore } from "./store";
import {
  ChartInterval,
  IndicatorKey,
  OHLCVBar,
  INTRADAY_BARS,
  SWING_BARS,
  INTRADAY_INDICATOR_WARMUP,
  PREMKT_START_MIN,
  RTH_START_MIN,
  RTH_END_MIN,
  POST_END_MIN,
  etMinsOfDay,
  etDateStr,
  isRthMs,
  isExtMs,
  safeCalcPOC,
  safeCalcValueArea,
  defaultIndicators,
  defaultBars,
  setIntradayChartBars,
  vpProfileOverlaySpec,
  registerCustomIndicators,
  registerHalfHourAxis,
  registerDayLevelOverlay,
  registerLastPriceMarkOverlay,
  registerVolProfileOverlay,
} from "./chart/tp";

// TradierPro's +ADD lists (App.tsx 1912-1913).
const KLINE_OVERLAYS = ["BOLL", "BBI", "SAR"];
const KLINE_LOWER_PANE = ["MACD", "RSI", "KDJ", "CCI", "DMI", "OBV", "WR", "BIAS"];

const IVL_MS: Record<string, number> = { "1m": 60_000, "5m": 300_000, "15m": 900_000 };

/// Scanner policy for extended hours (differs from TradierPro on purpose). TradierPro shows the
/// extended session on 5m only and drops the premarket at 9:25, which suits trading. The scanner
/// fires in the premarket, and a chart that followed a fire must show the bar that fired, so every
/// intraday timeframe keeps the shown day's premarket and post-market (and anything after the last
/// regular-hours bar), shaded. Earlier days stay regular-hours only, as in TradierPro.
function scannerDisplayBars(raw: OHLCVBar[]): OHLCVBar[] {
  if (!raw.length) return raw;
  const lastDay = etDateStr(raw[raw.length - 1].time * 1000);
  let lastRth = 0;
  for (const b of raw) if (isRthMs(b.time * 1000) && b.time * 1000 > lastRth) lastRth = b.time * 1000;
  return raw.filter((b) => {
    const ms = b.time * 1000;
    return isRthMs(ms) || (isExtMs(ms) && (etDateStr(ms) === lastDay || ms > lastRth));
  });
}

/// A translucent band over one run of extended-hours bars (TradierPro's extHoursShade spans from a
/// bar to the right edge; with the premarket kept, a day can have two runs, so this one has an end).
let _extRangeRegistered = false;
function registerExtRangeOverlay() {
  if (_extRangeRegistered) return;
  _extRangeRegistered = true;
  registerOverlay({
    name: "extHoursRange",
    totalStep: 2,
    needDefaultPointFigure: false,
    needDefaultXAxisFigure: false,
    needDefaultYAxisFigure: false,
    createPointFigures: (params: any) => {
      const [a, b] = params.coordinates ?? [];
      if (!a || !b) return [];
      const h = params.bounding?.height || 0;
      const x0 = Math.max(0, a.x - 3);
      const width = Math.max(0, b.x + 3 - x0);
      return [{ key: "shade", type: "rect", attrs: { x: x0, y: 0, width, height: h }, styles: { style: "fill", color: "rgba(110,130,170,0.10)" }, ignoreEvent: true }];
    },
  });
}

/// The scanner's signal marker: TradierPro's fill triangle (apex ON the price, body hanging away),
/// green up for long, red down for short. Unlike the fill marker it takes the mouse, so hovering
/// names the filters that fired.
let _signalMarkerRegistered = false;
function registerSignalMarkerOverlay() {
  if (_signalMarkerRegistered) return;
  _signalMarkerRegistered = true;
  registerOverlay({
    name: "signalMarker",
    totalStep: 1,
    needDefaultPointFigure: false,
    needDefaultXAxisFigure: false,
    needDefaultYAxisFigure: false,
    createPointFigures: (params: any) => {
      const c = params.coordinates?.[0];
      if (!c) return [];
      const ed = params.overlay.extendData || {};
      const buy: boolean = !!ed.buy;
      const color = buy ? "#00cc55" : "#ff4455";
      const cx = c.x,
        cy = c.y,
        s = 5;
      const baseY = buy ? cy + 8 : cy - 8;
      const figs: any[] = [{ key: "tri", type: "polygon", attrs: { coordinates: [{ x: cx, y: cy }, { x: cx - s, y: baseY }, { x: cx + s, y: baseY }] }, styles: { style: "fill", color } }];
      if (ed.count > 1)
        figs.push({
          key: "n",
          type: "text",
          attrs: { x: cx, y: buy ? baseY + 7 : baseY - 7, text: String(ed.count), align: "center", baseline: "middle" },
          styles: { color, size: 9, weight: 800, family: "Segoe UI", backgroundColor: "rgba(0,0,0,0)", borderSize: 0, paddingLeft: 0, paddingRight: 0, paddingTop: 0, paddingBottom: 0 },
          ignoreEvent: true,
        });
      return figs;
    },
    onMouseEnter: (e: any) => {
      e.overlay?.extendData?.hover?.(true);
      return false;
    },
    onMouseLeave: (e: any) => {
      e.overlay?.extendData?.hover?.(false);
      return false;
    },
  } as any);
}

interface ChartBarsReply {
  bars: [number, number, number, number, number, number][];
  last_minute: number | null;
}
type KBar = { timestamp: number; open: number; high: number; low: number; close: number; volume: number; _rth?: boolean };

export default function ScanChart({ id, initialColor, standalone }: { id: string; initialColor: string; standalone?: boolean }) {
  const [color, setColor] = useState(initialColor);
  const linked = useStore((s) => s.links[color]) ?? "";
  const [held, setHeld] = useState<string | null>(null);
  const symbol = held ?? linked;
  const status = useStore((s) => s.status);
  const replayDate = status?.mode === "replay" ? status.replay?.date ?? null : null;
  const marketMs = status?.market_ms ?? null;
  const signals = useStore((s) => s.signals);

  const [ivl, setIvl] = useState<ChartInterval>("5m");
  const [bars, setBars] = useState<number>(defaultBars("5m"));
  const [enabled, setEnabled] = useState<Set<IndicatorKey>>(() => defaultIndicators("5m"));
  const [extras, setExtras] = useState<string[]>([]);
  const [showVP, setShowVP] = useState(false);
  const [typed, setTyped] = useState("");
  const [ready, setReady] = useState(false);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState("");
  const [noData, setNoData] = useState(false);
  const [hover, setHover] = useState<string | null>(null);
  const isIntraday = ivl === "1m" || ivl === "5m" || ivl === "15m";

  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<any>(null);
  const dataRef = useRef<KBar[]>([]);
  const dataSymbolRef = useRef("");
  const lastMinuteRef = useRef<number>(-Infinity);
  const loadSeqRef = useRef(0);
  const activeIndsRef = useRef<Set<string>>(new Set());
  const extraPaneIdsRef = useRef<Map<string, string>>(new Map());
  const pocOverlayIdRef = useRef<string | null>(null);
  const vaOverlayIdsRef = useRef<string[]>([]);
  const daySepOverlayIdsRef = useRef<string[]>([]);
  const dayLevelOverlayIdsRef = useRef<string[]>([]);
  const extShadeOverlayIdsRef = useRef<string[]>([]);
  const lastPriceOverlayIdRef = useRef<string | null>(null);
  const signalOverlayIdsRef = useRef<string[]>([]);
  const vpIdRef = useRef<string | null>(null);
  const pocValueRef = useRef<number | null>(null);
  const vaValueRef = useRef<{ vah: number; val: number } | null>(null);
  const showVPRef = useRef(false);
  showVPRef.current = showVP;
  const currentBarsRef = useRef(bars);
  currentBarsRef.current = bars;
  const marketRef = useRef(marketMs);
  marketRef.current = marketMs;
  const replayRef = useRef(replayDate);
  replayRef.current = replayDate;

  /// The market clock: the replay's time, or now.
  const clock = () => (replayRef.current && marketRef.current != null ? marketRef.current : Date.now());

  function changeIvl(next: ChartInterval) {
    setIvl(next);
    setBars(defaultBars(next));
    setEnabled(defaultIndicators(next));
  }

  // ── chart lifecycle (TradierPro initChart, verbatim styles) ──
  useEffect(() => {
    registerCustomIndicators();
    registerHalfHourAxis();
    registerVolProfileOverlay();
    registerDayLevelOverlay();
    registerExtRangeOverlay();
    registerLastPriceMarkOverlay();
    registerSignalMarkerOverlay();
    let raf: number;
    function tryInit() {
      const el = containerRef.current;
      if (!el) {
        raf = requestAnimationFrame(tryInit);
        return;
      }
      const { width, height } = el.getBoundingClientRect();
      if (width < 10 || height < 10) {
        raf = requestAnimationFrame(tryInit);
        return;
      }
      initChart(el);
    }
    raf = requestAnimationFrame(tryInit);
    return () => {
      cancelAnimationFrame(raf);
      const el = containerRef.current;
      if (el) {
        try {
          dispose(el);
        } catch {}
      }
      chartRef.current = null;
    };
  }, []);

  function initChart(el: HTMLDivElement) {
    try {
      const chart = init(el, {
        layout: [
          { type: "candle" as unknown as LayoutChildType },
          { type: "xAxis" as unknown as LayoutChildType, options: { axisOptions: { name: "halfHourAxis" }, position: "bottom" as unknown as PanePosition } },
        ],
        styles: {
          grid: { show: true, horizontal: { show: true, color: "#0d1a2e", style: "solid" as unknown as LineType, size: 1 }, vertical: { show: true, color: "#0d1a2e", style: "solid" as unknown as LineType, size: 1 } },
          candle: {
            type: "candle_solid" as unknown as CandleType,
            bar: { upColor: "#00ff66", downColor: "#ff2244", noChangeColor: "#888888", upBorderColor: "#00ff88", downBorderColor: "#ff3355", upWickColor: "#00ee55", downWickColor: "#ee1133" },
            priceMark: { high: { show: false }, low: { show: false }, last: { show: true, upColor: "#00aa44", downColor: "#cc2200", line: { show: false } } },
            tooltip: { showRule: "none" as unknown as TooltipShowRule },
          },
          xAxis: { axisLine: { color: "#1a2a44" }, tickText: { color: "#88aacc" }, tickLine: { color: "#1a2a44" } },
          yAxis: { size: 60, axisLine: { color: "#1a2a44" }, tickText: { color: "#88aacc" }, tickLine: { color: "#1a2a44" } },
          separator: { size: 1, color: "#1a2a44" },
          crosshair: {
            horizontal: { line: { color: "#334466", style: "dashed" as unknown as LineType }, text: { color: "#c0cce0", backgroundColor: "#1a2a44", borderColor: "#1a2a44" } },
            vertical: { line: { color: "#334466", style: "dashed" as unknown as LineType }, text: { color: "#c0cce0", backgroundColor: "#1a2a44", borderColor: "#1a2a44" } },
          },
          indicator: {
            ohlc: { upColor: "#00ff66", downColor: "#ff2244", noChangeColor: "#888" },
            bars: [{ style: "fill" as unknown as PolygonType, borderStyle: "solid" as unknown as LineType, borderSize: 1, borderDashedValue: [2, 2], upColor: "#00ff66", downColor: "#ff2244", noChangeColor: "#888" }],
            tooltip: { showRule: "none" as unknown as TooltipShowRule },
          },
        } as any,
      } as any);
      if (!chart) {
        setErr("Failed to initialize chart");
        return;
      }
      chartRef.current = chart;
      // Inspection hook for verification over CDP (the page's own state; nothing leaves the window).
      (window as any).__scanChart = { chart, data: () => dataRef.current, signalIds: () => signalOverlayIdsRef.current };
      chart.createIndicator({ name: "VOL", calcParams: [] }, false, { id: "vol_pane", height: 90 });
      try {
        chart.setOffsetRightDistance(24);
      } catch {}
      try {
        chart.setPaneOptions({ id: "candle_pane", gap: { top: 0.08, bottom: 0.1 } });
      } catch {}
      const ro = new ResizeObserver(() => {
        try {
          chart.resize();
          autoFit();
        } catch {}
      });
      ro.observe(el);
      setReady(true);
    } catch (e: any) {
      setErr(String(e?.message || e));
    }
  }

  // ── data ──
  async function loadData() {
    const chart = chartRef.current;
    if (!chart || !symbol) return;
    const seq = ++loadSeqRef.current;
    setLoading(true);
    setErr("");
    setNoData(false);
    try {
      const until = replayRef.current && marketRef.current != null ? marketRef.current : null;
      const r = await invoke<ChartBarsReply>("chart_bars", { symbol, interval: ivl, bars, untilMs: until });
      if (seq !== loadSeqRef.current) return;
      let kData: KBar[];
      if (isIntraday) {
        const raw: OHLCVBar[] = r.bars.map(([t, o, h, l, c, v]) => ({ time: t / 1000, open: o, high: h, low: l, close: c, volume: v }));
        const all = scannerDisplayBars(raw);
        const out = all.length > bars + INTRADAY_INDICATOR_WARMUP ? all.slice(-(bars + INTRADAY_INDICATOR_WARMUP)) : all;
        kData = out.map((b) => ({ timestamp: b.time * 1000, open: b.open, high: b.high, low: b.low, close: b.close, volume: b.volume, _rth: isRthMs(b.time * 1000) }));
        const session = sessionBars(out);
        pocValueRef.current = safeCalcPOC(session);
        vaValueRef.current = safeCalcValueArea(session);
        lastMinuteRef.current = r.last_minute ?? -Infinity;
      } else {
        const out = r.bars.length > bars ? r.bars.slice(-bars) : r.bars;
        kData = out.map(([t, o, h, l, c, v]) => ({ timestamp: t, open: o, high: h, low: l, close: c, volume: v }));
        pocValueRef.current = null;
        vaValueRef.current = null;
        lastMinuteRef.current = -Infinity;
      }
      if (!kData.length) {
        applyNoData();
        return;
      }
      chart.applyNewData(kData);
      dataSymbolRef.current = symbol;
      dataRef.current = kData;
      if (isIntraday) setIntradayChartBars(kData);
      autoFit();
      syncAll();
    } catch (e: any) {
      if (seq === loadSeqRef.current) setErr(String(e?.message || e));
    }
    if (seq === loadSeqRef.current) setLoading(false);
  }

  /// The RTH bars of the session shown (the last bar's date): TradierPro's todaySessionBars, with
  /// "today" being the session on screen so replays profile the replay day.
  function sessionBars(out: OHLCVBar[]): OHLCVBar[] {
    if (!out.length) return [];
    const day = etDateStr(out[out.length - 1].time * 1000);
    return out.filter((b) => isRthMs(b.time * 1000) && etDateStr(b.time * 1000) === day);
  }

  function applyNoData() {
    const chart = chartRef.current;
    dataRef.current = [];
    dataSymbolRef.current = symbol;
    pocValueRef.current = null;
    vaValueRef.current = null;
    if (chart) {
      try {
        chart.applyNewData([]);
      } catch {}
    }
    if (isIntraday) setIntradayChartBars([]);
    syncAll();
    setNoData(true);
    setLoading(false);
  }

  // Tell the engine which symbol to forward; load on symbol / interval / bars / source change.
  useEffect(() => {
    api.chartWatch(id, symbol || null).catch(() => {});
  }, [id, symbol]);
  useEffect(() => () => void api.chartWatch(id, null).catch(() => {}), [id]);
  useEffect(() => {
    if (!ready || !symbol) return;
    loadData();
    // Live: TradierPro's resync polls (5 min intraday, 30 min daily). A replay is fed by the scan.
    if (replayDate) return;
    const ms = isIntraday ? 300_000 : ivl === "daily" ? 1_800_000 : 0;
    if (!ms) return;
    const t = setInterval(loadData, ms);
    return () => clearInterval(t);
  }, [ready, symbol, ivl, bars, replayDate]);

  // Minutes as the engine reads them (live, or the replay as it runs) roll into the last bar.
  useEffect(() => {
    if (!ready || !symbol) return;
    const ivlMs = IVL_MS[ivl] ?? 0;
    const un = listen<{ symbol: string; t: number; o: number; h: number; l: number; c: number; v: number }>("chart-bar", (e) => {
      const b = e.payload;
      if (b.symbol !== symbol || dataSymbolRef.current !== symbol) return;
      const chart = chartRef.current,
        data = dataRef.current;
      if (!chart || !data.length) return;
      if (b.t <= lastMinuteRef.current) return; // already inside the loaded history
      lastMinuteRef.current = b.t;
      const mins = etMinsOfDay(b.t);
      const rth = mins >= RTH_START_MIN && mins < RTH_END_MIN;
      const ext = (mins >= PREMKT_START_MIN && mins < RTH_START_MIN) || (mins >= RTH_END_MIN && mins < POST_END_MIN);
      if (isIntraday && !(rth || ext)) return;
      const last = data[data.length - 1];
      let key: number;
      if (ivlMs) key = Math.floor(b.t / ivlMs) * ivlMs;
      else if (ivl === "daily") {
        const [y, m, d] = etDateStr(b.t).split("-").map(Number);
        key = Date.UTC(y, m - 1, d);
      } else key = last.timestamp;
      if (key === last.timestamp) {
        last.high = Math.max(last.high, b.h);
        last.low = Math.min(last.low, b.l);
        last.close = b.c;
        // Daily and longer bars take price only; their volume comes with the resync (the REST
        // day bar and the live minutes can overlap by a minute or two).
        if (ivlMs) last.volume = (last.volume || 0) + b.v;
        try {
          chart.updateData(last);
        } catch {}
      } else if (key > last.timestamp) {
        const nb: KBar = { timestamp: key, open: b.o, high: b.h, low: b.l, close: b.c, volume: b.v, _rth: ivlMs ? isRthMs(key) : undefined };
        data.push(nb);
        if (data.length > bars + INTRADAY_INDICATOR_WARMUP) data.shift();
        try {
          chart.updateData(nb);
        } catch {}
      }
      syncLastPriceMark();
      syncDayLevels();
      syncSignals();
    });
    return () => {
      un.then((f) => f());
    };
  }, [ready, symbol, ivl, bars]);

  useEffect(() => {
    if (!ready || !chartRef.current) return;
    syncIndicators();
    try {
      chartRef.current.setStyles({ candle: { priceMark: { high: { show: !isIntraday }, low: { show: !isIntraday }, last: { show: true, line: { show: false } } } } });
    } catch {}
    syncDayLevels();
    syncLastPriceMark();
  }, [enabled, ready, ivl]);
  useEffect(() => {
    if (ready && chartRef.current) syncExtras();
  }, [extras, ready, ivl]);
  useEffect(() => {
    syncVolProfile();
  }, [showVP]);
  useEffect(() => {
    if (ready) syncSignals();
  }, [signals, ready, ivl]);

  function syncAll() {
    syncPOC();
    syncVA();
    syncDaySeparators();
    syncExtendedShade();
    syncDayLevels();
    syncSignals();
    syncLastPriceMark();
    syncVolProfile();
  }

  // ── TradierPro's sync helpers (ChartWindow), unchanged but for the market clock ──
  function syncIndicators() {
    const chart = chartRef.current;
    if (!chart) return;
    const pairs: Array<[IndicatorKey, string, boolean]> = [
      ["vwap", "VWAP", isIntraday],
      ["vwap_1sigma", "VWAPSD1", isIntraday],
      ["vwap_2sigma", "VWAPSD2", isIntraday],
      ["sma9", "MA9", true],
      ["sma20", "MA20", true],
      ["sma50", "MA50", true],
      ["sma200", "MA200", true],
      ["ema9", "EMA9", true],
      ["ema20", "EMA20", true],
      ["ema50", "EMA50", true],
      ["ema200", "EMA200", true],
    ];
    for (const [key, name, allowed] of pairs) {
      const wantOn = enabled.has(key) && allowed;
      const isOn = activeIndsRef.current.has(name);
      if (wantOn && !isOn) {
        try {
          chart.createIndicator(name, true, { id: "candle_pane" });
          activeIndsRef.current.add(name);
        } catch {}
      } else if (!wantOn && isOn) {
        try {
          chart.removeIndicator("candle_pane", name);
          activeIndsRef.current.delete(name);
        } catch {}
      }
    }
    syncPOC();
    syncVA();
  }

  function syncExtras() {
    const chart = chartRef.current;
    if (!chart) return;
    for (const name of extras) {
      if (extraPaneIdsRef.current.has(name)) continue;
      const overlay = KLINE_OVERLAYS.includes(name);
      try {
        const paneId = chart.createIndicator(name, overlay, overlay ? { id: "candle_pane" } : undefined);
        if (paneId) extraPaneIdsRef.current.set(name, paneId);
      } catch {}
    }
    for (const name of [...extraPaneIdsRef.current.keys()]) {
      if (extras.includes(name)) continue;
      const paneId = extraPaneIdsRef.current.get(name);
      try {
        if (paneId) chart.removeIndicator(paneId, name);
      } catch {}
      extraPaneIdsRef.current.delete(name);
    }
  }

  function syncPOC() {
    const chart = chartRef.current;
    if (!chart) return;
    if (pocOverlayIdRef.current != null) {
      try {
        chart.removeOverlay(pocOverlayIdRef.current);
      } catch {}
      pocOverlayIdRef.current = null;
    }
    if (enabled.has("poc") && isIntraday && pocValueRef.current != null) {
      try {
        pocOverlayIdRef.current = chart.createOverlay({ name: "priceLine", points: [{ value: pocValueRef.current }], styles: { line: { color: "rgba(255,195,40,0.9)", style: "dashed", size: 1 } } });
      } catch {}
    }
  }

  function syncVA() {
    const chart = chartRef.current;
    if (!chart) return;
    for (const id of vaOverlayIdsRef.current) {
      try {
        chart.removeOverlay(id);
      } catch {}
    }
    vaOverlayIdsRef.current = [];
    if (!(enabled.has("va") && isIntraday && vaValueRef.current)) return;
    const { vah, val } = vaValueRef.current;
    for (const v of [vah, val]) {
      try {
        const id = chart.createOverlay({ name: "priceLine", points: [{ value: v }], styles: { line: { color: "rgba(255,195,40,0.45)", style: "dotted", size: 1 } } });
        if (id != null) vaOverlayIdsRef.current.push(id as string);
      } catch {}
    }
  }

  const syncVolProfile = useCallback(() => {
    const chart = chartRef.current;
    if (!chart) return;
    if (vpIdRef.current) {
      try {
        chart.removeOverlay(vpIdRef.current);
      } catch {}
      vpIdRef.current = null;
    }
    if (!showVPRef.current) return;
    let spec: ReturnType<typeof vpProfileOverlaySpec>;
    try {
      spec = vpProfileOverlaySpec(dataRef.current);
    } catch {
      return;
    }
    if (!spec) return;
    try {
      const id = chart.createOverlay({ name: "volProfile", points: spec.points, extendData: spec.extendData, lock: true });
      if (id) vpIdRef.current = id as string;
    } catch {}
  }, []);

  function syncDaySeparators() {
    const chart = chartRef.current;
    if (!chart) return;
    for (const id of daySepOverlayIdsRef.current) {
      try {
        chart.removeOverlay(id);
      } catch {}
    }
    daySepOverlayIdsRef.current = [];
    if (!isIntraday) return;
    for (const b of dataRef.current) {
      if (etMinsOfDay(b.timestamp) !== RTH_START_MIN) continue;
      try {
        const id = chart.createOverlay({ name: "verticalStraightLine", points: [{ timestamp: b.timestamp }], styles: { line: { color: "rgba(120,160,220,0.35)", style: "dashed", size: 1 } }, lock: true });
        if (id) daySepOverlayIdsRef.current.push(id as string);
      } catch {}
    }
  }

  function syncExtendedShade() {
    const chart = chartRef.current;
    if (!chart) return;
    for (const id of extShadeOverlayIdsRef.current) {
      try {
        chart.removeOverlay(id);
      } catch {}
    }
    extShadeOverlayIdsRef.current = [];
    if (!isIntraday) return;
    const data = dataRef.current;
    // One band per run of consecutive extended-hours bars.
    let start: number | null = null;
    for (let i = 0; i <= data.length; i++) {
      const ext = i < data.length && data[i]._rth === false;
      if (ext && start === null) start = data[i].timestamp;
      if (!ext && start !== null) {
        try {
          const id = chart.createOverlay({ name: "extHoursRange", points: [{ timestamp: start }, { timestamp: data[i - 1].timestamp }], lock: true });
          if (id) extShadeOverlayIdsRef.current.push(id as string);
        } catch {}
        start = null;
      }
    }
  }

  function syncLastPriceMark() {
    const chart = chartRef.current;
    if (!chart) return;
    if (lastPriceOverlayIdRef.current != null) {
      try {
        chart.removeOverlay(lastPriceOverlayIdRef.current);
      } catch {}
      lastPriceOverlayIdRef.current = null;
    }
    const data = dataRef.current;
    if (!data.length) return;
    const last = data[data.length - 1];
    const prev = data.length > 1 ? data[data.length - 2] : last;
    const up = last.close >= (prev.close ?? last.open);
    try {
      lastPriceOverlayIdRef.current = chart.createOverlay({ name: "lastPriceMark", points: [{ timestamp: last.timestamp, value: last.close }], styles: { line: { color: up ? "#00aa44" : "#cc2200" } }, lock: true }) as string;
    } catch {}
  }

  function syncDayLevels() {
    const chart = chartRef.current;
    if (!chart) return;
    for (const id of dayLevelOverlayIdsRef.current) {
      try {
        chart.removeOverlay(id);
      } catch {}
    }
    dayLevelOverlayIdsRef.current = [];
    if (!isIntraday) return;
    const data = dataRef.current;
    if (!data.length) return;
    type Day = { first: number; last: number; open: number; high: number; low: number; close: number; highTs: number; lowTs: number };
    const days = new Map<string, Day>();
    const order: string[] = [];
    for (const b of data) {
      if (b._rth === false) continue;
      const d = etDateStr(b.timestamp);
      const g = days.get(d);
      if (!g) {
        days.set(d, { first: b.timestamp, last: b.timestamp, open: b.open, high: b.high, low: b.low, close: b.close, highTs: b.timestamp, lowTs: b.timestamp });
        order.push(d);
      } else {
        g.last = b.timestamp;
        g.close = b.close;
        if (b.high > g.high) {
          g.high = b.high;
          g.highTs = b.timestamp;
        }
        if (b.low < g.low) {
          g.low = b.low;
          g.lowTs = b.timestamp;
        }
      }
    }
    if (!order.length) return;
    const todayKey = order[order.length - 1];
    const now = clock();
    const lastSessionOver = todayKey !== etDateStr(now) || etMinsOfDay(now) >= RTH_END_MIN;
    const tag = (ts: number, value: number, text: string, color: string, above: boolean, rows = 0) => {
      try {
        const id = chart.createOverlay({ name: "dayLevelTag", points: [{ timestamp: ts, value }], extendData: { text, color, above, offsetRows: rows }, lock: true });
        if (id) dayLevelOverlayIdsRef.current.push(id as string);
      } catch {}
    };
    const fmt = (v: number) => v.toFixed(2);
    const GREY = "#76808F";
    let pMin = Infinity,
      pMax = -Infinity;
    for (const b of data) {
      if (b.low < pMin) pMin = b.low;
      if (b.high > pMax) pMax = b.high;
    }
    const range = pMax - pMin || 1;
    const usableH = (containerRef.current?.clientHeight || 400) * 0.82;
    const usableW = Math.max(200, (containerRef.current?.clientWidth || 800) - 60 - 24);
    const barSpace = Math.max(2, usableW / data.length);
    const px = (ts: number) => {
      const i = data.findIndex((b) => b.timestamp === ts);
      return (i < 0 ? 0 : i) * barSpace;
    };
    const py = (value: number) => ((pMax - value) / range) * usableH;
    const ROW = 15,
      XT = 58,
      BASE = 11;
    type DT = { ts: number; value: number; text: string; above: boolean };
    const above: DT[] = [],
      below: DT[] = [];
    for (const key of order.slice(-2)) {
      const g = days.get(key)!;
      below.push({ ts: g.lowTs, value: g.low, text: `${fmt(g.low)}`, above: false });
      above.push({ ts: g.highTs, value: g.high, text: `${fmt(g.high)}`, above: true });
      above.push({ ts: g.first, value: g.open, text: `O ${fmt(g.open)}`, above: true });
      if (key !== todayKey || lastSessionOver) above.push({ ts: g.last, value: g.close, text: `C ${fmt(g.close)}`, above: true });
    }
    const place = (arr: DT[]) => {
      const dir = arr[0]?.above ? -1 : 1;
      arr.sort((a, b) => (dir < 0 ? b.value - a.value : a.value - b.value));
      const placed: { x: number; y: number }[] = [];
      for (const t of arr) {
        const x = px(t.ts);
        let lane = 0,
          ey = py(t.value) + dir * (BASE + lane * ROW);
        while (placed.some((p) => Math.abs(p.x - x) < XT && Math.abs(p.y - ey) < ROW)) {
          lane++;
          ey = py(t.value) + dir * (BASE + lane * ROW);
        }
        placed.push({ x, y: ey });
        tag(t.ts, t.value, t.text, GREY, t.above, lane);
      }
    };
    place(above);
    place(below);
  }

  /// The scanner's signals on this symbol: one triangle per bar and side, its tip at the price
  /// that fired (the newest fire's `last` when several share a bar). Intraday only, as TradierPro's
  /// fill marks: a price on a daily bar says little about when.
  const signalsRef = useRef(signals);
  signalsRef.current = signals;
  function syncSignals() {
    const chart = chartRef.current;
    if (!chart) return;
    for (const id of signalOverlayIdsRef.current) {
      try {
        chart.removeOverlay(id);
      } catch {}
    }
    signalOverlayIdsRef.current = [];
    const ivlMs = IVL_MS[ivl];
    const data = dataRef.current;
    if (!ivlMs || !data.length || dataSymbolRef.current !== symbol) return;
    const first = data[0].timestamp,
      lastTs = data[data.length - 1].timestamp;
    const have = new Set(data.map((b) => b.timestamp));
    const groups = new Map<string, { ts: number; buy: boolean; price: number; names: string[]; at: string }>();
    for (const s of signalsRef.current) {
      if (s.symbol !== symbol || s.last == null) continue;
      const ts = Math.floor((s.at_ms - 60_000) / ivlMs) * ivlMs; // the bar that fired it
      if (ts < first || ts > lastTs || !have.has(ts)) continue;
      const buy = s.side !== "short";
      const k = `${ts}/${buy}`;
      const g = groups.get(k) ?? { ts, buy, price: s.last, names: [], at: s.time };
      g.price = s.last;
      g.at = s.time;
      if (!g.names.includes(s.filter)) g.names.push(s.filter);
      groups.set(k, g);
    }
    for (const g of groups.values()) {
      const label = `${g.buy ? "▲" : "▼"} ${g.at} @ ${g.price.toFixed(2)}  ${g.names.join(", ")}`;
      try {
        const id = chart.createOverlay({
          name: "signalMarker",
          points: [{ timestamp: g.ts, value: g.price }],
          extendData: { buy: g.buy, count: g.names.length, hover: (on: boolean) => setHover(on ? label : null) },
          lock: true,
        });
        if (id) signalOverlayIdsRef.current.push(id as string);
      } catch (e) {
        console.error("[CHART] signal marker failed", e);
      }
    }
  }

  function autoFit() {
    const chart = chartRef.current;
    const data = dataRef.current;
    if (!chart || !data.length) return;
    const w = containerRef.current?.clientWidth || 800;
    const usable = Math.max(200, w - 60 - 24);
    const fitCount = Math.min(data.length, currentBarsRef.current || data.length);
    try {
      chart.setBarSpace(Math.max(2, usable / fitCount));
    } catch {}
  }

  function fitContent() {
    autoFit();
    try {
      chartRef.current?.scrollToRealTime();
    } catch {}
  }

  function addExtra(name: string) {
    if (!name || extras.includes(name)) return;
    setExtras((prev) => [...prev, name]);
  }
  function removeExtra(name: string) {
    setExtras((prev) => prev.filter((x) => x !== name));
  }

  function tgl(key: IndicatorKey, label: string, c: string) {
    const isOn = enabled.has(key);
    return (
      <button
        onClick={() =>
          setEnabled((prev) => {
            const n = new Set(prev);
            if (n.has(key)) n.delete(key);
            else n.add(key);
            return n;
          })
        }
        style={{ padding: "3px 8px", background: isOn ? "#1a2a44" : "#0a141e", border: `1px solid ${isOn ? c : "#1a2a44"}`, color: isOn ? c : "#556677", cursor: "pointer", fontSize: 10, fontWeight: 800, fontFamily: "var(--sans)", letterSpacing: ".3px", marginLeft: 2 }}
      >
        {label}
      </button>
    );
  }

  const submit = () => {
    const s = typed.trim().toUpperCase();
    if (s) {
      setHeld(null);
      api.linkSet(color, s).catch((e) => setErr(String(e)));
    }
    setTyped("");
  };
  const cycle = () => {
    const c = nextColor(color);
    setColor(c);
    if (standalone) api.setChartColor(id, c).catch(() => {});
  };
  const lastBar = dataRef.current[dataRef.current.length - 1];
  const sessionLabel = useMemo(() => (replayDate ? `replay ${replayDate}` : ""), [replayDate]);

  return (
    <div className="chart-wrap">
      <div className="chart-toolbar">
        <button className="link" style={{ background: LINK_HEX[color] }} title={`${color} link group: click to change`} onClick={cycle} />
        <button
          className={"chart-ind-btn " + (held ? "on" : "")}
          title={held ? `holding ${held}: click to follow ${color} again` : "hold this symbol (stop following new fires)"}
          onClick={() => setHeld(held ? null : symbol || null)}
        >
          {held ? "🔒" : "🔓"}
        </button>
        <input className="symbox" placeholder="symbol" value={typed} onChange={(e) => setTyped(e.target.value.toUpperCase())} onKeyDown={(e) => e.key === "Enter" && submit()} />
        <span className="chart-sym">{symbol || "—"}</span>
        {lastBar && dataSymbolRef.current === symbol && <span className="chart-price">{lastBar.close.toFixed(2)}</span>}
        <div className="chart-sep" />
        <select className="opts-sel" value={ivl} onChange={(e) => changeIvl(e.target.value as ChartInterval)}>
          <option value="1m">1 MIN</option>
          <option value="5m">5 MIN</option>
          <option value="15m">15 MIN</option>
          <option value="daily">DAILY</option>
          <option value="weekly">WEEKLY</option>
          <option value="monthly">MONTHLY</option>
        </select>
        <select className="opts-sel" value={bars} onChange={(e) => setBars(parseInt(e.target.value))}>
          {(isIntraday ? INTRADAY_BARS : SWING_BARS).map((n) => (
            <option key={n} value={n}>
              {n} BARS
            </option>
          ))}
        </select>
        <div className="chart-sep" />
        {isIntraday && tgl("vwap", "VWAP", "#ffee00")}
        {isIntraday && tgl("vwap_1sigma", "±1σ", "#2277ee")}
        {isIntraday && tgl("vwap_2sigma", "±2σ", "#aa33cc")}
        {isIntraday && tgl("poc", "POC", "#ffc328")}
        {isIntraday && tgl("va", "VA", "#d4922a")}
        {isIntraday && <div className="chart-sep" />}
        {!isIntraday && tgl("sma9", "SMA9", "#88dd00")}
        {!isIntraday && tgl("sma20", "SMA20", "#00aaff")}
        {!isIntraday && tgl("sma50", "SMA50", "#ff8800")}
        {!isIntraday && tgl("sma200", "SMA200", "#ff4488")}
        {!isIntraday && <div className="chart-sep" />}
        {tgl("ema9", "EMA9", "#cc88ff")}
        {tgl("ema20", "EMA20", "#88ccff")}
        {!isIntraday && tgl("ema50", "EMA50", "#ffcc88")}
        {!isIntraday && tgl("ema200", "EMA200", "#ff88cc")}
        <div className="chart-sep" />
        <select
          className="opts-sel"
          value=""
          onChange={(e) => {
            const v = e.target.value;
            if (v) addExtra(v);
          }}
        >
          <option value="">+ ADD</option>
          <optgroup label="Overlays">
            {KLINE_OVERLAYS.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </optgroup>
          <optgroup label="Lower panes">
            {KLINE_LOWER_PANE.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </optgroup>
        </select>
        {extras.map((n) => (
          <span key={n} className="chart-extra" onClick={() => removeExtra(n)} title="Click to remove">
            {n} ×
          </span>
        ))}
        <div className="chart-sep" />
        <button
          onClick={() => setShowVP((v) => !v)}
          title="Volume profile: volume-by-price over the session (POC brighter), a faint watermark behind price"
          style={{ padding: "3px 8px", background: showVP ? "#1a2a44" : "#0a141e", border: `1px solid ${showVP ? "#6f9bff" : "#1a2a44"}`, color: showVP ? "#9fc0ff" : "#556677", cursor: "pointer", fontSize: 10, fontWeight: 800, fontFamily: "var(--sans)", letterSpacing: ".3px" }}
        >
          VP
        </button>
        {sessionLabel && <span className="chart-note">{sessionLabel}</span>}
        <button className="chart-fit-btn" onClick={fitContent}>
          ⤢ Fit
        </button>
        <button className="chart-fit-btn" style={{ marginLeft: 4 }} onClick={loadData}>
          ↻
        </button>
      </div>
      <div className="chart-container" ref={containerRef}>
        {hover && <div className="chart-legend sig">{hover}</div>}
        {loading && <div className="chart-loading">Loading {symbol}...</div>}
        {!symbol && <div className="chart-loading">Click a symbol in a {color} panel (or type one)</div>}
        {noData && !loading && !err && (
          <div className="chart-msg">
            <span style={{ fontSize: 13, fontWeight: 700 }}>No data for {symbol}</span>
          </div>
        )}
        {err && (
          <div className="chart-msg bad">
            <span>⚠ {err}</span>
            <button className="chart-fit-btn" onClick={() => loadData()}>
              Retry
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
