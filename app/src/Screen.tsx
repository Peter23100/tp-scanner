// A Primus-style screen: rows of columns of stacked panels and charts, from filters/display.json.
import { useEffect, useMemo, useState } from "react";
import { api, Item, LayoutReply } from "./api";
import { useStore } from "./store";
import Panel from "./Panel";
import ChartView from "./Chart";

export default function ScreenView({ name }: { name: string }) {
  const layoutVersion = useStore((s) => s.layoutVersion);
  const [reply, setReply] = useState<LayoutReply | null>(null);
  useEffect(() => {
    api.getLayout().then(setReply);
  }, [layoutVersion]);
  useEffect(() => {
    document.title = `Scanner: ${name}`;
  }, [name]);

  const screen = reply?.layout?.display.screens.find((s) => s.name === name);
  const meta = useMemo(() => new Map((reply?.layout?.filters ?? []).map((f) => [f.name, f])), [reply]);

  if (!reply) return <div className="empty">loading…</div>;
  if (!screen) return <div className="empty bad">{reply.error ?? `no screen named "${name}" in display.json`}</div>;

  return (
    <div className="screen">
      {screen.rows.map((row, r) => (
        <div className="row" key={r} style={{ flex: row.height }}>
          {row.columns.map((col, c) => (
            <div className="col" key={c} style={{ flex: col.width }}>
              {col.items.map((it: Item, i) => {
                const key = `${name}/${r}/${c}/${i}`;
                return (
                  <div className="cell" key={key} style={{ flex: it.height }}>
                    {it.chart ? (
                      <ChartView id={`screen-${name}/${key}`} initialColor={it.chart} />
                    ) : (
                      <Panel panelKey={key} item={it} meta={meta} color={reply.panel_links[key] ?? it.link ?? "red"} />
                    )}
                  </div>
                );
              })}
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}
