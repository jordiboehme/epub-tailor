<script lang="ts">
  import { slide } from "svelte/transition";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { coverUrl } from "../api/covers";
  import { formatSize } from "../api/format";
  import { books } from "../stores/books.svelte";
  import type { Book } from "../stores/books.svelte";
  import { jobs } from "../stores/jobs.svelte";
  import { edits } from "../stores/edits.svelte";
  import type { Stats } from "../api/contract";
  import CardDetails from "./CardDetails.svelte";

  let { book }: { book: Book } = $props();

  let imgError = $state(false);
  let showDetails = $state(false);

  $effect(() => {
    // Reset the load-error flag whenever this card's cover changes. Without it,
    // a card whose cover failed to load once (an ingest still writing the file,
    // say) shows its initials for the rest of the session, even after a new
    // cover has been fetched or picked. Same pattern as MetadataEditor's preview.
    void book.coverPath;
    imgError = false;
  });

  const selected = $derived(books.selectedIds.has(book.id));
  const edited = $derived(edits.hasEdits(book.id));
  const job = $derived(jobs.conversionJobFor(book.id));
  const running = $derived(job?.state === "running");
  const queued = $derived(job?.state === "queued");
  const unreadable = $derived(book.ingest === "failed");

  const stem = $derived(book.fileName.replace(/\.[^.]+$/, ""));
  const title = $derived(book.meta?.title?.trim() || stem);
  const subtitle = $derived(book.meta?.authors?.[0] ?? (book.kind === "md" ? "Markdown" : ""));
  const hasCover = $derived(!!book.coverPath && !imgError);

  // The written file, when there is one to reveal: a real conversion, not a
  // preview (a dry run writes nothing, so there is nothing to show anyone).
  const writtenPath = $derived(
    book.result?.kind === "fit" && !book.result.report.dry_run ? book.result.report.output : null,
  );

  // The failure this card can explain: a conversion that failed, or a book we
  // could not even read in the first place. Both carry their own stderr, so
  // the drawer keeps working long after the job behind it has been pruned.
  const failure = $derived(
    book.result?.kind === "failed"
      ? {
          friendly: book.result.friendly,
          code: book.result.failure.code,
          stderr: book.result.stderr,
        }
      : unreadable && book.ingestError
        ? book.ingestError
        : undefined,
  );
  const findings = $derived(book.result?.kind === "check" ? book.result.report.findings : undefined);
  const canExpand = $derived(Boolean(failure || findings));

  const initials = $derived(
    stem
      .split(/[\s_·—–-]+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w[0]?.toUpperCase() ?? "")
      .join(""),
  );

  interface Chip {
    label: string;
    tone: "good" | "warn" | "bad" | "neutral";
    title?: string;
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

  const toneClass: Record<Chip["tone"], string> = {
    good: "bg-emerald-100 text-emerald-700 dark:bg-emerald-500/15 dark:text-emerald-300",
    warn: "bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300",
    bad: "bg-rose-100 text-rose-700 dark:bg-rose-500/15 dark:text-rose-300",
    neutral: "bg-zinc-200 text-zinc-600 dark:bg-zinc-700 dark:text-zinc-300",
  };

  function onClick(event: MouseEvent) {
    if (event.shiftKey) books.range(book.id);
    else if (event.metaKey || event.ctrlKey) books.toggle(book.id);
    else books.select(book.id);
  }

  function onKey(event: KeyboardEvent) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      books.select(book.id);
    }
  }

  function reveal(event: MouseEvent) {
    event.stopPropagation();
    if (writtenPath) void revealItemInDir(writtenPath);
  }
</script>

{#snippet chip(c: Chip)}
  <span class="rounded-md px-1.5 py-0.5 text-[11px] font-medium {toneClass[c.tone]}" title={c.title}>
    {c.label}
  </span>
{/snippet}

<div
  role="button"
  tabindex="0"
  aria-pressed={selected}
  onclick={onClick}
  onkeydown={onKey}
  class="group relative flex cursor-default flex-col rounded-xl border bg-white text-left transition-shadow hover:shadow-md dark:bg-zinc-900 {selected
    ? 'border-indigo-500 ring-2 ring-indigo-500/60'
    : unreadable
      ? 'border-rose-300 dark:border-rose-500/40'
      : 'border-zinc-200 dark:border-zinc-800'}"
>
  <!-- Cover -->
  <div class="relative aspect-[2/3] overflow-hidden rounded-t-xl bg-zinc-100 dark:bg-zinc-800">
    {#if hasCover}
      <img
        src={coverUrl(book.coverPath!)}
        alt={"Cover of " + title}
        onerror={() => (imgError = true)}
        class="h-full w-full object-cover"
      />
    {:else if unreadable}
      <div
        class="flex h-full w-full flex-col items-center justify-center gap-1.5 bg-rose-50 p-3 text-center dark:bg-rose-950/30"
      >
        <svg
          class="h-6 w-6 text-rose-400 dark:text-rose-500"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
        >
          <path d="M12 8v5" stroke-linecap="round" />
          <circle cx="12" cy="16.5" r="0.9" fill="currentColor" stroke="none" />
          <path d="M10.6 3.9L2.7 17.6a1.6 1.6 0 001.4 2.4h15.8a1.6 1.6 0 001.4-2.4L13.4 3.9a1.6 1.6 0 00-2.8 0z" stroke-linejoin="round" />
        </svg>
        <span class="text-[11px] font-medium text-rose-600 dark:text-rose-300">Could not read</span>
        <span class="line-clamp-2 text-[10px] leading-tight text-rose-500/80 dark:text-rose-400/70">
          {stem}
        </span>
      </div>
    {:else}
      <div class="flex h-full w-full flex-col items-center justify-center gap-2 p-3 text-center">
        <span class="text-2xl font-semibold tracking-wide text-zinc-400 dark:text-zinc-500">
          {initials || "?"}
        </span>
        <span class="line-clamp-3 text-[11px] leading-tight text-zinc-500 dark:text-zinc-400">
          {title}
        </span>
      </div>
    {/if}

    <div class="absolute left-1.5 top-1.5 flex flex-col items-start gap-1">
      {#if book.kind === "md"}
        <span class="rounded bg-zinc-900/70 px-1 py-0.5 text-[10px] font-medium uppercase tracking-wide text-white">
          md
        </span>
      {/if}
      {#if edited}
        <span
          title="Has staged metadata edits, written on the next Tailor"
          class="inline-flex items-center gap-0.5 rounded bg-indigo-600/85 px-1 py-0.5 text-[10px] font-medium text-white"
        >
          <svg class="h-2.5 w-2.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M13.5 4.5l2 2L8 14l-3 1 1-3 7.5-7.5z" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
          edited
        </span>
      {/if}
    </div>

    <!-- Running veil -->
    {#if running}
      <div class="absolute inset-0 flex items-center justify-center bg-zinc-950/40 backdrop-blur-[1px]">
        <svg class="h-7 w-7 animate-spin text-white" viewBox="0 0 24 24" fill="none">
          <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" />
          <path class="opacity-90" d="M12 2a10 10 0 019 5.5" stroke="currentColor" stroke-width="3" stroke-linecap="round" />
        </svg>
      </div>
      <div class="absolute bottom-0 left-0 h-0.5 w-full overflow-hidden bg-white/20">
        <div class="h-full w-1/3 animate-[shimmer_1.1s_ease-in-out_infinite] bg-indigo-400"></div>
      </div>
    {:else if queued}
      <div class="absolute right-1.5 top-1.5 rounded bg-zinc-900/60 px-1.5 py-0.5 text-[10px] text-white">
        queued
      </div>
    {:else if book.ingest === "pending"}
      <div class="absolute inset-x-0 bottom-0 h-0.5 animate-pulse bg-indigo-300/70"></div>
    {/if}

    <!-- Hover actions (hidden while this book is busy) -->
    {#if !running && !queued}
      <div class="absolute right-1.5 top-1.5 hidden gap-1 group-hover:flex">
        {#if writtenPath}
          <button
            type="button"
            title="Show the tailored file in the file manager"
            onclick={reveal}
            class="rounded-full bg-zinc-900/70 p-1 text-white transition-colors hover:bg-indigo-600"
          >
            <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.7">
              <path d="M2.5 6.5A1.5 1.5 0 014 5h3.2l1.4 1.8H16A1.5 1.5 0 0117.5 8.3v6.2A1.5 1.5 0 0116 16H4a1.5 1.5 0 01-1.5-1.5v-8z" stroke-linejoin="round" />
            </svg>
          </button>
        {/if}
        <button
          type="button"
          title="Remove from the workbench (the file stays where it is)"
          onclick={(e) => {
            e.stopPropagation();
            books.remove([book.id]);
          }}
          class="rounded-full bg-zinc-900/70 p-1 text-white transition-colors hover:bg-rose-600"
        >
          <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M6 6l8 8M14 6l-8 8" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    {/if}
  </div>

  <!-- Title + subtitle -->
  <div class="min-w-0 px-2.5 pb-2 pt-2">
    <p class="truncate text-[13px] font-medium text-zinc-800 dark:text-zinc-100" title={title}>
      {title}
    </p>
    {#if subtitle}
      <p class="truncate text-[11px] text-zinc-500 dark:text-zinc-400" title={subtitle}>{subtitle}</p>
    {/if}

    <!-- Chips: the result, or the state the book is stuck in -->
    {#if book.result || unreadable}
      <div class="mt-1.5 flex flex-wrap items-center gap-1">
        {#if book.result?.kind === "fit"}
          {@render chip(sizeChip(book.result.report.stats))}
          {#if book.result.report.dry_run}
            {@render chip({ label: "preview", tone: "neutral" })}
          {/if}
          {#if book.result.report.stats.warnings > 0}
            {@render chip({ label: `${book.result.report.stats.warnings} warnings`, tone: "warn" })}
          {/if}
        {:else if book.result?.kind === "check"}
          {#if book.result.report.errors > 0}
            {@render chip({ label: `${book.result.report.errors} errors`, tone: "bad" })}
          {/if}
          {#if book.result.report.warnings > 0}
            {@render chip({ label: `${book.result.report.warnings} warnings`, tone: "warn" })}
          {/if}
          {#if book.result.report.errors === 0 && book.result.report.warnings === 0}
            {@render chip({ label: "clean", tone: "good" })}
          {/if}
        {:else if book.result?.kind === "failed"}
          {@render chip({ label: "failed", tone: "bad" })}
        {:else if book.result?.kind === "cancelled"}
          {@render chip({ label: "cancelled", tone: "neutral" })}
        {:else if unreadable}
          {@render chip({ label: "could not read", tone: "bad" })}
        {/if}

        {#if canExpand}
          <button
            type="button"
            onclick={(e) => {
              e.stopPropagation();
              showDetails = !showDetails;
            }}
            class="ml-auto rounded px-1 py-0.5 text-[11px] font-medium text-zinc-500 hover:text-zinc-700 dark:text-zinc-400 dark:hover:text-zinc-200"
          >
            {showDetails ? "Less" : "Details"}
          </button>
        {/if}
      </div>
    {/if}

    {#if failure && !showDetails}
      <p
        class="mt-1 line-clamp-2 text-[11px] leading-snug text-rose-600 dark:text-rose-400"
        title={failure.friendly}
      >
        {failure.friendly}
      </p>
    {/if}
  </div>

  {#if showDetails && canExpand}
    <div transition:slide={{ duration: 150 }}>
      <CardDetails {findings} {failure} />
    </div>
  {/if}
</div>
