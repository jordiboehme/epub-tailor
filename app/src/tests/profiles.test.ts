// profiles.svelte.ts turns settings.profileStack into what the CLI actually
// needs: a spec per layer (activeProfileSpecs) and a display label
// (stackLabel). Neither call reaches the sidecar, but the module transitively
// imports it, so @tauri-apps/plugin-shell is faked the same way jobs.test.ts
// does - just enough to let the import resolve outside a Tauri runtime.

import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-shell", () => ({
  Command: { sidecar: vi.fn() },
}));

import {
  profiles,
  baseName,
  addBuiltinLayer,
  addFileLayer,
  removeLayerAt,
  moveLayer,
  hasDeviceClash,
  hasScreen,
} from "../lib/stores/profiles.svelte";
import { settings } from "../lib/stores/settings.svelte";
import type { ProfileLayer } from "../lib/stores/settings.svelte";
import type { Profile } from "../lib/api/contract";

function setStack(stack: ProfileLayer[]): void {
  settings.profileStack = stack;
}

// The real CLI's `DeviceCaps::permissive()` sentinel for "no screen at all":
// u32::MAX, not 0. `generic` and `epub` both report this from the real
// `profiles --report json` (see contract.test.ts's PROFILES_JSON fixture).
// Defaulting the test helper to this instead of {w:0,h:0} is deliberate: a
// {w:0,h:0} default let earlier tests pass while encoding a premise (that
// screen-less profiles report 0) the real CLI does not share.
const NO_SCREEN = 4294967295;

/** A minimal built-in `Profile`, enough to exercise `hasDeviceClash`. Defaults
 * to the real device-neutral sentinel, not a device profile's shape. */
function builtin(name: string, screen: { w: number; h: number } = { w: NO_SCREEN, h: NO_SCREEN }): Profile {
  return {
    name,
    description: "",
    caps: {
      screen_w: screen.w,
      screen_h: screen.h,
      ppi: 0,
      panel: "gray4",
      max_src_px: [0, 0],
      inline_max: [0, 0],
      cover_max: [0, 0],
      inline_budget_bytes: 0,
      cover_budget_bytes: 0,
      css_max_bytes: 0,
      css_max_rules: 0,
    },
    features: {
      strip_fonts: false,
      filter_css: false,
      sanitize_css: false,
      relocate_styles: false,
      transcode_images: false,
      rasterize_svg: false,
      linearize_tables: false,
      degrade_boxes: false,
      bake_ordered_lists: false,
      preserve_code_blocks: false,
      normalize_footnotes: false,
      relocate_anchors: false,
      dedupe_ids: false,
      unicode_hygiene: false,
      chapter_split: false,
    },
    jpeg_quality: 0,
    tables: "text",
    split_tall_images: false,
    max_chapter_bytes: 0,
    appendix: null,
    filters: [],
  };
}

describe("activeProfileSpecs", () => {
  it("maps a mixed stack to specs in order: built-ins to their bare name, files to their full path", () => {
    setStack([
      { kind: "builtin", name: "x4" },
      { kind: "file", path: "/home/reader/manga.json" },
      { kind: "builtin", name: "generic" },
    ]);
    expect(profiles.activeProfileSpecs()).toEqual(["x4", "/home/reader/manga.json", "generic"]);
  });
});

describe("stackLabel", () => {
  it("renders a single-element stack as a bare name, not 'x4+' or '[x4]'", () => {
    setStack([{ kind: "builtin", name: "x4" }]);
    expect(profiles.stackLabel()).toBe("x4");
  });

  it("joins multiple layers with +", () => {
    setStack([
      { kind: "builtin", name: "x4" },
      { kind: "builtin", name: "generic" },
    ]);
    expect(profiles.stackLabel()).toBe("x4+generic");
  });

  it("uses a file layer's basename, never its full path", () => {
    setStack([
      { kind: "builtin", name: "x4" },
      { kind: "file", path: "/home/reader/manga.json" },
    ]);
    expect(profiles.stackLabel()).toBe("x4+manga.json");
    expect(profiles.stackLabel()).not.toContain("/home/reader");
  });
});

describe("baseName", () => {
  it("keeps the last path segment across either separator", () => {
    expect(baseName("/home/reader/manga.json")).toBe("manga.json");
    expect(baseName("C:\\Users\\reader\\manga.json")).toBe("manga.json");
  });

  it("returns a bare filename unchanged", () => {
    expect(baseName("manga.json")).toBe("manga.json");
  });
});

describe("addBuiltinLayer", () => {
  it("appends a new built-in layer", () => {
    const stack: ProfileLayer[] = [{ kind: "builtin", name: "epub" }];
    expect(addBuiltinLayer(stack, "x4")).toEqual([
      { kind: "builtin", name: "epub" },
      { kind: "builtin", name: "x4" },
    ]);
  });

  it("refuses a built-in already in the stack, returning the same reference", () => {
    const stack: ProfileLayer[] = [{ kind: "builtin", name: "x4" }];
    expect(addBuiltinLayer(stack, "x4")).toBe(stack);
  });
});

describe("addFileLayer", () => {
  it("appends a new file layer", () => {
    const stack: ProfileLayer[] = [{ kind: "builtin", name: "epub" }];
    expect(addFileLayer(stack, "/home/reader/manga.json")).toEqual([
      { kind: "builtin", name: "epub" },
      { kind: "file", path: "/home/reader/manga.json" },
    ]);
  });

  it("refuses a path already in the stack, returning the same reference", () => {
    const stack: ProfileLayer[] = [{ kind: "file", path: "/home/reader/manga.json" }];
    expect(addFileLayer(stack, "/home/reader/manga.json")).toBe(stack);
  });
});

describe("removeLayerAt", () => {
  it("removes the layer at the given index", () => {
    const stack: ProfileLayer[] = [
      { kind: "builtin", name: "x4" },
      { kind: "builtin", name: "generic" },
    ];
    expect(removeLayerAt(stack, 0)).toEqual([{ kind: "builtin", name: "generic" }]);
  });

  it("refuses to empty the last remaining layer, returning the same reference", () => {
    const stack: ProfileLayer[] = [{ kind: "builtin", name: "epub" }];
    expect(removeLayerAt(stack, 0)).toBe(stack);
  });
});

describe("moveLayer", () => {
  it("swaps a layer with its upward neighbour", () => {
    const stack: ProfileLayer[] = [
      { kind: "builtin", name: "x4" },
      { kind: "builtin", name: "generic" },
    ];
    expect(moveLayer(stack, 1, -1)).toEqual([
      { kind: "builtin", name: "generic" },
      { kind: "builtin", name: "x4" },
    ]);
  });

  it("swaps a layer with its downward neighbour", () => {
    const stack: ProfileLayer[] = [
      { kind: "builtin", name: "x4" },
      { kind: "builtin", name: "generic" },
    ];
    expect(moveLayer(stack, 0, 1)).toEqual([
      { kind: "builtin", name: "generic" },
      { kind: "builtin", name: "x4" },
    ]);
  });

  it("refuses to move past either end, returning the same reference", () => {
    const stack: ProfileLayer[] = [
      { kind: "builtin", name: "x4" },
      { kind: "builtin", name: "generic" },
    ];
    expect(moveLayer(stack, 0, -1)).toBe(stack);
    expect(moveLayer(stack, 1, 1)).toBe(stack);
  });
});

describe("hasDeviceClash", () => {
  const x4 = builtin("x4", { w: 758, h: 1024 });
  const kobo = builtin("kobo", { w: 1264, h: 1680 });
  const generic = builtin("generic");
  const epub = builtin("epub");
  const builtins = [x4, kobo, generic, epub];

  it("is false for a single device profile", () => {
    expect(hasDeviceClash([{ kind: "builtin", name: "x4" }], builtins)).toBe(false);
  });

  it("is false pairing a device profile with generic, which has no screen", () => {
    expect(
      hasDeviceClash(
        [
          { kind: "builtin", name: "x4" },
          { kind: "builtin", name: "generic" },
        ],
        builtins,
      ),
    ).toBe(false);
  });

  it("is false pairing a device profile with epub, which has no screen", () => {
    expect(
      hasDeviceClash(
        [
          { kind: "builtin", name: "epub" },
          { kind: "builtin", name: "x4" },
        ],
        builtins,
      ),
    ).toBe(false);
  });

  it("is true when two device profiles are both in the stack", () => {
    expect(
      hasDeviceClash(
        [
          { kind: "builtin", name: "x4" },
          { kind: "builtin", name: "kobo" },
        ],
        builtins,
      ),
    ).toBe(true);
  });

  it("ignores a file layer, which is not resolvable to caps without the CLI", () => {
    expect(
      hasDeviceClash(
        [
          { kind: "builtin", name: "x4" },
          { kind: "file", path: "/home/reader/custom.json" },
        ],
        builtins,
      ),
    ).toBe(false);
  });

  it("is false for the flagship [epub, generic, x4] stack, using the real u32::MAX sentinel", () => {
    // The exact composition the review measured as a false positive: every
    // stack a user builds by adding generic to the app's default [epub]
    // stack showed the clash warning before this fix.
    expect(
      hasDeviceClash(
        [
          { kind: "builtin", name: "epub" },
          { kind: "builtin", name: "generic" },
          { kind: "builtin", name: "x4" },
        ],
        builtins,
      ),
    ).toBe(false);
  });

});

describe("hasScreen", () => {
  it("is false for the u32::MAX device-neutral sentinel", () => {
    expect(hasScreen({ screen_w: 4294967295, screen_h: 4294967295 })).toBe(false);
  });

  it("is false for a zero screen, defensively", () => {
    expect(hasScreen({ screen_w: 0, screen_h: 0 })).toBe(false);
  });

  it("is true for a real device screen", () => {
    expect(hasScreen({ screen_w: 758, screen_h: 1024 })).toBe(true);
  });
});
