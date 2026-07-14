// Pure output-path planner: given a batch of books and the user's naming,
// destination and in-place settings, decide where each converted EPUB lands
// (or that it is written in place). No Tauri import - path math is plain string
// work so this runs, and is tested, outside a window. The one bit of real-world
// knowledge it needs, "does a file already sit here", is injected as
// `existsOnDisk` (backed by the `paths_exist` command in the app).

import { renderTemplate, resolveCollisions } from "./templates";
import type { TemplateBook } from "./templates";

export interface PlannedBook {
  input: string;
  kind: "epub" | "md";
  template: TemplateBook;
}

export interface OutputPlan {
  input: string;
  /** `null` => written in place (epub only). */
  output: string | null;
}

export interface PlanOptions {
  /** Filename template, e.g. "{author} - {title}". */
  template: string;
  /** Destination folder, or `null` to write alongside each original. */
  outputDir: string | null;
  /** In-place mode (epub only); md books are always given an output path. */
  inPlace: boolean;
  /** The active profile's appendix, inserted when an output would hit its own input. */
  appendix: string;
  /** Whether a file already exists at a given absolute path. */
  existsOnDisk: (path: string) => boolean;
}

/** Directory part of a path, splitting on both `/` and `\`. Empty when there is none. */
function dirOf(path: string): string {
  const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return slash >= 0 ? path.slice(0, slash) : "";
}

/** Normalize for a same-file comparison: unify separators and fold case. */
function sameFileKey(path: string): string {
  return path.replace(/\\/g, "/").toLowerCase();
}

// A directory-qualified collision key. `resolveCollisions` numbers the *end* of
// the string, so putting the stem last lets its " (2)" suffix land on the stem
// and not on the directory or extension. The separator is a newline, which
// `renderTemplate` strips from any stem it produces, so it can never collide
// with real filename content.
const KEY_SEP = "\n";

/**
 * Plan an output path for every book in the batch.
 *
 * - In-place: epub books get `output: null`; md books still get a real path
 *   (Markdown has no in-place mode).
 * - Otherwise the directory is `outputDir` (or the input's own folder) and the
 *   name comes from the template, always with an `.epub` extension.
 * - If a book's computed output would land on its own input, the appendix is
 *   inserted before the extension (`Dune.epub` -> `Dune.tailored.epub`).
 * - Names are then made unique across the whole batch and against anything
 *   already on disk, case-insensitively, comparing the full directory + name.
 */
export function planOutputs(books: PlannedBook[], opts: PlanOptions): OutputPlan[] {
  // First pass: fixed decisions (in-place vs a dir+stem), independent of collisions.
  interface Draft {
    input: string;
    inPlace: boolean;
    dir: string;
    stem: string;
  }

  const drafts: Draft[] = books.map((book) => {
    if (opts.inPlace && book.kind === "epub") {
      return { input: book.input, inPlace: true, dir: "", stem: "" };
    }

    const dir = opts.outputDir ?? dirOf(book.input);
    let stem = renderTemplate(opts.template, book.template);

    // Would this overwrite the book's own input? Insert the appendix if so.
    const candidate = dir.length > 0 ? `${dir}/${stem}.epub` : `${stem}.epub`;
    if (sameFileKey(candidate) === sameFileKey(book.input)) {
      stem = `${stem}.${opts.appendix}`;
    }

    return { input: book.input, inPlace: false, dir, stem };
  });

  // Second pass: resolve collisions over the dir-qualified keys of the books
  // that actually produce a file. In-place books do not participate.
  const producing = drafts.filter((d) => !d.inPlace);
  const keys = producing.map((d) => `${d.dir}${KEY_SEP}${d.stem}`);
  const resolvedKeys = resolveCollisions(keys, {
    existsOnDisk: (key) => opts.existsOnDisk(pathOfKey(key)),
  });

  // Stitch resolved names back onto the drafts, in order.
  const resolvedByInput = new Map<string, string>();
  producing.forEach((d, i) => {
    resolvedByInput.set(d.input, pathOfKey(resolvedKeys[i]));
  });

  return drafts.map((d) =>
    d.inPlace
      ? { input: d.input, output: null }
      : { input: d.input, output: resolvedByInput.get(d.input)! },
  );
}

/** Rebuild an absolute output path from a `dir\nstem` collision key. */
function pathOfKey(key: string): string {
  const sep = key.indexOf(KEY_SEP);
  const dir = key.slice(0, sep);
  const stem = key.slice(sep + 1);
  return dir.length > 0 ? `${dir}/${stem}.epub` : `${stem}.epub`;
}
