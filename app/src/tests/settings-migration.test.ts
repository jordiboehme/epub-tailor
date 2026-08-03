import { describe, expect, it } from "vitest";
import { migrateProfileStack, type ProfileLayer } from "../lib/stores/settings.svelte";

describe("profile stack migration", () => {
  it("builds a stack from the old two-key shape", () => {
    const stack = migrateProfileStack(undefined, "x4", ["/tmp/manga.json"]);
    expect(stack).toEqual<ProfileLayer[]>([
      { kind: "builtin", name: "x4" },
      { kind: "file", path: "/tmp/manga.json" },
    ]);
  });

  it("keeps an existing stack untouched", () => {
    const existing: ProfileLayer[] = [{ kind: "builtin", name: "generic" }];
    expect(migrateProfileStack(existing, "x4", [])).toEqual(existing);
  });

  it("falls back to the epub profile when nothing is stored", () => {
    expect(migrateProfileStack(undefined, undefined, undefined)).toEqual([
      { kind: "builtin", name: "epub" },
    ]);
  });

  it("rebuilds from the legacy keys when the stored stack is empty, not just absent", () => {
    // A zero-layer stack is never a valid composition, so it is treated the
    // same as no stack at all rather than accepted as "the user's choice".
    const stack = migrateProfileStack([], "x4", ["/tmp/manga.json"]);
    expect(stack).toEqual<ProfileLayer[]>([
      { kind: "builtin", name: "x4" },
      { kind: "file", path: "/tmp/manga.json" },
    ]);
  });

  it("dedupes a legacy path list that repeats the same file", () => {
    // A duplicate survives into two layers with the same layerKey, which
    // throws Svelte's each_key_duplicate at render in ProfilePicker.
    const stack = migrateProfileStack(undefined, "x4", [
      "/tmp/manga.json",
      "/tmp/manga.json",
    ]);
    expect(stack).toEqual<ProfileLayer[]>([
      { kind: "builtin", name: "x4" },
      { kind: "file", path: "/tmp/manga.json" },
    ]);
  });

  it("keeps distinct paths even when one repeats and another does not", () => {
    const stack = migrateProfileStack(undefined, "x4", [
      "/tmp/a.json",
      "/tmp/b.json",
      "/tmp/a.json",
    ]);
    expect(stack).toEqual<ProfileLayer[]>([
      { kind: "builtin", name: "x4" },
      { kind: "file", path: "/tmp/a.json" },
      { kind: "file", path: "/tmp/b.json" },
    ]);
  });
});
