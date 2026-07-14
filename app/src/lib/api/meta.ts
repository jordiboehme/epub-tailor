// Pure metadata shaping: turn a `metadata show` report into the compact view a
// book card needs. Lives here, not in the books store, so the jobs store can
// call it without a store-to-store import cycle - and so it stays trivially
// testable. No Tauri import.

import type { Creator, MetadataShowReport } from "./contract";

export interface BookMeta {
  title?: string;
  authors: string[];
  series?: string;
  seriesIndex?: string;
  /** Field names the book is missing, from `metadata show`. */
  missing: string[];
}

function creatorName(creator: Creator): string {
  return typeof creator === "string" ? creator : creator.name;
}

function toList<T>(value: T | T[] | undefined): T[] {
  if (value === undefined) return [];
  return Array.isArray(value) ? value : [value];
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
