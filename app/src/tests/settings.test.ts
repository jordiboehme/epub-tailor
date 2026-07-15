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

  it("remembers the view the user left the workbench in", async () => {
    stored.set("viewMode", "grid");
    await settings.load();
    expect(settings.viewMode).toBe("grid");
  });

  it("falls back to the list for a view mode that is not a view", async () => {
    stored.set("viewMode", "View.LIST");
    await settings.load();
    expect(settings.viewMode).toBe("list");
  });

  it("keeps the defaults when nothing is persisted yet", async () => {
    await settings.load();
    expect(settings.parallelism).toBe(3);
    expect(settings.mdSplitLevel).toBe(1);
    expect(settings.viewMode).toBe("list");
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
