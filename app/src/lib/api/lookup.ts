// The two network-facing metadata calls - search and fetch - wrapped so the
// dialog can await a plain outcome instead of juggling exit codes and parsers.
// These run through runSidecar (not the job queue): they are interactive, one
// document in and one out, and the caller wants the answer inline.
//
// Every call identifies the app to Open Library through EPUB_TAILOR_USER_AGENT,
// the env var the CLI's HTTP client reads (crates/cli/src/lookup.rs). The
// version in it is the app's own, asked of Tauri at runtime: a hand-synced
// constant would go stale on the next release and quietly start lying to a
// service whose only ask is that we say who we are.

import { getVersion } from "@tauri-apps/api/app";
import { runSidecar } from "./sidecar";
import { fetchArgv, searchArgv } from "./argv";
import type { SearchQuery } from "./argv";
import { isCliFailure, parseFetchedDoc, parseReport } from "./contract";
import type { MetadataDoc, SearchReport } from "./contract";
import { friendlyError } from "./errors";

const HOMEPAGE = "https://github.com/jordiboehme/epub-tailor";

/** How many candidates to ask Open Library for. */
export const SEARCH_LIMIT = 8;

const OFFLINE =
  "We could not reach Open Library. Check your connection and try again - the metadata is still there when you are back.";

let cachedUserAgent: string | null = null;

/** The user agent Open Library sees. Resolved once, from the app's real version. */
export async function lookupUserAgent(): Promise<string> {
  if (cachedUserAgent === null) {
    let version = "0.0.0";
    try {
      version = await getVersion();
    } catch {
      // No window to ask (a unit test, a broken IPC): still identify the app,
      // just without a version worth trusting.
    }
    cachedUserAgent = `epub-tailor-gui/${version} (${HOMEPAGE})`;
  }
  return cachedUserAgent;
}

async function lookupEnv(): Promise<Record<string, string>> {
  return { EPUB_TAILOR_USER_AGENT: await lookupUserAgent() };
}

export type SearchOutcome =
  | { ok: true; report: SearchReport }
  | { ok: false; message: string };

/** Search Open Library. A network or argument error becomes a friendly `ok: false`. */
export async function searchOnline(query: SearchQuery): Promise<SearchOutcome> {
  const argv = searchArgv({ ...query, limit: query.limit ?? SEARCH_LIMIT });
  try {
    const res = await runSidecar(argv, { env: await lookupEnv() });
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
    const res = await runSidecar(argv, { env: await lookupEnv() });
    if (res.code !== 0) return { ok: false, message: OFFLINE };
    return { ok: true, doc: parseFetchedDoc(res.stdout) };
  } catch {
    return { ok: false, message: OFFLINE };
  }
}
