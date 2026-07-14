// Window geometry across launches: put the window back where it was, and keep
// noting where that is. The decision of whether a remembered geometry is still
// reachable is pure and lives in api/geometry.ts; this module is only the
// Tauri glue around it - the window handle, the monitor list, the events.

import {
  PhysicalPosition,
  PhysicalSize,
  availableMonitors,
  getCurrentWindow,
} from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { fitsOnScreen } from "./geometry";
import type { ScreenRect, WindowGeometry } from "./geometry";

/** How long a drag or resize has to settle before it is worth persisting. */
const SETTLE_MS = 300;

/** The monitors as plain rectangles, or `[]` when they cannot be enumerated. */
async function monitorRects(): Promise<ScreenRect[]> {
  try {
    const monitors = await availableMonitors();
    return monitors.map((m) => ({
      x: m.position.x,
      y: m.position.y,
      width: m.size.width,
      height: m.size.height,
    }));
  } catch {
    return [];
  }
}

/**
 * Restore `geometry`, unless it would put the window somewhere this desk no
 * longer has a screen - in which case the window keeps the placement the OS
 * gave it, which is always somewhere you can see it.
 */
export async function restoreGeometry(geometry: WindowGeometry | null): Promise<void> {
  if (!geometry) return;
  if (!fitsOnScreen(geometry, await monitorRects())) return;

  const win = getCurrentWindow();
  await win.setSize(new PhysicalSize(geometry.width, geometry.height));
  await win.setPosition(new PhysicalPosition(geometry.x, geometry.y));
}

/**
 * Report the window's size and position whenever they settle after a move or a
 * resize (a drag fires a torrent of events; only the last one is worth
 * writing). A minimized window reports a geometry nobody wants restored, so it
 * is skipped. Returns a teardown that stops listening.
 */
export async function trackGeometry(onChange: (geometry: WindowGeometry) => void): Promise<UnlistenFn> {
  const win = getCurrentWindow();
  let timer: ReturnType<typeof setTimeout> | undefined;

  const record = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(() => {
      void (async () => {
        if (await win.isMinimized()) return;
        const size = await win.innerSize();
        const position = await win.outerPosition();
        onChange({ width: size.width, height: size.height, x: position.x, y: position.y });
      })();
    }, SETTLE_MS);
  };

  const unlistenResized = await win.onResized(record);
  const unlistenMoved = await win.onMoved(record);

  return () => {
    if (timer !== undefined) clearTimeout(timer);
    unlistenResized();
    unlistenMoved();
  };
}
