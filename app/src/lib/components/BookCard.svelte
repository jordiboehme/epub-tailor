<script lang="ts">
  import { slide } from "svelte/transition";
  import { coverUrl } from "../api/covers";
  import { books } from "../stores/books.svelte";
  import type { Book } from "../stores/books.svelte";
  import { jobs } from "../stores/jobs.svelte";
  import type { Stats } from "../api/contract";

  let { book }: { book: Book } = $props();

  let imgError = $state(false);
  let showFindings = $state(false);

  const selected = $derived(books.selectedIds.has(book.id));
  const job = $derived(jobs.conversionJobFor(book.id));
  const running = $derived(job?.state === "running");
  const queued = $derived(job?.state === "queued");

  const stem = $derived(book.fileName.replace(/\.[^.]+$/, ""));
  const title = $derived(book.meta?.title?.trim() || stem);
  const subtitle = $derived(book.meta?.authors?.[0] ?? (book.kind === "md" ? "Markdown" : ""));
  const hasCover = $derived(!!book.coverPath && !imgError);

  const initials = $derived(
    stem
      .split(/[\s_·—–-]+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w[0]?.toUpperCase() ?? "")
      .join(""),
  );

  function mib(bytes: number): string {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  interface Chip {
    label: string;
    tone: "good" | "warn" | "bad" | "neutral";
    title?: string;
  }

  function sizeChip(stats: Stats): Chip {
    if (stats.bytes_in > 0 && stats.bytes_out < stats.bytes_in) {
      const pct = Math.round((1 - stats.bytes_out / stats.bytes_in) * 100);
      return { label: `-${pct}%`, tone: "good", title: `${mib(stats.bytes_in)} to ${mib(stats.bytes_out)}` };
    }
    return { label: `wrote ${mib(stats.bytes_out)}`, tone: "neutral" };
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
    {:else}
      <div class="flex h-full w-full flex-col items-center justify-center gap-2 p-3 text-center">
        <span class="text-2xl font-semibold tracking-wide text-zinc-400 dark:text-zinc-500">
          {initials || "?"}
        </span>
        <span class="line-clamp-3 text-[11px] leading-tight text-zinc-400 dark:text-zinc-500">
          {title}
        </span>
      </div>
    {/if}

    {#if book.kind === "md"}
      <span
        class="absolute left-1.5 top-1.5 rounded bg-zinc-900/70 px-1 py-0.5 text-[10px] font-medium uppercase tracking-wide text-white"
      >
        md
      </span>
    {/if}

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

    <!-- Remove button, on hover (hidden while this book is busy) -->
    {#if !running && !queued}
      <button
        type="button"
        title="Remove"
        onclick={(e) => {
          e.stopPropagation();
          books.remove([book.id]);
        }}
        class="absolute right-1.5 top-1.5 hidden rounded-full bg-zinc-900/70 p-1 text-white transition-colors hover:bg-rose-600 group-hover:block"
      >
        <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M6 6l8 8M14 6l-8 8" stroke-linecap="round" />
        </svg>
      </button>
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

    <!-- Result chips -->
    {#if book.result}
      <div class="mt-1.5 flex flex-wrap items-center gap-1">
        {#if book.result.kind === "fit"}
          {@render chip(sizeChip(book.result.report.stats))}
          {#if book.result.report.dry_run}
            {@render chip({ label: "preview", tone: "neutral" })}
          {/if}
          {#if book.result.report.stats.warnings > 0}
            {@render chip({ label: `${book.result.report.stats.warnings} warnings`, tone: "warn" })}
          {/if}
        {:else if book.result.kind === "check"}
          <button
            type="button"
            onclick={(e) => {
              e.stopPropagation();
              showFindings = !showFindings;
            }}
            class="flex items-center gap-1"
          >
            {#if book.result.report.errors > 0}
              {@render chip({ label: `${book.result.report.errors} errors`, tone: "bad" })}
            {/if}
            {#if book.result.report.warnings > 0}
              {@render chip({ label: `${book.result.report.warnings} warnings`, tone: "warn" })}
            {/if}
            {#if book.result.report.errors === 0 && book.result.report.warnings === 0}
              {@render chip({ label: "clean", tone: "good" })}
            {/if}
          </button>
        {:else if book.result.kind === "failed"}
          {@render chip({ label: "Failed", tone: "bad", title: book.result.friendly })}
        {:else if book.result.kind === "cancelled"}
          {@render chip({ label: "cancelled", tone: "neutral" })}
        {/if}
      </div>

      {#if book.result.kind === "failed"}
        <p class="mt-1 line-clamp-2 text-[11px] leading-snug text-rose-600 dark:text-rose-400" title={book.result.friendly}>
          {book.result.friendly}
        </p>
      {/if}
    {/if}
  </div>

  <!-- Findings panel (check results) -->
  {#if showFindings && book.result?.kind === "check"}
    <div transition:slide={{ duration: 150 }} class="border-t border-zinc-200 px-2.5 py-2 dark:border-zinc-800">
      {#if book.result.report.findings.length === 0}
        <p class="text-[11px] text-zinc-500 dark:text-zinc-400">Nothing to report.</p>
      {:else}
        <ul class="flex max-h-40 flex-col gap-1.5 overflow-y-auto">
          {#each book.result.report.findings as finding (finding.code + finding.message)}
            <li class="flex items-start gap-1.5 text-[11px] leading-snug">
              <span
                class="mt-0.5 h-1.5 w-1.5 shrink-0 rounded-full {finding.severity === 'error'
                  ? 'bg-rose-500'
                  : finding.severity === 'warning'
                    ? 'bg-amber-500'
                    : 'bg-zinc-400'}"
              ></span>
              <span class="min-w-0">
                <span class="font-mono text-zinc-400 dark:text-zinc-500">{finding.code}</span>
                <span class="text-zinc-600 dark:text-zinc-300"> {finding.message}</span>
              </span>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</div>
