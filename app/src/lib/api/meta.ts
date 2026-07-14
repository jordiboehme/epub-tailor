// Pure metadata shaping: turn a `metadata show` report into the compact view a
// book card needs, plus the small normalizers (Creator-union, one-or-many) that
// the staged-edits merge reuses. Lives here, not in the books store, so the jobs
// store can call it without a store-to-store import cycle - and so it stays
// trivially testable. No Tauri import.

import type { Creator, MetadataDoc, MetadataIdentifier, MetadataShowReport } from "./contract";

export interface BookMeta {
  title?: string;
  authors: string[];
  series?: string;
  seriesIndex?: string;
  publisher?: string;
  description?: string;
  language?: string;
  date?: string;
  isbn?: string;
  subjects: string[];
  /** Field names the book is missing, from `metadata show`. */
  missing: string[];
}

/** The plain display name of one `Creator`, dropping any file-as or role. */
export function creatorName(creator: Creator): string {
  return typeof creator === "string" ? creator : creator.name;
}

function toList<T>(value: T | T[] | undefined): T[] {
  if (value === undefined) return [];
  return Array.isArray(value) ? value : [value];
}

/** Normalize `authors`/`contributors` (single-or-list, Creator-union) to plain names. */
export function creatorNames(value: Creator | Creator[] | undefined): string[] {
  return toList(value).map(creatorName);
}

/** Normalize `subjects` (single-or-list of strings) to a plain string list. */
export function stringList(value: string | string[] | undefined): string[] {
  return toList(value);
}

/** The ISBN a document carries, whether in its `isbn` field or an identifier. */
export function isbnOf(doc: MetadataDoc): string | undefined {
  if (doc.isbn) return doc.isbn;
  const ident = (doc.identifiers ?? []).find(
    (i: MetadataIdentifier) => i.scheme?.toLowerCase() === "isbn",
  );
  return ident?.value;
}

/** Turn a `metadata show` report into the compact `BookMeta` a card needs. */
export function normalizeMeta(report: MetadataShowReport): BookMeta {
  const doc = report.metadata;
  return {
    title: doc.title,
    authors: creatorNames(doc.authors),
    series: doc.series,
    seriesIndex: doc.series_index,
    publisher: doc.publisher,
    description: doc.description,
    language: doc.language,
    date: doc.date,
    isbn: isbnOf(doc),
    subjects: stringList(doc.subjects),
    missing: report.missing,
  };
}
