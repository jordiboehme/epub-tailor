// Pure output-path planner: given a batch of books and the user's naming and
// destination settings, decide where each converted EPUB lands. Fit mode
// always produces a copy - in-place writes are Edit mode's business and never
// come out of here. No Tauri import - path math is plain string work so this
// runs, and is tested, outside a window. The one bit of real-world knowledge
// it needs, "does a file already sit here", is injected as `existsOnDisk`
// (backed by the `paths_exist` command in the app).

import { renderTemplate, resolveCollisions } from "./templates";
import type { TemplateBook } from "./templates";

export interface PlannedBook {
  input: string;
  kind: "epub" | "md";
  template: TemplateBook;
}

export interface OutputPlan {
  input: string;
  output: string;
}

export interface PlanOptions {
  /** Filename template, e.g. "{author} - {title}". */
  template: string;
  /** Destination folder, or `null` to write alongside each original. */
  outputDir: string | null;
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
 * - The directory is `outputDir` (or the input's own folder) and the name
 *   comes from the template, always with an `.epub` extension.
 * - If a book's computed output would land on its own input, the appendix is
 *   inserted before the extension (`Dune.epub` -> `Dune.tailored.epub`).
 * - Names are then made unique across the whole batch and against anything
 *   already on disk, case-insensitively, comparing the full directory + name.
 */
export function planOutputs(books: PlannedBook[], opts: PlanOptions): OutputPlan[] {
  // First pass: fixed decisions (dir + stem), independent of collisions.
  interface Draft {
    input: string;
    dir: string;
    stem: string;
  }

  const drafts: Draft[] = books.map((book) => {
    const dir = opts.outputDir ?? dirOf(book.input);
    let stem = renderTemplate(opts.template, book.template);

    // Would this overwrite the book's own input - or render to the input's
    // own name up to sanitization? Insert the appendix either way: in the
    // default "{original}" configuration every book must come out as
    // "<stem>.<appendix>.epub", even when the sanitizer had to alter a stem
    // (writing the altered name bare would just be a silent rename).
    const candidate = dir.length > 0 ? `${dir}/${stem}.epub` : `${stem}.epub`;
    const selfNamed =
      book.kind === "epub" &&
      sameFileKey(dir) === sameFileKey(dirOf(book.input)) &&
      sameFileKey(stem) === sameFileKey(renderTemplate("{original}", book.template));
    if (sameFileKey(candidate) === sameFileKey(book.input) || selfNamed) {
      stem = `${stem}.${opts.appendix}`;
    }

    return { input: book.input, dir, stem };
  });

  // Second pass: resolve collisions over the dir-qualified keys.
  const keys = drafts.map((d) => `${d.dir}${KEY_SEP}${d.stem}`);
  const resolvedKeys = resolveCollisions(keys, {
    existsOnDisk: (key) => opts.existsOnDisk(pathOfKey(key)),
  });

  return drafts.map((d, i) => ({ input: d.input, output: pathOfKey(resolvedKeys[i]) }));
}

/**
 * The file name one book would be written as, for the naming preview: the real
 * planner, run on that book alone, so the preview cannot drift from what the
 * app actually writes.
 *
 * Disk is not consulted (a preview that stats the filesystem on every keystroke
 * would be a poor trade), so the " (2)" a real collision earns is not shown -
 * but the appendix, which the *default* configuration hits on every single
 * book, is.
 */
export function previewOutputName(
  book: PlannedBook,
  opts: Omit<PlanOptions, "existsOnDisk">,
): string {
  const [plan] = planOutputs([book], { ...opts, existsOnDisk: () => false });
  const slash = Math.max(plan.output.lastIndexOf("/"), plan.output.lastIndexOf("\\"));
  return slash >= 0 ? plan.output.slice(slash + 1) : plan.output;
}

/** Rebuild an absolute output path from a `dir\nstem` collision key. */
function pathOfKey(key: string): string {
  const sep = key.indexOf(KEY_SEP);
  const dir = key.slice(0, sep);
  const stem = key.slice(sep + 1);
  return dir.length > 0 ? `${dir}/${stem}.epub` : `${stem}.epub`;
}
