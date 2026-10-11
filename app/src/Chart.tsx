// A linked chart: candles + volume + session VWAP, with the scanner's own signals marked on it.
// It follows its link colour: any panel (or chart symbol box) of that colour sets its symbol.
// History comes from Massive REST; new minutes arrive from the engine as it reads them, so live
// and replay both extend the chart as the scan runs.
import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { createChart, IChartApi, ISeriesApi, SeriesMarker, Time, UTCTimestamp } from "lightweight-charts";
import { api, LINK_HEX, nextColor } from "./api";
import { useStore } from "./store";

type Bar = { t: number; o: number; h: number; l: number; c: number; v: number };
const TFS = [1, 5, 15];

// Lightweight-charts draws UTC; shift every time to New York wall clock so the axis reads ET.
const nyFmt = new Intl.DateTimeFormat("en-US", {
  timeZone: "America/New_York",
  hourCycle: "h23",
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});
const offsetCache = new Map<number, number>();
function nyOffsetMs(ms: number): number {
  const hour = Math.floor(ms / 3_600_000);
  const hit = offsetCache.get(hour);
  if (hit !== undefined) return hit;
  const p = Object.fromEntries(nyFmt.formatToParts(new Date(hour * 3_600_000)).map((x) => [x.type, x.value]));
  const wall = Date.UTC(+p.year, +p.month - 1, +p.day, +p.hour % 24, +p.minute, +p.second);
  const off = wall - hour * 3_600_000;
  offsetCache.set(hour, off);
  return off;
}
const ny = (ms: number) => ms + nyOffsetMs(ms);
const toTime = (nyMs: number) => Math.floor(nyMs / 1000) as UTCTimestamp;

function aggregate(bars: Bar[], tf: number): Bar[] {
  const w = tf * 60_000;
  const out: Bar[] = [];
  for (const b of bars) {
    const k = Math.floor(ny(b.t) / w) * w;
    const last = out[out.length - 1];
    if (last && last.t === k) {
      last.h = Math.max(last.h, b.h);
      last.l = Math.min(last.l, b.l);
      last.c = b.c;
      last.v += b.v;
    } else out.push({ t: k, o: b.o, h: b.h, l: b.l, c: b.c, v: b.v });
  }
  return out;
}

/// Regular-session VWAP per day, from 1-minute bars (times in NY wall ms).
function vwap(bars: Bar[]): { time: UTCTimestamp; value: number }[] {
  const out: { time: UTCTimestamp; value: number }[] = [];
  let day = -1,
    pv = 0,
    vol = 0;
  for (const b of bars) {
    const n = ny(b.t);
    const d = Math.floor(n / 86_400_000);
    const minute = (n % 86_400_000) / 60_000;
    if (minute < 570 || minute >= 960) continue;
    if (d !== day) {
      day = d;
      pv = 0;
      vol = 0;
    }
    pv += ((b.h + b.l + b.c) / 3) * b.v;
    vol += b.v;
    if (vol > 0) out.push({ time: toTime(n), value: pv / vol });
  }
  return out;
}

export default function ChartView({ id, initialColor, standalone }: { id: string; initialColor: string; standalone?: boolean }) {
  const [color, setColor] = useState(initialColor);
  const linked = useStore((s) => s.links[color]);
  const replayDate = useStore((s) => s.status?.replay?.date ?? null);
  const marketMs = useStore((s) => s.status?.market_ms ?? null);
  const signals = useStore((s) => s.signals);
  const [tf, setTf] = useState(5);
  const [typed, setTyped] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [bars, setBars] = useState<Bar[]>([]);
  const symbol = linked ?? "";

  const box = useRef<HTMLDivElement>(null);
  const chart = useRef<IChartApi | null>(null);
  const candles = useRef<ISeriesApi<"Candlestick"> | null>(null);
  const volume = useRef<ISeriesApi<"Histogram"> | null>(null);
  const vwapLine = useRef<ISeriesApi<"Line"> | null>(null);
  const marketRef = useRef(marketMs);
  marketRef.current = marketMs;

  useEffect(() => {
    if (!box.current) return;
    const c = createChart(box.current, {
      layout: { background: { color: "#131722" }, textColor: "#c8ccd4", fontSize: 11 },
      grid: { vertLines: { color: "#1e2230" }, horzLines: { color: "#1e2230" } },
      timeScale: { timeVisible: true, secondsVisible: false, borderColor: "#2a2e39" },
      rightPriceScale: { borderColor: "#2a2e39" },
      crosshair: { mode: 0 },
    });
    candles.current = c.addCandlestickSeries({ upColor: "#26a69a", downColor: "#ef5350", borderVisible: false, wickUpColor: "#26a69a", wickDownColor: "#ef5350" });
    candles.current.priceScale().applyOptions({ scaleMargins: { top: 0.05, bottom: 0.25 } });
    volume.current = c.addHistogramSeries({ priceFormat: { type: "volume" }, priceScaleId: "" });
    volume.current.priceScale().applyOptions({ scaleMargins: { top: 0.8, bottom: 0 } });
    vwapLine.current = c.addLineSeries({ color: "#f5c542", lineWidth: 1, priceLineVisible: false, lastValueVisible: false });
    chart.current = c;
    const ro = new ResizeObserver(() => {
      if (box.current) c.resize(box.current.clientWidth, box.current.clientHeight);
    });
    ro.observe(box.current);
    return () => {
      ro.disconnect();
      c.remove();
      chart.current = null;
    };
  }, []);

  // A new symbol: tell the engine to forward its bars, then load history.
  useEffect(() => {
    api.chartWatch(id, symbol || null).catch(() => {});
    setBars([]);
    setErr(null);
    if (!symbol) return;
    let alive = true;
    api
      .chartHistory(symbol, replayDate, 5)
      .then((rows) => {
        if (!alive) return;
        const cut = replayDate && marketRef.current != null ? marketRef.current : Infinity;
        const hist = rows.filter((r) => r[0] + 60_000 <= cut).map(([t, o, h, l, c, v]) => ({ t, o, h, l, c, v }));
        setBars((prev) => merge(hist, prev));
      })
      .catch((e) => alive && setErr(String(e)));
    return () => {
      alive = false;
    };
  }, [symbol, replayDate, id]);

  useEffect(() => () => void api.chartWatch(id, null).catch(() => {}), [id]);

  // Minutes as the engine reads them.
  useEffect(() => {
    const un = listen<Bar & { symbol: string }>("chart-bar", (e) => {
      if (e.payload.symbol !== symbol) return;
      const { t, o, h, l, c, v } = e.payload;
      setBars((prev) => merge(prev, [{ t, o, h, l, c, v }]));
    });
    return () => {
      un.then((f) => f());
    };
  }, [symbol]);

  const agg = useMemo(() => aggregate(bars, tf), [bars, tf]);

  useEffect(() => {
    if (!candles.current || !volume.current || !vwapLine.current) return;
    candles.current.setData(agg.map((b) => ({ time: toTime(b.t), open: b.o, high: b.h, low: b.l, close: b.c })));
    volume.current.setData(agg.map((b) => ({ time: toTime(b.t), value: b.v, color: b.c >= b.o ? "rgba(38,166,154,0.5)" : "rgba(239,83,80,0.5)" })));
    vwapLine.current.setData(vwap(bars));
  }, [agg, bars]);

  // The scanner's own signals on this symbol, at the bar that fired them.
  useEffect(() => {
    if (!candles.current) return;
    const w = tf * 60_000;
    const first = agg.length ? agg[0].t : Infinity;
    // One arrow per bar per side; the filter name only when it's the bar's only fire and the
    // chart isn't crowded (a filter firing every bar would bury the candles in labels).
    const groups = new Map<string, { k: number; side: string; filters: Set<string> }>();
    for (const s of signals) {
      if (s.symbol !== symbol) continue;
      const k = Math.floor(ny(s.at_ms - 60_000) / w) * w;
      if (k < first) continue;
      const side = s.side === "short" ? "short" : "long";
      const g = groups.get(k + side) ?? { k, side, filters: new Set<string>() };
      g.filters.add(s.filter);
      groups.set(k + side, g);
    }
    const crowded = groups.size > 25;
    const markers: SeriesMarker<Time>[] = [...groups.values()]
      .sort((a, b) => a.k - b.k)
      .map((g) => ({
        time: toTime(g.k),
        position: g.side === "short" ? "aboveBar" : "belowBar",
        color: g.side === "short" ? "#ff5c5c" : "#3ddc6a",
        shape: g.side === "short" ? "arrowDown" : "arrowUp",
        text: crowded ? "" : g.filters.size === 1 ? [...g.filters][0] : `${g.filters.size} filters`,
      }));
    candles.current.setMarkers(markers);
  }, [signals, symbol, agg, tf]);

  const submit = () => {
    const s = typed.trim().toUpperCase();
    if (s) api.linkSet(color, s).catch((e) => setErr(String(e)));
    setTyped("");
  };
  const cycle = () => {
    const c = nextColor(color);
    setColor(c);
    if (standalone) api.setChartColor(id, c).catch(() => {});
  };

  return (
    <div className="chart">
      <div className="chead">
        <button className="link" style={{ background: LINK_HEX[color] }} title={`${color} link group: click to change`} onClick={cycle} />
        <input
          className="symbox"
          placeholder={symbol || "symbol"}
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && submit()}
        />
        <span className="csym">{symbol}</span>
        {TFS.map((m) => (
          <button key={m} className={m === tf ? "on" : ""} onClick={() => setTf(m)}>
            {m}m
          </button>
        ))}
        {agg.length > 0 && <span className="clast">{agg[agg.length - 1].c.toFixed(2)}</span>}
        {err && <span className="bad cerr">{err}</span>}
      </div>
      <div className="cbody" ref={box} />
    </div>
  );
}

function merge(a: Bar[], b: Bar[]): Bar[] {
  if (!b.length) return a;
  const m = new Map<number, Bar>();
  for (const x of a) m.set(x.t, x);
  for (const x of b) m.set(x.t, x);
  return [...m.values()].sort((x, y) => x.t - y.t);
}
