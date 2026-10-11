// Hash routes, one per window kind:
//   (none)                       the launcher bar
//   #/screen/<name>              a Primus-style screen from filters/display.json
//   #/chart/<label>/<color>      a free-standing chart window
import { useEffect, useState } from "react";
import { startStore } from "./store";
import Launcher from "./Launcher";
import ScreenView from "./Screen";
import ChartView from "./Chart";

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
  if (r[0] === "screen" && r[1]) return <ScreenView name={r[1]} />;
  if (r[0] === "chart" && r[1]) return <ChartWindow label={r[1]} color={r[2] ?? "red"} />;
  return <Launcher />;
}

function ChartWindow({ label, color }: { label: string; color: string }) {
  useEffect(() => {
    document.title = `Chart (${color})`;
  }, [color]);
  return (
    <div className="chart-window">
      <ChartView id={label} initialColor={color} standalone />
    </div>
  );
}
