// "Why isn't SYMBOL firing FILTER?" — the filter run on that one symbol over the day, minute by
// minute: when it fired, how often each clause passed (the bottleneck shows as the lowest bar),
// the closest it came, and where it stands at the last bar. Plus the running scan's own view now.
import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, Diagnosis, fmtNum } from "./api";

export default function WhyWindow({ filter, symbol: initial }: { filter: string; symbol: string }) {
  const [symbol, setSymbol] = useState(initial);
  const [typed, setTyped] = useState(initial);
  const [d, setD] = useState<Diagnosis | null>(null);
  const [now, setNow] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = (sym: string) => {
    setBusy(true);
    setErr(null);
    api
      .diagnose(filter, sym)
      .then(setD)
      .catch((e) => (setD(null), setErr(String(e))))
      .finally(() => setBusy(false));
    api
      .explain(filter, sym)
      .then(setNow)
      .catch(() => setNow(null));
  };
  useEffect(() => {
    const t = `Why: ${symbol} on ${filter}`;
    document.title = t;
    getCurrentWindow().setTitle(t).catch(() => {});
    run(symbol);
  }, [symbol, filter]);

  const fired = d && d.fires.length > 0;
  return (
    <div className="tool">
      <div className="tool-bar">
        <span className="tool-title">{filter}</span>
        <input
          className="menu-in"
          value={typed}
          onChange={(e) => setTyped(e.target.value.toUpperCase())}
          onKeyDown={(e) => e.key === "Enter" && typed.trim() && setSymbol(typed.trim())}
        />
        <button className="dk-btn" onClick={() => (typed.trim() && typed.trim() !== symbol ? setSymbol(typed.trim()) : run(symbol))}>
          {busy ? "running…" : "Check"}
        </button>
        <button className="dk-btn" onClick={() => api.openDialog("edit", filter, null).catch(() => {})}>
          Adjust filter…
        </button>
      </div>
      <div className="tool-body">
        {err && <p className="bad">{err}</p>}
        {!d && !err && <p className="dim">running {filter} on {symbol}…</p>}
        {d && (
          <>
            <div className={"verdict " + (fired ? "yes" : "no")}>
              {fired
                ? `${d.symbol} ${d.kind === "list" ? "entered the list" : "fired"} ${d.fires.length}× on ${d.day}`
                : `${d.symbol} never ${d.kind === "list" ? "entered the list" : "fired"} on ${d.day}`}
            </div>
            <p className="dim">
              {d.bars} minute bars from {d.source}
              {d.first_bar && `, first at ${d.first_bar} ET`}. Hours: {d.hours} ({d.minutes_in_hours} minutes evaluated). Whole filter true in {d.true_minutes} of them
              {d.kind === "trigger" && d.true_minutes > d.fires.length && " (after a fire it waits out its cooldown)"}.
            </p>
            {fired && (
              <>
                <h4>{d.kind === "list" ? "Entered" : "Fires"}</h4>
                <div className="chips">
                  {d.fires.map(([t, last], i) => (
                    <span key={i} className="chip">
                      {t}
                      {last != null && ` @ ${fmtNum(last)}`}
                    </span>
                  ))}
                </div>
              </>
            )}
            {d.closest && !fired && (
              <>
                <h4>Closest it came: {d.closest.time} ET</h4>
                <ul className="blocks">
                  {d.closest.blocking.map((b, i) => (
                    <li key={i}>{b}</li>
                  ))}
                </ul>
              </>
            )}
            <h4>How often each part passed (minutes in hours)</h4>
            <table className="clauses">
              <tbody>
                {d.clauses.map((c, i) => {
                  const pct = c.of ? (100 * c.passed) / c.of : 0;
                  return (
                    <tr key={i} className={c.passed === 0 ? "never" : ""}>
                      <td className="cl">{c.clause}</td>
                      <td className="bar">
                        <span style={{ width: `${pct}%` }} />
                      </td>
                      <td className="num">
                        {c.passed}/{c.of}
                        {c.unknown > 0 && <span className="dim"> ({c.unknown} unknown)</span>}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            {d.last && (
              <>
                <h4>At its last bar ({d.last.time} ET)</h4>
                {d.last.blocking.length === 0 ? (
                  <p className="ok">every part true</p>
                ) : (
                  <ul className="blocks">
                    {d.last.blocking.map((b, i) => (
                      <li key={i}>{b}</li>
                    ))}
                  </ul>
                )}
              </>
            )}
            {now && (
              <details>
                <summary>The running scan's view of {symbol} now</summary>
                <pre>{now}</pre>
              </details>
            )}
          </>
        )}
      </div>
    </div>
  );
}
