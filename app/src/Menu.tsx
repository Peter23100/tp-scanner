// The panel's right-click menu, in its own small window at the mouse pointer (a panel can be too
// small to hold it), and the panel settings window it opens for rename and the filter list. Tools
// that need room (why, adjust) open their own windows too.
import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, FONT_DEFAULT, FONT_MAX, FONT_MIN, LINK_COLORS, LINK_HEX, PanelDef } from "./api";
import { panelTitle } from "./Panel";
import { useStore } from "./store";
import { dragBar, WinButtons } from "./win";

/// Open the menu for a panel, at the mouse pointer. `symbol`/`filter`: the row right-clicked.
export function openPanelMenu(panel: string, symbol?: string, filter?: string) {
  invoke("open_menu", { panel, symbol: symbol ?? null, filter: filter ?? null }).catch(() => {});
}

/// The menu window: `#/menu/<panel>/<symbol | ->/<filter | ->`.
export function MenuWindow({ panelId, symbol, filter }: { panelId: string; symbol: string | null; filter: string | null }) {
  const panels = useStore((s) => s.panels);
  const def = panels?.panels.find((p) => p.id === panelId);
  const box = useRef<HTMLDivElement>(null);
  const [sym, setSym] = useState(symbol ?? "");
  const [shown, setShown] = useState(false);
  const close = () => getCurrentWindow().close().catch(() => {});

  // Size the window to the menu and show it; close it when it loses focus or on Escape.
  useEffect(() => {
    if (!def || shown) return;
    const r = box.current?.getBoundingClientRect();
    if (!r) return;
    setShown(true);
    invoke("popup_fit", { width: r.width + 2, height: r.height + 2 }).catch(() => {});
  }, [def, shown]);
  useEffect(() => {
    const un = getCurrentWindow().onFocusChanged(({ payload }) => {
      if (!payload) close();
    });
    const k = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", k);
    return () => {
      un.then((f) => f());
      window.removeEventListener("keydown", k);
    };
  }, []);

  if (!def) return null;
  const whyFilters = filter ? [filter] : def.filters;
  const update = (patch: Partial<PanelDef>) => api.panelUpdate({ ...def, ...patch }).catch(() => {});
  const font = def.font ?? FONT_DEFAULT;
  const why = (f: string) => {
    if (!sym.trim()) return;
    api.openDialog("why", f, sym.trim().toUpperCase()).finally(close);
  };

  return (
    <div className="menu page" ref={box}>
      <div className="menu-title">{panelTitle(def)}</div>
      <div className="menu-row">
        <span className="menu-lbl">Link</span>
        {LINK_COLORS.map((c) => (
          <button key={c} className={"swatch " + (c === def.link ? "cur" : "")} title={c} style={{ background: LINK_HEX[c] }} onClick={() => update({ link: c })} />
        ))}
      </div>
      <div className="menu-item" onClick={() => update({ follow: !def.follow })} title="each new fire sends its symbol to this panel's charts">
        <span className="chk">{def.follow ? "✓" : ""}</span>Charts follow new fires
      </div>
      <div className="menu-item" onClick={() => update({ sound: !def.sound })}>
        <span className="chk">{def.sound ? "✓" : ""}</span>Sound on new fires (live)
      </div>
      <div className="menu-row" title="also Ctrl + mouse wheel over the panel">
        <span className="menu-lbl">Text size</span>
        <button className="dk-btn" onClick={() => update({ font: Math.max(FONT_MIN, font - 1) })}>
          A−
        </button>
        <span className="menu-lbl">{font}px</span>
        <button className="dk-btn" onClick={() => update({ font: Math.min(FONT_MAX, font + 1) })}>
          A+
        </button>
        {def.font != null && def.font !== FONT_DEFAULT && (
          <button className="dk-btn" onClick={() => update({ font: null })}>
            reset
          </button>
        )}
      </div>
      <div className="menu-sep" />
      <div className="menu-item" onClick={() => api.openDialog("panel", def.id, null).finally(close)}>
        <span className="chk" />
        Rename, filters in this panel…
      </div>
      {def.filters.length > 0 && (
        <>
          <div className="menu-sep" />
          <div className="menu-row">
            <span className="menu-lbl">Why isn't</span>
            <input
              className="menu-in"
              autoFocus={!symbol}
              placeholder="SYMBOL"
              value={sym}
              onChange={(e) => setSym(e.target.value.toUpperCase())}
              onKeyDown={(e) => e.key === "Enter" && whyFilters.length === 1 && why(whyFilters[0])}
            />
            <span className="menu-lbl">firing?</span>
          </div>
          {whyFilters.map((f) => (
            <div key={f} className={"menu-item " + (sym.trim() ? "" : "off")} onClick={() => why(f)}>
              <span className="chk">?</span>
              {whyFilters.length > 1 ? `on ${f}` : `Check ${sym.trim() || "…"} on ${f}`}
            </div>
          ))}
          <div className="menu-sep" />
          {(filter ? [filter] : def.filters).map((f) => (
            <div key={f} className="menu-item" onClick={() => api.openDialog("edit", f, null).finally(close)}>
              <span className="chk">✎</span>Adjust {f}…
            </div>
          ))}
        </>
      )}
    </div>
  );
}

/// Panel settings: its name and which filters it shows. `#/panelset/<panel>`.
export function PanelSettingsWindow({ panelId }: { panelId: string }) {
  const panels = useStore((s) => s.panels);
  const filters = useStore((s) => s.filters);
  const def = panels?.panels.find((p) => p.id === panelId);
  const [title, setTitle] = useState<string | null>(null);
  const [sel, setSel] = useState<string[] | null>(null);
  const [q, setQ] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  useEffect(() => {
    if (!def) return;
    if (title === null) setTitle(def.title ?? "");
    if (sel === null) setSel(def.filters);
    const t = `Panel: ${panelTitle(def)}`;
    document.title = t;
    getCurrentWindow().setTitle(t).catch(() => {});
  }, [def]);
  const shown = useMemo(() => filters.filter((f) => f.name.toLowerCase().includes(q.toLowerCase())), [filters, q]);
  if (!def || sel === null || title === null) return <div className="tool" />;
  const toggle = (n: string) => setSel(sel.includes(n) ? sel.filter((x) => x !== n) : [...sel, n]);
  const save = () =>
    api
      .panelUpdate({ ...def, title: title.trim() || null, filters: sel })
      .then(() => setMsg("saved"))
      .catch((e) => setMsg(String(e)));
  return (
    <div className="tool">
      <div className="tool-bar" {...dragBar}>
        <span className="tool-title" data-drag>
          {panelTitle(def)}
        </span>
        <WinButtons />
      </div>
      <div className="tool-body">
        <h4>Name (empty = the filter names)</h4>
        <input className="menu-in wide" value={title} placeholder={sel.join(" | ")} onChange={(e) => setTitle(e.target.value)} />
        <h4>Filters in this panel ({sel.length})</h4>
        <input className="menu-in wide" placeholder="search" value={q} onChange={(e) => setQ(e.target.value)} />
        <div className="flist">
          {shown.map((f) => (
            <label key={f.name} className="frow">
              <input type="checkbox" checked={sel.includes(f.name)} onChange={() => toggle(f.name)} />
              <span className={"fside " + f.side}>{f.side === "long" ? "L" : f.side === "short" ? "S" : "·"}</span>
              <span className="fn">{f.name}</span>
              <span className="fk">
                {f.kind} · {f.hours}
              </span>
            </label>
          ))}
        </div>
      </div>
      <div className="tool-foot">
        {msg && <span className={msg === "saved" ? "ok" : "bad"}>{msg}</span>}
        <span style={{ marginLeft: "auto" }} />
        <button className="dk-btn save" onClick={save}>
          Save
        </button>
        <button className="dk-btn" onClick={() => getCurrentWindow().close()}>
          Close
        </button>
      </div>
    </div>
  );
}
