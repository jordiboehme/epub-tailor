// The two network-facing metadata calls - search and fetch - wrapped so the
// dialog can await a plain outcome instead of juggling exit codes and parsers.
// These run through runSidecar (not the job queue): they are interactive, one
// document in and one out, and the caller wants the answer inline.
//
// Every call identifies the app to Open Library through EPUB_TAILOR_USER_AGENT,
// the env var the CLI's HTTP client reads (crates/cli/src/lookup.rs). The app
// package.json version is a placeholder 0.0.0, so the string mirrors the
// workspace crate version instead - bump it here when the crate version moves.

import { runSidecar } from "./sidecar";
import { fetchArgv, searchArgv } from "./argv";
import type { SearchQuery } from "./argv";
import { isCliFailure, parseFetchedDoc, parseReport } from "./contract";
import type { MetadataDoc, SearchReport } from "./contract";
import { friendlyError } from "./errors";

const APP_VERSION = "0.4.2";
export const LOOKUP_USER_AGENT = `epub-tailor-gui/${APP_VERSION} (https://github.com/jordiboehme/epub-tailor)`;

/** How many candidates to ask Open Library for. */
export const SEARCH_LIMIT = 8;

const OFFLINE =
  "We could not reach Open Library. Check your connection and try again - the metadata is still there when you are back.";

function lookupEnv(): Record<string, string> {
  return { EPUB_TAILOR_USER_AGENT: LOOKUP_USER_AGENT };
}

export type SearchOutcome =
  | { ok: true; report: SearchReport }
  | { ok: false; message: string };

/** Search Open Library. A network or argument error becomes a friendly `ok: false`. */
export async function searchOnline(query: SearchQuery): Promise<SearchOutcome> {
  const argv = searchArgv({ ...query, limit: query.limit ?? SEARCH_LIMIT });
  try {
    const res = await runSidecar(argv, { env: lookupEnv() });
    if (res.code !== 0) return { ok: false, message: OFFLINE };
    const report = parseReport<SearchReport>(res.stdout, "metadata search");
    if (isCliFailure(report)) return { ok: false, message: friendlyError(report.code, report.message) };
    return { ok: true, report };
  } catch {
    return { ok: false, message: OFFLINE };
  }
}

export type FetchOutcome =
  | { ok: true; doc: MetadataDoc }
  | { ok: false; message: string };

/**
 * Fetch one full record by reference. `coverOut`, when set, downloads the cover
 * image to that path and points the returned doc's `cover` at it. The payload
 * is a bare, schema-less document, so it is read with parseFetchedDoc.
 */
export async function fetchRecord(reference: string, coverOut?: string): Promise<FetchOutcome> {
  const argv = fetchArgv(reference, coverOut);
  try {
    const res = await runSidecar(argv, { env: lookupEnv() });
    if (res.code !== 0) return { ok: false, message: OFFLINE };
    return { ok: true, doc: parseFetchedDoc(res.stdout) };
  } catch {
    return { ok: false, message: OFFLINE };
  }
}
