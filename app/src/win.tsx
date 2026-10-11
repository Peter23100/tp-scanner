// Windows have no OS title bar (they took too much room, and repeated the panel's name): each
// window's own header is the bar. Drag it to move the window, double-click it to maximize, and use
// its small – and × buttons. The edges still resize.
import type { MouseEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/// Whether a press lands on the bar itself (its empty space) or on a part marked `data-drag` (a
/// title, the logo). Anything else on the bar (menu items, buttons, inputs) keeps its own click.
function onHandle(e: MouseEvent): boolean {
  const t = e.target as HTMLElement;
  return t === e.currentTarget || !!t.closest("[data-drag]");
}

/// Handlers that make an element a drag bar.
export const dragBar = {
  onMouseDown: (e: MouseEvent) => {
    if (e.button !== 0 || e.detail > 1 || !onHandle(e)) return;
    getCurrentWindow()
      .startDragging()
      .catch(() => {});
  },
  onDoubleClick: (e: MouseEvent) => {
    if (!onHandle(e)) return;
    getCurrentWindow()
      .toggleMaximize()
      .catch(() => {});
  },
};

/// Minimize and close, for a window without an OS title bar.
export function WinButtons({ closeTitle = "close" }: { closeTitle?: string }) {
  const w = () => getCurrentWindow();
  return (
    <span className="winbtns nodrag">
      <button className="winbtn" title="minimize" onClick={() => w().minimize().catch(() => {})}>
        –
      </button>
      <button className="winbtn x" title={closeTitle} onClick={() => w().close().catch(() => {})}>
        ×
      </button>
    </span>
  );
}
