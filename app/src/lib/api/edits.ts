// Staged metadata edits: the pure shape and the two pure helpers around it.
// The reactive store (stores/edits.svelte.ts) owns the Map<bookId, StagedEdits>
// and the debounced UI wiring; everything that can be a plain function lives
// here so vitest reaches it without a window - and so argv.ts can build flags
// from it without importing a store.

import type { MetadataDoc } from "./contract";
import { creatorNames, stringList } from "./meta";

/**
 * One book's pending metadata changes, waiting to be written on the next Tailor
 * run (or by "Write metadata only"). Every field is optional: an absent field
 * means "leave the book's value alone", a present one means "write this".
 */
export interface StagedEdits {
  title?: string;
  authors?: string[];
  series?: string;
  seriesIndex?: string;
  publisher?: string;
  description?: string;
  language?: string;
  date?: string;
  isbn?: string;
  subjects?: string[];
  coverPath?: string;
}

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

/** True when the edits carry at least one thing worth writing. */
export function hasAnyEdit(edits: StagedEdits): boolean {
  return Boolean(
    edits.title ||
      edits.authors?.length ||
      edits.series ||
      edits.seriesIndex ||
      edits.publisher ||
      edits.description ||
      edits.language ||
      edits.date ||
      edits.isbn ||
      edits.subjects?.length ||
      edits.coverPath,
  );
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
