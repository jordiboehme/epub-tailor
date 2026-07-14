// The books on the workbench: what was dropped or browsed in, their ingested
// metadata and covers, their conversion results, and the selection. Adding
// paths expands them through the Rust `expand_inputs` command, then queues a
// low-priority metadata/cover ingestion per EPUB so a card can fill in its
// title and thumbnail without blocking anything the user asked for.

import { invoke } from "@tauri-apps/api/core";
import type { CheckReport, CliFailure, Creator, FitReport, MetadataShowReport } from "../api/contract";
import { coverCacheKey, coverCachePath } from "../api/covers";
import { showArgv } from "../api/argv";
import type { TemplateBook } from "../api/templates";
import { settings } from "./settings.svelte";
import { jobs } from "./jobs.svelte";

/** What `expand_inputs` returns for one file. */
interface InputEntry {
  path: string;
  kind: "epub" | "md";
  size: number;
  modified_ms: number;
}

export interface BookMeta {
  title?: string;
  authors: string[];
  series?: string;
  seriesIndex?: string;
  /** Field names the book is missing, from `metadata show`. */
  missing: string[];
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
  result?: PerBookResult;
}

/** The file name (last path segment), splitting on both separators. */
function baseName(path: string): string {
  const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return slash >= 0 ? path.slice(slash + 1) : path;
}

function creatorName(creator: Creator): string {
  return typeof creator === "string" ? creator : creator.name;
}

function toList<T>(value: T | T[] | undefined): T[] {
  if (value === undefined) return [];
  return Array.isArray(value) ? value : [value];
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

/** Turn a `metadata show` report into the compact `BookMeta` a card needs. */
export function normalizeMeta(report: MetadataShowReport): BookMeta {
  const doc = report.metadata;
  return {
    title: doc.title,
    authors: toList(doc.authors).map(creatorName),
    series: doc.series,
    seriesIndex: doc.series_index,
    missing: report.missing,
  };
}

class BooksStore {
  books = $state<Book[]>([]);
  selectedIds = $state<Set<string>>(new Set());
  #anchor: string | null = null;

  selected = $derived(this.books.filter((b) => this.selectedIds.has(b.id)));

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

  remove(ids: string[]): void {
    const drop = new Set(ids);
    this.books = this.books.filter((b) => !drop.has(b.id));
    const next = new Set(this.selectedIds);
    for (const id of ids) next.delete(id);
    this.selectedIds = next;
  }

  clear(): void {
    this.books = [];
    this.selectedIds = new Set();
    this.#anchor = null;
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
