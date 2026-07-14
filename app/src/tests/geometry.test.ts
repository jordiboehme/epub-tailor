import { describe, expect, it } from "vitest";
import { fitsOnScreen } from "../lib/api/geometry";
import type { ScreenRect } from "../lib/api/geometry";

/** One 1920x1080 monitor at the origin, and a second one to its right. */
const main: ScreenRect = { x: 0, y: 0, width: 1920, height: 1080 };
const secondary: ScreenRect = { x: 1920, y: 0, width: 1920, height: 1080 };

describe("fitsOnScreen", () => {
  it("accepts a window sitting comfortably on a monitor", () => {
    expect(fitsOnScreen({ x: 100, y: 100, width: 1100, height: 720 }, [main])).toBe(true);
  });

  it("accepts a window on the second monitor of a two-monitor desk", () => {
    expect(fitsOnScreen({ x: 2100, y: 80, width: 1100, height: 720 }, [main, secondary])).toBe(true);
  });

  it("rejects that same window once the second monitor is gone", () => {
    expect(fitsOnScreen({ x: 2100, y: 80, width: 1100, height: 720 }, [main])).toBe(false);
  });

  it("rejects a window parked entirely above or left of every monitor", () => {
    expect(fitsOnScreen({ x: -1500, y: 100, width: 1100, height: 720 }, [main])).toBe(false);
    expect(fitsOnScreen({ x: 100, y: -900, width: 1100, height: 720 }, [main])).toBe(false);
  });

  it("accepts a window hanging off the edge as long as a real grab handle is left", () => {
    // Bottom-right corner of the main monitor: a strip is still on screen.
    expect(fitsOnScreen({ x: 1600, y: 800, width: 1100, height: 720 }, [main])).toBe(true);
  });

  it("rejects a sliver too small to grab", () => {
    expect(fitsOnScreen({ x: 1910, y: 100, width: 1100, height: 720 }, [main])).toBe(false);
  });

  it("rejects a nonsense or degenerate size", () => {
    expect(fitsOnScreen({ x: 0, y: 0, width: 0, height: 0 }, [main])).toBe(false);
    expect(fitsOnScreen({ x: 0, y: 0, width: Number.NaN, height: 720 }, [main])).toBe(false);
  });

  it("rejects everything when the monitor list is empty, rather than guessing", () => {
    expect(fitsOnScreen({ x: 100, y: 100, width: 1100, height: 720 }, [])).toBe(false);
  });
});
