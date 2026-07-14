// Staged metadata edits: the pure shape and the two pure helpers around it.
// The reactive store (stores/edits.svelte.ts) owns the Map<bookId, StagedEdits>
// and the debounced UI wiring; everything that can be a plain function lives
// here so vitest reaches it without a window - and so argv.ts can build flags
// from it without importing a store.

import type { MetadataDoc } from "./contract";
import type { BookMeta } from "./meta";
import { creatorNames, stringList } from "./meta";

/**
 * One book's pending metadata changes, waiting to be written on the next
 * Tailor run (or by "Write metadata only"). Every field is optional: an
 * absent field means "leave the book's value alone", a present one means
 * "write this" - and on the clearable fields, `null` means "remove this from
 * the book" (the CLI's `--clear`). Title, language, isbn and cover can never
 * be null: a book must keep a title and a language, identifiers are only
 * ever added, and a cover is only ever replaced.
 */
export interface StagedEdits {
  title?: string;
  authors?: string[] | null;
  series?: string | null;
  seriesIndex?: string | null;
  publisher?: string | null;
  description?: string | null;
  language?: string;
  date?: string | null;
  isbn?: string;
  subjects?: string[] | null;
  coverPath?: string;
}

/** The fields a staged `null` may clear - the CLI's `--clear` vocabulary. */
export const CLEARABLE_FIELDS: ReadonlySet<keyof StagedEdits> = new Set<keyof StagedEdits>([
  "authors",
  "series",
  "seriesIndex",
  "publisher",
  "description",
  "date",
  "subjects",
]);

/**
 * The checkbox/field vocabulary shared by the editor, the search-accept step and
 * the argv builder. `cover` is the odd one out: it maps to `coverPath` on the
 * edits and to the document's `cover` path.
 */
export type EditField =
  | "title"
  | "authors"
  | "series"
  | "seriesIndex"
  | "publisher"
  | "description"
  | "language"
  | "date"
  | "isbn"
  | "subjects"
  | "cover";

/** How many fields these edits stage - values and clears both count. */
export function countEdits(edits: StagedEdits): number {
  let count = 0;
  for (const key of Object.keys(edits) as (keyof StagedEdits)[]) {
    const value = edits[key];
    if (value === null) count += 1;
    else if (Array.isArray(value)) count += value.length > 0 ? 1 : 0;
    else if (typeof value === "string" && value.trim()) count += 1;
  }
  return count;
}

/** True when the edits carry at least one thing worth writing. */
export function hasAnyEdit(edits: StagedEdits): boolean {
  return countEdits(edits) > 0;
}

function nonEmpty(value: string | undefined): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

/**
 * Fold the `fields` a user ticked in the search-accept step out of a fetched
 * `doc` and onto a copy of their `existing` edits. Only the checked fields are
 * touched, and only when the document actually carries a value for them - a
 * ticked-but-empty field never clobbers what is already staged. The Creator
 * union and the one-or-many `authors`/`subjects` shapes are normalized to the
 * plain string lists the flags need. `existing` is not mutated.
 */
export function mergeDocIntoEdits(
  existing: StagedEdits,
  doc: MetadataDoc,
  fields: Set<string>,
): StagedEdits {
  const next: StagedEdits = { ...existing };
  const take = (field: EditField) => fields.has(field);

  if (take("title") && nonEmpty(doc.title)) next.title = doc.title.trim();
  if (take("authors")) {
    const authors = creatorNames(doc.authors);
    if (authors.length > 0) next.authors = authors;
  }
  if (take("series") && nonEmpty(doc.series)) next.series = doc.series.trim();
  if (take("seriesIndex") && nonEmpty(doc.series_index)) next.seriesIndex = doc.series_index.trim();
  if (take("publisher") && nonEmpty(doc.publisher)) next.publisher = doc.publisher.trim();
  if (take("description") && nonEmpty(doc.description)) next.description = doc.description.trim();
  if (take("language") && nonEmpty(doc.language)) next.language = doc.language.trim();
  if (take("date") && nonEmpty(doc.date)) next.date = doc.date.trim();
  if (take("isbn") && nonEmpty(doc.isbn)) next.isbn = doc.isbn.trim();
  if (take("subjects")) {
    const subjects = stringList(doc.subjects);
    if (subjects.length > 0) next.subjects = subjects;
  }
  if (take("cover") && nonEmpty(doc.cover)) next.coverPath = doc.cover;

  return next;
}

/** Which `missing_fields` name each edit key clears once written (some keys clear none). */
const CLEARS_MISSING: Partial<Record<keyof StagedEdits, string>> = {
  title: "title",
  authors: "authors",
  series: "series",
  publisher: "publisher",
  description: "description",
  date: "date",
  isbn: "isbn",
  subjects: "subjects",
};

/**
 * Fold written edits into a book's compact `BookMeta`, for refreshing a card
 * after an in-place write without re-ingesting. Fields the edits set win; a
 * `null` removes the field. The `missing` list loses what a value just
 * filled and gains what a clear just removed. A cleared series takes its
 * index with it: the book model nests the index under the series.
 */
export function mergeEditsIntoMeta(base: BookMeta | undefined, edits: StagedEdits): BookMeta {
  const meta: BookMeta = base ?? { authors: [], subjects: [], missing: [] };
  const filled = new Set<string>();
  const cleared = new Set<string>();
  for (const key of Object.keys(edits) as (keyof StagedEdits)[]) {
    const name = CLEARS_MISSING[key];
    if (!name) continue;
    if (edits[key] === null) cleared.add(name);
    else filled.add(name);
  }
  const pick = (edit: string | null | undefined, own: string | undefined): string | undefined =>
    edit === null ? undefined : (edit ?? own);
  const seriesGone = edits.series === null;
  return {
    title: edits.title ?? meta.title,
    authors: edits.authors === null ? [] : (edits.authors ?? meta.authors),
    series: pick(edits.series, meta.series),
    seriesIndex: seriesGone ? undefined : pick(edits.seriesIndex, meta.seriesIndex),
    publisher: pick(edits.publisher, meta.publisher),
    description: pick(edits.description, meta.description),
    language: edits.language ?? meta.language,
    date: pick(edits.date, meta.date),
    isbn: edits.isbn ?? meta.isbn,
    subjects: edits.subjects === null ? [] : (edits.subjects ?? meta.subjects),
    missing: [
      ...meta.missing.filter((name) => !filled.has(name) && !cleared.has(name)),
      ...[...cleared].filter((name) => !meta.missing.includes(name)),
    ],
  };
}
