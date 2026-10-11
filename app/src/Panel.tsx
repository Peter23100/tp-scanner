// One panel: a trigger feed (newest first, older rows dimmed) or a ranked list.
// Click a symbol: every chart of the panel's link colour shows it.
// Right-click a row: why does (or doesn't) this filter fire on this symbol right now.
import { useEffect, useMemo, useRef, useState } from "react";
import { api, FilterMeta, fmtNum, Item, LINK_HEX, nextColor, SignalDto } from "./api";
import { useStore } from "./store";

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

function loadFlag(key: string): boolean {
  try {
    return localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

export default function Panel({ panelKey, item, meta, color: initial }: { panelKey: string; item: Item; meta: Map<string, FilterMeta>; color: string }) {
  const [color, setColor] = useState(initial);
  const [explain, setExplain] = useState<{ filter: string; symbol: string; text: string } | null>(null);
  const soundKey = `sound:${panelKey}`;
  const [sound, setSound] = useState(() => loadFlag(soundKey));
  const filters = item.filters;
  const metas = filters.map((f) => meta.get(f)).filter((m): m is FilterMeta => !!m);
  const isList = metas.length > 0 && metas.every((m) => m.kind === "list");
  const sides = new Set(metas.map((m) => m.side));
  const tone = sides.size === 1 ? [...sides][0] : "neutral";
  const multi = filters.length > 1;
  const title = item.title ?? filters.join(" | ");
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
    if (top && lastId.current !== null && top.id > lastId.current && sound && mode === "live") beep(top.side);
    lastId.current = top ? top.id : lastId.current ?? 0;
  }, [rows, sound, mode]);
  const toggleSound = () => {
    setSound(!sound);
    try {
      localStorage.setItem(soundKey, sound ? "0" : "1");
    } catch {
      /* not persisted */
    }
  };

  const listRows = isList ? filters.flatMap((f) => (lists[f] ?? []).map((r) => ({ ...r, filter: f }))) : [];

  const pick = (symbol: string) => api.linkSet(color, symbol).catch(() => {});
  const why = (filter: string, symbol: string) => {
    api
      .explain(filter, symbol)
      .then((text) => setExplain({ filter, symbol, text }))
      .catch((e) => setExplain({ filter, symbol, text: String(e) }));
  };
  const cycle = () => {
    const c = nextColor(color);
    setColor(c);
    api.setPanelLink(panelKey, c).catch(() => {});
  };

  return (
    <div className={`panel tone-${tone}`}>
      <div className="phead">
        <span className="ptitle" title={filters.map((f) => describe(meta.get(f))).join("\n")}>
          {title}
          {isList && <span className="count"> #{listRows.length}</span>}
        </span>
        {!isList && (
          <button className={"bell " + (sound ? "on" : "")} title={sound ? "sound on new fires (live): on" : "sound on new fires (live): off"} onClick={toggleSound}>
            {sound ? "♪" : "·"}
          </button>
        )}
        <button className="link" style={{ background: LINK_HEX[color] }} title={`linked to ${color} charts${linked ? ` (${linked})` : ""}: click to change`} onClick={cycle} />
      </div>
      <div className="pbody">
        <table>
          <thead>
            <tr>
              {isList ? (
                <>
                  <th>Symbol</th>
                  {multi && <th>Filter</th>}
                  <th>Last</th>
                  {extra.map((c) => (
                    <th key={c}>{c}</th>
                  ))}
                  <th>Since</th>
                </>
              ) : (
                <>
                  <th>Time</th>
                  <th>Symbol</th>
                  {multi && <th>Filter</th>}
                  <th>Last</th>
                  {extra.map((c) => (
                    <th key={c}>{c}</th>
                  ))}
                </>
              )}
            </tr>
          </thead>
          <tbody>
            {isList
              ? listRows.map((r) => (
                  <tr key={r.filter + r.symbol} className={r.symbol === linked ? "sel" : ""} onClick={() => pick(r.symbol)} onContextMenu={(e) => (e.preventDefault(), why(r.filter, r.symbol))}>
                    <td className="sym">{r.symbol}</td>
                    {multi && <td>{r.filter}</td>}
                    <td>{fmtNum(r.last)}</td>
                    {extra.map((c) => (
                      <td key={c}>{fmtNum(r.columns.find((x) => x[0] === c)?.[1])}</td>
                    ))}
                    <td>{r.since}</td>
                  </tr>
                ))
              : rows.map((s) => {
                  const fresh = marketMs == null || marketMs - s.at_ms < FRESH_MS;
                  return (
                    <tr key={s.id} className={(fresh ? "" : "old ") + (s.symbol === linked ? "sel" : "")} onClick={() => pick(s.symbol)} onContextMenu={(e) => (e.preventDefault(), why(s.filter, s.symbol))}>
                      <td>{s.time}</td>
                      <td className="sym">{s.symbol}</td>
                      {multi && <td>{s.filter}</td>}
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
      {explain && (
        <div className="explain" onClick={() => setExplain(null)}>
          <div className="explain-box" onClick={(e) => e.stopPropagation()}>
            <div className="explain-head">
              {explain.filter} on {explain.symbol}, now
              <button onClick={() => why(explain.filter, explain.symbol)}>refresh</button>
              <button onClick={() => setExplain(null)}>close</button>
            </div>
            <pre>{explain.text}</pre>
          </div>
        </div>
      )}
    </div>
  );
}

function describe(m: FilterMeta | undefined): string {
  if (!m) return "";
  return `${m.name}: ${m.kind}, ${m.side}, ${m.hours}${m.kind === "trigger" ? `, cooldown ${m.cooldown_s}s` : ""}`;
}
