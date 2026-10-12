// Adjust one filter: its block from the .scan file with every number editable in place, hours and
// side as choices, or the raw text for anything structural. Test runs the edit against the
// unedited filter over a recorded day before anything is written; Save writes the file (backed up),
// reloads the scan and logs the change with your reason; Undo puts the previous file back.
import { useEffect, useMemo, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { dragBar, WinButtons } from "./win";
import { api, FilterText, TestResult } from "./api";
import { useStore } from "./store";

const HOURS = ["pre", "rth", "extended", "any"];
const SIDES = ["long", "short", "neutral"];

function setting(block: string, key: string): string | null {
  return block.match(new RegExp(`^\\s*${key}\\s+(\\w+)\\s*$`, "m"))?.[1] ?? null;
}

/// Replace (or add, right after the opening brace) a one-word setting line. `null` removes it.
function withSetting(segments: string[], key: string, value: string | null, dflt: string | null): string[] {
  const re = new RegExp(`^(\\s*)${key}\\s+\\w+(\\s*)$`, "m");
  const i = segments.findIndex((s) => re.test(s));
  if (i >= 0 && value === null) return segments.map((s, k) => (k === i ? s.replace(new RegExp(`^[ \\t]*${key}\\s+\\w+[ \\t]*\\r?\\n`, "m"), "") : s));
  if (value === null) return segments;
  if (i >= 0) return segments.map((s, k) => (k === i ? s.replace(re, `$1${key} ${value}$2`) : s));
  if (value === dflt) return segments;
  const out = segments.slice();
  out[0] = out[0].replace(/\{\s*\n/, (m) => `${m}    ${key} ${value}\n`);
  return out;
}

export default function EditWindow({ filter }: { filter: string }) {
  const [src, setSrc] = useState<FilterText | null>(null);
  const [vals, setVals] = useState<string[]>([]);
  const [hours, setHours] = useState("rth");
  const [side, setSide] = useState("neutral");
  const [basket, setBasket] = useState<string | null>(null);
  const baskets = useStore((s) => s.baskets);
  const [raw, setRaw] = useState<string | null>(null);
  const [test, setTest] = useState<TestResult | null>(null);
  const [testedBlock, setTestedBlock] = useState<string | null>(null);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [msg, setMsg] = useState<{ text: string; bad: boolean } | null>(null);

  const load = () =>
    api
      .filterSource(filter)
      .then((s) => {
        setSrc(s);
        setVals(s.literals.map(([a, b]) => s.block.slice(a, b)));
        setHours(setting(s.block, "hours") ?? "rth");
        setSide(setting(s.block, "side") ?? "neutral");
        setBasket(setting(s.block, "basket"));
        setRaw(null);
      })
      .catch((e) => setMsg({ text: String(e), bad: true }));
  useEffect(() => {
    const t = `Adjust ${filter}`;
    document.title = t;
    getCurrentWindow().setTitle(t).catch(() => {});
    load();
  }, [filter]);

  // The block as text segments between the editable numbers, with hours/side applied.
  const segments = useMemo(() => {
    if (!src) return [];
    const out: string[] = [];
    let at = 0;
    for (const [a, b] of src.literals) {
      out.push(src.block.slice(at, a));
      at = b;
    }
    out.push(src.block.slice(at));
    return withSetting(withSetting(withSetting(out, "hours", hours, "rth"), "side", side, "neutral"), "basket", basket, null);
  }, [src, hours, side, basket]);
  const formBlock = segments.map((s, i) => s + (i < vals.length ? vals[i] : "")).join("");
  const block = raw ?? formBlock;
  const changed = !!src && block.trim() !== src.block.trim();

  const run = (what: string, p: Promise<unknown>, after?: (r: unknown) => void) => {
    setBusy(what);
    setMsg(null);
    p.then((r) => after?.(r))
      .catch((e) => setMsg({ text: String(e), bad: true }))
      .finally(() => setBusy(null));
  };
  const doTest = () =>
    run("testing", api.filterTest(filter, block), (r) => {
      setTest(r as TestResult);
      setTestedBlock(block);
    });
  const summary = test && testedBlock === block ? `${test.day}: ${test.before.fires} → ${test.after.fires} fires, ${test.before.symbols.length} → ${test.after.symbols.length} symbols` : null;
  const doSave = () =>
    run("saving", api.filterSave(filter, block, reason, summary), (r) => {
      setMsg({ text: String(r), bad: false });
      setReason("");
      setTest(null);
      load();
    });
  const doUndo = () =>
    run("undoing", api.filterUndo(filter), (r) => {
      setMsg({ text: String(r), bad: false });
      setTest(null);
      load();
    });

  if (!src) return <div className="tool">{msg ? <p className="bad pad">{msg.text}</p> : <p className="dim pad">loading…</p>}</div>;

  return (
    <div className="tool">
      <div className="tool-bar" {...dragBar}>
        <span className="tool-title" data-drag>{filter}</span>
        <span className="dim">{src.file}</span>
        <label className="sel">
          hours
          <select value={hours} disabled={raw !== null} onChange={(e) => setHours(e.target.value)}>
            {HOURS.map((h) => (
              <option key={h}>{h}</option>
            ))}
          </select>
        </label>
        <label className="sel">
          side
          <select value={side} disabled={raw !== null} onChange={(e) => setSide(e.target.value)}>
            {SIDES.map((h) => (
              <option key={h}>{h}</option>
            ))}
          </select>
        </label>
        <label className="sel" title="which stocks this filter may draw from">
          basket
          <select value={basket ?? ""} disabled={raw !== null} onChange={(e) => setBasket(e.target.value || null)}>
            <option value="">(none)</option>
            {baskets.map((b) => (
              <option key={b.name} value={b.name}>
                {b.name}
              </option>
            ))}
          </select>
        </label>
        <button className="dk-btn" onClick={() => api.openDialog("basket", basket ?? "new", null).catch(() => {})}>
          {basket ? "Edit basket…" : "New basket…"}
        </button>
        <span style={{ marginLeft: "auto" }} />
        {raw === null ? (
          <button className="dk-btn" onClick={() => setRaw(formBlock)} title="edit the text itself (add or remove a condition)">
            Edit text
          </button>
        ) : (
          <button className="dk-btn" onClick={() => setRaw(null)}>
            Discard text edits
          </button>
        )}
        <WinButtons />
      </div>
      <div className="tool-body">
        {raw === null ? (
          <pre className="code">
            {segments.map((s, i) => (
              <span key={i}>
                {s}
                {i < vals.length && (
                  <input
                    className={"lit " + (vals[i] !== src.block.slice(src.literals[i][0], src.literals[i][1]) ? "chg" : "")}
                    value={vals[i]}
                    style={{ width: `${Math.max(2, vals[i].length) + 1}ch` }}
                    onChange={(e) => setVals(vals.map((v, k) => (k === i ? e.target.value : v)))}
                  />
                )}
              </span>
            ))}
          </pre>
        ) : (
          <textarea className="code raw" spellCheck={false} value={raw} onChange={(e) => setRaw(e.target.value)} />
        )}
        {msg && <p className={msg.bad ? "bad pre" : "ok"}>{msg.text}</p>}
        {test && (
          <div className={"testres " + (testedBlock === block ? "" : "stale")}>
            <h4>
              Tested on {test.day} ({test.seconds.toFixed(1)}s){testedBlock !== block && " — edited since; test again"}
            </h4>
            <div className="ba">
              <div>
                <b>{test.before.fires}</b> fires · {test.before.symbols.length} symbols <span className="dim">as loaded</span>
              </div>
              <div>
                <b>{test.after.fires}</b> fires · {test.after.symbols.length} symbols <span className="dim">edited</span>
              </div>
            </div>
            {test.removed.length > 0 && (
              <p>
                <span className="dn">no longer:</span> {test.removed.slice(0, 40).join(", ")}
                {test.removed.length > 40 && ` … +${test.removed.length - 40}`}
              </p>
            )}
            {test.added.length > 0 && (
              <p>
                <span className="up">new:</span> {test.added.slice(0, 40).join(", ")}
                {test.added.length > 40 && ` … +${test.added.length - 40}`}
              </p>
            )}
            <p className="dim">
              Edited, most fires:{" "}
              {test.after.symbols
                .slice(0, 15)
                .map(([s, n]) => `${s} ${n}`)
                .join(", ") || "none"}
            </p>
          </div>
        )}
      </div>
      <div className="tool-foot">
        <button className="dk-btn" disabled={!!busy} onClick={doTest} title="run the edit against the filter as loaded, over a recorded day (on its minute bars: fast; live runs per-second, so live can fire a little earlier)">
          {busy === "testing" ? "Testing…" : "Test"}
        </button>
        <input className="menu-in wide" placeholder="why this change (goes in CHANGES.md)" value={reason} onChange={(e) => setReason(e.target.value)} />
        <button className="dk-btn save" disabled={!!busy || !changed} onClick={doSave}>
          {busy === "saving" ? "Saving…" : "Save"}
        </button>
        <button className="dk-btn" disabled={!!busy || !changed} onClick={() => (setTest(null), load())}>
          Revert
        </button>
        <button className="dk-btn" disabled={!!busy} onClick={doUndo} title="put back the file as it was before the last saved change">
          Undo last save
        </button>
      </div>
    </div>
  );
}
