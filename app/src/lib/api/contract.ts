// The CLI's `--report json` payload types (schema 1) and the parsers that
// turn raw stdout into them. Field names are verbatim from the Rust side -
// see task-3-report.md for the file:line each shape was checked against.
//
// No Tauri import here: this module only deals with strings the caller
// already collected from the sidecar (see sidecar.ts), so it is as easy to
// unit test as templates.ts.

/** The JSON contract version every `--report json` payload carries. */
export const SCHEMA_VERSION = 1;

// ---------------------------------------------------------------------------
// fit / md: ConvertReport (crates/core/src/report.rs)
// ---------------------------------------------------------------------------

/**
 * `Transformation.file` and `Warning.file` (crates/core/src/report.rs) have
 * no `skip_serializing_if`, so the CLI always emits the key - `null`, never
 * absent, when there is no associated file.
 */
export interface Transformation {
  kind: string;
  detail: string;
  file: string | null;
}

export interface CliWarning {
  message: string;
  file: string | null;
}

export interface Stats {
  bytes_in: number;
  bytes_out: number;
  images_processed: number;
  chapters: number;
  chapters_split: number;
  warnings: number;
}

export interface FitReport {
  schema: 1;
  /** `null` under `--dry-run` (nothing was written); a path otherwise. */
  output: string | null;
  dry_run: boolean;
  transformations: Transformation[];
  warnings: CliWarning[];
  stats: Stats;
}

// ---------------------------------------------------------------------------
// Any command's failure payload
// ---------------------------------------------------------------------------

/** The stable `{code, message}` pair every CLI failure carries. */
export interface CliFailure {
  code: string;
  message: string;
}

export interface ErrorReport {
  schema: 1;
  error: CliFailure;
}

// ---------------------------------------------------------------------------
// check: LintFinding (crates/core/src/validate.rs)
// ---------------------------------------------------------------------------

export interface Finding {
  severity: "info" | "warning" | "error";
  code: string;
  message: string;
  /** No `skip_serializing_if` on `LintFinding.path` either: `null`, not absent. */
  path: string | null;
}

export interface CheckReport {
  schema: 1;
  findings: Finding[];
  errors: number;
  warnings: number;
}

// ---------------------------------------------------------------------------
// profiles --report json (crates/core/src/profile/{mod,caps,features}.rs,
// crates/core/src/filter/mod.rs)
// ---------------------------------------------------------------------------

export type Panel = "gray4" | "gray16" | "color";

export interface DeviceCaps {
  screen_w: number;
  screen_h: number;
  ppi: number;
  panel: Panel;
  max_src_px: [number, number];
  inline_max: [number, number];
  cover_max: [number, number];
  inline_budget_bytes: number;
  cover_budget_bytes: number;
  css_max_bytes: number;
  css_max_rules: number;
}

export interface ProfileFeatures {
  strip_fonts: boolean;
  filter_css: boolean;
  sanitize_css: boolean;
  relocate_styles: boolean;
  transcode_images: boolean;
  rasterize_svg: boolean;
  linearize_tables: boolean;
  degrade_boxes: boolean;
  bake_ordered_lists: boolean;
  preserve_code_blocks: boolean;
  normalize_footnotes: boolean;
  relocate_anchors: boolean;
  dedupe_ids: boolean;
  unicode_hygiene: boolean;
  chapter_split: boolean;
}

export type TableMode = "text" | "image" | "image-all";

export interface FilterRule {
  action: "remove" | "replace";
  match: string;
  /** `Option<String>` with no `skip_serializing_if`: `null`, not absent. */
  with: string | null;
  in: ("text" | "href" | "file")[];
}

export interface Profile {
  name: string;
  description: string;
  caps: DeviceCaps;
  features: ProfileFeatures;
  jpeg_quality: number;
  tables: TableMode;
  split_tall_images: boolean;
  max_chapter_bytes: number;
  appendix: string | null;
  filters: FilterRule[];
}

export interface ProfilesReport {
  schema: 1;
  profiles: Profile[];
}

// ---------------------------------------------------------------------------
// MetadataDoc (crates/core/src/metadata/mod.rs) and metadata show/search
// ---------------------------------------------------------------------------

/**
 * The wire shape of a `Creator` (crates/core/src/epub/model.rs): a bare name
 * string in the common case, or `{name, file_as?, role?}` when the book (or a
 * looked-up record) carries a sort name or a MARC relator role. The task
 * brief describes `authors` as "string or string[]"; the fuller union here
 * is a deliberate correction verified against `CreatorField`
 * (crates/core/src/epub/model.rs) - a plain-string-only type would silently
 * mistype a real book's author entry the moment it carries a file-as name.
 */
export type Creator = string | { name: string; file_as?: string; role?: string };

export interface MetadataIdentifier {
  value: string;
  scheme?: string;
}

/**
 * Every field optional; absent means "say nothing", not "clear it" (see the
 * module doc in crates/core/src/metadata/mod.rs). `authors`/`contributors`/
 * `subjects` accept a single value or a list on input; the CLI's own output
 * (`metadata show`/`search`/`fetch`) always emits a list.
 */
export interface MetadataDoc {
  title?: string;
  authors?: Creator | Creator[];
  contributors?: Creator | Creator[];
  language?: string;
  identifier?: string;
  identifiers?: MetadataIdentifier[];
  isbn?: string;
  description?: string;
  publisher?: string;
  subjects?: string | string[];
  date?: string;
  rights?: string;
  series?: string;
  series_index?: string;
  cover?: string;
}

export interface MetadataShowReport {
  schema: 1;
  metadata: MetadataDoc;
  missing: string[];
}

/**
 * One Open Library search result (crates/core/src/metadata/openlibrary.rs
 * `Candidate`). `r#ref` is a raw identifier only to dodge the Rust keyword;
 * serde strips the `r#` and serializes the field as plain `ref`. `cover_url`
 * carries `skip_serializing_if = "Option::is_none"`, so it is *absent*, not
 * null, when Open Library has no cover for this record.
 */
export interface Candidate {
  ref: string;
  source: string;
  metadata: MetadataDoc;
  cover_url?: string;
  score: number;
}

export interface SearchReport {
  schema: 1;
  candidates: Candidate[];
  source_licence: string;
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/** A schema mismatch, or stdout that was not the JSON document it claimed to be. */
export class ContractError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ContractError";
  }
}

/** Narrows a `parseReport` result to the failure branch. */
export function isCliFailure(value: unknown): value is CliFailure {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value &&
    !("schema" in value)
  );
}

/**
 * Parse one `--report json` document: JSON.parse it, assert `schema === 1`
 * (a schema bump must fail loudly rather than be silently misread as the
 * old shape), then discriminate an `ErrorReport` from the success payload by
 * the presence of an `error` key. `kind` only labels the error messages
 * (e.g. "fit", "check", "metadata search").
 */
export function parseReport<T>(stdout: string, kind: string): T | CliFailure {
  let parsed: unknown;
  try {
    parsed = JSON.parse(stdout);
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    throw new ContractError(`${kind}: could not parse output as JSON (${reason})`);
  }

  if (typeof parsed !== "object" || parsed === null) {
    throw new ContractError(`${kind}: output was not a JSON object`);
  }
  const obj = parsed as Record<string, unknown>;

  if (obj.schema !== SCHEMA_VERSION) {
    throw new ContractError(
      `${kind}: output has schema ${JSON.stringify(obj.schema)}, this app understands schema ${SCHEMA_VERSION}`,
    );
  }

  if ("error" in obj) {
    const error = obj.error as CliFailure;
    return { code: error.code, message: error.message };
  }

  return parsed as T;
}
