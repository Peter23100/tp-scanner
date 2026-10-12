// The launcher bar, styled as TradierPro's: a menubar (screens, panels, charts, filters) over a
// status bar (data source and the status that must never be missed: feed state, delay, errors).
// Lists open by growing the window downward: the bar is too thin for a dropdown to paint in.
import { useEffect, useMemo, useState } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { api, LINK_COLORS, LINK_HEX } from "./api";
import { useStore } from "./store";
import { panelTitle } from "./Panel";
import { dragBar, WinButtons } from "./win";

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

const BAR_H = 46;
type Open = null | "panels" | "baskets" | "layouts" | "details";

export default function Launcher() {
  const status = useStore((s) => s.status);
  const panels = useStore((s) => s.panels);
  const filters = useStore((s) => s.filters);
  const filtersError = useStore((s) => s.filtersError);
  const baskets = useStore((s) => s.baskets);
  const [date, setDate] = useState(lastWeekday());
  const [startAt, setStartAt] = useState("09:00");
  const [speed, setSpeed] = useState(60);
  // Per-second replay (the live scan's granularity). The first replay of a day downloads its
  // per-second bars (a few minutes); after that it's on disk.
  const [perSecond, setPerSecond] = useState(() => {
    try {
      return localStorage.getItem("replay-seconds") === "1";
    } catch {
      return false;
    }
  });
  const [chartColor, setChartColor] = useState("red");
  const [msg, setMsg] = useState<{ text: string; bad: boolean } | null>(null);
  const [open, setOpen] = useState<Open>(null);
  const [confirmReset, setConfirmReset] = useState(false);
  const [layouts, setLayouts] = useState<{ name: string; saved_at: string; windows: number }[]>([]);
  const [layoutName, setLayoutName] = useState("");
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const refreshLayouts = () => api.layoutsList().then(setLayouts).catch(() => {});
  useEffect(() => {
    if (open === "layouts") refreshLayouts();
  }, [open]);

  useEffect(() => {
    getCurrentWindow()
      .setSize(new LogicalSize(window.innerWidth, open ? 380 : BAR_H + (msg ? 18 : 0)))
      .catch(() => {});
  }, [open, msg]);

  const run = (p: Promise<unknown>, ok?: string) =>
    p.then((r) => setMsg(typeof r === "string" && r ? { text: r, bad: false } : ok ? { text: ok, bad: false } : null)).catch((e) => setMsg({ text: String(e), bad: true }));

  const groups = useMemo(() => {
    const out: string[] = [];
    for (const g of [...(panels?.panels ?? []).map((p) => p.group), ...(panels?.charts ?? []).map((c) => c.group)]) if (g && !out.includes(g)) out.push(g);
    return out;
  }, [panels]);
  const notShown = useMemo(() => {
    const shown = new Set((panels?.panels ?? []).flatMap((p) => p.filters));
    return filters.map((f) => f.name).filter((n) => !shown.has(n));
  }, [panels, filters]);

  const errors = status?.errors ?? 0;
  const warnings = (status?.warnings ?? []).length + (notShown.length ? 1 : 0) + (filtersError ? 1 : 0);
  const mode = status?.mode ?? "idle";
  const toggle = (o: Open) => setOpen(open === o ? null : o);

  return (
    <div className="launcher">
      <div className="menubar" {...dragBar}>
        <span className="mlogo" data-drag>
          TP<span>SCANNER</span>
        </span>
        {groups.map((g) => (
          <span key={g} className="mitem" title={`open the ${g} screen's panels`} onClick={() => run(api.openScreen(g))}>
            {g.toUpperCase()}
          </span>
        ))}
        <span className="msep" />
        <span className={"mitem " + (open === "panels" ? "on" : "")} onClick={() => toggle("panels")}>
          Panels ▾
        </span>
        <span className={"mitem " + (open === "layouts" ? "on" : "")} title="save the whole desk under a name, and switch between saved desks" onClick={() => toggle("layouts")}>
          Layouts ▾
        </span>
        <span className={"mitem " + (open === "baskets" ? "on" : "")} title="the stocks each filter may draw from" onClick={() => toggle("baskets")}>
          Baskets ▾
        </span>
        <span className="mitem" title="a new empty panel" onClick={() => run(api.panelCreate())}>
          + Panel
        </span>
        <span className="mitem" onClick={() => run(api.openChart(chartColor))}>
          + Chart
        </span>
        <button
          className="swatch"
          title="link colour of the next chart"
          style={{ background: LINK_HEX[chartColor] }}
          onClick={() => setChartColor(LINK_COLORS[(LINK_COLORS.indexOf(chartColor as never) + 1) % LINK_COLORS.length])}
        />
        <span className="msep" />
        <span className="mitem" title="re-read filters/*.scan" onClick={() => run(api.reloadFilters())}>
          Reload Filters
        </span>
        <span className="mright">
          <span className={"mbadge " + (mode === "live" ? "badge-live" : mode === "replay" ? "badge-paper" : "badge-idle")}>{mode.toUpperCase()}</span>
          <span className={"mbadge " + (errors > 0 ? "badge-err" : "badge-idle")} style={{ cursor: "pointer" }} title="errors and warnings" onClick={() => toggle("details")}>
            {errors} ERR{warnings > 0 && ` · ${warnings} ⚠`}
          </span>
          <WinButtons closeTitle="quit TP Scanner (windows are remembered)" />
        </span>
      </div>
      <div className="statusbar">
        <button className={"sb-btn " + (mode === "live" ? "on" : "")} onClick={() => run(api.startLive(false))} title="Massive WebSocket, delayed feed">
          LIVE
        </button>
        <span className="sb-sep" />
        <input className="sb-in" type="date" value={date} onChange={(e) => setDate(e.target.value)} title="replay day" />
        <input className="sb-in hhmm" value={startAt} onChange={(e) => setStartAt(e.target.value)} title="run fast up to this time (HH:MM ET)" />
        <select
          className="sb-in"
          value={speed}
          onChange={(e) => {
            const v = Number(e.target.value);
            setSpeed(v);
            if (mode === "replay") run(api.setSpeed(v));
          }}
        >
          {SPEEDS.map(([l, v]) => (
            <option key={l} value={v}>
              {l}
            </option>
          ))}
        </select>
        <label className="sb-chk" title="replay the day's per-second bars, as the live scan sees them (the first time, the day is downloaded: a few minutes)">
          <input
            type="checkbox"
            checked={perSecond}
            onChange={(e) => {
              setPerSecond(e.target.checked);
              try {
                localStorage.setItem("replay-seconds", e.target.checked ? "1" : "0");
              } catch {
                /* not remembered */
              }
            }}
          />
          1s
        </label>
        <button className={"sb-btn " + (mode === "replay" ? "on" : "")} onClick={() => run(api.startReplay(date, speed, startAt || null, perSecond))}>
          REPLAY
        </button>
        <button className="sb-btn" onClick={() => run(api.stopSource())}>
          STOP
        </button>
        <span className="sb-sep" />
        <span className={"sbi " + (status?.ok ? "ok" : mode === "idle" ? "" : "warn")}>{status ? status.feed : "starting"}</span>
        {status?.market_time && (
          <span className="sbi">
            <span className="sbv">{status.market_time}</span> ET
          </span>
        )}
        {status?.delay_s != null && <span className="sbi">{Math.round(status.delay_s / 60)}m behind</span>}
        {status && (
          <span className="sbi">
            <span className="sbv">{status.bars.toLocaleString()}</span> bars · <span className="sbv">{status.signals.toLocaleString()}</span> signals
          </span>
        )}
      </div>
      {msg && (
        <div className={"lmsg " + (msg.bad ? "bad" : "")} onClick={() => setMsg(null)}>
          {msg.text}
        </div>
      )}
      {open === "panels" && (
        <div className="ldrop">
          <div className="ldrop-head">
            Panels ({panels?.panels.length ?? 0})
            <span style={{ marginLeft: "auto" }} />
            <button className="lbtn" title="close the gaps between the open windows, as you have them arranged" onClick={() => run(api.tidyWindows())}>
              Tidy
            </button>
            <button className="lbtn" title="put every open panel and chart back on its Primus grid position" onClick={() => run(api.arrangeWindows())}>
              Arrange as Primus
            </button>
            {confirmReset ? (
              <>
                <span className="bad">Close every panel and go back to the Primus screens?</span>
                <button className="lbtn" onClick={() => (setConfirmReset(false), run(api.resetLayout(), "layout reset to the Primus screens"))}>
                  Reset
                </button>
                <button className="lbtn" onClick={() => setConfirmReset(false)}>
                  Cancel
                </button>
              </>
            ) : (
              <button className="lbtn" title="back to the Primus screens from display.json" onClick={() => setConfirmReset(true)}>
                Reset layout…
              </button>
            )}
          </div>
          {groups.concat("").map((g) => {
            const ps = (panels?.panels ?? []).filter((p) => p.group === g);
            if (!ps.length) return null;
            return (
              <div key={g || "own"} className="lgroup">
                <div className="lgroup-name">{g ? g.toUpperCase() : "YOUR PANELS"}</div>
                {ps.map((p) => (
                  <div key={p.id} className="lrow">
                    <span className="ldot" style={{ background: LINK_HEX[p.link] }} />
                    <span className="lname" onClick={() => run(api.openPanel(p.id))} title="open">
                      {panelTitle(p)}
                    </span>
                    <span className="lsub">{p.filters.length} filters</span>
                    <button className="lx" title="delete this panel" onClick={() => run(api.panelDelete(p.id))}>
                      ×
                    </button>
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      )}
      {open === "layouts" && (
        <div className="ldrop">
          <div className="ldrop-head">
            Layouts ({layouts.length})
            <span style={{ marginLeft: "auto" }} />
            <input
              className="sb-in"
              style={{ width: 160 }}
              placeholder="name for the desk as it is now"
              value={layoutName}
              onChange={(e) => setLayoutName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && layoutName.trim() && run(api.layoutSave(layoutName).then((m) => (setLayoutName(""), refreshLayouts(), m)))}
            />
            <button className="lbtn" disabled={!layoutName.trim()} onClick={() => run(api.layoutSave(layoutName).then((m) => (setLayoutName(""), refreshLayouts(), m)))}>
              Save current
            </button>
          </div>
          {layouts.length === 0 && <p>No saved layouts. Arrange the windows the way you like, name it above and Save: the open windows, where each sits, and every panel's filters, colour and text size.</p>}
          {layouts.map((l) => (
            <div key={l.name} className="lrow">
              <span className="lname" title="switch to this layout" onClick={() => run(api.layoutLoad(l.name))}>
                {l.name}
              </span>
              <span className="lsub">
                {l.windows} windows · saved {l.saved_at}
              </span>
              <span style={{ marginLeft: "auto" }} />
              <button className="lbtn" title="replace it with the desk as it is now" onClick={() => run(api.layoutSave(l.name).then((m) => (refreshLayouts(), m)))}>
                Overwrite
              </button>
              {confirmDelete === l.name ? (
                <>
                  <button className="lbtn" onClick={() => (setConfirmDelete(null), run(api.layoutDelete(l.name).then((m) => (refreshLayouts(), m))))}>
                    Delete it
                  </button>
                  <button className="lbtn" onClick={() => setConfirmDelete(null)}>
                    Keep
                  </button>
                </>
              ) : (
                <button className="lx" title="delete this layout" onClick={() => setConfirmDelete(l.name)}>
                  ×
                </button>
              )}
            </div>
          ))}
        </div>
      )}
      {open === "baskets" && (
        <div className="ldrop">
          <div className="ldrop-head">
            Baskets ({baskets.length})
            <span style={{ marginLeft: "auto" }} />
            <button className="lbtn" onClick={() => run(api.openDialog("basket", "new", null))}>
              New basket…
            </button>
          </div>
          {baskets.length === 0 && <p>No baskets yet. A basket says which stocks a filter may draw from (type, exchange, price, float, average volume); a filter picks one in its Adjust window.</p>}
          {baskets.map((b) => (
            <div key={b.name} className="lrow">
              <span className="lname" onClick={() => run(api.openDialog("basket", b.name, null))} title="edit">
                {b.name}
              </span>
              <span className="lsub">
                {[b.types.join("/"), b.exchanges.join("/"), ...b.ranges.map(([k, lo, hi]) => `${k} ${lo ?? ""}–${hi ?? ""}`)].filter(Boolean).join(" · ")}
              </span>
              <span className="lsub" style={{ marginLeft: "auto" }}>
                {b.used_by.length ? `${b.used_by.length} filter${b.used_by.length > 1 ? "s" : ""}` : "unused"}
              </span>
            </div>
          ))}
        </div>
      )}
      {open === "details" && (
        <div className="ldrop">
          {filtersError && <p className="bad">Filters: {filtersError}</p>}
          {(status?.warnings ?? []).map((w) => (
            <p key={w}>⚠ {w}</p>
          ))}
          {notShown.length > 0 && <p>⚠ on no panel: {notShown.join(", ")}</p>}
          {status?.since && <p>History since {status.since} ET (a reload or source change starts it over).</p>}
          {status && status.reconnects > 0 && <p>Reconnects: {status.reconnects}</p>}
          {status && status.bad_messages > 0 && <p>Bad feed messages: {status.bad_messages}</p>}
          {(status?.error_samples ?? []).map((e, i) => (
            <p key={i} className="bad">
              {e}
            </p>
          ))}
          {!filtersError && !(status?.warnings ?? []).length && !notShown.length && !errors && <p>Nothing to report.</p>}
        </div>
      )}
    </div>
  );
}
