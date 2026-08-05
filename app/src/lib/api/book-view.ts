// Shared per-file display logic: title/subtitle fallbacks, cover initials,
// the failure a row can explain, the findings a check produced, and the
// status chips. A gallery card, a list row and a file row need exactly the
// same data, and duplicating it would let the views drift - worst in the
// failure states, where users are already unhappy. So it lives here once.
// The book-level views pass the ORIGINAL file (files[0]) for the header.
//
// Pure functions only - no store imports, no Tauri imports - so vitest
// reaches them directly, same as format.ts, meta.ts and templates.ts.

import { stemOf } from "../stores/books.svelte";
import type { Book, BookFile } from "../stores/books.svelte";
import { CLEANUP_PROFILE } from "./argv";
import { parseCopyName } from "./copies";
import type { Finding, Stats } from "./contract";
import { formatSize } from "./format";
import type { StagedEdits } from "./edits";
import { mergeEditsIntoMeta } from "./edits";
import type { BookMeta } from "./meta";

/**
 * The failure a row can explain: a conversion that failed, or a file that
 * could not even be read in the first place. Carries its own stderr tail
 * rather than pointing at the job that produced it, since jobs are pruned
 * once the next batch starts.
 */
export interface Failure {
  friendly: string;
  code: string;
  stderr: string[];
}

export type Tone = "good" | "warn" | "bad" | "neutral";

export interface Chip {
  label: string;
  tone: Tone;
  title?: string;
  /** A stable handle for chips a view treats specially (e.g. "needs-cleanup"). */
  id?: string;
}

/** Tone to Tailwind classes, so every view styles chips identically. */
export const TONE_CLASS: Record<Tone, string> = {
  good: "bg-emerald-100 text-emerald-700 dark:bg-emerald-500/15 dark:text-emerald-300",
  warn: "bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300",
  bad: "bg-rose-100 text-rose-700 dark:bg-rose-500/15 dark:text-rose-300",
  neutral: "bg-ink-200 text-ink-600 dark:bg-ink-700 dark:text-ink-300",
};

/**
 * The metadata a view should display: the file's own, with any staged edits
 * folded over it. Views pass `edits.get(file.id)`; this stays a pure function
 * so vitest reaches it without a store in sight.
 */
export function effectiveMeta(file: BookFile, staged?: StagedEdits): BookMeta | undefined {
  if (!staged) return file.meta;
  return mergeEditsIntoMeta(file.meta, staged);
}

/** The title to show: the effective metadata title, else the file name's stem. */
export function fileTitle(file: BookFile, staged?: StagedEdits): string {
  return effectiveMeta(file, staged)?.title?.trim() || stemOf(file.fileName);
}

/** The subtitle: the first author, else "Markdown" for a Markdown file, else nothing. */
export function fileSubtitle(file: BookFile, staged?: StagedEdits): string {
  return effectiveMeta(file, staged)?.authors?.[0] ?? (file.kind === "md" ? "Markdown" : "");
}

/** Every author, joined for a list column. Empty when the file names none. */
export function fileAuthors(file: BookFile, staged?: StagedEdits): string {
  return (effectiveMeta(file, staged)?.authors ?? []).join(", ");
}

/**
 * The series a file belongs to, with its position when it has one: "Dune #2".
 * Empty when the file carries no series at all.
 */
export function fileSeries(file: BookFile, staged?: StagedEdits): string {
  const meta = effectiveMeta(file, staged);
  const series = meta?.series?.trim();
  if (!series) return "";
  const index = meta?.seriesIndex?.trim();
  return index ? `${series} #${index}` : series;
}

/**
 * The one line under a row's title: the author and the series, whichever of
 * them the file has. A row has the width to say both; a card only shows the
 * subtitle, which is why this lives beside `fileSubtitle` rather than in it.
 */
export function fileByline(file: BookFile, staged?: StagedEdits): string {
  return [fileSubtitle(file, staged), fileSeries(file, staged)].filter(Boolean).join(" · ");
}

/** The 4-digit year out of the effective date, for a narrow list column. */
export function fileYear(file: BookFile, staged?: StagedEdits): string {
  return effectiveMeta(file, staged)?.date?.match(/\d{4}/)?.[0] ?? "";
}

/** Up to two initials from the stem's words, for a coverless placeholder. */
export function fileInitials(file: BookFile): string {
  return stemOf(file.fileName)
    .split(/[\s_·—–-]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]?.toUpperCase() ?? "")
    .join("");
}

/**
 * The findings to show for a file: an explicit check's, a fit's own warnings,
 * or - failing both - whatever the automatic check-on-add turned up. The
 * explicit results win because the user asked for them, possibly against a
 * device profile; the automatic probe only ever speaks `epub`. A fit's
 * warnings wear the `Finding` shape so the same drawer shows them; they have
 * no lint code, and the inner file rides along in the message - unless the
 * message already names it - because the drawer never renders `path`.
 */
export function findingsOf(file: BookFile): Finding[] | undefined {
  if (file.result?.kind === "check") return file.result.report.findings;
  if (file.result?.kind === "fit" && file.result.report.warnings.length > 0) {
    return file.result.report.warnings.map((w) => ({
      severity: "warning" as const,
      code: "",
      message: w.file && !w.message.includes(w.file) ? `${w.message} (${w.file})` : w.message,
      path: w.file,
    }));
  }
  return file.cleanup && file.cleanup.findings.length > 0 ? file.cleanup.findings : undefined;
}

/**
 * What a finding means for the user, as opposed to what it is technically
 * about. `blocked` is the one that is not a `Category`: DRM is structural in
 * the CLI's terms, but for the app it is the only concern no profile can fix,
 * and offering a repair button for it is worse than offering nothing.
 */
export type Concern = "blocked" | "defect" | "watermark" | "bloat" | "device";

/**
 * A file's overall state. `checking` and `unknown` exist because "nothing
 * found" and "we never looked" must never render the same: a row that says
 * "clean" and then flips to "defective" is worse than one that says nothing
 * for another second.
 */
export type Verdict = "unknown" | "checking" | "clean" | "attention" | "broken";

export interface Condition {
  verdict: Verdict;
  /** Concerns present, in precedence order. */
  concerns: Concern[];
  errors: number;
  warnings: number;
  infos: number;
  /** Bytes an in-place repair would reclaim, where findings report a size. */
  wastedBytes: number;
  /** Concerns an in-place repair can actually remove. */
  fixable: Concern[];
}

/**
 * Which concern a finding belongs to. Keys off the CLI's `category` and falls
 * back to the code, so this keeps working against a sidecar that predates the
 * field - and, more importantly, an unrecognised future code still lands
 * somewhere visible (`defect`) instead of vanishing. That fallback is safe
 * because the row tint is driven by severity, not concern: an unknown `info`
 * earns the quiet treatment either way.
 */
export function concernOf(finding: Finding): Concern {
  // DRM first, and by code: the CLI files it under `structure` because that is
  // what it is, but no profile can repair it, and that distinction is the
  // whole difference between a useful button and one that fails.
  if (finding.code === "drm" && finding.severity === "error") return "blocked";
  switch (finding.category) {
    case "watermark":
      return "watermark";
    case "waste":
      return "bloat";
    case "device":
      return "device";
    case "structure":
      return "defect";
  }
  // No category: an older sidecar. Recover from the code where we can.
  if (finding.code.startsWith("watermark-") || finding.code === "media-metadata") {
    return "watermark";
  }
  if (finding.code === "unreferenced") return "bloat";
  if (["fonts", "css-caps", "image-format"].includes(finding.code)) return "device";
  return "defect";
}

/** Precedence, worst first - the order a verdict pill picks its label from. */
const CONCERN_ORDER: Concern[] = ["blocked", "defect", "watermark", "bloat", "device"];

/**
 * Concerns an in-place repair removes. `device` is absent because fitting for
 * a device produces a *copy*, which is Fit mode's job, not a rewrite of the
 * user's book; `blocked` is absent because nothing removes DRM.
 */
const FIXABLE: Concern[] = ["defect", "watermark", "bloat"];

const EMPTY_CONDITION: Condition = {
  verdict: "unknown",
  concerns: [],
  errors: 0,
  warnings: 0,
  infos: 0,
  wastedBytes: 0,
  fixable: [],
};

/**
 * The condition of one file, from the automatic check-on-add.
 *
 * Reads `file.check` (the lifecycle) before `file.cleanup` (the payload), so a
 * stale report from before an in-place write cannot outrank the fact that a
 * fresh check is still running.
 */
export function fileCondition(file: BookFile): Condition {
  if (file.ingest === "failed") {
    return { ...EMPTY_CONDITION, verdict: "broken" };
  }
  if (file.check === "pending") return { ...EMPTY_CONDITION, verdict: "checking" };
  if (file.check === "failed" || !file.cleanup) return EMPTY_CONDITION;

  const findings = file.cleanup.findings;
  const concerns = CONCERN_ORDER.filter((c) => findings.some((f) => concernOf(f) === c));
  const count = (s: Finding["severity"]) => findings.filter((f) => f.severity === s).length;
  const errors = count("error");
  const warnings = count("warning");
  const infos = count("info");
  const fixable = concerns.filter((c) => FIXABLE.includes(c));
  // Severity, not concern, decides how loud this is. An `info`-only book is
  // worth mentioning but is not "needing attention" - which is what stops a
  // font-obfuscated book (a lone `drm` info saying "safe to process") from
  // shouting exactly as loudly as a broken one.
  const verdict: Verdict = errors > 0 ? "broken" : warnings > 0 ? "attention" : "clean";
  return {
    verdict,
    concerns,
    errors,
    warnings,
    infos,
    wastedBytes: findings.reduce((sum, f) => sum + (f.bytes ?? 0), 0),
    fixable,
  };
}

/** The worst condition across a book's files, for the row-level tint. */
export function bookCondition(book: Book): Condition {
  const RANK: Record<Verdict, number> = {
    unknown: 0,
    checking: 1,
    clean: 2,
    attention: 3,
    broken: 4,
  };
  return book.files
    .map(fileCondition)
    .reduce((worst, c) => (RANK[c.verdict] > RANK[worst.verdict] ? c : worst), EMPTY_CONDITION);
}

/** Whether an in-place repair has anything to do for this file. */
export function isFixable(file: BookFile): boolean {
  return fileCondition(file).fixable.length > 0;
}

/**
 * The `--profile` specs an in-place repair needs for `concerns`.
 *
 * Structure is repaired by `epub` alone, which is bookmark-safe. Watermarks
 * and dead weight live behind `generic`, which replaces the unique identifier
 * and so orphans the reader's position - so this NEVER returns it implicitly.
 * A caller that wants it asks for it: see `WATERMARK_PROFILE` and the separate
 * action built on it. The consequence of getting this wrong is a button
 * labelled "Clean up" quietly costing the user their place in the book.
 */
export function repairProfiles(_concerns: Concern[]): string[] {
  return [CLEANUP_PROFILE];
}

/** How many of `files` need the user to do something, and how many are broken. */
export function conditionSummary(files: BookFile[]): { attention: number; broken: number } {
  let attention = 0;
  let broken = 0;
  for (const file of files) {
    const { verdict } = fileCondition(file);
    if (verdict === "broken") broken += 1;
    else if (verdict === "attention") attention += 1;
  }
  return { attention, broken };
}

/**
 * The badge text for a book whose ORIGINAL is itself a produced copy (added
 * without its source), or `null` for a plain source. Driven by the
 * *filename* (`<stem>.<appendix>.epub`), never by the provenance stamp
 * alone: a stamp proves the file was fitted, not that it is a copy - a book
 * fitted in place (the old "Replace originals", or the CLI) carries a
 * device-profile stamp and is still the user's only original. The stamp only
 * refines the badge text: when it names a device profile, that beats the
 * appendix parsed from the name.
 */
export function copyBadge(original: BookFile, appendixes: string[]): string | null {
  const parsed = parseCopyName(original.fileName, appendixes);
  if (!parsed) return null;
  const profile = original.fitted?.profile;
  return profile && profile !== CLEANUP_PROFILE ? profile : parsed.appendix;
}

/**
 * The badge on a file row: `original` for the file the book stands for, else
 * the profile (or naming appendix) the copy was made under.
 */
export function fileBadge(file: BookFile): string {
  if (file.role === "original") return "original";
  return file.profile ?? file.appendix ?? "copy";
}

/**
 * The failure this file can explain: a conversion that failed, or a file we
 * could not even read in the first place. Both carry their own stderr, so
 * the drawer keeps working long after the job behind it has been pruned.
 */
export function failureOf(file: BookFile): Failure | undefined {
  if (file.result?.kind === "failed") {
    return {
      friendly: file.result.friendly,
      code: file.result.failure.code,
      stderr: file.result.stderr,
    };
  }
  return file.ingest === "failed" && file.ingestError ? file.ingestError : undefined;
}

/** "1 warning" / "3 warnings" - the label every counting chip uses. */
function counted(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

function sizeChip(stats: Stats): Chip {
  if (stats.bytes_in > 0 && stats.bytes_out < stats.bytes_in) {
    const pct = Math.round((1 - stats.bytes_out / stats.bytes_in) * 100);
    return {
      label: `-${pct}%`,
      tone: "good",
      title: `${formatSize(stats.bytes_in)} to ${formatSize(stats.bytes_out)}`,
    };
  }
  return { label: `wrote ${formatSize(stats.bytes_out)}`, tone: "neutral" };
}

/** How a concern reads on the verdict pill, worst first. */
const CONCERN_LABEL: Record<Concern, { label: string; tone: Tone }> = {
  blocked: { label: "copy protected", tone: "bad" },
  defect: { label: "needs cleanup", tone: "warn" },
  watermark: { label: "watermarked", tone: "warn" },
  bloat: { label: "extra files", tone: "neutral" },
  device: { label: "over device limits", tone: "neutral" },
};

/**
 * One pill for everything the automatic check found, or `null` when it found
 * nothing worth saying.
 *
 * One pill and not one per concern: three 10px chips per row, across twenty
 * rows, is the wall of noise that trains an eye to skip the status column
 * entirely. The worst concern names the pill, a `+N` admits there is more, and
 * the tooltip and drawer carry the detail.
 *
 * `id: "condition"` is what `FileRow` keys its repair affordance off.
 */
export function conditionChip(file: BookFile): Chip | null {
  const condition = fileCondition(file);
  if (file.check === "failed") {
    return {
      id: "condition",
      label: "not checked",
      tone: "neutral",
      title: file.checkError?.friendly ?? "The automatic check did not complete.",
    };
  }
  // A `defect` concern with no error is a warning-level cleanup; with an error
  // it is a real defect, and the label has to say so.
  const [worst] = condition.concerns;
  if (!worst) return null;
  const base = CONCERN_LABEL[worst];
  const label =
    worst === "defect" && condition.errors > 0 ? "defective" : base.label;
  const tone: Tone = worst === "defect" && condition.errors > 0 ? "bad" : base.tone;
  const extra = condition.concerns.length - 1;
  const detail = condition.concerns.map((c) => CONCERN_LABEL[c].label).join(", ");
  return {
    id: "condition",
    label: extra > 0 ? `${label} +${extra}` : label,
    tone,
    title: `${detail} - ${counted(condition.errors + condition.warnings + condition.infos, "finding")} from the automatic check`,
  };
}

/**
 * The status chips for a file: the result of its last job, or the state it
 * is stuck in. Empty when there is nothing yet to say (no result, and the
 * file read in fine).
 */
export function chipsFor(file: BookFile): Chip[] {
  const chips: Chip[] = [];

  // The automatic check's verdict rides along unless an explicit check is
  // showing its own findings, or a failure has bigger news to break.
  if (file.result?.kind !== "check" && file.result?.kind !== "failed") {
    const chip = conditionChip(file);
    if (chip) chips.push(chip);
  }

  if (file.result?.kind === "fit") {
    chips.push(sizeChip(file.result.report.stats));
    if (file.result.report.dry_run) {
      chips.push({ label: "preview", tone: "neutral" });
    }
    if (file.result.report.stats.warnings > 0) {
      const messages = file.result.report.warnings.map((w) => w.message).join("\n");
      chips.push({
        label: counted(file.result.report.stats.warnings, "warning"),
        tone: "warn",
        title: messages || undefined,
      });
    }
  } else if (file.result?.kind === "check") {
    if (file.result.report.errors > 0) {
      chips.push({ label: counted(file.result.report.errors, "error"), tone: "bad" });
    }
    if (file.result.report.warnings > 0) {
      chips.push({ label: counted(file.result.report.warnings, "warning"), tone: "warn" });
    }
    if (file.result.report.errors === 0 && file.result.report.warnings === 0) {
      chips.push({ label: "clean", tone: "good" });
    }
  } else if (file.result?.kind === "failed") {
    chips.push({ label: "failed", tone: "bad" });
  } else if (file.result?.kind === "cancelled") {
    chips.push({ label: "cancelled", tone: "neutral" });
  } else if (file.ingest === "failed") {
    chips.push({ label: "could not read", tone: "bad" });
  }

  return chips;
}
