import { describe, expect, it } from "vitest";
import {
  ContractError,
  parseReport,
  isCliFailure,
} from "../lib/api/contract";
import type {
  CheckReport,
  ErrorReport,
  FitReport,
  MetadataShowReport,
  ProfilesReport,
  SearchReport,
} from "../lib/api/contract";

// Every fixture below is real `--report json` output captured by running the
// built sidecar binary directly (see task-3-report.md for the exact
// commands), except SEARCH_JSON: `metadata search` reaches the network, so
// that one is hand-built from the field names verified against
// crates/cli/src/lookup_cmd.rs and crates/core/src/metadata/openlibrary.rs
// instead of a live capture.

const FIT_JSON = `{
  "dry_run": false,
  "output": "/tmp/book.fit.epub",
  "schema": 1,
  "stats": {
    "bytes_in": 11848,
    "bytes_out": 11902,
    "chapters": 1,
    "chapters_split": 0,
    "images_processed": 0,
    "warnings": 0
  },
  "transformations": [],
  "warnings": []
}`;

const FIT_WITH_WARNING_JSON = `{
  "dry_run": false,
  "output": "/tmp/broken.epub",
  "schema": 1,
  "stats": {
    "bytes_in": 116,
    "bytes_out": 2098,
    "chapters": 1,
    "chapters_split": 0,
    "images_processed": 0,
    "warnings": 1
  },
  "transformations": [],
  "warnings": [
    {
      "file": "OEBPS/ch-001.xhtml",
      "message": "could not resolve local image 'missing.png'; left the reference unchanged"
    }
  ]
}`;

const FIT_DRY_RUN_JSON = `{
  "dry_run": true,
  "output": null,
  "schema": 1,
  "stats": {
    "bytes_in": 2098,
    "bytes_out": 2151,
    "chapters": 1,
    "chapters_split": 0,
    "images_processed": 0,
    "warnings": 0
  },
  "transformations": [],
  "warnings": []
}`;

const ERROR_JSON = `{
  "error": {
    "code": "read-failed",
    "message": "cannot read /tmp/does-not-exist.epub: No such file or directory (os error 2)"
  },
  "schema": 1
}`;

const CHECK_JSON = `{
  "errors": 0,
  "findings": [],
  "schema": 1,
  "warnings": 0
}`;

// findings[].path is never omitted by the CLI (LintFinding has no
// skip_serializing_if on `path`); it is `null`, not absent, when a finding is
// not about one specific resource.
const CHECK_WITH_FINDINGS_JSON = `{
  "errors": 1,
  "findings": [
    {
      "severity": "error",
      "code": "missing-nav",
      "message": "no navigation document declared",
      "path": null
    },
    {
      "severity": "warning",
      "code": "large-image",
      "message": "image exceeds the device's decode cap",
      "path": "OEBPS/images/cover.jpg"
    }
  ],
  "schema": 1,
  "warnings": 1
}`;

const METADATA_SHOW_JSON = `{
  "metadata": {
    "identifier": "urn:epub-tailor:927497135cdabe78",
    "language": "en",
    "title": "Untitled"
  },
  "missing": [
    "title",
    "authors",
    "description",
    "publisher",
    "subjects",
    "date",
    "series",
    "isbn"
  ],
  "schema": 1
}`;

// Trimmed to two of the 27 built-in profiles (epub, x4); each object is
// copied verbatim from a real `profiles --report json` capture.
const PROFILES_JSON = `{
  "schema": 1,
  "profiles": [
    {
      "appendix": null,
      "caps": {
        "cover_budget_bytes": 18446744073709551615,
        "cover_max": [4294967295, 4294967295],
        "css_max_bytes": 18446744073709551615,
        "css_max_rules": 18446744073709551615,
        "inline_budget_bytes": 18446744073709551615,
        "inline_max": [4294967295, 4294967295],
        "max_src_px": [4294967295, 4294967295],
        "panel": "color",
        "ppi": 0,
        "screen_h": 4294967295,
        "screen_w": 4294967295
      },
      "description": "Repair and cleanup only - everything the EPUB standard allows stays",
      "features": {
        "bake_ordered_lists": false,
        "chapter_split": false,
        "dedupe_ids": true,
        "degrade_boxes": false,
        "filter_css": false,
        "linearize_tables": false,
        "normalize_footnotes": false,
        "preserve_code_blocks": false,
        "rasterize_svg": false,
        "relocate_anchors": false,
        "relocate_styles": false,
        "sanitize_css": false,
        "strip_fonts": false,
        "transcode_images": false,
        "unicode_hygiene": true
      },
      "filters": [],
      "jpeg_quality": 82,
      "max_chapter_bytes": 204800,
      "name": "epub",
      "split_tall_images": false,
      "tables": "text"
    },
    {
      "appendix": "x4",
      "caps": {
        "cover_budget_bytes": 130048,
        "cover_max": [480, 800],
        "css_max_bytes": 131072,
        "css_max_rules": 1500,
        "inline_budget_bytes": 102400,
        "inline_max": [480, 730],
        "max_src_px": [2048, 1536],
        "panel": "gray4",
        "ppi": 220,
        "screen_h": 800,
        "screen_w": 480
      },
      "description": "Xteink X4 running CrossPoint firmware",
      "features": {
        "bake_ordered_lists": true,
        "chapter_split": true,
        "dedupe_ids": true,
        "degrade_boxes": true,
        "filter_css": true,
        "linearize_tables": true,
        "normalize_footnotes": true,
        "preserve_code_blocks": true,
        "rasterize_svg": true,
        "relocate_anchors": true,
        "relocate_styles": true,
        "sanitize_css": false,
        "strip_fonts": true,
        "transcode_images": true,
        "unicode_hygiene": true
      },
      "filters": [],
      "jpeg_quality": 82,
      "max_chapter_bytes": 204800,
      "name": "x4",
      "split_tall_images": false,
      "tables": "text"
    }
  ]
}`;

// Hand-built: `metadata search` touches the network, so this mirrors
// crates/core/src/metadata/openlibrary.rs::Candidate (r#ref -> "ref" is
// serde's normal raw-identifier stripping; cover_url has
// skip_serializing_if = "Option::is_none", so it is *absent*, not null, when
// Open Library has no cover) plus lookup_cmd.rs::print_candidates_json.
const SEARCH_JSON = `{
  "schema": 1,
  "candidates": [
    {
      "ref": "openlibrary:OL262758W",
      "source": "openlibrary",
      "metadata": {
        "title": "The Fellowship of the Ring",
        "authors": ["J.R.R. Tolkien"]
      },
      "cover_url": "https://covers.openlibrary.org/b/id/1.jpg",
      "score": 0.92
    },
    {
      "ref": "openlibrary:OL27482W",
      "source": "openlibrary",
      "metadata": { "title": "Fellowship" },
      "score": 0.4
    }
  ],
  "source_licence": "Open Library metadata is CC0; cover images are not."
}`;

describe("parseReport", () => {
  it("parses a FitReport", () => {
    const report = parseReport<FitReport>(FIT_JSON, "fit");
    expect(isCliFailure(report)).toBe(false);
    const fit = report as FitReport;
    expect(fit.schema).toBe(1);
    expect(fit.output).toBe("/tmp/book.fit.epub");
    expect(fit.dry_run).toBe(false);
    expect(fit.stats.bytes_in).toBe(11848);
    expect(fit.warnings).toEqual([]);
  });

  it("parses a FitReport with a null dry-run output", () => {
    const fit = parseReport<FitReport>(FIT_DRY_RUN_JSON, "fit") as FitReport;
    expect(fit.dry_run).toBe(true);
    expect(fit.output).toBeNull();
  });

  it("keeps a warning's null file as null, not undefined", () => {
    const fit = parseReport<FitReport>(FIT_JSON, "fit") as FitReport;
    expect(fit.warnings).toEqual([]);
    const withWarning = parseReport<FitReport>(
      FIT_WITH_WARNING_JSON,
      "fit",
    ) as FitReport;
    expect(withWarning.warnings[0].file).toBe("OEBPS/ch-001.xhtml");
  });

  it("parses a CheckReport", () => {
    const check = parseReport<CheckReport>(CHECK_JSON, "check") as CheckReport;
    expect(check.errors).toBe(0);
    expect(check.findings).toEqual([]);
  });

  it("parses a CheckReport with findings, path null or set", () => {
    const check = parseReport<CheckReport>(
      CHECK_WITH_FINDINGS_JSON,
      "check",
    ) as CheckReport;
    expect(check.findings).toHaveLength(2);
    expect(check.findings[0].severity).toBe("error");
    expect(check.findings[0].path).toBeNull();
    expect(check.findings[1].path).toBe("OEBPS/images/cover.jpg");
  });

  it("parses a MetadataShowReport", () => {
    const show = parseReport<MetadataShowReport>(
      METADATA_SHOW_JSON,
      "metadata show",
    ) as MetadataShowReport;
    expect(show.metadata.title).toBe("Untitled");
    expect(show.missing).toContain("authors");
  });

  it("parses a ProfilesReport", () => {
    const profiles = parseReport<ProfilesReport>(
      PROFILES_JSON,
      "profiles",
    ) as ProfilesReport;
    expect(profiles.profiles).toHaveLength(2);
    const epub = profiles.profiles.find((p) => p.name === "epub")!;
    expect(epub.appendix).toBeNull();
    expect(epub.tables).toBe("text");
    expect(epub.caps.panel).toBe("color");
    const x4 = profiles.profiles.find((p) => p.name === "x4")!;
    expect(x4.appendix).toBe("x4");
    expect(x4.features.strip_fonts).toBe(true);
  });

  it("parses a SearchReport, ref spelled literally and cover_url optional", () => {
    const search = parseReport<SearchReport>(
      SEARCH_JSON,
      "metadata search",
    ) as SearchReport;
    expect(search.candidates).toHaveLength(2);
    expect(search.candidates[0].ref).toBe("openlibrary:OL262758W");
    expect(search.candidates[0].cover_url).toBe(
      "https://covers.openlibrary.org/b/id/1.jpg",
    );
    expect(search.candidates[1].cover_url).toBeUndefined();
  });

  it("discriminates an ErrorReport via the `error` key", () => {
    const result = parseReport<FitReport>(ERROR_JSON, "fit");
    expect(isCliFailure(result)).toBe(true);
    expect(result).toEqual({
      code: "read-failed",
      message:
        "cannot read /tmp/does-not-exist.epub: No such file or directory (os error 2)",
    });
  });

  it("keeps the ErrorReport shape itself parseable as ErrorReport", () => {
    const parsed = JSON.parse(ERROR_JSON) as ErrorReport;
    expect(parsed.schema).toBe(1);
    expect(parsed.error.code).toBe("read-failed");
  });

  it("throws a ContractError when the schema does not match", () => {
    const futureJson = FIT_JSON.replace('"schema": 1', '"schema": 2');
    expect(() => parseReport<FitReport>(futureJson, "fit")).toThrow(
      ContractError,
    );
  });

  it("throws a ContractError on unparsable JSON", () => {
    expect(() => parseReport<FitReport>("not json", "fit")).toThrow(
      ContractError,
    );
  });
});
