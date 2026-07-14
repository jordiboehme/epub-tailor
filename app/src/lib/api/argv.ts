// Pure command-line builders for the `epub-tailor` sidecar. No Tauri import:
// the argv is a plain string array the jobs store hands to `spawnSidecar`, and
// every rule here is unit-tested without a window in sight. Flag order is
// fixed so the tests can pin it and a reader can predict it.

import type { StagedEdits } from "./edits";
import { hasAnyEdit } from "./edits";

export interface RunOptions {
  /** Resolved profile specs, composed left to right - one `--profile` pair each. */
  profiles: string[];
  /** `low`/`std`/`high` or a raw number as a string; `null` leaves the profile default. */
  quality: string | null;
  /** `text`/`image`/`image-all`; `null` leaves the profile default. */
  tables: string | null;
  /** Analyze without writing anything. */
  dryRun: boolean;
  /** Staged metadata to write into this book, if any. */
  edits?: StagedEdits;
}

/**
 * The per-field metadata flags for a set of staged edits, or `[]` when there is
 * nothing to write. Always closes with `--metadata-merge replace`: staged edits
 * are the user's explicit intent, and the CLI's default `fill` merge would
 * silently ignore any edit to a value the book already has. `--author` and
 * `--subject` repeat once per entry. The unique identifier is never touched by
 * the CLI, and `--isbn` only ever adds one - both are the CLI's rules, not ours.
 */
export function metadataArgv(edits: StagedEdits | undefined): string[] {
  if (!edits || !hasAnyEdit(edits)) return [];
  const argv: string[] = [];
  if (edits.title) argv.push("--title", edits.title);
  for (const author of edits.authors ?? []) argv.push("--author", author);
  if (edits.language) argv.push("--language", edits.language);
  if (edits.publisher) argv.push("--publisher", edits.publisher);
  if (edits.description) argv.push("--description", edits.description);
  for (const subject of edits.subjects ?? []) argv.push("--subject", subject);
  if (edits.date) argv.push("--date", edits.date);
  if (edits.isbn) argv.push("--isbn", edits.isbn);
  if (edits.series) argv.push("--series", edits.series);
  if (edits.seriesIndex) argv.push("--series-index", edits.seriesIndex);
  if (edits.coverPath) argv.push("--cover", edits.coverPath);
  argv.push("--metadata-merge", "replace");
  return argv;
}

/**
 * The shared body of `fit` and `md`: both take the same flags in the same
 * order. `output === null` means an in-place run (`--lets-get-dangerous`);
 * any other value is written with `-o`.
 */
function convertArgv(
  command: "fit" | "md",
  input: string,
  output: string | null,
  opts: RunOptions,
): string[] {
  const argv = [command, input, "--report", "json"];

  for (const profile of opts.profiles) {
    argv.push("--profile", profile);
  }
  if (opts.quality !== null) {
    argv.push("--quality", opts.quality);
  }
  if (opts.tables !== null) {
    argv.push("--tables", opts.tables);
  }
  if (opts.dryRun) {
    argv.push("--dry-run");
  }

  argv.push(...metadataArgv(opts.edits));

  if (output === null) {
    argv.push("--lets-get-dangerous");
  } else {
    argv.push("-o", output);
  }

  return argv;
}

/** `epub-tailor fit <input> ...`. `output === null` runs in place. */
export function fitArgv(input: string, output: string | null, opts: RunOptions): string[] {
  return convertArgv("fit", input, output, opts);
}

/**
 * `epub-tailor md <input> ...`: same shape as {@link fitArgv}, different verb.
 * Markdown never runs in place, so `output` is expected to be a real path -
 * the planner never hands this a `null`.
 */
export function mdArgv(input: string, output: string | null, opts: RunOptions): string[] {
  return convertArgv("md", input, output, opts);
}

/** `epub-tailor check <input> --report json` plus one `--profile` pair per spec. */
export function checkArgv(input: string, profiles: string[]): string[] {
  const argv = ["check", input, "--report", "json"];
  for (const profile of profiles) {
    argv.push("--profile", profile);
  }
  return argv;
}

/** `epub-tailor metadata show <input> --report json --cover-out <coverOut>`. */
export function showArgv(input: string, coverOut: string): string[] {
  return ["metadata", "show", input, "--report", "json", "--cover-out", coverOut];
}

/** A metadata search, from a book path, some typed fields, or both. */
export interface SearchQuery {
  /** An EPUB to seed the title and author from. Omitted for all-manual queries. */
  input?: string;
  title?: string;
  author?: string;
  isbn?: string;
  limit?: number;
}

/**
 * `epub-tailor metadata search [input] [--title --author --isbn --limit] --report json`.
 * A typed field overrides what the book carries, so passing both `input` and a
 * `--title` is fine - the flag wins.
 */
export function searchArgv(query: SearchQuery): string[] {
  const argv = ["metadata", "search"];
  if (query.input) argv.push(query.input);
  if (query.title) argv.push("--title", query.title);
  if (query.author) argv.push("--author", query.author);
  if (query.isbn) argv.push("--isbn", query.isbn);
  if (query.limit !== undefined) argv.push("--limit", String(query.limit));
  argv.push("--report", "json");
  return argv;
}

/**
 * `epub-tailor metadata fetch <reference> --report json [--cover-out <coverOut>]`.
 * The cover is opt-in: Open Library's metadata is CC0 but its cover images are
 * not, so one is only downloaded when the user asked for it.
 */
export function fetchArgv(reference: string, coverOut?: string): string[] {
  const argv = ["metadata", "fetch", reference, "--report", "json"];
  if (coverOut) argv.push("--cover-out", coverOut);
  return argv;
}
