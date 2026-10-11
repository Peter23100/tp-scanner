// The launcher bar: open screens and charts, pick the data source, reload filters, and the status
// that must never be missed (feed state, delay, errors, what can't fire and why).
import { useEffect, useState } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { api, LayoutReply, LINK_COLORS, LINK_HEX } from "./api";
import { useStore } from "./store";

function lastWeekday(): string {
  const d = new Date();
  do d.setDate(d.getDate() - 1);
  while (d.getDay() === 0 || d.getDay() === 6);
  return d.toISOString().slice(0, 10);
}

const SPEEDS: [string, number][] = [
  ["1x", 1],
  ["10x", 10],
  ["30x", 30],
  ["60x", 60],
  ["300x", 300],
  ["max", 0],
];

export default function Launcher() {
  const status = useStore((s) => s.status);
  const layoutVersion = useStore((s) => s.layoutVersion);
  const [layout, setLayout] = useState<LayoutReply | null>(null);
  const [date, setDate] = useState(lastWeekday());
  const [startAt, setStartAt] = useState("09:00");
  const [speed, setSpeed] = useState(60);
  const [chartColor, setChartColor] = useState("red");
  const [msg, setMsg] = useState<{ text: string; bad: boolean } | null>(null);
  const [details, setDetails] = useState(false);

  useEffect(() => {
    api.getLayout().then(setLayout);
  }, [layoutVersion]);

  useEffect(() => {
    getCurrentWindow()
      .setSize(new LogicalSize(window.innerWidth, details ? 360 : 64))
      .catch(() => {});
  }, [details]);

  const run = (p: Promise<unknown>, ok?: string) =>
    p.then((r) => setMsg({ text: typeof r === "string" ? r : ok ?? "", bad: false })).catch((e) => setMsg({ text: String(e), bad: true }));

  const errors = status?.errors ?? 0;
  const warnings = (status?.warnings ?? []).length + (layout?.layout?.not_displayed.length ? 1 : 0);
  const screens = layout?.layout?.display.screens ?? [];

  return (
    <div className="launcher">
      <div className="bar">
        <span className="brand">TP&nbsp;Scanner</span>
        <span className="group">
          {screens.map((s) => (
            <button key={s.name} title={s.title} onClick={() => run(api.openScreen(s.name))}>
              {s.name}
            </button>
          ))}
        </span>
        <span className="group">
          <button onClick={() => run(api.openChart(chartColor))}>+ chart</button>
          <button
            className="swatch"
            title="link colour of the new chart"
            style={{ background: LINK_HEX[chartColor] }}
            onClick={() => setChartColor(LINK_COLORS[(LINK_COLORS.indexOf(chartColor as never) + 1) % LINK_COLORS.length])}
          />
        </span>
        <span className="group">
          <button className={status?.mode === "live" ? "on" : ""} onClick={() => run(api.startLive(false))} title="Massive WebSocket, delayed feed">
            Live
          </button>
          <span className="sep">|</span>
          <input type="date" value={date} onChange={(e) => setDate(e.target.value)} title="replay day" />
          <input className="hhmm" value={startAt} onChange={(e) => setStartAt(e.target.value)} title="run fast up to this time (HH:MM ET)" />
          <select
            value={speed}
            onChange={(e) => {
              const v = Number(e.target.value);
              setSpeed(v);
              if (status?.mode === "replay") run(api.setSpeed(v));
            }}
          >
            {SPEEDS.map(([l, v]) => (
              <option key={l} value={v}>
                {l}
              </option>
            ))}
          </select>
          <button className={status?.mode === "replay" ? "on" : ""} onClick={() => run(api.startReplay(date, speed, startAt || null))}>
            Replay
          </button>
          <button onClick={() => run(api.stopSource())}>Stop</button>
        </span>
        <span className="group">
          <button onClick={() => run(api.reloadFilters())} title="re-read filters/*.scan and display.json">
            Reload filters
          </button>
        </span>
        <span className={"status " + (status?.ok ? "ok" : status?.mode === "idle" ? "" : "warn")}>
          {status ? status.feed : "starting"}
          {status?.market_time && <> · {status.market_time} ET</>}
          {status?.delay_s != null && <> · {Math.round(status.delay_s / 60)}m behind</>}
          {status && <> · {status.bars.toLocaleString()} bars · {status.signals} signals</>}
        </span>
        <button className={"badge " + (errors > 0 ? "bad" : "")} onClick={() => setDetails(!details)} title="errors and warnings">
          {errors > 0 ? `${errors} errors` : "0 errors"}
          {warnings > 0 && ` · ${warnings} ⚠`}
        </button>
      </div>
      {(msg || layout?.error) && (
        <div className={"msg " + (msg?.bad || layout?.error ? "bad" : "")} onClick={() => setMsg(null)}>
          {layout?.error ?? msg?.text}
        </div>
      )}
      {details && (
        <div className="details">
          {layout?.error && <p className="bad">Layout: {layout.error}</p>}
          {(status?.warnings ?? []).map((w) => (
            <p key={w}>⚠ {w}</p>
          ))}
          {layout?.layout?.not_displayed.length ? <p>⚠ not on any screen: {layout.layout.not_displayed.join(", ")}</p> : null}
          {status?.since && <p>History since {status.since} ET (a reload or source change starts it over).</p>}
          {status && status.reconnects > 0 && <p>Reconnects: {status.reconnects}</p>}
          {status && status.bad_messages > 0 && <p>Bad feed messages: {status.bad_messages}</p>}
          {(status?.error_samples ?? []).map((e, i) => (
            <p key={i} className="bad">
              {e}
            </p>
          ))}
        </div>
      )}
    </div>
  );
}
