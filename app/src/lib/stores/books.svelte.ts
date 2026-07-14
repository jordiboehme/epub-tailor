// The books on the workbench: what was dropped or browsed in, their ingested
// metadata and covers, their conversion results, and the selection. Adding
// paths expands them through the Rust `expand_inputs` command, then queues a
// low-priority metadata/cover ingestion per EPUB so a card can fill in its
// title and thumbnail without blocking anything the user asked for.

import { invoke } from "@tauri-apps/api/core";
import type { CheckReport, CliFailure, FitReport } from "../api/contract";
import { coverCacheKey, coverCachePath } from "../api/covers";
import { showArgv } from "../api/argv";
import type { TemplateBook } from "../api/templates";
import type { BookMeta } from "../api/meta";
import { settings } from "./settings.svelte";
import { jobs } from "./jobs.svelte";
import { edits } from "./edits.svelte";

export type { BookMeta };

/** What `expand_inputs` returns for one file. */
interface InputEntry {
  path: string;
  kind: "epub" | "md";
  size: number;
  modified_ms: number;
}

/** A book's conversion or check outcome, shaped for the card to render. */
export type PerBookResult =
  | { kind: "fit"; report: FitReport }
  | { kind: "check"; report: CheckReport }
  | { kind: "failed"; failure: CliFailure; friendly: string }
  | { kind: "cancelled" };

export interface Book {
  id: string;
  path: string;
  kind: "epub" | "md";
  fileName: string;
  size: number;
  modifiedMs: number;
  meta?: BookMeta;
  coverPath?: string;
  ingest: "pending" | "done" | "failed";
  /**
   * Why the ingestion failed, kept on the book rather than looked up from its
   * job: ingestion jobs are pruned once they are done, and a card that says
   * "could not read" has to be able to say *why* for as long as it says it.
   */
  ingestError?: { friendly: string; code: string; stderr: string[] };
  result?: PerBookResult;
}

/** The file name (last path segment), splitting on both separators. */
function baseName(path: string): string {
  const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return slash >= 0 ? path.slice(slash + 1) : path;
}

/** The input stem (file name without its extension). */
export function stemOf(fileName: string): string {
  return fileName.replace(/\.[^.]+$/, "");
}

/** Build the template-engine view of a book from its ingested metadata. */
export function toTemplateBook(book: Book): TemplateBook {
  return {
    title: book.meta?.title,
    authors: book.meta?.authors ?? [],
    series: book.meta?.series,
    seriesIndex: book.meta?.seriesIndex,
    originalStem: stemOf(book.fileName),
  };
}

class BooksStore {
  books = $state<Book[]>([]);
  selectedIds = $state<Set<string>>(new Set());
  #anchor: string | null = null;

  selected = $derived(this.books.filter((b) => this.selectedIds.has(b.id)));

  /**
   * What an action acts on: the selection, or - when nothing is selected -
   * every book on the workbench. One definition, because the ActionBar's
   * buttons and the Inspector's options have to agree on what "these books"
   * means down to the last card.
   */
  targets = $derived(this.selected.length > 0 ? this.selected : this.books);

  /** Expand and add paths (files or folders), deduping against what is already here. */
  async addPaths(paths: string[]): Promise<void> {
    const entries = await invoke<InputEntry[]>("expand_inputs", {
      paths,
      recursive: settings.recursive,
    });
    for (const entry of entries) {
      if (this.books.some((b) => b.path === entry.path)) continue;
      const book: Book = {
        id: crypto.randomUUID(),
        path: entry.path,
        kind: entry.kind,
        fileName: baseName(entry.path),
        size: entry.size,
        modifiedMs: entry.modified_ms,
        ingest: entry.kind === "epub" ? "pending" : "done",
      };
      this.books.push(book);
      // Operate on the proxied element (not the local literal) so the job
      // store's write-back reaches the reactive graph.
      const stored = this.books[this.books.length - 1];
      if (stored.kind === "epub") {
        void this.#ingest(stored);
      }
    }
  }

  async #ingest(book: Book): Promise<void> {
    const key = coverCacheKey(book.path, book.size, book.modifiedMs);
    const coverOut = await coverCachePath(key);
    jobs.enqueueIngest(book, showArgv(book.path, coverOut));
  }

  /**
   * Take books off the workbench. Nothing on disk is touched - these are list
   * entries - so this asks no questions. Their staged edits go with them: an
   * edit to a book that is no longer here has nothing left to be written into.
   */
  remove(ids: string[]): void {
    const drop = new Set(ids);
    this.books = this.books.filter((b) => !drop.has(b.id));
    const next = new Set(this.selectedIds);
    for (const id of ids) next.delete(id);
    this.selectedIds = next;
    if (drop.has(this.#anchor ?? "")) this.#anchor = null;
    edits.clear(ids);
  }

  clear(): void {
    this.books = [];
    this.selectedIds = new Set();
    this.#anchor = null;
    edits.clear();
  }

  // -- selection --------------------------------------------------------------

  /** Plain click: select just this book, and anchor a future shift-range here. */
  select(id: string): void {
    this.selectedIds = new Set([id]);
    this.#anchor = id;
  }

  /** Cmd/Ctrl-click: toggle this book in or out of the selection. */
  toggle(id: string): void {
    const next = new Set(this.selectedIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    this.selectedIds = next;
    this.#anchor = id;
  }

  /** Shift-click: select the contiguous range from the anchor to this book. */
  range(id: string): void {
    const ids = this.books.map((b) => b.id);
    const anchor = this.#anchor ?? ids[0];
    const from = ids.indexOf(anchor);
    const to = ids.indexOf(id);
    if (from < 0 || to < 0) {
      this.select(id);
      return;
    }
    const [lo, hi] = from <= to ? [from, to] : [to, from];
    this.selectedIds = new Set(ids.slice(lo, hi + 1));
  }

  selectAll(): void {
    this.selectedIds = new Set(this.books.map((b) => b.id));
  }

  clearSelection(): void {
    this.selectedIds = new Set();
    this.#anchor = null;
  }
}

export const books = new BooksStore();
