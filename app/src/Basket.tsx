// Edit (or create) a named basket: which stocks a filter may draw from. Checkboxes for security
// type and listing exchange, min/max boxes for price, float, average daily volume and ATR. The
// count below says how many symbols pass on the latest daily data, so a change can be judged before
// it's saved. Saving writes filters/baskets.scan (backed up, logged in CHANGES.md) and reloads.
import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, BasketCount, fmtNum } from "./api";
import { useStore } from "./store";

const TYPES: [string, string][] = [
  ["stock", "Stock"],
  ["adr", "ADR"],
  ["etf", "ETF"],
  ["etn", "ETN"],
  ["preferred", "Preferred"],
  ["warrant", "Warrant"],
  ["right", "Right"],
  ["unit", "Unit"],
  ["fund", "Fund"],
  ["other", "Other"],
];
const EXCHANGES: [string, string][] = [
  ["nasdaq", "NASDAQ"],
  ["nyse", "NYSE"],
  ["amex", "NYSE American"],
  ["arca", "NYSE Arca"],
  ["cboe", "Cboe"],
  ["iex", "IEX"],
  ["otc", "OTC"],
  ["other", "Other"],
];
const RANGES: [string, string, string][] = [
  ["price", "Price ($)", "last price"],
  ["float", "Float (shares)", "free float"],
  ["adv", "Avg volume (shares/day)", "20-day average"],
  ["atr", "ATR ($)", "14-day average true range"],
];

/// "10M", "1.5k", "100_000", "2.5" → number; "" → null; anything else → NaN.
function parseNum(s: string): number | null {
  const t = s.trim().replace(/[_,]/g, "").toLowerCase();
  if (!t) return null;
  const m = t.match(/^(\d*\.?\d+)([kmb]?)$/);
  if (!m) return NaN;
  return parseFloat(m[1]) * ({ "": 1, k: 1e3, m: 1e6, b: 1e9 } as Record<string, number>)[m[2]];
}
/// A number as the .scan file writes it: whole numbers with `_` thousands, decimals as they are.
function scanNum(x: number): string {
  if (Number.isInteger(x) && Math.abs(x) >= 10_000) return x.toLocaleString("en-US").replace(/,/g, "_");
  return String(x);
}
function shortNum(x: number | null): string {
  if (x == null) return "";
  if (x >= 1e9 && x % 1e7 === 0) return x / 1e9 + "B";
  if (x >= 1e6 && x % 1e4 === 0) return x / 1e6 + "M";
  if (x >= 1e3 && x % 10 === 0) return x / 1e3 + "K";
  return String(x);
}

export default function BasketWindow({ name: initial }: { name: string }) {
  const isNew = initial === "new";
  const baskets = useStore((s) => s.baskets);
  const existing = baskets.find((b) => b.name === initial);
  const [name, setName] = useState(isNew ? "" : initial);
  const [types, setTypes] = useState<string[]>([]);
  const [exchanges, setExchanges] = useState<string[]>([]);
  const [ranges, setRanges] = useState<Record<string, [string, string]>>({});
  const [loaded, setLoaded] = useState(isNew);
  const [count, setCount] = useState<BasketCount | null>(null);
  const [counting, setCounting] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [msg, setMsg] = useState<{ text: string; bad: boolean } | null>(null);

  useEffect(() => {
    const t = isNew ? "New basket" : `Basket ${initial}`;
    document.title = t;
    getCurrentWindow().setTitle(t).catch(() => {});
  }, [initial]);
  // Seed the form once from the loaded basket.
  useEffect(() => {
    if (loaded || !existing) return;
    setTypes(existing.types);
    setExchanges(existing.exchanges);
    setRanges(Object.fromEntries(existing.ranges.map(([k, lo, hi]) => [k, [shortNum(lo), shortNum(hi)]])));
    setLoaded(true);
  }, [existing, loaded]);

  const nameOk = /^[A-Za-z][A-Za-z0-9_]*$/.test(name);
  const problems: string[] = [];
  if (!nameOk) problems.push("name: a letter, then letters, digits or _");
  const lines: string[] = [];
  if (types.length) lines.push(`    types ${types.join(", ")}`);
  if (exchanges.length) lines.push(`    exchanges ${exchanges.join(", ")}`);
  for (const [k] of RANGES) {
    const [a, b] = ranges[k] ?? ["", ""];
    const lo = parseNum(a),
      hi = parseNum(b);
    if (Number.isNaN(lo) || Number.isNaN(hi)) {
      problems.push(`${k}: numbers like 5, 2.5, 100K, 10M`);
      continue;
    }
    if (lo == null && hi == null) continue;
    lines.push(`    ${k} ${lo != null ? scanNum(lo) + " " : ""}to${hi != null ? " " + scanNum(hi) : ""}`);
  }
  if (!lines.length) problems.push("a basket needs at least one rule");
  const block = `basket ${name} {\n${lines.join("\n")}\n}`;

  // Count as the basket changes (debounced).
  useEffect(() => {
    if (problems.length || !loaded) {
      setCount(null);
      return;
    }
    setCounting(true);
    const t = setTimeout(() => {
      api
        .basketCount(name, block)
        .then((c) => (setCount(c), setMsg((m) => (m?.bad ? null : m))))
        .catch((e) => (setCount(null), setMsg({ text: String(e), bad: true })))
        .finally(() => setCounting(false));
    }, 500);
    return () => clearTimeout(t);
  }, [block, loaded]);

  const toggle = (list: string[], set: (v: string[]) => void, v: string) => set(list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);
  const run = (what: string, p: Promise<unknown>, after?: (r: unknown) => void) => {
    setBusy(what);
    setMsg(null);
    p.then((r) => after?.(r))
      .catch((e) => setMsg({ text: String(e), bad: true }))
      .finally(() => setBusy(null));
  };
  const tested = count ? `${count.pass} symbols pass (${count.as_of})` : null;
  const save = () =>
    run("saving", api.filterSave(name, block, reason, tested, "basket"), (r) => {
      setMsg({ text: String(r), bad: false });
      setReason("");
    });
  const used = existing?.used_by ?? [];

  return (
    <div className="tool">
      <div className="tool-bar">
        <span className="tool-title">basket</span>
        {isNew ? <input className="menu-in" style={{ width: 160, textTransform: "none" }} placeholder="Name" value={name} onChange={(e) => setName(e.target.value)} /> : <span className="tool-title">{name}</span>}
        <span className="dim">{used.length ? `used by ${used.join(", ")}` : "used by no filter yet"}</span>
      </div>
      <div className="tool-body">
        <h4>Security type (none ticked = any)</h4>
        <div className="checks">
          {TYPES.map(([v, l]) => (
            <label key={v}>
              <input type="checkbox" checked={types.includes(v)} onChange={() => toggle(types, setTypes, v)} /> {l}
            </label>
          ))}
        </div>
        <h4>Exchange (none ticked = any)</h4>
        <div className="checks">
          {EXCHANGES.map(([v, l]) => (
            <label key={v}>
              <input type="checkbox" checked={exchanges.includes(v)} onChange={() => toggle(exchanges, setExchanges, v)} /> {l}
            </label>
          ))}
        </div>
        <h4>Ranges (blank = no limit; 100K, 10M allowed)</h4>
        <table className="ranges">
          <tbody>
            {RANGES.map(([k, label, hint]) => {
              const [a, b] = ranges[k] ?? ["", ""];
              const set = (i: 0 | 1, v: string) => setRanges({ ...ranges, [k]: i === 0 ? [v, b] : [a, v] });
              return (
                <tr key={k}>
                  <td className="rl">
                    {label}
                    <span className="dim"> · {hint}</span>
                  </td>
                  <td>
                    <input className="menu-in rng" placeholder="min" value={a} onChange={(e) => set(0, e.target.value)} />
                  </td>
                  <td className="dim">to</td>
                  <td>
                    <input className="menu-in rng" placeholder="max" value={b} onChange={(e) => set(1, e.target.value)} />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        {problems.length > 0 && <p className="bad">{problems.join(" · ")}</p>}
        <div className="testres">
          {counting ? (
            <p className="dim">counting…</p>
          ) : count ? (
            <>
              <h4>
                {count.pass.toLocaleString()} symbols pass · {count.fail.toLocaleString()} don't · {count.unknown.toLocaleString()} can't tell
              </h4>
              <p className="dim">
                On {count.as_of}, price = last close. "Can't tell" = a rule's data is missing for the symbol (no float, or not in the ticker list); those never pass in a live scan.
              </p>
              <p className="dim">Most traded inside: {count.top.map(([s, v]) => `${s} ${fmtNum(v, 0)}`).join(", ") || "none"}</p>
            </>
          ) : (
            <p className="dim">fill in the basket to see how many symbols pass</p>
          )}
        </div>
        <details>
          <summary>As written to baskets.scan</summary>
          <pre>{block}</pre>
        </details>
        {msg && <p className={msg.bad ? "bad pre" : "ok"}>{msg.text}</p>}
      </div>
      <div className="tool-foot">
        <input className="menu-in wide" placeholder="why this change (goes in CHANGES.md)" value={reason} onChange={(e) => setReason(e.target.value)} />
        <button className="dk-btn save" disabled={!!busy || problems.length > 0} onClick={save}>
          {busy === "saving" ? "Saving…" : isNew ? "Create" : "Save"}
        </button>
        {!isNew && (
          <button className="dk-btn" disabled={!!busy} onClick={() => run("undoing", api.filterUndo(name, "basket"), (r) => setMsg({ text: String(r), bad: false }))}>
            Undo last save
          </button>
        )}
      </div>
    </div>
  );
}
