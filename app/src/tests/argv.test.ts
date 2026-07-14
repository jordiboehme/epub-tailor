import { describe, expect, it } from "vitest";
import { checkArgv, fitArgv, mdArgv, showArgv } from "../lib/api/argv";
import type { RunOptions } from "../lib/api/argv";

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
