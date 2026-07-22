// Window-level Tauri glue: geometry across launches (the pure decision lives
// in api/geometry.ts) and the close guard (its pure decision lives in
// api/close-guard.ts). This module only holds the window handle, the monitor
// list, the events and the native dialog.

import {
  PhysicalPosition,
  PhysicalSize,
  availableMonitors,
  getCurrentWindow,
} from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import { fitsOnScreen } from "./geometry";
import type { ScreenRect, WindowGeometry } from "./geometry";
import { closePrompt } from "./close-guard";
import type { WorkbenchLoad } from "./close-guard";

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

/**
 * Ask before the window closes while work is on the bench. Covers the red
 * button, Cmd+W and (via the Rust-side Quit menu swap) Cmd+Q - a confirmed
 * close of this single window exits the app on its own. Requires the
 * `core:window:allow-destroy` capability: once this listener is registered,
 * every close goes through the JS wrapper's destroy(), even with an empty
 * workbench. `load` is sampled at close time; `onClosing` runs once the user
 * has confirmed, just before the window is destroyed.
 */
export async function guardClose(
  load: () => WorkbenchLoad,
  onClosing?: () => void,
): Promise<UnlistenFn> {
  const win = getCurrentWindow();
  let asking = false;

  return win.onCloseRequested(async (event) => {
    if (asking) {
      // A second Cmd+W/Cmd+Q while the dialog is up must not stack dialogs.
      event.preventDefault();
      return;
    }
    const prompt = closePrompt(load());
    if (!prompt) return; // nothing at stake: the close proceeds

    asking = true;
    try {
      const ok = await ask(prompt.message, {
        title: prompt.title,
        kind: "warning",
        okLabel: "Close",
        cancelLabel: "Keep working",
      });
      if (!ok) event.preventDefault();
      else onClosing?.();
    } catch {
      // The dialog could not be shown; trapping the user in an uncloseable
      // window would be worse than closing unasked.
      onClosing?.();
    } finally {
      asking = false;
    }
  });
}
