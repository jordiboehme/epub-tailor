// The books on the workbench. A book is a virtual folder of files: its
// original (always files[0]) plus every tracked copy - what Fit runs produced
// this session, and files recognized as prior outputs when they were added.
// FILES are the selectable, actionable unit: Edit and Fit run on selected
// files, and clicking a book's body stands for its original. Adding paths
// expands them through the Rust `expand_inputs` command, folds recognized
// copies under their sources, then queues a low-priority metadata/cover
// ingestion plus an automatic check per EPUB file.

import { invoke } from "@tauri-apps/api/core";
import type { CheckReport, CliFailure, FitReport, FittedStamp } from "../api/contract";
import { coverCacheKey, coverCachePath } from "../api/covers";
import { checkArgv, showArgv, CLEANUP_PROFILE } from "../api/argv";
import { knownAppendixes, planRegroup, profileForAppendix, samePath } from "../api/copies";
import type { TemplateBook } from "../api/templates";
import type { BookMeta } from "../api/meta";
import { settings } from "./settings.svelte";
import { jobs } from "./jobs.svelte";
import { edits } from "./edits.svelte";
import { profiles } from "./profiles.svelte";

export type { BookMeta };

/** What `expand_inputs` returns for one file. */
interface InputEntry {
  path: string;
  kind: "epub" | "md";
  size: number;
  modified_ms: number;
}

/**
 * A file's conversion or check outcome, shaped for its row to render. A
 * failure carries its own stderr tail rather than pointing at the job that
 * produced it: jobs are pruned when the next batch starts, and a row that
 * still says "failed" has to still be able to say why.
 */
export type PerFileResult =
  | { kind: "fit"; report: FitReport }
  | { kind: "check"; report: CheckReport }
  | { kind: "failed"; failure: CliFailure; friendly: string; stderr: string[] }
  | { kind: "cancelled" };

/**
 * One file of a book: the original, or a tracked copy. Everything that can
 * happen to a file - its ingested metadata, its staged-edit target id, its
 * conversion result, its automatic-check verdict - lives here, so any file
 * can be selected and worked on like any other.
 */
export interface BookFile {
  id: string;
  path: string;
  fileName: string;
  /** Copies are always epub; only an original can be md. */
  kind: "epub" | "md";
  role: "original" | "copy";
  /** The profile that made a copy, when known (stamp or appendix lookup). */
  profile: string | null;
  /** The naming appendix a copy carries, when its name encodes one. */
  appendix: string | null;
  /** Produced by a Fit run this session, or recognized at add time. */
  origin?: "fit" | "recognized";
  size: number;
  modifiedMs: number;
  meta?: BookMeta;
  coverPath?: string;
  ingest: "pending" | "done" | "failed";
  /**
   * Why the ingestion failed, kept on the file rather than looked up from its
   * job: ingestion jobs are pruned once they are done, and a row that says
   * "could not read" has to be able to say *why* for as long as it says it.
   */
  ingestError?: { friendly: string; code: string; stderr: string[] };
  /** The provenance stamp ingest found: set when this file is a fitted one. */
  fitted?: FittedStamp;
  result?: PerFileResult;
  /**
   * The automatic epub-profile check's outcome, driving the "needs cleanup"
   * indicator. Cleared (and re-probed) after an in-place write; never mixed
   * into `result`, which belongs to actions the user explicitly ran.
   */
  cleanup?: CheckReport;
}

/** A book: a virtual folder of files. `files[0]` is always the original. */
export interface Book {
  id: string;
  files: BookFile[];
}

/** The pair every jobs-store entry point takes: a file plus its book. */
export interface FileTarget {
  book: Book;
  file: BookFile;
}

/** The file a book's body stands for: its original, always `files[0]`. */
export function originalOf(book: Book): BookFile {
  return book.files[0];
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

/** Build the template-engine view of a file from its ingested metadata. */
export function toTemplateFile(file: BookFile): TemplateBook {
  return {
    title: file.meta?.title,
    authors: file.meta?.authors ?? [],
    series: file.meta?.series,
    seriesIndex: file.meta?.seriesIndex,
    originalStem: stemOf(file.fileName),
  };
}

class BooksStore {
  books = $state<Book[]>([]);
  selectedFileIds = $state<Set<string>>(new Set());
  /**
   * Why the last add failed, or `null`. Every route in (a drop, the Add
   * buttons, a file the OS handed us) funnels through `addPaths`, and a drop
   * that quietly vanishes because `expand_inputs` rejected is the one outcome
   * a workbench must never have. App.svelte shows this.
   */
  addError = $state<string | null>(null);
  /** The range-selection anchor: a FILE id. */
  #anchor: string | null = null;

  constructor() {
    // The jobs store tracks a produced copy the moment its fit settles; the
    // stat + ingest + check that follow are filesystem work, which lives
    // here - the jobs store stays free of `invoke`.
    jobs.onCopyTracked = (book: Book, file: BookFile) => void this.#statAndIngest(book, file);
  }

  /** Every file, in the order the views render them: books, originals first. */
  allFiles = $derived(this.books.flatMap((b) => b.files));

  selectedFiles = $derived(this.allFiles.filter((f) => this.selectedFileIds.has(f.id)));

  /**
   * What an action acts on: the selected files, or - when nothing is
   * selected - every book's ORIGINAL. Copies are only ever processed when
   * chosen explicitly. One definition, because the ActionBar's buttons and
   * the Inspector's options have to agree on what "these files" means down
   * to the last row.
   */
  targets = $derived<BookFile[]>(
    this.selectedFiles.length > 0 ? this.selectedFiles : this.books.map((b) => b.files[0]),
  );

  /**
   * Expand and add paths (files or folders), deduping against every file
   * already here. A failed expansion is reported through `addError` rather
   * than thrown: every caller fires this and forgets it, and a drop that
   * lands on nothing at all has to say why.
   */
  async addPaths(paths: string[]): Promise<void> {
    let entries: InputEntry[];
    try {
      entries = await invoke<InputEntry[]>("expand_inputs", {
        paths,
        recursive: settings.recursive,
      });
    } catch (err) {
      this.addError = `We could not open what you just added. ${String(err)}`;
      return;
    }
    this.addError = null;

    const addedFileIds = new Set<string>();
    for (const entry of entries) {
      const known = this.books.some((b) => b.files.some((f) => samePath(f.path, entry.path)));
      if (known) continue;
      const book: Book = {
        id: crypto.randomUUID(),
        files: [
          {
            id: crypto.randomUUID(),
            path: entry.path,
            fileName: baseName(entry.path),
            kind: entry.kind,
            role: "original",
            profile: null,
            appendix: null,
            size: entry.size,
            modifiedMs: entry.modified_ms,
            ingest: entry.kind === "epub" ? "pending" : "done",
          },
        ],
      };
      this.books.push(book);
      // Track the proxied element's file (not the local literal) so the job
      // store's write-back reaches the reactive graph.
      addedFileIds.add(this.books[this.books.length - 1].files[0].id);
    }

    // Fold recognized copies under their sources first, then ingest every
    // new epub file wherever the fold moved it - a folded copy is a full
    // citizen with its own metadata, cover and check verdict.
    this.#regroupCopies();
    const fresh: FileTarget[] = [];
    for (const book of this.books) {
      for (const file of book.files) {
        if (addedFileIds.has(file.id) && file.kind === "epub") fresh.push({ book, file });
      }
    }
    for (const t of fresh) {
      void this.#ingestFile(t.book, t.file);
    }
    // Enqueued after the ingests so rows fill their titles and covers before
    // the low-priority lane spends time on lint probes.
    for (const t of fresh) {
      jobs.enqueueAutoCheck(t.book, t.file, checkArgv(t.file.path, [CLEANUP_PROFILE]));
    }
  }

  async #ingestFile(book: Book, file: BookFile): Promise<void> {
    const key = coverCacheKey(file.path, file.size, file.modifiedMs);
    const coverOut = await coverCachePath(key);
    jobs.enqueueIngest(book, file, showArgv(file.path, coverOut));
  }

  /**
   * A copy a fit just produced: stat it (the cover cache key wants size and
   * mtime; a failed stat degrades to zeros, still unique per path), then give
   * it the same ingest + automatic check every added file gets.
   */
  async #statAndIngest(book: Book, file: BookFile): Promise<void> {
    try {
      const entries = await invoke<InputEntry[]>("expand_inputs", {
        paths: [file.path],
        recursive: false,
      });
      if (entries.length > 0) {
        file.size = entries[0].size;
        file.modifiedMs = entries[0].modified_ms;
      }
    } catch {
      // Tolerated: the key falls back to path|0|0.
    }
    await this.#ingestFile(book, file);
    jobs.enqueueAutoCheck(book, file, checkArgv(file.path, [CLEANUP_PROFILE]));
  }

  /**
   * Fold every single-file book whose name reads as a produced copy of a
   * file another book owns (same directory, `<stem>.<appendix>.epub`) into
   * that book's file list; chains land whole on their root (see
   * planRegroup). Runs after every add; order-independent. File ids are
   * stable through a fold, so staged edits and the selection ride along -
   * which is why the emptied shells go through #dropBooks, never remove().
   */
  #regroupCopies(): void {
    const candidates = this.books.map((b) => ({
      id: b.id,
      files: b.files.map((f) => ({ path: f.path, fileName: f.fileName })),
    }));
    const folds = planRegroup(candidates, knownAppendixes(profiles.builtins));
    if (folds.length === 0) return;

    const byId = new Map(this.books.map((b) => [b.id, b]));
    for (const fold of folds) {
      const source = byId.get(fold.sourceId);
      const copyBook = byId.get(fold.id);
      if (!source || !copyBook) continue;
      const file = copyBook.files[0];
      if (source.files.some((f) => samePath(f.path, file.path))) continue;
      file.role = "copy";
      file.appendix = fold.appendix;
      file.profile = file.fitted?.profile ?? profileForAppendix(fold.appendix, profiles.builtins);
      file.origin = "recognized";
      source.files.push(file);
    }
    this.#dropBooks(folds.map((f) => f.id));
  }

  /**
   * Re-run copy recognition over the whole workbench. `addPaths` does this
   * itself; the one other caller is startup, right after the profile list
   * loads - files the OS hands us at launch can arrive before the appendixes
   * are known, and their grouping resolves here.
   */
  regroup(): void {
    this.#regroupCopies();
  }

  /** The book that owns a file, if it is still on the workbench. */
  owningBook(fileId: string): Book | undefined {
    return this.books.find((b) => b.files.some((f) => f.id === fileId));
  }

  /**
   * Pair files with their owning books for the jobs store, dropping any
   * whose book vanished between the click and the run.
   */
  refsFor(files: BookFile[]): FileTarget[] {
    const out: FileTarget[] = [];
    for (const file of files) {
      const book = this.owningBook(file.id);
      if (book) out.push({ book, file });
    }
    return out;
  }

  /**
   * Drop one tracked copy entry from its book. The caller decides what
   * happens to the file itself (the trash action asks the OS, never deletes
   * permanently); this only maintains the list, its edits and the selection.
   * The original cannot be dropped this way - removing the book is the
   * card's remove, a different gesture entirely.
   */
  removeFile(bookId: string, fileId: string): void {
    const book = this.books.find((b) => b.id === bookId);
    if (!book) return;
    const file = book.files.find((f) => f.id === fileId);
    if (!file || file.role === "original") return;
    book.files = book.files.filter((f) => f.id !== fileId);
    edits.clear([fileId]);
    this.#pruneSelection(new Set([fileId]));
  }

  /**
   * Take books off the workbench. Nothing on disk is touched - these are
   * list entries - so this asks no questions. Every contained file's staged
   * edits and selection go too: an edit to a file that is no longer here has
   * nothing left to be written into.
   */
  remove(bookIds: string[]): void {
    const drop = new Set(bookIds);
    const fileIds = new Set<string>();
    for (const book of this.books) {
      if (drop.has(book.id)) {
        for (const file of book.files) fileIds.add(file.id);
      }
    }
    this.books = this.books.filter((b) => !drop.has(b.id));
    edits.clear([...fileIds]);
    this.#pruneSelection(fileIds);
  }

  /**
   * Remove emptied book shells after a fold. Unlike remove(), this clears
   * NEITHER edits nor selection: the shells' files live on under their new
   * parents, ids intact, and whatever the user staged or selected on them
   * must survive the reorganization.
   */
  #dropBooks(bookIds: string[]): void {
    const drop = new Set(bookIds);
    this.books = this.books.filter((b) => !drop.has(b.id));
  }

  #pruneSelection(droppedFileIds: Set<string>): void {
    const next = new Set(this.selectedFileIds);
    for (const id of droppedFileIds) next.delete(id);
    this.selectedFileIds = next;
    if (droppedFileIds.has(this.#anchor ?? "")) this.#anchor = null;
  }

  clear(): void {
    this.books = [];
    this.selectedFileIds = new Set();
    this.#anchor = null;
    edits.clear();
  }

  // -- selection (of files) ----------------------------------------------------

  /** Plain click: select just this file, and anchor a future shift-range here. */
  select(fileId: string): void {
    this.selectedFileIds = new Set([fileId]);
    this.#anchor = fileId;
  }

  /** Cmd/Ctrl-click: toggle this file in or out of the selection. */
  toggle(fileId: string): void {
    const next = new Set(this.selectedFileIds);
    if (next.has(fileId)) next.delete(fileId);
    else next.add(fileId);
    this.selectedFileIds = next;
    this.#anchor = fileId;
  }

  /**
   * Shift-click: select the contiguous range from the anchor to this file,
   * over the flattened order the views render - so the range covers exactly
   * the rows the user sees between the two clicks, copies included.
   */
  range(fileId: string): void {
    const ids = this.allFiles.map((f) => f.id);
    const anchor = this.#anchor ?? ids[0];
    const from = ids.indexOf(anchor);
    const to = ids.indexOf(fileId);
    if (from < 0 || to < 0) {
      this.select(fileId);
      return;
    }
    const [lo, hi] = from <= to ? [from, to] : [to, from];
    this.selectedFileIds = new Set(ids.slice(lo, hi + 1));
  }

  /**
   * Select every book's ORIGINAL: Cmd-A means "act on every book", and
   * copies stay opt-in, exactly like the no-selection default.
   */
  selectAll(): void {
    this.selectedFileIds = new Set(this.books.map((b) => b.files[0].id));
  }

  clearSelection(): void {
    this.selectedFileIds = new Set();
    this.#anchor = null;
  }
}

export const books = new BooksStore();
