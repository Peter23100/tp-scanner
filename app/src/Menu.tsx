// The panel's right-click menu, and the two overlays it opens inside the panel window (rename,
// filters in this panel). Tools that need room (why, adjust) open their own windows.
import { useEffect, useMemo, useRef, useState } from "react";
import { api, FilterMeta, FONT_DEFAULT, FONT_MAX, FONT_MIN, LINK_COLORS, LINK_HEX, PanelDef } from "./api";
import { panelTitle } from "./Panel";

export interface MenuAt {
  x: number;
  y: number;
  /** The row right-clicked, if any. */
  symbol?: string;
  filter?: string;
}

export function PanelMenu({ at, def, onClose, onRename, onFilters }: { at: MenuAt; def: PanelDef; onClose: () => void; onRename: () => void; onFilters: () => void }) {
  const box = useRef<HTMLDivElement>(null);
  const [sym, setSym] = useState(at.symbol ?? "");
  const [pos, setPos] = useState({ x: at.x, y: at.y });
  const whyFilters = at.filter ? [at.filter] : def.filters;

  // Keep the menu inside the (often small) panel window.
  useEffect(() => {
    const r = box.current?.getBoundingClientRect();
    if (!r) return;
    setPos({ x: Math.max(0, Math.min(at.x, window.innerWidth - r.width - 2)), y: Math.max(0, Math.min(at.y, window.innerHeight - r.height - 2)) });
  }, [at]);
  useEffect(() => {
    const k = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  }, [onClose]);

  const update = (patch: Partial<PanelDef>) => api.panelUpdate({ ...def, ...patch }).catch(() => {});
  const why = (filter: string) => {
    if (!sym.trim()) return;
    api.openDialog("why", filter, sym.trim().toUpperCase()).catch(() => {});
    onClose();
  };

  return (
    <div className="menu-veil" onMouseDown={onClose} onContextMenu={(e) => (e.preventDefault(), onClose())}>
      <div className="menu" ref={box} style={{ left: pos.x, top: pos.y }} onMouseDown={(e) => e.stopPropagation()} onContextMenu={(e) => e.preventDefault()}>
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
          <button className="dk-btn" onClick={() => update({ font: Math.max(FONT_MIN, (def.font ?? FONT_DEFAULT) - 1) })}>
            A−
          </button>
          <span className="menu-lbl">{def.font ?? FONT_DEFAULT}px</span>
          <button className="dk-btn" onClick={() => update({ font: Math.min(FONT_MAX, (def.font ?? FONT_DEFAULT) + 1) })}>
            A+
          </button>
          {def.font != null && def.font !== FONT_DEFAULT && (
            <button className="dk-btn" onClick={() => update({ font: null })}>
              reset
            </button>
          )}
        </div>
        <div className="menu-sep" />
        <div className="menu-item" onClick={() => (onClose(), onRename())}>
          <span className="chk" />
          Rename…
        </div>
        <div className="menu-item" onClick={() => (onClose(), onFilters())}>
          <span className="chk" />
          Filters in this panel…
        </div>
        {def.filters.length > 0 && (
          <>
            <div className="menu-sep" />
            <div className="menu-row">
              <span className="menu-lbl">Why isn't</span>
              <input
                className="menu-in"
                autoFocus={!at.symbol}
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
            {(at.filter ? [at.filter] : def.filters).map((f) => (
              <div key={f} className="menu-item" onClick={() => (api.openDialog("edit", f, null).catch(() => {}), onClose())}>
                <span className="chk">✎</span>Adjust {f}…
              </div>
            ))}
          </>
        )}
      </div>
    </div>
  );
}

export function RenameBox({ def, onClose }: { def: PanelDef; onClose: () => void }) {
  const [t, setT] = useState(def.title ?? "");
  const save = () => {
    api.panelUpdate({ ...def, title: t.trim() || null }).catch(() => {});
    onClose();
  };
  return (
    <div className="explain" onMouseDown={onClose}>
      <div className="explain-box small" onMouseDown={(e) => e.stopPropagation()}>
        <div className="explain-head">Panel name</div>
        <div className="pad">
          <input className="menu-in wide" autoFocus value={t} placeholder={def.filters.join(" | ")} onChange={(e) => setT(e.target.value)} onKeyDown={(e) => (e.key === "Enter" ? save() : e.key === "Escape" && onClose())} />
          <div className="btns">
            <button className="dk-btn" onClick={save}>
              Save
            </button>
            <button className="dk-btn" onClick={onClose}>
              Cancel
            </button>
          </div>
          <div className="hint">Empty = the filter names.</div>
        </div>
      </div>
    </div>
  );
}

export function FiltersBox({ def, filters, onClose }: { def: PanelDef; filters: FilterMeta[]; onClose: () => void }) {
  const [q, setQ] = useState("");
  const [sel, setSel] = useState<string[]>(def.filters);
  const shown = useMemo(() => filters.filter((f) => f.name.toLowerCase().includes(q.toLowerCase())), [filters, q]);
  const toggle = (n: string) => setSel(sel.includes(n) ? sel.filter((x) => x !== n) : [...sel, n]);
  const save = () => {
    api.panelUpdate({ ...def, filters: sel }).catch(() => {});
    onClose();
  };
  return (
    <div className="explain" onMouseDown={onClose}>
      <div className="explain-box tall" onMouseDown={(e) => e.stopPropagation()}>
        <div className="explain-head">
          Filters in {panelTitle(def)}
          <span style={{ marginLeft: "auto" }} />
          <button onClick={save}>Save ({sel.length})</button>
          <button onClick={onClose}>Cancel</button>
        </div>
        <div className="pad">
          <input className="menu-in wide" autoFocus placeholder="search" value={q} onChange={(e) => setQ(e.target.value)} />
        </div>
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
    </div>
  );
}
