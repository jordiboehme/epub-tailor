// Cover image cache paths. Dir creation lives on the Rust side
// (`ensure_covers_dir` in app/src-tauri/src/commands.rs): the fs plugin is
// deliberately not installed (see the task brief), and `@tauri-apps/api/path`
// only resolves paths, it does not create directories - so the one bit of
// filesystem mutation this module needs is a command call, not a new plugin.

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { join } from "@tauri-apps/api/path";

/**
 * A stable cache key for a cover derived from a book at `path`: FNV-1a (a
 * small, dependency-free non-cryptographic hash) over `path|size|modifiedMs`,
 * so a re-fitted or replaced file (different size or mtime) misses the cache
 * instead of showing a stale cover.
 */
export function coverCacheKey(path: string, size: number, modifiedMs: number): string {
  const input = `${path}|${size}|${modifiedMs}`;
  // FNV-1a, 32-bit.
  let hash = 0x811c9dc5;
  for (let i = 0; i < input.length; i += 1) {
    hash ^= input.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

/** The on-disk path for `key`'s cached cover, creating the cache dir if needed. */
export async function coverCachePath(key: string): Promise<string> {
  const dir = await invoke<string>("ensure_covers_dir");
  return join(dir, `${key}.img`);
}

/**
 * Copy a cover the user picked from anywhere on disk into the cover cache, and
 * return the copy's path.
 *
 * The webview can only load images from the cache dir (the asset protocol's
 * scope, see tauri.conf.json), so a path from the user's Pictures folder would
 * stage happily and then render as nothing at all. Everything downstream - the
 * preview, the card, the `--cover` flag - gets the cached path instead, and
 * the CLI reads a copy that is byte-for-byte the image they chose.
 */
export async function cacheCover(source: string): Promise<string> {
  return invoke<string>("cache_cover", { source });
}

/** The `asset:`-protocol URL a webview can load `absolutePath` from directly. */
export function coverUrl(absolutePath: string): string {
  return convertFileSrc(absolutePath);
}
