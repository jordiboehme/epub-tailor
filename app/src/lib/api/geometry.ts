// Is a remembered window position still somewhere a person could reach it?
// Pure rectangle arithmetic, no Tauri import - api/window.ts supplies the real
// monitors, and this decides whether last launch's geometry may be restored
// onto them. Monitors get unplugged, resolutions change, laptops go from a
// desk to a train: a window restored to where the second screen used to be is
// a window the user cannot get back.

/** A monitor (or a window) as a plain physical-pixel rectangle. */
export interface ScreenRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** A window's last size and position, in physical pixels. Persisted by the settings store. */
export interface WindowGeometry {
  width: number;
  height: number;
  x: number;
  y: number;
}

/**
 * How much of the window must land on a monitor for the geometry to count as
 * reachable: enough of the top edge to see it and grab it, and no less.
 */
const MIN_VISIBLE_W = 120;
const MIN_VISIBLE_H = 60;

/** The overlap of two rectangles, as a width and height (zero when they miss). */
function overlap(a: ScreenRect, b: ScreenRect): { width: number; height: number } {
  const width = Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x);
  const height = Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y);
  return { width: Math.max(0, width), height: Math.max(0, height) };
}

/**
 * Whether `geometry` still lands on one of `monitors` with enough of itself
 * showing to be seen and dragged. An empty monitor list is a "no": with nothing
 * known about the screens, the window manager's own default placement is a
 * better guess than a remembered one.
 */
export function fitsOnScreen(geometry: WindowGeometry, monitors: ScreenRect[]): boolean {
  const { width, height, x, y } = geometry;
  if (![width, height, x, y].every(Number.isFinite)) return false;
  if (width < MIN_VISIBLE_W || height < MIN_VISIBLE_H) return false;

  return monitors.some((monitor) => {
    const seen = overlap(geometry, monitor);
    return seen.width >= MIN_VISIBLE_W && seen.height >= MIN_VISIBLE_H;
  });
}
