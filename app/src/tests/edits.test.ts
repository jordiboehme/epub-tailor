import { describe, expect, it } from "vitest";
import { mergeDocIntoEdits } from "../lib/api/edits";
import type { StagedEdits } from "../lib/api/edits";
import type { MetadataDoc } from "../lib/api/contract";

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
