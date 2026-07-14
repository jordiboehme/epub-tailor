// The staged metadata edits, keyed by book id. Nothing here is written to a
// book until the next Tailor run (or "Write metadata only") picks the edits up
// through argv.ts. The pure shape and the merge helper live in api/edits.ts;
// this store is only the reactive Map plus a small batch-friendly API.
//
// Reactivity idiom follows the books store's Sets: mutate a fresh Map and
// reassign, so `$derived` and every reader re-run. A field set to blank is
// pruned rather than stored, and a book with no fields left is dropped, so
// `hasEdits`/`count` stay honest and argv never emits an empty flag.

import type { MetadataDoc } from "../api/contract";
import { hasAnyEdit, mergeDocIntoEdits } from "../api/edits";
import type { StagedEdits } from "../api/edits";

export type { StagedEdits };

/** Drop keys whose value is blank or an empty list; return undefined if nothing is left. */
function prune(edits: StagedEdits): StagedEdits | undefined {
  const next: StagedEdits = {};
  if (edits.title?.trim()) next.title = edits.title.trim();
  if (edits.authors && edits.authors.length > 0) next.authors = edits.authors;
  if (edits.series?.trim()) next.series = edits.series.trim();
  if (edits.seriesIndex?.trim()) next.seriesIndex = edits.seriesIndex.trim();
  if (edits.publisher?.trim()) next.publisher = edits.publisher.trim();
  if (edits.description?.trim()) next.description = edits.description.trim();
  if (edits.language?.trim()) next.language = edits.language.trim();
  if (edits.date?.trim()) next.date = edits.date.trim();
  if (edits.isbn?.trim()) next.isbn = edits.isbn.trim();
  if (edits.subjects && edits.subjects.length > 0) next.subjects = edits.subjects;
  if (edits.coverPath) next.coverPath = edits.coverPath;
  return hasAnyEdit(next) ? next : undefined;
}

class EditsStore {
  #map = $state<Map<string, StagedEdits>>(new Map());
  #flushers = new Set<() => void>();

  /** How many books have staged edits right now. */
  count = $derived(this.#map.size);

  /** The staged edits for a book, or undefined when it has none. */
  get(bookId: string): StagedEdits | undefined {
    return this.#map.get(bookId);
  }

  /** True when this book has anything staged. */
  hasEdits(bookId: string): boolean {
    return this.#map.has(bookId);
  }

  /**
   * Register a callback that commits any edit a UI still has sitting in a
   * debounce timer - there is one metadata editor mounted at a time, but this
   * stays a set instead of a single slot so it never has to assume that.
   * Returns an unregister function for the caller's teardown.
   */
  onFlush(fn: () => void): () => void {
    this.#flushers.add(fn);
    return () => this.#flushers.delete(fn);
  }

  /**
   * Run every registered flush callback. Call this before snapshotFor at
   * each consuming action (Tailor, write metadata only) so a keystroke still
   * inside its debounce window lands here instead of being silently dropped.
   */
  flushPending(): void {
    for (const fn of this.#flushers) fn();
  }

  /**
   * Apply a field patch to one or many books at once (the batch series/author
   * workflow). The patch merges over each book's existing edits; blank values
   * prune their field, and a book left with nothing is dropped.
   */
  stage(bookIds: string[], patch: Partial<StagedEdits>): void {
    const next = new Map(this.#map);
    for (const id of bookIds) {
      const merged = prune({ ...(next.get(id) ?? {}), ...patch });
      if (merged) next.set(id, merged);
      else next.delete(id);
    }
    this.#map = next;
  }

  /** Fold the checked fields of a fetched document onto one book's edits. */
  applyDoc(bookId: string, doc: MetadataDoc, fields: Set<string>): void {
    const next = new Map(this.#map);
    const merged = prune(mergeDocIntoEdits(next.get(bookId) ?? {}, doc, fields));
    if (merged) next.set(bookId, merged);
    else next.delete(bookId);
    this.#map = next;
  }

  /** Drop one field from a book (revert it), or the book's whole entry when no field is given. */
  unstage(bookId: string, field?: keyof StagedEdits): void {
    const current = this.#map.get(bookId);
    if (!current) return;
    const next = new Map(this.#map);
    if (field === undefined) {
      next.delete(bookId);
    } else {
      const { [field]: _dropped, ...rest } = current;
      const merged = prune(rest);
      if (merged) next.set(bookId, merged);
      else next.delete(bookId);
    }
    this.#map = next;
  }

  /**
   * A plain, de-proxied edits lookup for the given books - the shape runFit
   * wants. Snapshotting here is the one place a $state proxy is unwrapped before
   * it can drift toward the sidecar IPC boundary.
   */
  snapshotFor(bookIds: string[]): Record<string, StagedEdits> {
    const out: Record<string, StagedEdits> = {};
    for (const id of bookIds) {
      const staged = this.#map.get(id);
      if (staged) out[id] = $state.snapshot(staged) as StagedEdits;
    }
    return out;
  }

  /** Clear the given books' edits, or every book's when no ids are given. */
  clear(bookIds?: string[]): void {
    if (bookIds === undefined) {
      this.#map = new Map();
      return;
    }
    const next = new Map(this.#map);
    for (const id of bookIds) next.delete(id);
    this.#map = next;
  }
}

export const edits = new EditsStore();
