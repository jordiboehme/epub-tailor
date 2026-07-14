// Shared per-book display logic: title/subtitle fallbacks, cover initials,
// the failure a card can explain, the findings a check produced, and the
// status chips. A gallery card and a list row need exactly the same data, and
// duplicating it would let the two views drift - worst in the failure states,
// where users are already unhappy. So it lives here once.
//
// Pure functions only - no store imports, no Tauri imports - so vitest
// reaches them directly, same as format.ts, meta.ts and templates.ts.

import { stemOf } from "../stores/books.svelte";
import type { Book } from "../stores/books.svelte";
import type { Finding, Stats } from "./contract";
import { formatSize } from "./format";

/**
 * The failure a card (or row) can explain: a conversion that failed, or a
 * book that could not even be read in the first place. Carries its own
 * stderr tail rather than pointing at the job that produced it, since jobs
 * are pruned once the next batch starts.
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
}

/** Tone to Tailwind classes, so a gallery card and a list row style chips identically. */
export const TONE_CLASS: Record<Tone, string> = {
  good: "bg-emerald-100 text-emerald-700 dark:bg-emerald-500/15 dark:text-emerald-300",
  warn: "bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300",
  bad: "bg-rose-100 text-rose-700 dark:bg-rose-500/15 dark:text-rose-300",
  neutral: "bg-zinc-200 text-zinc-600 dark:bg-zinc-700 dark:text-zinc-300",
};

/** The title to show: the book's own metadata title, else the file name's stem. */
export function bookTitle(book: Book): string {
  return book.meta?.title?.trim() || stemOf(book.fileName);
}

/** The subtitle: the first author, else "Markdown" for a Markdown book, else nothing. */
export function bookSubtitle(book: Book): string {
  return book.meta?.authors?.[0] ?? (book.kind === "md" ? "Markdown" : "");
}

/**
 * The series a book belongs to, with its position when it has one: "Dune #2".
 * Empty when the book carries no series at all.
 */
export function bookSeries(book: Book): string {
  const series = book.meta?.series?.trim();
  if (!series) return "";
  const index = book.meta?.seriesIndex?.trim();
  return index ? `${series} #${index}` : series;
}

/**
 * The one line under a row's title: the author and the series, whichever of
 * them the book has. A row has the width to say both; a card only shows the
 * subtitle, which is why this lives beside `bookSubtitle` rather than in it.
 */
export function bookByline(book: Book): string {
  return [bookSubtitle(book), bookSeries(book)].filter(Boolean).join(" · ");
}

/** Up to two initials from the stem's words, for a coverless placeholder. */
export function bookInitials(book: Book): string {
  return stemOf(book.fileName)
    .split(/[\s_·—–-]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]?.toUpperCase() ?? "")
    .join("");
}

/**
 * The written file, when there is one to reveal: a real conversion, not a
 * preview (a dry run writes nothing, so there is nothing to show anyone).
 */
export function writtenPathOf(book: Book): string | null {
  return book.result?.kind === "fit" && !book.result.report.dry_run
    ? book.result.report.output
    : null;
}

/** The findings a check produced, when this book's last job was a check. */
export function findingsOf(book: Book): Finding[] | undefined {
  return book.result?.kind === "check" ? book.result.report.findings : undefined;
}

/**
 * The failure this book can explain: a conversion that failed, or a book we
 * could not even read in the first place. Both carry their own stderr, so
 * the drawer keeps working long after the job behind it has been pruned.
 */
export function failureOf(book: Book): Failure | undefined {
  if (book.result?.kind === "failed") {
    return {
      friendly: book.result.friendly,
      code: book.result.failure.code,
      stderr: book.result.stderr,
    };
  }
  return book.ingest === "failed" && book.ingestError ? book.ingestError : undefined;
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
 * The status chips for a book: the result of its last job, or the state it
 * is stuck in. Empty when there is nothing yet to say (no result, and the
 * book read in fine).
 */
export function chipsFor(book: Book): Chip[] {
  const chips: Chip[] = [];

  if (book.result?.kind === "fit") {
    chips.push(sizeChip(book.result.report.stats));
    if (book.result.report.dry_run) {
      chips.push({ label: "preview", tone: "neutral" });
    }
    if (book.result.report.stats.warnings > 0) {
      chips.push({ label: `${book.result.report.stats.warnings} warnings`, tone: "warn" });
    }
  } else if (book.result?.kind === "check") {
    if (book.result.report.errors > 0) {
      chips.push({ label: `${book.result.report.errors} errors`, tone: "bad" });
    }
    if (book.result.report.warnings > 0) {
      chips.push({ label: `${book.result.report.warnings} warnings`, tone: "warn" });
    }
    if (book.result.report.errors === 0 && book.result.report.warnings === 0) {
      chips.push({ label: "clean", tone: "good" });
    }
  } else if (book.result?.kind === "failed") {
    chips.push({ label: "failed", tone: "bad" });
  } else if (book.result?.kind === "cancelled") {
    chips.push({ label: "cancelled", tone: "neutral" });
  } else if (book.ingest === "failed") {
    chips.push({ label: "could not read", tone: "bad" });
  }

  return chips;
}
