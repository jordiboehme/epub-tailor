// The copy-recognition rules: how a produced file's name is parsed back into
// its source stem and profile appendix, and how the implied source path is
// built. Pure string math (copies.ts has no Tauri import), so this pins the
// whole matrix directly.

import { describe, expect, it } from "vitest";

import { impliedSourcePath, parseCopyName, planRegroup, samePath } from "../lib/api/copies";

const APPENDIXES = ["x4", "kobo-clara-bw", "tailored"];

describe("parseCopyName", () => {
  it("splits a produced copy's name into stem and appendix", () => {
    expect(parseCopyName("Dune.x4.epub", APPENDIXES)).toEqual({
      stem: "Dune",
      appendix: "x4",
    });
    expect(parseCopyName("Dune.tailored.epub", APPENDIXES)).toEqual({
      stem: "Dune",
      appendix: "tailored",
    });
  });

  it("keeps multi-dot stems intact", () => {
    expect(parseCopyName("My.Novel.x4.epub", APPENDIXES)).toEqual({
      stem: "My.Novel",
      appendix: "x4",
    });
  });

  it("matches the appendix case-insensitively but keeps the canonical form", () => {
    expect(parseCopyName("Dune.X4.epub", APPENDIXES)).toEqual({
      stem: "Dune",
      appendix: "x4",
    });
  });

  it("strips one trailing collision number before matching", () => {
    // The planner numbers collisions after the appendix: "Dune.x4 (2).epub".
    expect(parseCopyName("Dune.x4 (2).epub", APPENDIXES)).toEqual({
      stem: "Dune",
      appendix: "x4",
    });
    expect(parseCopyName("My.Novel.x4 (12).epub", APPENDIXES)).toEqual({
      stem: "My.Novel",
      appendix: "x4",
    });
  });

  it("rejects names without a known appendix", () => {
    expect(parseCopyName("Dune.epub", APPENDIXES)).toBeNull();
    expect(parseCopyName("My.Novel.epub", APPENDIXES)).toBeNull();
    expect(parseCopyName("Dune.x5.epub", APPENDIXES)).toBeNull();
  });

  it("rejects non-epub files and empty stems", () => {
    expect(parseCopyName("Dune.x4.md", APPENDIXES)).toBeNull();
    expect(parseCopyName("Dune.x4", APPENDIXES)).toBeNull();
    // A file that is nothing but appendix and extension has no stem to
    // resolve a source from.
    expect(parseCopyName(".x4.epub", APPENDIXES)).toBeNull();
  });
});

describe("impliedSourcePath", () => {
  it("puts the source next to the copy", () => {
    expect(impliedSourcePath("/books/Dune.x4.epub", "Dune")).toBe("/books/Dune.epub");
  });

  it("handles backslash separators and bare names", () => {
    expect(impliedSourcePath("C:\\books\\Dune.x4.epub", "Dune")).toBe("C:\\books/Dune.epub");
    expect(impliedSourcePath("Dune.x4.epub", "Dune")).toBe("Dune.epub");
  });
});

describe("samePath", () => {
  it("folds case and unifies separators", () => {
    expect(samePath("/Books/Dune.epub", "/books/dune.EPUB")).toBe(true);
    expect(samePath("C:\\books\\Dune.epub", "C:/books/dune.epub")).toBe(true);
    expect(samePath("/books/Dune.epub", "/books/Dune2.epub")).toBe(false);
    // Same name in a different folder is a different file.
    expect(samePath("/a/Dune.epub", "/b/Dune.epub")).toBe(false);
  });
});

describe("planRegroup", () => {
  const book = (id: string, ...paths: string[]) => ({
    id,
    files: paths.map((path) => ({
      path,
      fileName: path.slice(path.lastIndexOf("/") + 1),
    })),
  });

  it("folds a copy under its co-located source, in either add order", () => {
    const source = book("s", "/books/Dune.epub");
    const copy = book("c", "/books/Dune.x4.epub");
    const expected = [{ id: "c", sourceId: "s", appendix: "x4" }];
    expect(planRegroup([source, copy], APPENDIXES)).toEqual(expected);
    expect(planRegroup([copy, source], APPENDIXES)).toEqual(expected);
  });

  it("never folds across directories", () => {
    const source = book("s", "/downloads/Dune.epub");
    const copy = book("c", "/books/Dune.x4.epub");
    expect(planRegroup([source, copy], APPENDIXES)).toEqual([]);
  });

  it("leaves a copy without a source alone", () => {
    expect(planRegroup([book("c", "/books/Dune.x4.epub")], APPENDIXES)).toEqual([]);
  });

  it("folds numbered copies and several copies of one source", () => {
    const folds = planRegroup(
      [
        book("s", "/b/Dune.epub"),
        book("c1", "/b/Dune.x4.epub"),
        book("c2", "/b/Dune.x4 (2).epub"),
        book("c3", "/b/Dune.tailored.epub"),
      ],
      APPENDIXES,
    );
    expect(folds).toEqual([
      { id: "c1", sourceId: "s", appendix: "x4" },
      { id: "c2", sourceId: "s", appendix: "x4" },
      { id: "c3", sourceId: "s", appendix: "tailored" },
    ]);
  });

  it("folds a whole chain onto its root", () => {
    // "Dune.x4.kobo-clara-bw.epub" reads as a copy of "Dune.x4.epub", which
    // itself folds under "Dune.epub": both land on the root book, whatever
    // order the batch arrived in.
    const sorted = (folds: { id: string }[]) => [...folds].sort((a, b) => a.id.localeCompare(b.id));
    const expected = [
      { id: "c", sourceId: "s", appendix: "x4" },
      { id: "g", sourceId: "s", appendix: "kobo-clara-bw" },
    ];
    const s = () => book("s", "/b/Dune.epub");
    const c = () => book("c", "/b/Dune.x4.epub");
    const g = () => book("g", "/b/Dune.x4.kobo-clara-bw.epub");
    expect(sorted(planRegroup([s(), c(), g()], APPENDIXES))).toEqual(expected);
    expect(sorted(planRegroup([g(), c(), s()], APPENDIXES))).toEqual(expected);
    expect(sorted(planRegroup([c(), g(), s()], APPENDIXES))).toEqual(expected);
  });

  it("folds a grandchild under a book that already tracks the child as a file", () => {
    const folds = planRegroup(
      [
        book("s", "/b/Dune.epub", "/b/Dune.x4.epub"),
        book("g", "/b/Dune.x4.kobo-clara-bw.epub"),
      ],
      APPENDIXES,
    );
    expect(folds).toEqual([{ id: "g", sourceId: "s", appendix: "kobo-clara-bw" }]);
  });

  it("never folds a multi-file book, even when its original is copy-named", () => {
    // A book that already tracks files is someone's folder in its own right;
    // merging two file lists is not a fold.
    const folds = planRegroup(
      [
        book("s", "/b/Dune.epub"),
        book("m", "/b/Dune.x4.epub", "/b/Dune.x4.tailored.epub"),
      ],
      APPENDIXES,
    );
    expect(folds).toEqual([]);
  });

  it("matches sources case-insensitively", () => {
    const folds = planRegroup(
      [book("s", "/b/DUNE.epub"), book("c", "/b/dune.x4.epub")],
      APPENDIXES,
    );
    expect(folds).toEqual([{ id: "c", sourceId: "s", appendix: "x4" }]);
  });
});
