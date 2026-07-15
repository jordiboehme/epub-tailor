// Tests for the shared per-book display logic (title/subtitle fallbacks,
// initials, failure/finding shaping, status chips) that both the gallery
// card and the upcoming list row consume. Follows the fixture style of
// jobs.test.ts and templates.test.ts.

import { describe, expect, it } from "vitest";
import {
  fileAuthors,
  fileByline,
  fileInitials,
  fileSeries,
  fileSubtitle,
  fileTitle,
  fileYear,
  chipsFor,
  copyBadge,
  effectiveMeta,
  failureOf,
  findingsOf,
  needsCleanup,
  TONE_CLASS,
} from "../lib/api/book-view";
import type { BookFile, BookMeta } from "../lib/stores/books.svelte";
import type { CheckReport, FitReport, FittedStamp, Stats } from "../lib/api/contract";
import type { StagedEdits } from "../lib/api/edits";

function makeFile(overrides: Partial<BookFile> = {}): BookFile {
  return {
    id: "1",
    path: "/tmp/book.epub",
    kind: "epub",
    fileName: "book.epub",
    role: "original",
    profile: null,
    appendix: null,
    size: 100,
    modifiedMs: 0,
    ingest: "done",
    ...overrides,
  };
}

function fitted(overrides: Partial<FittedStamp> = {}): FittedStamp {
  return {
    stamp: "x4 0.4.2",
    appendix: "x4",
    version: "0.4.2",
    profile: "x4",
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

describe("fileTitle", () => {
  it("uses the metadata title when present", () => {
    const book = makeFile({ meta: meta({ title: "Real Title" }) });
    expect(fileTitle(book)).toBe("Real Title");
  });

  it("falls back to the file name's stem when there is no title", () => {
    const book = makeFile({ fileName: "some-book.epub" });
    expect(fileTitle(book)).toBe("some-book");
  });

  it("falls back to the stem when the metadata title is blank", () => {
    const book = makeFile({ fileName: "some-book.epub", meta: meta({ title: "   " }) });
    expect(fileTitle(book)).toBe("some-book");
  });
});

describe("fileSubtitle", () => {
  it("uses the first author", () => {
    const book = makeFile({ meta: meta({ authors: ["Jane Author", "Other Writer"] }) });
    expect(fileSubtitle(book)).toBe("Jane Author");
  });

  it("labels a markdown book with no author", () => {
    const book = makeFile({ kind: "md", fileName: "notes.md" });
    expect(fileSubtitle(book)).toBe("Markdown");
  });

  it("is empty for an epub with no author", () => {
    expect(fileSubtitle(makeFile())).toBe("");
  });
});

describe("fileSeries", () => {
  it("pairs the series with its position", () => {
    const book = makeFile({ meta: meta({ series: "Dune", seriesIndex: "2" }) });
    expect(fileSeries(book)).toBe("Dune #2");
  });

  it("is the series alone when there is no position", () => {
    expect(fileSeries(makeFile({ meta: meta({ series: "Dune" }) }))).toBe("Dune");
  });

  it("ignores a blank position", () => {
    const book = makeFile({ meta: meta({ series: "Dune", seriesIndex: "  " }) });
    expect(fileSeries(book)).toBe("Dune");
  });

  it("is empty when the book has no series", () => {
    expect(fileSeries(makeFile({ meta: meta({ series: "   " }) }))).toBe("");
    expect(fileSeries(makeFile())).toBe("");
  });
});

describe("fileByline", () => {
  it("joins the author and the series", () => {
    const book = makeFile({
      meta: meta({ authors: ["Frank Herbert"], series: "Dune", seriesIndex: "2" }),
    });
    expect(fileByline(book)).toBe("Frank Herbert · Dune #2");
  });

  it("is the author alone when there is no series", () => {
    expect(fileByline(makeFile({ meta: meta({ authors: ["Jane Author"] }) }))).toBe("Jane Author");
  });

  it("is the series alone when there is no author", () => {
    expect(fileByline(makeFile({ meta: meta({ series: "Dune" }) }))).toBe("Dune");
  });

  it("is empty when the book has neither", () => {
    expect(fileByline(makeFile())).toBe("");
  });
});

describe("fileInitials", () => {
  it("takes the first letter of a one-word stem", () => {
    expect(fileInitials(makeFile({ fileName: "Dune.epub" }))).toBe("D");
  });

  it("takes the first letters of up to two words of a multi-word stem", () => {
    expect(fileInitials(makeFile({ fileName: "The Great Gatsby.epub" }))).toBe("TG");
  });
});

describe("findingsOf", () => {
  const finding = { severity: "warning" as const, code: "W1", message: "m", path: null };

  it("returns the check report's findings", () => {
    const findings = [finding];
    const book = makeFile({
      result: { kind: "check", report: checkReport({ findings, warnings: 1 }) },
    });
    expect(findingsOf(book)).toBe(findings);
  });

  it("is undefined outside a check result", () => {
    expect(findingsOf(makeFile())).toBeUndefined();
  });

  it("falls back to the automatic check's findings", () => {
    const findings = [finding];
    const book = makeFile({ cleanup: checkReport({ findings, warnings: 1 }) });
    expect(findingsOf(book)).toBe(findings);
  });

  it("prefers an explicit check over the automatic one", () => {
    const explicit = [finding];
    const auto = [{ ...finding, code: "AUTO" }];
    const book = makeFile({
      result: { kind: "check", report: checkReport({ findings: explicit, warnings: 1 }) },
      cleanup: checkReport({ findings: auto, warnings: 1 }),
    });
    expect(findingsOf(book)).toBe(explicit);
  });

  it("says nothing for a clean automatic check", () => {
    expect(findingsOf(makeFile({ cleanup: checkReport() }))).toBeUndefined();
  });
});

describe("needsCleanup", () => {
  const finding = { severity: "warning" as const, code: "W1", message: "m", path: null };

  it("is true only when the automatic check found something", () => {
    expect(needsCleanup(makeFile())).toBe(false);
    expect(needsCleanup(makeFile({ cleanup: checkReport() }))).toBe(false);
    expect(
      needsCleanup(makeFile({ cleanup: checkReport({ findings: [finding], warnings: 1 }) })),
    ).toBe(true);
  });
});

describe("copyBadge", () => {
  const APPENDIXES = ["x4", "tailored"];

  it("badges a copy-named file, preferring the stamp's profile for the text", () => {
    const book = makeFile({ fileName: "Dune.x4.epub", fitted: fitted() });
    expect(copyBadge(book, APPENDIXES)).toBe("x4");
  });

  it("badges a copy-named file even without a stamp, from the appendix", () => {
    expect(copyBadge(makeFile({ fileName: "Dune.x4.epub" }), APPENDIXES)).toBe("x4");
  });

  it("never badges a normally-named file, whatever its stamp says", () => {
    // A stamp proves the file was fitted, not that it is a copy: a book
    // fitted *in place* (the old "Replace originals", or the CLI) carries a
    // device-profile stamp and is still the user's only original.
    expect(copyBadge(makeFile({ fitted: fitted() }), APPENDIXES)).toBeNull();
    expect(
      copyBadge(makeFile({ fitted: fitted({ profile: null }) }), APPENDIXES),
    ).toBeNull();
    expect(copyBadge(makeFile(), APPENDIXES)).toBeNull();
  });

  it("falls back to the parsed appendix when the stamp names the repair profile", () => {
    // A cleaned copy gets re-stamped with `epub`; its name still says x4.
    const book = makeFile({
      fileName: "Dune.x4.epub",
      fitted: fitted({ profile: "epub", appendix: "tailored" }),
    });
    expect(copyBadge(book, APPENDIXES)).toBe("x4");
  });

  it("ignores unknown appendixes", () => {
    expect(copyBadge(makeFile({ fileName: "Dune.x5.epub" }), APPENDIXES)).toBeNull();
  });
});

describe("failureOf", () => {
  it("shapes a failed conversion", () => {
    const book = makeFile({
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
    const book = makeFile({
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
    expect(failureOf(makeFile({ ingest: "failed" }))).toBeUndefined();
  });

  it("is undefined when nothing failed", () => {
    expect(failureOf(makeFile())).toBeUndefined();
  });
});

describe("chipsFor", () => {
  it("shows a shrink percentage with a from-to size title", () => {
    const book = makeFile({
      result: { kind: "fit", report: fitReport({ stats: stats({ bytes_in: 1000, bytes_out: 500 }) }) },
    });
    expect(chipsFor(book)).toEqual([{ label: "-50%", tone: "good", title: "1000 B to 500 B" }]);
  });

  it("shows a wrote-size chip when the output grew", () => {
    const book = makeFile({
      result: { kind: "fit", report: fitReport({ stats: stats({ bytes_in: 500, bytes_out: 800 }) }) },
    });
    expect(chipsFor(book)).toEqual([{ label: "wrote 800 B", tone: "neutral" }]);
  });

  it("shows a wrote-size chip when the output is the same size", () => {
    const book = makeFile({
      result: { kind: "fit", report: fitReport({ stats: stats({ bytes_in: 500, bytes_out: 500 }) }) },
    });
    expect(chipsFor(book)).toEqual([{ label: "wrote 500 B", tone: "neutral" }]);
  });

  it("adds a preview chip on a dry run", () => {
    const book = makeFile({
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
    const book = makeFile({
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
    const book = makeFile({ result: { kind: "check", report: checkReport({ errors: 2, warnings: 1 }) } });
    expect(chipsFor(book)).toEqual([
      { label: "2 errors", tone: "bad" },
      { label: "1 warnings", tone: "warn" },
    ]);
  });

  it("shows clean when a check finds nothing", () => {
    const book = makeFile({ result: { kind: "check", report: checkReport() } });
    expect(chipsFor(book)).toEqual([{ label: "clean", tone: "good" }]);
  });

  it("flags a book the automatic check wants cleaned", () => {
    const finding = { severity: "warning" as const, code: "W1", message: "m", path: null };
    const book = makeFile({ cleanup: checkReport({ findings: [finding], warnings: 1 }) });
    expect(chipsFor(book)).toEqual([
      {
        id: "needs-cleanup",
        label: "needs cleanup",
        tone: "warn",
        title: "1 finding from the automatic check",
      },
    ]);
  });

  it("keeps the cleanup flag next to a fit result but not over a check or failure", () => {
    const finding = { severity: "warning" as const, code: "W1", message: "m", path: null };
    const cleanup = checkReport({ findings: [finding], warnings: 1 });

    const fitted = makeFile({ cleanup, result: { kind: "fit", report: fitReport() } });
    expect(chipsFor(fitted)).toContainEqual(expect.objectContaining({ label: "needs cleanup" }));

    // An explicit check shows its own findings; a failure has bigger problems.
    const checked = makeFile({ cleanup, result: { kind: "check", report: checkReport() } });
    expect(chipsFor(checked)).not.toContainEqual(
      expect.objectContaining({ label: "needs cleanup" }),
    );
    const failed = makeFile({
      cleanup,
      result: { kind: "failed", failure: { code: "x", message: "m" }, friendly: "f", stderr: [] },
    });
    expect(chipsFor(failed)).not.toContainEqual(
      expect.objectContaining({ label: "needs cleanup" }),
    );
  });

  it("shows failed", () => {
    const book = makeFile({
      result: { kind: "failed", failure: { code: "E", message: "m" }, friendly: "f", stderr: [] },
    });
    expect(chipsFor(book)).toEqual([{ label: "failed", tone: "bad" }]);
  });

  it("shows cancelled", () => {
    const book = makeFile({ result: { kind: "cancelled" } });
    expect(chipsFor(book)).toEqual([{ label: "cancelled", tone: "neutral" }]);
  });

  it("shows could not read for a book that failed ingestion", () => {
    const book = makeFile({ ingest: "failed" });
    expect(chipsFor(book)).toEqual([{ label: "could not read", tone: "bad" }]);
  });

  it("is empty when there is nothing to report yet", () => {
    expect(chipsFor(makeFile())).toEqual([]);
  });
});

describe("TONE_CLASS", () => {
  it("has a Tailwind class for every tone", () => {
    expect(Object.keys(TONE_CLASS).sort()).toEqual(["bad", "good", "neutral", "warn"]);
  });
});

describe("staged-aware display helpers", () => {
  const book = makeFile({
    meta: meta({
      title: "Dune Messiah",
      authors: ["Frank Herbert", "Brian Herbert"],
      series: "Dune",
      seriesIndex: "2",
      date: "1969-07-15",
    }),
  });

  it("without staged edits everything reads from the book", () => {
    expect(fileTitle(book)).toBe("Dune Messiah");
    expect(fileAuthors(book)).toBe("Frank Herbert, Brian Herbert");
    expect(fileSeries(book)).toBe("Dune #2");
    expect(fileYear(book)).toBe("1969");
  });

  it("staged values win over the book's own", () => {
    const staged: StagedEdits = { title: "Dune II", date: "1970" };
    expect(fileTitle(book, staged)).toBe("Dune II");
    expect(fileYear(book, staged)).toBe("1970");
    expect(fileSeries(book, staged)).toBe("Dune #2");
  });

  it("a staged series clear hides the series and its index", () => {
    expect(fileSeries(book, { series: null })).toBe("");
  });

  it("a staged authors clear empties the author line", () => {
    expect(fileAuthors(book, { authors: null })).toBe("");
    expect(fileSubtitle(book, { authors: null })).toBe("");
  });

  it("fileYear finds the year inside a fuller date and stays quiet without one", () => {
    expect(fileYear(makeFile({ meta: meta({ date: "September 1937" }) }))).toBe("1937");
    expect(fileYear(makeFile({ meta: meta({}) }))).toBe("");
  });

  it("effectiveMeta without staged edits is the book's own meta object", () => {
    expect(effectiveMeta(book)).toBe(book.meta);
    expect(effectiveMeta(makeFile({}))).toBeUndefined();
  });
});
