// Tests for the shared per-book display logic (title/subtitle fallbacks,
// initials, failure/finding shaping, status chips) that the list row
// consumes. Follows the fixture style of jobs.test.ts and templates.test.ts.

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
  isFixable,
  concernOf,
  conditionActions,
  repairProfiles,
  conditionSummary,
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
    // Explicit rather than defaulted: every test then states whether the
    // automatic check has run, which is what `fileCondition` keys off.
    check: "done",
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

  it("shapes a fit report's warnings as findings", () => {
    const book = makeFile({
      result: {
        kind: "fit",
        report: fitReport({
          warnings: [
            { message: "could not parse the CSS in style.css; dropped it", file: "style.css" },
            { message: "cover image is enormous", file: "cover.jpg" },
            { message: "empty chapter", file: null },
          ],
          stats: stats({ warnings: 3 }),
        }),
      },
    });
    expect(findingsOf(book)).toEqual([
      // The message already names the file, so nothing is appended.
      {
        severity: "warning",
        code: "",
        message: "could not parse the CSS in style.css; dropped it",
        path: "style.css",
      },
      { severity: "warning", code: "", message: "cover image is enormous (cover.jpg)", path: "cover.jpg" },
      { severity: "warning", code: "", message: "empty chapter", path: null },
    ]);
  });

  it("falls back to the automatic check when a fit had no warnings", () => {
    const findings = [{ severity: "info" as const, code: "I1", message: "m", path: null }];
    const book = makeFile({
      result: { kind: "fit", report: fitReport() },
      cleanup: checkReport({ findings, warnings: 1 }),
    });
    expect(findingsOf(book)).toBe(findings);
    expect(findingsOf(makeFile({ result: { kind: "fit", report: fitReport() } }))).toBeUndefined();
  });

  it("prefers a fit's own warnings over the automatic check", () => {
    const auto = [{ severity: "info" as const, code: "I1", message: "auto", path: null }];
    const book = makeFile({
      result: {
        kind: "fit",
        report: fitReport({
          warnings: [{ message: "empty chapter", file: null }],
          stats: stats({ warnings: 1 }),
        }),
      },
      cleanup: checkReport({ findings: auto, warnings: 1 }),
    });
    expect(findingsOf(book)).toEqual([
      { severity: "warning", code: "", message: "empty chapter", path: null },
    ]);
  });
});

describe("concernOf", () => {
  const f = (code: string, category?: string, severity = "warning") =>
    ({ severity, code, category, message: "m", path: null }) as never;

  it("maps every real category to the concern the UI acts on", () => {
    expect(concernOf(f("manifest-sync", "structure"))).toBe("defect");
    expect(concernOf(f("watermark-identifier", "watermark"))).toBe("watermark");
    expect(concernOf(f("unreferenced", "waste"))).toBe("bloat");
    expect(concernOf(f("image-format", "device"))).toBe("device");
  });

  it("singles out DRM, the one concern no profile can repair", () => {
    // The CLI files DRM under `structure` because that is what it is. The app
    // has to treat it differently anyway: every other structural finding gets
    // a repair button, and this one must not.
    expect(concernOf(f("drm", "structure", "error"))).toBe("blocked");
    // ...but the Info "font obfuscation only, safe to process" is not a wall.
    expect(concernOf(f("drm", "structure", "info"))).toBe("defect");
  });

  it("still classifies a finding from a sidecar that predates the category field", () => {
    expect(concernOf(f("watermark-invisible", undefined))).toBe("watermark");
    expect(concernOf(f("unreferenced", undefined))).toBe("bloat");
    expect(concernOf(f("fonts", undefined))).toBe("device");
  });

  it("files an unknown future code as a defect rather than dropping it", () => {
    // The graceful-degradation guard. A code this app has never heard of must
    // still reach the user; the row tint is driven by severity, so an unknown
    // info stays quiet regardless.
    expect(concernOf(f("some-check-invented-next-year", undefined))).toBe("defect");
    expect(concernOf(f("another-new-one", "structure"))).toBe("defect");
  });
});

describe("repairProfiles", () => {
  it("composes only the repair-only profile for Clean up", () => {
    // Running `generic` replaces the book's unique identifier, which orphans
    // the reader's bookmarks. That is a decision the user makes by name, via
    // the separate action - never a side effect of a button called "Clean up".
    expect(repairProfiles("cleanup")).toEqual(["epub"]);
  });

  it("layers generic on top of the repair for Remove watermarks", () => {
    expect(repairProfiles("watermarks")).toEqual(["epub", "generic"]);
  });
});

describe("conditionSummary", () => {
  it("counts broken and needs-attention files separately", () => {
    const warn = { severity: "warning" as const, code: "W", message: "m", path: null };
    const err = { severity: "error" as const, code: "E", message: "m", path: null };
    const files = [
      makeFile({ id: "a" }),
      makeFile({ id: "b", cleanup: checkReport({ findings: [warn], warnings: 1 }) }),
      makeFile({ id: "c", cleanup: checkReport({ findings: [err], errors: 1 }) }),
    ];
    expect(conditionSummary(files)).toEqual({ attention: 1, broken: 1 });
  });

  it("does not count an info-only book as needing attention", () => {
    // Every font-obfuscated EPUB carries a `drm` Info saying it is safe to
    // process. Before severity mattered, all of them shouted.
    const info = { severity: "info" as const, code: "drm", message: "safe", path: null };
    const files = [makeFile({ cleanup: checkReport({ findings: [info] }) })];
    expect(conditionSummary(files)).toEqual({ attention: 0, broken: 0 });
  });
});

describe("isFixable", () => {
  const finding = { severity: "warning" as const, code: "W1", message: "m", path: null };

  it("is false for a watermark-only book, which Clean up cannot change", () => {
    // `epub` repairs structure and nothing else. Offering it as the answer to
    // a "watermarked" chip opened a dialog that then said it would not help.
    const mark = {
      severity: "warning" as const,
      code: "watermark-identifier",
      category: "watermark" as const,
      message: "per-copy id",
      path: null,
    };
    expect(isFixable(makeFile({ cleanup: checkReport({ findings: [mark], warnings: 1 }) }))).toBe(
      false,
    );
  });

  it("is true only when the automatic check found something a repair can fix", () => {
    expect(isFixable(makeFile())).toBe(false);
    expect(isFixable(makeFile({ cleanup: checkReport() }))).toBe(false);
    expect(
      isFixable(makeFile({ cleanup: checkReport({ findings: [finding], warnings: 1 }) })),
    ).toBe(true);
  });

  it("is false for DRM, which no profile can repair", () => {
    // The bug this replaces: `needsCleanup` was severity- and code-blind, so a
    // copy-protected book was offered a Clean up button that could only fail
    // after parking a junk copy in the Trash.
    const drm = {
      severity: "error" as const,
      code: "drm",
      category: "structure" as const,
      message: "encrypted",
      path: null,
    };
    expect(isFixable(makeFile({ cleanup: checkReport({ findings: [drm], errors: 1 }) }))).toBe(
      false,
    );
  });

  it("is false while the check is still running or never ran", () => {
    const cleanup = checkReport({ findings: [finding], warnings: 1 });
    expect(isFixable(makeFile({ cleanup, check: "pending" }))).toBe(false);
    expect(isFixable(makeFile({ cleanup, check: "failed" }))).toBe(false);
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

  it("adds a warnings chip alongside the size chip, its messages in the title", () => {
    const book = makeFile({
      result: {
        kind: "fit",
        report: fitReport({
          warnings: [
            { message: "could not parse the CSS in style.css; dropped it", file: "style.css" },
            { message: "cover image is enormous", file: null },
            { message: "empty chapter", file: "ch3.xhtml" },
          ],
          stats: stats({ bytes_in: 1000, bytes_out: 500, warnings: 3 }),
        }),
      },
    });
    expect(chipsFor(book)).toEqual([
      { label: "-50%", tone: "good", title: "1000 B to 500 B" },
      {
        label: "3 warnings",
        tone: "warn",
        title:
          "could not parse the CSS in style.css; dropped it\ncover image is enormous\nempty chapter",
      },
    ]);
  });

  it("singularizes a lone fit warning", () => {
    const book = makeFile({
      result: {
        kind: "fit",
        report: fitReport({
          warnings: [{ message: "empty chapter", file: null }],
          stats: stats({ bytes_in: 1000, bytes_out: 500, warnings: 1 }),
        }),
      },
    });
    expect(chipsFor(book)).toContainEqual({
      label: "1 warning",
      tone: "warn",
      title: "empty chapter",
    });
  });

  it("shows errors and warnings for a check", () => {
    const book = makeFile({ result: { kind: "check", report: checkReport({ errors: 2, warnings: 1 }) } });
    expect(chipsFor(book)).toEqual([
      { label: "2 errors", tone: "bad" },
      { label: "1 warning", tone: "warn" },
    ]);
  });

  it("singularizes a lone check error", () => {
    const book = makeFile({ result: { kind: "check", report: checkReport({ errors: 1, warnings: 2 }) } });
    expect(chipsFor(book)).toEqual([
      { label: "1 error", tone: "bad" },
      { label: "2 warnings", tone: "warn" },
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
        id: "condition",
        label: "needs cleanup",
        tone: "warn",
        title: "needs cleanup - 1 finding from the automatic check",
      },
    ]);
  });

  it("names the worst concern and admits to the rest with a +N", () => {
    // One pill, not three: a chip per concern is a wall of noise across a
    // twenty-book drop, and the eye stops reading the column entirely.
    const cleanup = checkReport({
      findings: [
        { severity: "warning", code: "watermark-identifier", category: "watermark", message: "m", path: null },
        { severity: "info", code: "unreferenced", category: "waste", message: "m", path: null },
        { severity: "error", code: "manifest-sync", category: "structure", message: "m", path: null },
      ],
      errors: 1,
      warnings: 1,
    });
    const [chip] = chipsFor(makeFile({ cleanup }));
    expect(chip.label).toBe("defective +2");
    expect(chip.tone).toBe("bad");
    expect(chip.title).toContain("watermarked");
    expect(chip.title).toContain("extra files");
  });

  it("says a copy-protected book is copy protected, not that it needs cleaning", () => {
    const cleanup = checkReport({
      findings: [
        { severity: "error", code: "drm", category: "structure", message: "encrypted", path: null },
      ],
      errors: 1,
    });
    expect(chipsFor(makeFile({ cleanup }))[0]).toMatchObject({
      label: "copy protected",
      tone: "bad",
    });
  });

  it("says nothing at all while the check is still running", () => {
    // A row must never read "clean" and then flip to "defective".
    expect(chipsFor(makeFile({ check: "pending" }))).toEqual([]);
  });

  it("admits it never checked rather than implying nothing was found", () => {
    const chip = chipsFor(makeFile({ check: "failed", checkError: { code: "io-error", friendly: "boom" } }))[0];
    expect(chip).toMatchObject({ label: "not checked", tone: "neutral", title: "boom" });
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

describe("conditionActions", () => {
  const defect = { severity: "warning" as const, code: "W1", message: "m", path: null };
  const mark = {
    severity: "warning" as const,
    code: "watermark-identifier",
    category: "watermark" as const,
    message: "per-copy id",
    path: null,
  };
  const extra = {
    severity: "info" as const,
    code: "unreferenced",
    category: "waste" as const,
    message: "nothing references it",
    path: "OEBPS/junk.png",
  };
  const drm = {
    severity: "error" as const,
    code: "drm",
    category: "structure" as const,
    message: "encrypted",
    path: null,
  };
  const withFindings = (findings: CheckReport["findings"]) =>
    makeFile({
      cleanup: checkReport({
        findings,
        errors: findings.filter((f) => f.severity === "error").length,
        warnings: findings.filter((f) => f.severity === "warning").length,
      }),
    });

  it("offers Clean up for a structural defect", () => {
    expect(conditionActions(withFindings([defect]))).toEqual(["cleanup"]);
  });

  it("offers Remove watermarks for a per-copy mark, not Clean up", () => {
    expect(conditionActions(withFindings([mark]))).toEqual(["watermarks"]);
  });

  it("offers Remove watermarks for extra files alone, even at info level", () => {
    // The panel used to key off the verdict, so an extra-files-only book had a
    // chip and no way to act on it.
    expect(conditionActions(withFindings([extra]))).toEqual(["watermarks"]);
  });

  it("offers both when both apply, Clean up first", () => {
    expect(conditionActions(withFindings([mark, defect]))).toEqual(["cleanup", "watermarks"]);
  });

  it("offers nothing for DRM, a clean book or no report", () => {
    expect(conditionActions(withFindings([drm]))).toEqual([]);
    expect(conditionActions(withFindings([]))).toEqual([]);
    expect(conditionActions(makeFile())).toEqual([]);
  });

  it("offers nothing while the check is pending or after it failed", () => {
    const cleanup = checkReport({ findings: [defect, mark], warnings: 2 });
    expect(conditionActions(makeFile({ cleanup, check: "pending" }))).toEqual([]);
    expect(conditionActions(makeFile({ cleanup, check: "failed" }))).toEqual([]);
  });
});
