import { describe, expect, it } from "vitest";
import { mergeDocIntoEdits, mergeEditsIntoMeta } from "../lib/api/edits";
import type { StagedEdits } from "../lib/api/edits";
import type { BookMeta } from "../lib/api/meta";
import type { MetadataDoc } from "../lib/api/contract";
import { edits } from "../lib/stores/edits.svelte";

const all = new Set([
  "title",
  "authors",
  "series",
  "seriesIndex",
  "publisher",
  "description",
  "language",
  "date",
  "isbn",
  "subjects",
  "cover",
]);

describe("mergeDocIntoEdits", () => {
  it("pulls every checked field off the document", () => {
    const doc: MetadataDoc = {
      title: "The Hobbit",
      authors: ["J.R.R. Tolkien"],
      series: "Middle-earth",
      series_index: "0",
      publisher: "Allen & Unwin",
      description: "There and back again.",
      language: "en",
      date: "1937",
      isbn: "9780261102217",
      subjects: ["Fantasy", "Adventure"],
      cover: "/cache/fetched-ol1.img",
    };
    expect(mergeDocIntoEdits({}, doc, all)).toEqual({
      title: "The Hobbit",
      authors: ["J.R.R. Tolkien"],
      series: "Middle-earth",
      seriesIndex: "0",
      publisher: "Allen & Unwin",
      description: "There and back again.",
      language: "en",
      date: "1937",
      isbn: "9780261102217",
      subjects: ["Fantasy", "Adventure"],
      coverPath: "/cache/fetched-ol1.img",
    });
  });

  it("normalizes the Creator union to plain names", () => {
    const doc: MetadataDoc = {
      authors: [
        { name: "J.R.R. Tolkien", file_as: "Tolkien, J.R.R.", role: "aut" },
        "Christopher Tolkien",
      ],
    };
    expect(mergeDocIntoEdits({}, doc, new Set(["authors"])).authors).toEqual([
      "J.R.R. Tolkien",
      "Christopher Tolkien",
    ]);
  });

  it("normalizes a single Creator (not a list) to a one-element array", () => {
    const doc: MetadataDoc = { authors: { name: "Ursula K. Le Guin" } };
    expect(mergeDocIntoEdits({}, doc, new Set(["authors"])).authors).toEqual([
      "Ursula K. Le Guin",
    ]);
  });

  it("normalizes a scalar subjects value to a list", () => {
    const doc: MetadataDoc = { subjects: "Fantasy" };
    expect(mergeDocIntoEdits({}, doc, new Set(["subjects"])).subjects).toEqual(["Fantasy"]);
  });

  it("only takes the fields in the set, leaving the rest of existing untouched", () => {
    const existing: StagedEdits = { authors: ["Kept Author"], series: "Kept Series" };
    const doc: MetadataDoc = { title: "New Title", authors: ["Ignored"], series: "Ignored" };
    expect(mergeDocIntoEdits(existing, doc, new Set(["title"]))).toEqual({
      title: "New Title",
      authors: ["Kept Author"],
      series: "Kept Series",
    });
  });

  it("overwrites an existing staged value when its field is checked", () => {
    const existing: StagedEdits = { title: "Old" };
    const doc: MetadataDoc = { title: "New" };
    expect(mergeDocIntoEdits(existing, doc, new Set(["title"])).title).toBe("New");
  });

  it("skips a checked field the document does not carry", () => {
    const doc: MetadataDoc = { title: "Only a title" };
    const result = mergeDocIntoEdits({}, doc, all);
    expect(result).toEqual({ title: "Only a title" });
    expect("publisher" in result).toBe(false);
  });

  it("does not mutate the existing edits object it is handed", () => {
    const existing: StagedEdits = { title: "Old" };
    mergeDocIntoEdits(existing, { title: "New" }, new Set(["title"]));
    expect(existing.title).toBe("Old");
  });

  it("maps series_index onto seriesIndex and cover onto coverPath", () => {
    const doc: MetadataDoc = { series_index: "3", cover: "/c.img" };
    expect(mergeDocIntoEdits({}, doc, all)).toEqual({ seriesIndex: "3", coverPath: "/c.img" });
  });
});

describe("mergeEditsIntoMeta", () => {
  const base: BookMeta = {
    title: "Old Title",
    authors: ["Old Author"],
    subjects: [],
    missing: ["title", "publisher", "subjects"],
  };

  it("overlays the written fields and drops what they filled from missing", () => {
    const merged = mergeEditsIntoMeta(base, {
      title: "New Title",
      publisher: "New Press",
      subjects: ["Fantasy"],
    });
    expect(merged.title).toBe("New Title");
    expect(merged.publisher).toBe("New Press");
    expect(merged.subjects).toEqual(["Fantasy"]);
    expect(merged.missing).toEqual([]);
  });

  it("keeps a field the edits do not mention", () => {
    const merged = mergeEditsIntoMeta(base, { publisher: "New Press" });
    expect(merged.title).toBe("Old Title");
    expect(merged.authors).toEqual(["Old Author"]);
    expect(merged.missing).toEqual(["title", "subjects"]);
  });

  it("builds a sane meta from nothing", () => {
    const merged = mergeEditsIntoMeta(undefined, { title: "Fresh" });
    expect(merged.title).toBe("Fresh");
    expect(merged.authors).toEqual([]);
    expect(merged.subjects).toEqual([]);
    expect(merged.missing).toEqual([]);
  });

  it("does not clear missing for fields with no missing-name (series index, language, cover)", () => {
    const meta: BookMeta = { authors: [], subjects: [], missing: ["title"] };
    const merged = mergeEditsIntoMeta(meta, { seriesIndex: "2", language: "en", coverPath: "/c.img" });
    expect(merged.seriesIndex).toBe("2");
    expect(merged.language).toBe("en");
    expect(merged.missing).toEqual(["title"]);
  });
});

describe("EditsStore flush registry", () => {
  // MetadataEditor debounces staging by ~200ms and registers its own
  // flushPending as a callback here, so a Tailor/write-metadata click (or a
  // selection change) can settle a still-debouncing keystroke instead of
  // losing it. This exercises that registry in isolation from the timer.

  it("runs a registered flush callback, landing a still-pending edit", () => {
    edits.clear();
    const bookId = "book-1";
    // Stands in for MetadataEditor's flushPending(): applies the last typed
    // value that a debounce timer has not committed yet.
    edits.onFlush(() => edits.stage([bookId], { title: "Typed but not yet staged" }));

    expect(edits.get(bookId)).toBeUndefined();
    edits.flushPending();
    expect(edits.get(bookId)?.title).toBe("Typed but not yet staged");
  });

  it("runs every registered callback, not just the first", () => {
    edits.clear();
    edits.onFlush(() => edits.stage(["a"], { title: "A" }));
    edits.onFlush(() => edits.stage(["b"], { title: "B" }));

    edits.flushPending();
    expect(edits.get("a")?.title).toBe("A");
    expect(edits.get("b")?.title).toBe("B");
  });

  it("stops calling a callback once it is unregistered", () => {
    edits.clear();
    let calls = 0;
    const unregister = edits.onFlush(() => calls++);

    edits.flushPending();
    unregister();
    edits.flushPending();

    expect(calls).toBe(1);
  });
});
