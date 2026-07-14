import { describe, expect, it } from "vitest";
import {
  checkArgv,
  fetchArgv,
  fitArgv,
  mdArgv,
  metadataArgv,
  searchArgv,
  showArgv,
} from "../lib/api/argv";
import type { RunOptions } from "../lib/api/argv";
import type { StagedEdits } from "../lib/api/edits";

const opts = (overrides: Partial<RunOptions> = {}): RunOptions => ({
  profiles: ["epub"],
  quality: null,
  tables: null,
  dryRun: false,
  ...overrides,
});

describe("fitArgv", () => {
  it("builds a plain copy conversion with an output path", () => {
    expect(fitArgv("/in.epub", "/out.epub", opts())).toEqual([
      "fit",
      "/in.epub",
      "--report",
      "json",
      "--profile",
      "epub",
      "-o",
      "/out.epub",
    ]);
  });

  it("uses --lets-get-dangerous for an in-place run and never emits -o", () => {
    const argv = fitArgv("/in.epub", null, opts());
    expect(argv).toEqual([
      "fit",
      "/in.epub",
      "--report",
      "json",
      "--profile",
      "epub",
      "--lets-get-dangerous",
    ]);
    expect(argv).not.toContain("-o");
  });

  it("emits every flag in a stable order", () => {
    const argv = fitArgv("/in.epub", "/out.epub", {
      profiles: ["epub", "manga"],
      quality: "high",
      tables: "text",
      dryRun: true,
    });
    expect(argv).toEqual([
      "fit",
      "/in.epub",
      "--report",
      "json",
      "--profile",
      "epub",
      "--profile",
      "manga",
      "--quality",
      "high",
      "--tables",
      "text",
      "--dry-run",
      "-o",
      "/out.epub",
    ]);
  });

  it("omits quality and tables when they are null", () => {
    const argv = fitArgv("/in.epub", "/out.epub", opts({ quality: null, tables: null }));
    expect(argv).not.toContain("--quality");
    expect(argv).not.toContain("--tables");
  });
});

describe("mdArgv", () => {
  it("mirrors fitArgv but with the md command", () => {
    expect(mdArgv("/in.md", "/out.epub", opts())).toEqual([
      "md",
      "/in.md",
      "--report",
      "json",
      "--profile",
      "epub",
      "-o",
      "/out.epub",
    ]);
  });
});

describe("checkArgv", () => {
  it("builds a check with one --profile pair per entry", () => {
    expect(checkArgv("/in.epub", ["epub", "manga"])).toEqual([
      "check",
      "/in.epub",
      "--report",
      "json",
      "--profile",
      "epub",
      "--profile",
      "manga",
    ]);
  });
});

describe("showArgv", () => {
  it("builds a metadata show with a cover-out target", () => {
    expect(showArgv("/in.epub", "/cache/abc.img")).toEqual([
      "metadata",
      "show",
      "/in.epub",
      "--report",
      "json",
      "--cover-out",
      "/cache/abc.img",
    ]);
  });
});

const fullEdits: StagedEdits = {
  title: "The Hobbit",
  authors: ["J.R.R. Tolkien", "Christopher Tolkien"],
  language: "en",
  publisher: "Allen & Unwin",
  description: "There and back again.",
  subjects: ["Fantasy", "Adventure"],
  date: "1937",
  isbn: "9780261102217",
  series: "Middle-earth",
  seriesIndex: "0",
  coverPath: "/cache/fetched-ol1.img",
};

describe("metadataArgv", () => {
  it("is empty when there are no edits", () => {
    expect(metadataArgv(undefined)).toEqual([]);
    expect(metadataArgv({})).toEqual([]);
  });

  it("treats an all-blank edits object as no edits", () => {
    expect(metadataArgv({ authors: [], subjects: [] })).toEqual([]);
  });

  it("emits every field in a stable order and always ends with --metadata-merge replace", () => {
    expect(metadataArgv(fullEdits)).toEqual([
      "--title",
      "The Hobbit",
      "--author",
      "J.R.R. Tolkien",
      "--author",
      "Christopher Tolkien",
      "--language",
      "en",
      "--publisher",
      "Allen & Unwin",
      "--description",
      "There and back again.",
      "--subject",
      "Fantasy",
      "--subject",
      "Adventure",
      "--date",
      "1937",
      "--isbn",
      "9780261102217",
      "--series",
      "Middle-earth",
      "--series-index",
      "0",
      "--cover",
      "/cache/fetched-ol1.img",
      "--metadata-merge",
      "replace",
    ]);
  });

  it("repeats --author and --subject once per entry", () => {
    const argv = metadataArgv({ authors: ["A", "B", "C"], subjects: ["x", "y"] });
    expect(argv.filter((a) => a === "--author")).toHaveLength(3);
    expect(argv.filter((a) => a === "--subject")).toHaveLength(2);
  });

  it("includes --metadata-merge replace whenever any single field is set", () => {
    expect(metadataArgv({ series: "Dune" })).toEqual([
      "--series",
      "Dune",
      "--metadata-merge",
      "replace",
    ]);
  });
});

describe("fitArgv with staged edits", () => {
  it("leaves the argv unchanged when there are no edits", () => {
    const argv = fitArgv("/in.epub", "/out.epub", opts());
    expect(argv).not.toContain("--metadata-merge");
    expect(argv).not.toContain("--title");
  });

  it("splices the metadata flags in before the output flag and keeps replace present", () => {
    const argv = fitArgv("/in.epub", "/out.epub", opts({ edits: { series: "Dune", seriesIndex: "1" } }));
    expect(argv).toEqual([
      "fit",
      "/in.epub",
      "--report",
      "json",
      "--profile",
      "epub",
      "--series",
      "Dune",
      "--series-index",
      "1",
      "--metadata-merge",
      "replace",
      "-o",
      "/out.epub",
    ]);
  });

  it("carries the cover flag into an in-place run too", () => {
    const argv = fitArgv("/in.epub", null, opts({ edits: { coverPath: "/c.img" } }));
    expect(argv).toContain("--cover");
    expect(argv).toContain("/c.img");
    expect(argv).toContain("--metadata-merge");
    expect(argv[argv.length - 1]).toBe("--lets-get-dangerous");
  });
});

describe("searchArgv", () => {
  it("builds an all-manual query with no input path", () => {
    expect(searchArgv({ title: "Dune", author: "Herbert" })).toEqual([
      "metadata",
      "search",
      "--title",
      "Dune",
      "--author",
      "Herbert",
      "--report",
      "json",
    ]);
  });

  it("passes the book path and the fields that are set", () => {
    expect(searchArgv({ input: "/b.epub", isbn: "123", limit: 8 })).toEqual([
      "metadata",
      "search",
      "/b.epub",
      "--isbn",
      "123",
      "--limit",
      "8",
      "--report",
      "json",
    ]);
  });
});

describe("fetchArgv", () => {
  it("builds a fetch by reference, report json", () => {
    expect(fetchArgv("openlibrary:OL262758W")).toEqual([
      "metadata",
      "fetch",
      "openlibrary:OL262758W",
      "--report",
      "json",
    ]);
  });

  it("adds --cover-out only when a cover path is given", () => {
    expect(fetchArgv("openlibrary:OL1W", "/cache/fetched-ol1.img")).toEqual([
      "metadata",
      "fetch",
      "openlibrary:OL1W",
      "--report",
      "json",
      "--cover-out",
      "/cache/fetched-ol1.img",
    ]);
  });
});
