// The backend's commands and events, typed. Mirrors app/src-tauri/src/{main,scanner,layout}.rs.
import { invoke } from "@tauri-apps/api/core";

export type Side = "long" | "short" | "neutral";
export type Cols = [string, number | null][];

export interface SignalDto {
  id: number;
  filter: string;
  symbol: string;
  side: Side;
  at_ms: number;
  time: string;
  last: number | null;
  columns: Cols;
}

export interface ListRowDto {
  symbol: string;
  last: number | null;
  rank: number | null;
  since: string;
  columns: Cols;
}

export interface StatusDto {
  mode: "idle" | "loading" | "live" | "replay";
  feed: string;
  ok: boolean;
  bars: number;
  market_time: string | null;
  market_ms: number | null;
  delay_s: number | null;
  bad_messages: number;
  reconnects: number;
  signals: number;
  errors: number;
  error_samples: string[];
  warnings: string[];
  replay: { date: string; speed: number; done: boolean } | null;
  filters: number;
  since: string | null;
}

export interface Snapshot {
  signals: SignalDto[];
  lists: Record<string, ListRowDto[]>;
  status: StatusDto;
}

export interface FilterMeta {
  name: string;
  kind: "trigger" | "list";
  side: Side;
  hours: string;
  cooldown_s: number;
  columns: string[];
  ranked: boolean;
}

export interface Item {
  title?: string | null;
  filters: string[];
  link?: string | null;
  chart?: string | null;
  height: number;
}
export interface Column {
  width: number;
  items: Item[];
}
export interface Row {
  height: number;
  columns: Column[];
}
export interface Screen {
  name: string;
  title: string;
  rows: Row[];
}
export interface Layout {
  display: { screens: Screen[] };
  filters: FilterMeta[];
  not_displayed: string[];
}
export interface LayoutReply {
  layout: Layout | null;
  error: string | null;
  panel_links: Record<string, string>;
}

export const LINK_COLORS = ["red", "blue", "green", "yellow", "orange", "purple", "pink", "cyan"] as const;
export const LINK_HEX: Record<string, string> = {
  red: "#ff4d4d",
  blue: "#4d8dff",
  green: "#3ddc6a",
  yellow: "#ffd23d",
  orange: "#ff9a3d",
  purple: "#b46dff",
  pink: "#ff6dc8",
  cyan: "#3de0e0",
};
export function nextColor(c: string): string {
  const i = LINK_COLORS.indexOf(c as (typeof LINK_COLORS)[number]);
  return LINK_COLORS[(i + 1) % LINK_COLORS.length];
}

export const api = {
  getLayout: () => invoke<LayoutReply>("get_layout"),
  getSnapshot: () => invoke<Snapshot>("get_snapshot"),
  reloadFilters: () => invoke<string>("reload_filters"),
  startLive: (realtime: boolean) => invoke<void>("start_live", { realtime }),
  startReplay: (date: string, speed: number, startAt: string | null) => invoke<void>("start_replay", { date, speed, startAt }),
  setSpeed: (speed: number) => invoke<void>("set_speed", { speed }),
  stopSource: () => invoke<void>("stop_source"),
  explain: (filter: string, symbol: string) => invoke<string>("explain", { filter, symbol }),
  linkSet: (color: string, symbol: string) => invoke<void>("link_set", { color, symbol }),
  linkGet: () => invoke<Record<string, string>>("link_get"),
  setPanelLink: (key: string, color: string) => invoke<void>("set_panel_link", { key, color }),
  chartWatch: (id: string, symbol: string | null) => invoke<void>("chart_watch", { id, symbol }),
  chartHistory: (symbol: string, until: string | null, days: number) => invoke<[number, number, number, number, number, number][]>("chart_history", { symbol, until, days }),
  openScreen: (name: string) => invoke<void>("open_screen", { name }),
  openChart: (color: string) => invoke<void>("open_chart", { color }),
  setChartColor: (label: string, color: string) => invoke<void>("set_chart_color", { label, color }),
};

export function fmtNum(v: number | null | undefined, digits = 2): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "?";
  const a = Math.abs(v);
  if (a >= 1e9) return (v / 1e9).toFixed(2) + "B";
  if (a >= 1e6) return (v / 1e6).toFixed(2) + "M";
  if (a >= 1e4) return (v / 1e3).toFixed(1) + "K";
  return v.toFixed(digits);
}
