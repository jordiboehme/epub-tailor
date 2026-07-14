// Tests for the shared per-book display logic (title/subtitle fallbacks,
// initials, failure/finding shaping, status chips) that both the gallery
// card and the upcoming list row consume. Follows the fixture style of
// jobs.test.ts and templates.test.ts.

import { describe, expect, it } from "vitest";
import {
  bookInitials,
  bookSubtitle,
  bookTitle,
  chipsFor,
  failureOf,
  findingsOf,
  TONE_CLASS,
  writtenPathOf,
} from "../lib/api/book-view";
import type { Book, BookMeta } from "../lib/stores/books.svelte";
import type { CheckReport, FitReport, Stats } from "../lib/api/contract";

function makeBook(overrides: Partial<Book> = {}): Book {
  return {
    id: "1",
    path: "/tmp/book.epub",
    kind: "epub",
    fileName: "book.epub",
    size: 100,
    modifiedMs: 0,
    ingest: "done",
    ...overrides,
  };
}

function meta(overrides: Partial<BookMeta> = {}): BookMeta {
  return {
    authors: [],
    subjects: [],
    missing: [],
    ...overrides,
  };
}

function stats(overrides: Partial<Stats> = {}): Stats {
  return {
    bytes_in: 1000,
    bytes_out: 800,
    images_processed: 0,
    chapters: 1,
    chapters_split: 0,
    warnings: 0,
    ...overrides,
  };
}

function fitReport(overrides: Partial<FitReport> = {}): FitReport {
  return {
    schema: 1,
    output: "/tmp/out.epub",
    dry_run: false,
    transformations: [],
    warnings: [],
    stats: stats(),
    ...overrides,
  };
}

function checkReport(overrides: Partial<CheckReport> = {}): CheckReport {
  return {
    schema: 1,
    findings: [],
    errors: 0,
    warnings: 0,
    ...overrides,
  };
}

describe("bookTitle", () => {
  it("uses the metadata title when present", () => {
    const book = makeBook({ meta: meta({ title: "Real Title" }) });
    expect(bookTitle(book)).toBe("Real Title");
  });

  it("falls back to the file name's stem when there is no title", () => {
    const book = makeBook({ fileName: "some-book.epub" });
    expect(bookTitle(book)).toBe("some-book");
  });

  it("falls back to the stem when the metadata title is blank", () => {
    const book = makeBook({ fileName: "some-book.epub", meta: meta({ title: "   " }) });
    expect(bookTitle(book)).toBe("some-book");
  });
});

describe("bookSubtitle", () => {
  it("uses the first author", () => {
    const book = makeBook({ meta: meta({ authors: ["Jane Author", "Other Writer"] }) });
    expect(bookSubtitle(book)).toBe("Jane Author");
  });

  it("labels a markdown book with no author", () => {
    const book = makeBook({ kind: "md", fileName: "notes.md" });
    expect(bookSubtitle(book)).toBe("Markdown");
  });

  it("is empty for an epub with no author", () => {
    expect(bookSubtitle(makeBook())).toBe("");
  });
});

describe("bookInitials", () => {
  it("takes the first letter of a one-word stem", () => {
    expect(bookInitials(makeBook({ fileName: "Dune.epub" }))).toBe("D");
  });

  it("takes the first letters of up to two words of a multi-word stem", () => {
    expect(bookInitials(makeBook({ fileName: "The Great Gatsby.epub" }))).toBe("TG");
  });
});

describe("writtenPathOf", () => {
  it("is null on a dry run", () => {
    const book = makeBook({
      result: { kind: "fit", report: fitReport({ dry_run: true, output: null }) },
    });
    expect(writtenPathOf(book)).toBeNull();
  });

  it("is the output path for a real conversion", () => {
    const book = makeBook({
      result: { kind: "fit", report: fitReport({ output: "/tmp/x.epub" }) },
    });
    expect(writtenPathOf(book)).toBe("/tmp/x.epub");
  });

  it("is null when there is no fit result at all", () => {
    expect(writtenPathOf(makeBook())).toBeNull();
  });
});

describe("findingsOf", () => {
  it("returns the check report's findings", () => {
    const findings = [{ severity: "warning" as const, code: "W1", message: "m", path: null }];
    const book = makeBook({
      result: { kind: "check", report: checkReport({ findings, warnings: 1 }) },
    });
    expect(findingsOf(book)).toBe(findings);
  });

  it("is undefined outside a check result", () => {
    expect(findingsOf(makeBook())).toBeUndefined();
  });
});

describe("failureOf", () => {
  it("shapes a failed conversion", () => {
    const book = makeBook({
      result: {
        kind: "failed",
        failure: { code: "E_BAD", message: "boom" },
        friendly: "That did not go well.",
        stderr: ["line1"],
      },
    });
    expect(failureOf(book)).toEqual({
      friendly: "That did not go well.",
      code: "E_BAD",
      stderr: ["line1"],
    });
  });

  it("shapes a failed ingest", () => {
    const book = makeBook({
      ingest: "failed",
      ingestError: { friendly: "Could not open it.", code: "E_IO", stderr: [] },
    });
    expect(failureOf(book)).toEqual({
      friendly: "Could not open it.",
      code: "E_IO",
      stderr: [],
    });
  });

  it("is undefined for a failed ingest with no recorded error", () => {
    expect(failureOf(makeBook({ ingest: "failed" }))).toBeUndefined();
  });

  it("is undefined when nothing failed", () => {
    expect(failureOf(makeBook())).toBeUndefined();
  });
});

describe("chipsFor", () => {
  it("shows a shrink percentage with a from-to size title", () => {
    const book = makeBook({
      result: { kind: "fit", report: fitReport({ stats: stats({ bytes_in: 1000, bytes_out: 500 }) }) },
    });
    expect(chipsFor(book)).toEqual([{ label: "-50%", tone: "good", title: "1000 B to 500 B" }]);
  });

  it("shows a wrote-size chip when the output grew", () => {
    const book = makeBook({
      result: { kind: "fit", report: fitReport({ stats: stats({ bytes_in: 500, bytes_out: 800 }) }) },
    });
    expect(chipsFor(book)).toEqual([{ label: "wrote 800 B", tone: "neutral" }]);
  });

  it("shows a wrote-size chip when the output is the same size", () => {
    const book = makeBook({
      result: { kind: "fit", report: fitReport({ stats: stats({ bytes_in: 500, bytes_out: 500 }) }) },
    });
    expect(chipsFor(book)).toEqual([{ label: "wrote 500 B", tone: "neutral" }]);
  });

  it("adds a preview chip on a dry run", () => {
    const book = makeBook({
      result: {
        kind: "fit",
        report: fitReport({ dry_run: true, output: null, stats: stats({ bytes_in: 1000, bytes_out: 500 }) }),
      },
    });
    expect(chipsFor(book)).toEqual([
      { label: "-50%", tone: "good", title: "1000 B to 500 B" },
      { label: "preview", tone: "neutral" },
    ]);
  });

  it("adds a warnings chip alongside the size chip", () => {
    const book = makeBook({
      result: {
        kind: "fit",
        report: fitReport({ stats: stats({ bytes_in: 1000, bytes_out: 500, warnings: 3 }) }),
      },
    });
    expect(chipsFor(book)).toEqual([
      { label: "-50%", tone: "good", title: "1000 B to 500 B" },
      { label: "3 warnings", tone: "warn" },
    ]);
  });

  it("shows errors and warnings for a check", () => {
    const book = makeBook({ result: { kind: "check", report: checkReport({ errors: 2, warnings: 1 }) } });
    expect(chipsFor(book)).toEqual([
      { label: "2 errors", tone: "bad" },
      { label: "1 warnings", tone: "warn" },
    ]);
  });

  it("shows clean when a check finds nothing", () => {
    const book = makeBook({ result: { kind: "check", report: checkReport() } });
    expect(chipsFor(book)).toEqual([{ label: "clean", tone: "good" }]);
  });

  it("shows failed", () => {
    const book = makeBook({
      result: { kind: "failed", failure: { code: "E", message: "m" }, friendly: "f", stderr: [] },
    });
    expect(chipsFor(book)).toEqual([{ label: "failed", tone: "bad" }]);
  });

  it("shows cancelled", () => {
    const book = makeBook({ result: { kind: "cancelled" } });
    expect(chipsFor(book)).toEqual([{ label: "cancelled", tone: "neutral" }]);
  });

  it("shows could not read for a book that failed ingestion", () => {
    const book = makeBook({ ingest: "failed" });
    expect(chipsFor(book)).toEqual([{ label: "could not read", tone: "bad" }]);
  });

  it("is empty when there is nothing to report yet", () => {
    expect(chipsFor(makeBook())).toEqual([]);
  });
});

describe("TONE_CLASS", () => {
  it("has a Tailwind class for every tone", () => {
    expect(Object.keys(TONE_CLASS).sort()).toEqual(["bad", "good", "neutral", "warn"]);
  });
});
