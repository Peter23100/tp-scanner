// Per-window store of what the engine publishes. Each Tauri window has its own JS context, so each
// window seeds itself from get_snapshot and then follows the events.
import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";
import { api, BasketDto, FilterMeta, ListRowDto, Panels, SignalDto, Snapshot, StatusDto } from "./api";

const KEEP = 5000;

interface State {
  signals: SignalDto[];
  lists: Record<string, ListRowDto[]>;
  status: StatusDto | null;
  links: Record<string, string>;
  /** Link colour → the filter that set its symbol (null when typed). */
  linkFilters: Record<string, string | null>;
  /** What each loaded filter is (kind, side, columns); empty until the engine has loaded. */
  filters: FilterMeta[];
  baskets: BasketDto[];
  filtersError: string | null;
  panels: Panels | null;
}

let state: State = { signals: [], lists: {}, status: null, links: {}, linkFilters: {}, filters: [], baskets: [], filtersError: null, panels: null };
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

const loadFilters = () => api.getFilters().then((r) => set({ filters: r.filters, baskets: r.baskets, filtersError: r.error }));
const loadPanels = () => api.panelsGet().then((panels) => set({ panels }));

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
  listen("filters-changed", loadFilters);
  listen("panels-changed", loadPanels);
  listen<{ color: string; symbol: string; filter?: string | null }>("link-changed", (e) => {
    set({ links: { ...state.links, [e.payload.color]: e.payload.symbol }, linkFilters: { ...state.linkFilters, [e.payload.color]: e.payload.filter ?? null } });
  });
  api.getSnapshot().then((snap: Snapshot) => {
    const seen = new Set(snap.signals.map((s) => s.id));
    const merged = snap.signals.concat((pending ?? []).filter((s) => !seen.has(s.id)));
    pending = null;
    set({ signals: merged, lists: snap.lists, status: snap.status });
  });
  api.linkGet().then((l) => set({ links: { ...l.symbols, ...state.links }, linkFilters: { ...l.filters, ...state.linkFilters } }));
  loadFilters();
  loadPanels();
}

export function useStore<T>(pick: (s: State) => T): T {
  return useSyncExternalStore(subscribe, () => pick(state));
}
