// Per-window store of what the engine publishes. Each Tauri window has its own JS context, so each
// window seeds itself from get_snapshot and then follows the events.
import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";
import { api, ListRowDto, SignalDto, Snapshot, StatusDto } from "./api";

const KEEP = 5000;

interface State {
  signals: SignalDto[];
  lists: Record<string, ListRowDto[]>;
  status: StatusDto | null;
  links: Record<string, string>;
  layoutVersion: number;
}

let state: State = { signals: [], lists: {}, status: null, links: {}, layoutVersion: 0 };
const subs = new Set<() => void>();
function set(patch: Partial<State>) {
  state = { ...state, ...patch };
  subs.forEach((f) => f());
}
function subscribe(f: () => void) {
  subs.add(f);
  return () => subs.delete(f);
}

let started = false;
// Signals that arrive before the snapshot resolves are buffered, then merged by id.
let pending: SignalDto[] | null = [];

export function startStore() {
  if (started) return;
  started = true;
  listen<SignalDto>("scan-signal", (e) => {
    if (pending) {
      pending.push(e.payload);
      return;
    }
    const s = state.signals.length >= KEEP ? state.signals.slice(-KEEP + 1) : state.signals.slice();
    s.push(e.payload);
    set({ signals: s });
  });
  listen<{ lists: Record<string, ListRowDto[]>; status: StatusDto }>("scan-tick", (e) => {
    set({ lists: e.payload.lists, status: e.payload.status });
  });
  listen("scan-reset", () => set({ signals: [], lists: {} }));
  listen("layout-changed", () => set({ layoutVersion: state.layoutVersion + 1 }));
  listen<{ color: string; symbol: string }>("link-changed", (e) => {
    set({ links: { ...state.links, [e.payload.color]: e.payload.symbol } });
  });
  api.getSnapshot().then((snap: Snapshot) => {
    const seen = new Set(snap.signals.map((s) => s.id));
    const merged = snap.signals.concat((pending ?? []).filter((s) => !seen.has(s.id)));
    pending = null;
    set({ signals: merged, lists: snap.lists, status: snap.status });
  });
  api.linkGet().then((links) => set({ links: { ...links, ...state.links } }));
}

export function useStore<T>(pick: (s: State) => T): T {
  return useSyncExternalStore(subscribe, () => pick(state));
}
