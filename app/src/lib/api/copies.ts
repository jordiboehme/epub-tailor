// Recognizing a produced copy by its name: `<stem>.<appendix>.epub`, the same
// convention the CLI's own batch skip logic reads (crates/cli/src/batch.rs,
// `output_suffixes`). No Tauri import - pure string math, unit-tested in
// copies.test.ts - so the books store can regroup synchronously at add time.

import type { Profile } from "./contract";

/** The default appendix profiles without one (e.g. `epub`) fall back to. */
const FALLBACK_APPENDIX = "tailored";

export interface ParsedCopyName {
  /** The source's stem: `Dune` out of `Dune.x4.epub`. */
  stem: string;
  /** The matched appendix in its canonical (profile-list) form. */
  appendix: string;
}

/** Normalize for a same-file comparison: unify separators and fold case. */
function fileKey(path: string): string {
  return path.replace(/\\/g, "/").toLowerCase();
}

/** Whether two absolute paths name the same file, per {@link fileKey}. */
export function samePath(a: string, b: string): boolean {
  return fileKey(a) === fileKey(b);
}

/**
 * The appendixes a copy's name may carry: every built-in profile's own, plus
 * the default one profiles without an appendix fall back to.
 */
export function knownAppendixes(builtins: Profile[]): string[] {
  const out = builtins.map((p) => p.appendix).filter((a): a is string => a !== null);
  if (!out.includes(FALLBACK_APPENDIX)) out.push(FALLBACK_APPENDIX);
  return out;
}

/**
 * The built-in profile behind an appendix, or `null` when no built-in claims
 * it (the `tailored` fallback, a user layer's custom appendix).
 */
export function profileForAppendix(appendix: string, builtins: Profile[]): string | null {
  const folded = appendix.toLowerCase();
  return builtins.find((p) => p.appendix?.toLowerCase() === folded)?.name ?? null;
}

/**
 * Parse a file name as a produced copy: `<stem>.<appendix>.epub`, optionally
 * carrying one collision number after the appendix (`Dune.x4 (2).epub` - the
 * planner numbers the end of the stem, which sits after the appendix).
 * `null` when the name does not read as a copy: not an `.epub`, no dot-split
 * appendix, an appendix no profile claims, or nothing left over as a stem.
 * The appendix match is case-insensitive, mirroring the CLI's own
 * `is_prior_output` rule; the canonical form is returned either way.
 */
export function parseCopyName(fileName: string, appendixes: string[]): ParsedCopyName | null {
  const match = /^(.+)\.epub$/i.exec(fileName);
  if (!match) return null;
  // One trailing " (N)" is collision numbering, not name content.
  const base = match[1].replace(/ \(\d+\)$/, "");

  const lastDot = base.lastIndexOf(".");
  if (lastDot <= 0) return null;
  const stem = base.slice(0, lastDot);
  const candidate = base.slice(lastDot + 1).toLowerCase();

  const appendix = appendixes.find((a) => a.toLowerCase() === candidate);
  if (!appendix || stem.length === 0) return null;
  return { stem, appendix };
}

/**
 * The source path a copy's name implies: `<stem>.epub` in the copy's own
 * directory. Recognition is deliberately same-directory only - a stray
 * `Dune.x4.epub` never folds under a `Dune.epub` somewhere else.
 */
export function impliedSourcePath(copyPath: string, stem: string): string {
  const slash = Math.max(copyPath.lastIndexOf("/"), copyPath.lastIndexOf("\\"));
  const dir = slash >= 0 ? copyPath.slice(0, slash) : "";
  return dir.length > 0 ? `${dir}/${stem}.epub` : `${stem}.epub`;
}

/** What {@link planRegroup} needs to know about one file of a book. */
export interface RegroupFile {
  path: string;
  fileName: string;
}

/** What {@link planRegroup} needs to know about one book: its files, original first. */
export interface RegroupBook {
  id: string;
  files: RegroupFile[];
}

/** One fold {@link planRegroup} decided on: this book becomes a file of that one. */
export interface Fold {
  /** The folded (single-file) book. */
  id: string;
  /** The book it folds into. */
  sourceId: string;
  /** The appendix its name carries, canonical form. */
  appendix: string;
}

/**
 * Which books read, by name, as copies of files other books own. Only
 * single-file books fold - a book that already tracks files is someone's
 * folder in its own right, and merging file lists is not a fold. Sources are
 * matched against EVERY file of every book, so a grandchild finds a book
 * that already tracks its parent copy; and each fold is chased through any
 * source that is itself folding, so a whole chain
 * (`Dune.x4.kobo.epub -> Dune.x4.epub -> Dune.epub`) lands on the root book
 * regardless of the order the batch arrived in. Idempotent - the caller
 * reruns it after every add.
 */
export function planRegroup(books: RegroupBook[], appendixes: string[]): Fold[] {
  // Which book owns each file path - all files, copies included.
  const ownerByPath = new Map<string, string>();
  for (const book of books) {
    for (const file of book.files) ownerByPath.set(fileKey(file.path), book.id);
  }

  // Draft pass: every one-file book whose name reads as a copy of a file
  // some other book owns.
  const draft = new Map<string, { sourceId: string; appendix: string }>();
  for (const book of books) {
    if (book.files.length !== 1) continue;
    const parsed = parseCopyName(book.files[0].fileName, appendixes);
    if (!parsed) continue;
    const sourcePath = impliedSourcePath(book.files[0].path, parsed.stem);
    const ownerId = ownerByPath.get(fileKey(sourcePath));
    if (ownerId !== undefined && ownerId !== book.id) {
      draft.set(book.id, { sourceId: ownerId, appendix: parsed.appendix });
    }
  }

  // Chase each fold through sources that are themselves folding. Stems
  // strictly shrink along a chain so a cycle cannot occur, but the guard is
  // cheap insurance against a pathological input.
  const folds: Fold[] = [];
  for (const [id, { sourceId, appendix }] of draft) {
    const seen = new Set([id]);
    let root = sourceId;
    while (draft.has(root) && !seen.has(root)) {
      seen.add(root);
      root = draft.get(root)!.sourceId;
    }
    if (draft.has(root)) continue;
    folds.push({ id, sourceId: root, appendix });
  }
  return folds;
}
