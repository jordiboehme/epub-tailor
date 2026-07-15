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
import type { BookFile } from "../stores/books.svelte";
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
  neutral: "bg-zinc-200 text-zinc-600 dark:bg-zinc-700 dark:text-zinc-300",
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
 * The findings to show for a file: an explicit check's, or - when the user
 * never ran one - whatever the automatic check-on-add turned up. The explicit
 * result wins because the user asked for it, possibly against a device
 * profile; the automatic probe only ever speaks `epub`.
 */
export function findingsOf(file: BookFile): Finding[] | undefined {
  if (file.result?.kind === "check") return file.result.report.findings;
  return file.cleanup && file.cleanup.findings.length > 0 ? file.cleanup.findings : undefined;
}

/** Whether the automatic check found anything worth an in-place cleanup. */
export function needsCleanup(file: BookFile): boolean {
  return file.cleanup !== undefined && file.cleanup.findings.length > 0;
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

/**
 * The status chips for a file: the result of its last job, or the state it
 * is stuck in. Empty when there is nothing yet to say (no result, and the
 * file read in fine).
 */
export function chipsFor(file: BookFile): Chip[] {
  const chips: Chip[] = [];

  // The automatic check's verdict rides along unless an explicit check is
  // showing its own findings, or a failure has bigger news to break.
  if (needsCleanup(file) && file.result?.kind !== "check" && file.result?.kind !== "failed") {
    const count = file.cleanup!.findings.length;
    chips.push({
      id: "needs-cleanup",
      label: "needs cleanup",
      tone: "warn",
      title: `${count} finding${count === 1 ? "" : "s"} from the automatic check`,
    });
  }

  if (file.result?.kind === "fit") {
    chips.push(sizeChip(file.result.report.stats));
    if (file.result.report.dry_run) {
      chips.push({ label: "preview", tone: "neutral" });
    }
    if (file.result.report.stats.warnings > 0) {
      chips.push({ label: `${file.result.report.stats.warnings} warnings`, tone: "warn" });
    }
  } else if (file.result?.kind === "check") {
    if (file.result.report.errors > 0) {
      chips.push({ label: `${file.result.report.errors} errors`, tone: "bad" });
    }
    if (file.result.report.warnings > 0) {
      chips.push({ label: `${file.result.report.warnings} warnings`, tone: "warn" });
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
