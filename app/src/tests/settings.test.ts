// settings.json is a plain file on disk: it can be hand-edited, half-written or
// corrupted, and what comes back is not automatically a value the app can run
// on. A parallelism of 0 in particular hands the job pump zero slots forever -
// a batch that sits at "0 of N" and never starts anything. So the numbers are
// clamped on the way in, and this drives that through a faked plugin-store.

import { beforeEach, describe, expect, it, vi } from "vitest";

const stored = new Map<string, unknown>();

vi.mock("@tauri-apps/plugin-store", () => ({
  Store: {
    load: vi.fn(async () => ({
      get: async (key: string) => stored.get(key),
      set: async () => {},
    })),
  },
}));

const { settings } = await import("../lib/stores/settings.svelte");

describe("settings.load", () => {
  beforeEach(() => stored.clear());

  it("keeps a sane persisted parallelism", async () => {
    stored.set("parallelism", 5);
    await settings.load();
    expect(settings.parallelism).toBe(5);
  });

  it("never hands the job pump zero slots", async () => {
    stored.set("parallelism", 0);
    await settings.load();
    expect(settings.parallelism).toBe(1);
  });

  it("caps an absurd parallelism", async () => {
    stored.set("parallelism", 500);
    await settings.load();
    expect(settings.parallelism).toBe(8);
  });

  it("falls back to the default for a parallelism that is not a number", async () => {
    stored.set("parallelism", "lots");
    await settings.load();
    expect(settings.parallelism).toBe(3);
  });

  it("clamps a Markdown split level to a heading the CLI knows", async () => {
    stored.set("mdSplitLevel", 0);
    await settings.load();
    expect(settings.mdSplitLevel).toBe(1);

    stored.set("mdSplitLevel", 9);
    await settings.load();
    expect(settings.mdSplitLevel).toBe(2);

    stored.set("mdSplitLevel", 2);
    await settings.load();
    expect(settings.mdSplitLevel).toBe(2);
  });

  it("keeps the defaults when nothing is persisted yet", async () => {
    await settings.load();
    expect(settings.parallelism).toBe(3);
    expect(settings.mdSplitLevel).toBe(1);
    expect(settings.mode).toBe("fit");
  });

  it("remembers the mode the user left the workbench in", async () => {
    stored.set("mode", "edit");
    await settings.load();
    expect(settings.mode).toBe("edit");
  });

  it("falls back to Fit for a mode that is not a mode", async () => {
    stored.set("mode", "Mode.EDIT");
    await settings.load();
    expect(settings.mode).toBe("fit");
  });
});

// migrateProfileStack itself is covered in settings-migration.test.ts as a
// pure function, and profiles.test.ts covers hasDeviceClash/stackLabel given
// a stack - but nothing drove `load()`'s actual call site, which passes the
// legacy `profile` and `userProfilePaths` keys through to it. A mutation
// there (swapping the argument order, or passing undefined/undefined) left
// all 255 tests green before these: an upgrading user's profile
// configuration would have silently vanished on first launch after the
// update and nothing in the suite would have noticed.
describe("settings.load migrates the legacy profile keys", () => {
  beforeEach(() => stored.clear());

  it("rebuilds profileStack from a single legacy profile and path", async () => {
    stored.set("profile", "x4");
    stored.set("userProfilePaths", ["/home/reader/manga.json"]);
    await settings.load();
    expect(settings.profileStack).toEqual([
      { kind: "builtin", name: "x4" },
      { kind: "file", path: "/home/reader/manga.json" },
    ]);
  });

  it("rebuilds profileStack from multiple legacy paths, in order", async () => {
    stored.set("profile", "generic");
    stored.set("userProfilePaths", ["/home/reader/a.json", "/home/reader/b.json"]);
    await settings.load();
    expect(settings.profileStack).toEqual([
      { kind: "builtin", name: "generic" },
      { kind: "file", path: "/home/reader/a.json" },
      { kind: "file", path: "/home/reader/b.json" },
    ]);
  });

  it("dedupes a legacy path list that repeats the same file", async () => {
    // A duplicate path produces two layers with the same key, and the
    // keyed #each in ProfilePicker throws on a duplicate key at render.
    stored.set("profile", "x4");
    stored.set("userProfilePaths", ["/home/reader/manga.json", "/home/reader/manga.json"]);
    await settings.load();
    expect(settings.profileStack).toEqual([
      { kind: "builtin", name: "x4" },
      { kind: "file", path: "/home/reader/manga.json" },
    ]);
  });

  it("falls back to the epub profile when no legacy keys are stored either", async () => {
    await settings.load();
    expect(settings.profileStack).toEqual([{ kind: "builtin", name: "epub" }]);
  });

  it("ignores a corrupted string profileStack and falls back to the legacy keys", async () => {
    // A string has .length too, so a naive `stored.length > 0` guard would
    // treat this as a real (one-character-per-layer, once iterated) stack.
    stored.set("profileStack", "generic");
    stored.set("profile", "x4");
    stored.set("userProfilePaths", []);
    await settings.load();
    expect(settings.profileStack).toEqual([{ kind: "builtin", name: "x4" }]);
  });

  it("leaves an already-migrated profileStack untouched", async () => {
    stored.set("profileStack", [{ kind: "builtin", name: "generic" }]);
    stored.set("profile", "x4");
    stored.set("userProfilePaths", ["/home/reader/manga.json"]);
    await settings.load();
    expect(settings.profileStack).toEqual([{ kind: "builtin", name: "generic" }]);
  });
});
