// The backend's commands and events, typed. Mirrors app/src-tauri/src/{main,scanner,layout,panels}.rs.
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
  basket: string | null;
}

/** A named basket (filters/baskets.scan): `ranges` are [key, min, max]. */
export interface BasketDto {
  name: string;
  types: string[];
  exchanges: string[];
  ranges: [string, number | null, number | null][];
  used_by: string[];
}
export interface BasketCount {
  pass: number;
  fail: number;
  unknown: number;
  top: [string, number][];
  as_of: string;
}

/** One panel window (panels.rs). */
export interface PanelDef {
  id: string;
  title?: string | null;
  filters: string[];
  link: string;
  follow: boolean;
  sound: boolean;
  group: string;
  tile?: [number, number, number, number] | null;
}
export interface ChartSeed {
  label: string;
  color: string;
  group: string;
}
export interface Panels {
  panels: PanelDef[];
  charts: ChartSeed[];
}
export interface FiltersReply {
  filters: FilterMeta[];
  baskets: BasketDto[];
  error: string | null;
}

export interface ClauseStat {
  clause: string;
  passed: number;
  unknown: number;
  of: number;
}
export interface Miss {
  time: string;
  blocking: string[];
}
export interface Diagnosis {
  filter: string;
  symbol: string;
  day: string;
  source: string;
  kind: "trigger" | "list";
  hours: string;
  bars: number;
  first_bar: string | null;
  minutes_in_hours: number;
  true_minutes: number;
  fires: [string, number | null][];
  clauses: ClauseStat[];
  closest: Miss | null;
  last: Miss | null;
}
export interface Count {
  fires: number;
  symbols: [string, number][];
}
export interface TestResult {
  day: string;
  before: Count;
  after: Count;
  added: string[];
  removed: string[];
  seconds: number;
}
export interface FilterText {
  file: string;
  block: string;
  literals: [number, number][];
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
  getFilters: () => invoke<FiltersReply>("get_filters"),
  getSnapshot: () => invoke<Snapshot>("get_snapshot"),
  reloadFilters: () => invoke<string>("reload_filters"),
  startLive: (realtime: boolean) => invoke<void>("start_live", { realtime }),
  startReplay: (date: string, speed: number, startAt: string | null) => invoke<void>("start_replay", { date, speed, startAt }),
  setSpeed: (speed: number) => invoke<void>("set_speed", { speed }),
  stopSource: () => invoke<void>("stop_source"),
  explain: (filter: string, symbol: string) => invoke<string>("explain", { filter, symbol }),
  linkSet: (color: string, symbol: string) => invoke<void>("link_set", { color, symbol }),
  linkGet: () => invoke<Record<string, string>>("link_get"),
  panelsGet: () => invoke<Panels>("panels_get"),
  panelUpdate: (panel: PanelDef) => invoke<void>("panel_update", { panel }),
  panelCreate: () => invoke<string>("panel_create"),
  panelDelete: (id: string) => invoke<void>("panel_delete", { id }),
  openPanel: (id: string) => invoke<void>("open_panel", { id }),
  resetLayout: () => invoke<void>("reset_layout"),
  diagnose: (filter: string, symbol: string) => invoke<Diagnosis>("diagnose", { filter, symbol }),
  filterSource: (name: string, kind: "filter" | "basket" = "filter") => invoke<FilterText>("filter_source", { name, kind }),
  filterTest: (name: string, block: string) => invoke<TestResult>("filter_test", { name, block }),
  filterSave: (name: string, block: string, reason: string, tested: string | null, kind: "filter" | "basket" = "filter") =>
    invoke<string>("filter_save", { name, block, reason, tested, kind }),
  filterUndo: (name: string, kind: "filter" | "basket" = "filter") => invoke<string>("filter_undo", { name, kind }),
  basketCount: (name: string, block: string) => invoke<BasketCount>("basket_count", { name, block }),
  openDialog: (kind: "why" | "edit" | "basket", filter: string, symbol: string | null) => invoke<void>("open_dialog", { kind, filter, symbol }),
  chartWatch: (id: string, symbol: string | null) => invoke<void>("chart_watch", { id, symbol }),
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
