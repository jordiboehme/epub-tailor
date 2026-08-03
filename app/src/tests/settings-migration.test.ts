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
});
