// One panel window: a trigger feed (newest first, older rows dimmed) or a ranked list.
// Click a symbol: every chart of the panel's link colour shows it.
// Right-click anywhere: the panel menu (link colour, follow, sound, filters, why isn't X firing,
// adjust a filter). On a row, the menu starts from that row's symbol and filter.
import { useEffect, useMemo, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, FilterMeta, fmtNum, FONT_DEFAULT, FONT_MAX, FONT_MIN, LINK_HEX, nextColor, PanelDef, SignalDto } from "./api";
import { dragBar, WinButtons } from "./win";
import { useStore } from "./store";
import { openPanelMenu } from "./Menu";

const MAX_ROWS = 200;
/// Rows newer than this (market time) are bright; older are dimmed, as in Primus.
const FRESH_MS = 5 * 60_000;

// A short tone for a new fire: higher for long, lower for short.
let audio: AudioContext | null = null;
function beep(side: string) {
  try {
    audio ??= new AudioContext();
    const o = audio.createOscillator();
    const g = audio.createGain();
    o.frequency.value = side === "short" ? 440 : 880;
    g.gain.setValueAtTime(0.15, audio.currentTime);
    g.gain.exponentialRampToValueAtTime(0.001, audio.currentTime + 0.25);
    o.connect(g).connect(audio.destination);
    o.start();
    o.stop(audio.currentTime + 0.25);
  } catch {
    /* no audio device: silent */
  }
}

export function panelTitle(p: PanelDef): string {
  return p.title || p.filters.join(" | ") || "Empty panel";
}

export function PanelWindow({ id }: { id: string }) {
  const panels = useStore((s) => s.panels);
  const filters = useStore((s) => s.filters);
  const meta = useMemo(() => new Map(filters.map((f) => [f.name, f])), [filters]);
  const def = panels?.panels.find((p) => p.id === id);
  useEffect(() => {
    // The OS title bar (and taskbar) doesn't follow document.title in Tauri: set both.
    if (!def) return;
    document.title = panelTitle(def);
    getCurrentWindow().setTitle(panelTitle(def)).catch(() => {});
  }, [def]);
  if (!panels) return <div className="dk-empty">loading…</div>;
  if (!def) return <div className="dk-empty bad">This panel was deleted. Close the window.</div>;
  return <Panel def={def} meta={meta} />;
}

export default function Panel({ def, meta }: { def: PanelDef; meta: Map<string, FilterMeta> }) {
  const filters = def.filters;
  const color = def.link;
  const metas = filters.map((f) => meta.get(f)).filter((m): m is FilterMeta => !!m);
  const missing = meta.size > 0 ? filters.filter((f) => !meta.has(f)) : [];
  const isList = metas.length > 0 && metas.every((m) => m.kind === "list");
  const sides = new Set(metas.map((m) => m.side));
  const tone = sides.size === 1 ? [...sides][0] : "neutral";
  const multi = filters.length > 1;
  const extra = useMemo(() => [...new Set(metas.flatMap((m) => m.columns))], [metas]);

  const signals = useStore((s) => s.signals);
  const lists = useStore((s) => s.lists);
  const marketMs = useStore((s) => s.status?.market_ms ?? null);
  const linked = useStore((s) => s.links[color]);

  const rows: SignalDto[] = useMemo(() => {
    if (isList) return [];
    const want = new Set(filters);
    const out: SignalDto[] = [];
    for (let i = signals.length - 1; i >= 0 && out.length < MAX_ROWS; i--) if (want.has(signals[i].filter)) out.push(signals[i]);
    return out;
  }, [signals, filters, isList]);

  // Sound on new fires (live only: a replay at speed would be a siren).
  const mode = useStore((s) => s.status?.mode);
  const lastId = useRef<number | null>(null);
  useEffect(() => {
    const top = rows[0];
    if (top && lastId.current !== null && top.id > lastId.current && def.sound && mode === "live") beep(top.side);
    lastId.current = top ? top.id : lastId.current ?? 0;
  }, [rows, def.sound, mode]);

  const update = (patch: Partial<PanelDef>) => api.panelUpdate({ ...def, ...patch }).catch(() => {});
  const font = def.font ?? FONT_DEFAULT;
  // Ctrl + mouse wheel sizes the panel's text (saved with the panel).
  const onWheel = (e: React.WheelEvent) => {
    if (!e.ctrlKey) return;
    e.preventDefault();
    const next = Math.min(FONT_MAX, Math.max(FONT_MIN, font + (e.deltaY < 0 ? 1 : -1)));
    if (next !== font) update({ font: next });
  };
  const listRows = isList ? filters.flatMap((f) => (lists[f] ?? []).map((r) => ({ ...r, filter: f }))) : [];

  const pick = (symbol: string) => api.linkSet(color, symbol).catch(() => {});
  const openMenu = (e: React.MouseEvent, symbol?: string, filter?: string) => {
    e.preventDefault();
    e.stopPropagation();
    openPanelMenu(def.id, symbol, filter);
  };
  const symClass = (side: string) => "sym " + (side === "long" ? "up" : side === "short" ? "dn" : "");

  return (
    <div className={`panel tone-${tone}`} style={{ "--pfs": `${font}px` } as React.CSSProperties} onContextMenu={(e) => openMenu(e)} onWheel={onWheel}>
      <div className="phead" {...dragBar} title="drag to move · double-click to maximize · right-click for the menu">
        <span className="ptitle" data-drag title={filters.map((f) => describe(meta.get(f))).join("\n")}>
          {panelTitle(def)}
        </span>
        <span className="pcount">{isList ? listRows.length : rows.length}</span>
        {!isList && (
          <button className={"bell " + (def.sound ? "on" : "")} title={def.sound ? "sound on new fires (live): on" : "sound on new fires (live): off"} onClick={() => update({ sound: !def.sound })}>
            {def.sound ? "♪" : "·"}
          </button>
        )}
        <button className="link" style={{ background: LINK_HEX[color] }} title={`linked to ${color} charts${linked ? ` (${linked})` : ""}: click to change`} onClick={() => update({ link: nextColor(color) })} />
        <WinButtons closeTitle="close this panel (it stays in Panels ▾)" />
      </div>
      {missing.length > 0 && <div className="pwarn">not loaded: {missing.join(", ")}</div>}
      {filters.length === 0 && <div className="dk-empty">No filters in this panel yet. Right-click → Filters in this panel…</div>}
      <div className="pbody">
        <table>
          <thead>
            <tr>
              {!isList && <th>Time</th>}
              <th>Symbol</th>
              {multi && <th>Filter</th>}
              <th>Last</th>
              {extra.map((c) => (
                <th key={c}>{c}</th>
              ))}
              {isList && <th>Since</th>}
            </tr>
          </thead>
          <tbody>
            {isList
              ? listRows.map((r) => (
                  <tr key={r.filter + r.symbol} className={r.symbol === linked ? "sel" : ""} onClick={() => pick(r.symbol)} onContextMenu={(e) => openMenu(e, r.symbol, r.filter)}>
                    <td className={symClass(meta.get(r.filter)?.side ?? "")}>{r.symbol}</td>
                    {multi && <td className="fname">{r.filter}</td>}
                    <td>{fmtNum(r.last)}</td>
                    {extra.map((c) => (
                      <td key={c}>{fmtNum(r.columns.find((x) => x[0] === c)?.[1])}</td>
                    ))}
                    <td className="tm">{r.since}</td>
                  </tr>
                ))
              : rows.map((s) => {
                  const fresh = marketMs == null || marketMs - s.at_ms < FRESH_MS;
                  return (
                    <tr key={s.id} className={(fresh ? "" : "old ") + (s.symbol === linked ? "sel" : "")} onClick={() => pick(s.symbol)} onContextMenu={(e) => openMenu(e, s.symbol, s.filter)}>
                      <td className="tm">{s.time}</td>
                      <td className={symClass(s.side)}>{s.symbol}</td>
                      {multi && <td className="fname">{s.filter}</td>}
                      <td>{fmtNum(s.last)}</td>
                      {extra.map((c) => (
                        <td key={c}>{fmtNum(s.columns.find((x) => x[0] === c)?.[1])}</td>
                      ))}
                    </tr>
                  );
                })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function describe(m: FilterMeta | undefined): string {
  if (!m) return "";
  return `${m.name}: ${m.kind}, ${m.side}, ${m.hours}${m.kind === "trigger" ? `, cooldown ${m.cooldown_s}s` : ""}`;
}
