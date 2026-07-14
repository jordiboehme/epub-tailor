// covers.ts talks to Tauri (a command for the cache dir, the path API to join),
// so both are faked here - the interesting part is the *name* it hands back,
// because the CLI's `--cover` infers an embedded cover's media type from the
// file extension (`media_type_for` in crates/cli/src/main.rs).

import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (command: string) => {
    if (command === "ensure_covers_dir") return "/cache/covers";
    throw new Error(`unexpected command: ${command}`);
  }),
  convertFileSrc: (path: string) => `asset://${path}`,
}));

vi.mock("@tauri-apps/api/path", () => ({
  join: async (...parts: string[]) => parts.join("/"),
}));

const { coverCacheKey, coverCachePath } = await import("../lib/api/covers");

describe("coverCachePath", () => {
  it("names an ingested cover .img: only the webview ever reads it, and it sniffs", async () => {
    expect(await coverCachePath("deadbeef")).toBe("/cache/covers/deadbeef.img");
  });

  it("names a fetched cover with the extension it is asked for", async () => {
    // A cover staged for `fit --cover` must carry a real extension: `.img`
    // would be embedded declared as image/jpeg whatever the bytes are.
    expect(await coverCachePath("fetched-OL123W", "jpg")).toBe("/cache/covers/fetched-OL123W.jpg");
  });
});

describe("coverCacheKey", () => {
  it("changes when the file changes, so a re-fitted book misses the cache", () => {
    const base = coverCacheKey("/books/Dune.epub", 100, 1);
    expect(coverCacheKey("/books/Dune.epub", 100, 1)).toBe(base);
    expect(coverCacheKey("/books/Dune.epub", 101, 1)).not.toBe(base);
    expect(coverCacheKey("/books/Dune.epub", 100, 2)).not.toBe(base);
    expect(base).toMatch(/^[0-9a-f]{8}$/);
  });
});
