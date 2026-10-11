// Hash routes, one per window kind:
//   (none)                       the launcher bar
//   #/panel/<id>                 one filter panel (panels.json)
//   #/chart/<label>/<color>      a chart window
//   #/why/<filter>/<symbol>      why a symbol does or doesn't fire a filter
//   #/edit/<filter>              adjust a filter
//   #/basket/<name | new>        edit or create a basket
import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { startStore } from "./store";
import Launcher from "./Launcher";
import { PanelWindow } from "./Panel";
import ScanChart from "./ScanChart";
import WhyWindow from "./Why";
import EditWindow from "./Edit";
import BasketWindow from "./Basket";

function route(): string[] {
  return window.location.hash.replace(/^#\/?/, "").split("/").filter(Boolean);
}

export default function App() {
  const [r, setR] = useState(route());
  useEffect(() => {
    startStore();
    const f = () => setR(route());
    window.addEventListener("hashchange", f);
    return () => window.removeEventListener("hashchange", f);
  }, []);
  if (r[0] === "panel" && r[1]) return <PanelWindow id={decodeURIComponent(r[1])} />;
  if (r[0] === "chart" && r[1]) return <ChartWindow label={r[1]} color={r[2] ?? "red"} />;
  if (r[0] === "why" && r[1] && r[2]) return <WhyWindow filter={r[1]} symbol={decodeURIComponent(r[2])} />;
  if (r[0] === "edit" && r[1]) return <EditWindow filter={r[1]} />;
  if (r[0] === "basket" && r[1]) return <BasketWindow name={r[1]} />;
  return <Launcher />;
}

function ChartWindow({ label, color }: { label: string; color: string }) {
  useEffect(() => {
    document.title = `Chart (${color})`;
    getCurrentWindow().setTitle(document.title).catch(() => {});
  }, [color]);
  return (
    <div className="chart-window">
      <ScanChart id={label} initialColor={color} standalone />
    </div>
  );
}
