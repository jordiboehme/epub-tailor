// Pure filename-template engine: token substitution plus sanitization. No
// Tauri import here on purpose - the workbench's live filename preview and
// this module's tests both run it outside a Tauri context.

export interface TemplateBook {
  title?: string;
  authors?: string[];
  series?: string;
  seriesIndex?: string;
  /** The input file's stem (no extension), the fallback when everything else sanitizes away. */
  originalStem: string;
}

/** Characters illegal (or awkward) in a filename on at least one of Windows/macOS/Linux. */
const ILLEGAL_CHARS = /[/\\:*?"<>|\x00-\x1f]/g;

/**
 * Render a filename (without extension) from `template`, substituting the
 * recognized `{token}`s from `book`. Unknown tokens are left as-is, so a typo
 * in a user-edited template stays visible in the live preview rather than
 * silently vanishing.
 */
export function renderTemplate(template: string, book: TemplateBook): string {
  const authors = book.authors ?? [];
  const values: Record<string, string> = {
    title: book.title ?? "",
    author: authors[0] ?? "",
    authors: authors.join(" & "),
    series: book.series ?? "",
    series_index: book.seriesIndex ?? "",
    original: book.originalStem,
  };

  const substituted = template.replace(/\{([a-z_]+)\}/g, (whole, token: string) =>
    token in values ? values[token] : whole,
  );

  const sanitized = sanitizeFilename(substituted);
  return sanitized.length > 0 ? sanitized : book.originalStem;
}

/** Sanitize a rendered filename: strip what filesystems reject, tidy the rest. */
function sanitizeFilename(name: string): string {
  let result = name.replace(ILLEGAL_CHARS, "-");
  // Collapse runs of the same separator - spaces and hyphens independently -
  // so a run of missing/adjacent tokens does not leave "----" or "    "
  // behind, without touching a deliberate " - " a template author wrote.
  result = result.replace(/ {2,}/g, " ").replace(/-{2,}/g, "-");
  result = result.trim();
  // Windows rejects a trailing dot or space; strip them (repeatedly, in case
  // stripping one exposes another).
  result = result.replace(/[. ]+$/, "");
  result = result.slice(0, 120);
  // Re-trim trailing dots and spaces after the cap in case slicing reintroduced them.
  result = result.replace(/[. ]+$/, "");
  return result;
}

/**
 * Suffix duplicate names with " (2)", " (3)", ... so every name in the batch,
 * and every name already on disk, is unique. Comparison is case-insensitive
 * (macOS and Windows filesystems are), and the first occurrence of a name
 * keeps the plain, unsuffixed form.
 */
export function resolveCollisions(
  names: string[],
  opts: { existsOnDisk: (name: string) => boolean },
): string[] {
  const claimed = new Set<string>();
  const result: string[] = [];

  for (const name of names) {
    let candidate = name;
    let n = 2;
    while (claimed.has(candidate.toLowerCase()) || opts.existsOnDisk(candidate)) {
      candidate = `${name} (${n})`;
      n += 1;
    }
    claimed.add(candidate.toLowerCase());
    result.push(candidate);
  }

  return result;
}
