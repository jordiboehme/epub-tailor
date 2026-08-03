// profiles.svelte.ts turns settings.profileStack into what the CLI actually
// needs: a spec per layer (activeProfileSpecs) and a display label
// (stackLabel). Neither call reaches the sidecar, but the module transitively
// imports it, so @tauri-apps/plugin-shell is faked the same way jobs.test.ts
// does - just enough to let the import resolve outside a Tauri runtime.

import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/plugin-shell", () => ({
  Command: { sidecar: vi.fn() },
}));

import { profiles } from "../lib/stores/profiles.svelte";
import { settings } from "../lib/stores/settings.svelte";
import type { ProfileLayer } from "../lib/stores/settings.svelte";

function setStack(stack: ProfileLayer[]): void {
  settings.profileStack = stack;
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
