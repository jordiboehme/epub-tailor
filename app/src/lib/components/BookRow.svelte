<script lang="ts">
  // One book, one row: the same book a BookCard shows, laid out horizontally
  // for the list view. Everything it says about a book - the title, the byline,
  // the chips, the failure - comes from api/book-view, the same helpers the card
  // consumes, so the two views can never disagree about what a book's state is.
  import { slide } from "svelte/transition";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { coverUrl } from "../api/covers";
  import { books } from "../stores/books.svelte";
  import type { Book } from "../stores/books.svelte";
  import { jobs } from "../stores/jobs.svelte";
  import { edits } from "../stores/edits.svelte";
  import {
    bookByline,
    bookInitials,
    bookTitle,
    chipsFor,
    failureOf,
    findingsOf,
    TONE_CLASS,
    writtenPathOf,
  } from "../api/book-view";
  import type { Chip } from "../api/book-view";
  import CardDetails from "./CardDetails.svelte";

  let { book }: { book: Book } = $props();

  let imgError = $state(false);
  let showDetails = $state(false);

  $effect(() => {
    // Reset the load-error flag whenever this row's cover changes, same as the
    // card does: a thumbnail that failed to load once (an ingest still writing
    // the file, say) would otherwise show its initials for the rest of the
    // session, even after a cover has been fetched or picked.
    void book.coverPath;
    imgError = false;
  });

  const selected = $derived(books.selectedIds.has(book.id));
  const edited = $derived(edits.hasEdits(book.id));
  const job = $derived(jobs.conversionJobFor(book.id));
  const running = $derived(job?.state === "running");
  const queued = $derived(job?.state === "queued");
  const unreadable = $derived(book.ingest === "failed");

  const title = $derived(bookTitle(book));
  const byline = $derived(bookByline(book));
  const initials = $derived(bookInitials(book));
  const hasCover = $derived(!!book.coverPath && !imgError);

  const writtenPath = $derived(writtenPathOf(book));
  const failure = $derived(failureOf(book));
  const findings = $derived(findingsOf(book));
  const canExpand = $derived(Boolean(failure || findings));
  const chips = $derived(chipsFor(book));

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
  <span class="rounded-md px-1.5 py-0.5 text-[11px] font-medium {TONE_CLASS[c.tone]}" title={c.title}>
    {c.label}
  </span>
{/snippet}

<div
  role="button"
  tabindex="0"
  aria-pressed={selected}
  onclick={onClick}
  onkeydown={onKey}
  class="group relative cursor-default text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-indigo-500 {selected
    ? 'bg-indigo-50 ring-2 ring-inset ring-indigo-500/60 dark:bg-indigo-500/10'
    : unreadable
      ? 'bg-rose-50/70 ring-1 ring-inset ring-rose-200 hover:bg-rose-50 dark:bg-rose-950/20 dark:ring-rose-500/30 dark:hover:bg-rose-950/40'
      : running
        ? 'bg-indigo-50/60 dark:bg-indigo-500/5'
        : 'hover:bg-white dark:hover:bg-zinc-900'}"
>
  <div class="relative flex items-center gap-3 px-4 py-2">
    <!-- Cover thumbnail -->
    <div
      class="relative aspect-[2/3] h-12 w-8 shrink-0 overflow-hidden rounded bg-zinc-100 dark:bg-zinc-800"
    >
      {#if hasCover}
        <img
          src={coverUrl(book.coverPath!)}
          alt={"Cover of " + title}
          onerror={() => (imgError = true)}
          class="h-full w-full object-cover"
        />
      {:else if unreadable}
        <div class="flex h-full w-full items-center justify-center bg-rose-50 dark:bg-rose-950/40">
          <svg
            class="h-4 w-4 text-rose-400 dark:text-rose-500"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
          >
            <path d="M12 8v5" stroke-linecap="round" />
            <circle cx="12" cy="16.5" r="0.9" fill="currentColor" stroke="none" />
            <path d="M10.6 3.9L2.7 17.6a1.6 1.6 0 001.4 2.4h15.8a1.6 1.6 0 001.4-2.4L13.4 3.9a1.6 1.6 0 00-2.8 0z" stroke-linejoin="round" />
          </svg>
        </div>
      {:else}
        <div class="flex h-full w-full items-center justify-center">
          <span class="text-[11px] font-semibold tracking-wide text-zinc-400 dark:text-zinc-500">
            {initials || "?"}
          </span>
        </div>
      {/if}

      <!-- Running veil, scaled to the thumbnail -->
      {#if running}
        <div class="absolute inset-0 flex items-center justify-center bg-zinc-950/50 backdrop-blur-[1px]">
          <svg class="h-4 w-4 animate-spin text-white" viewBox="0 0 24 24" fill="none">
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" />
            <path class="opacity-90" d="M12 2a10 10 0 019 5.5" stroke="currentColor" stroke-width="3" stroke-linecap="round" />
          </svg>
        </div>
      {/if}
    </div>

    <!-- Title, byline, and the failure beneath it when there is one: a failed
         row is a line taller, but the author stays visible instead of being
         swapped out for the failure text. -->
    <div class="min-w-0 flex-1">
      <div class="flex min-w-0 items-center gap-1.5">
        <p class="truncate text-[13px] font-medium text-zinc-800 dark:text-zinc-100" title={title}>
          {title}
        </p>
        {#if book.kind === "md"}
          <span
            class="shrink-0 rounded px-1 py-0.5 text-[10px] font-medium uppercase tracking-wide {TONE_CLASS.neutral}"
          >
            md
          </span>
        {/if}
        {#if edited}
          <span
            title="Has staged metadata edits, written on the next Tailor"
            class="inline-flex shrink-0 items-center gap-0.5 rounded bg-indigo-600/85 px-1 py-0.5 text-[10px] font-medium text-white"
          >
            <svg class="h-2.5 w-2.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M13.5 4.5l2 2L8 14l-3 1 1-3 7.5-7.5z" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
            edited
          </span>
        {/if}
      </div>

      {#if byline}
        <p class="truncate text-[11px] text-zinc-500 dark:text-zinc-400" title={byline}>{byline}</p>
      {/if}
      {#if failure && !showDetails}
        <p
          class="line-clamp-1 text-[11px] leading-snug text-rose-600 dark:text-rose-400"
          title={failure.friendly}
        >
          {failure.friendly}
        </p>
      {/if}
    </div>

    <!-- Chips, the details toggle, the hover actions -->
    <div class="flex shrink-0 items-center gap-1.5">
      {#each chips as c}
        {@render chip(c)}
      {/each}

      {#if queued}
        <span class="rounded-md px-1.5 py-0.5 text-[11px] font-medium {TONE_CLASS.neutral}">queued</span>
      {/if}

      {#if canExpand}
        <button
          type="button"
          onclick={(e) => {
            e.stopPropagation();
            showDetails = !showDetails;
          }}
          class="rounded px-1 py-0.5 text-[11px] font-medium text-zinc-500 hover:text-zinc-700 dark:text-zinc-400 dark:hover:text-zinc-200"
        >
          {showDetails ? "Less" : "Details"}
        </button>
      {/if}

      <!--
        The row's actions. They are laid out even while hidden and only faded
        in on hover: a list is a column of aligned things, and popping two
        buttons into existence would shove every chip on the row sideways.
      -->
      {#if !running && !queued}
        <div
          class="flex items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100"
        >
          {#if writtenPath}
            <button
              type="button"
              title="Show the tailored file in the file manager"
              onclick={reveal}
              class="rounded-md p-1 text-zinc-500 transition-colors hover:bg-indigo-100 hover:text-indigo-700 dark:text-zinc-400 dark:hover:bg-indigo-500/20 dark:hover:text-indigo-300"
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
            class="rounded-md p-1 text-zinc-500 transition-colors hover:bg-rose-100 hover:text-rose-600 dark:text-zinc-400 dark:hover:bg-rose-500/20 dark:hover:text-rose-300"
          >
            <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M6 6l8 8M14 6l-8 8" stroke-linecap="round" />
            </svg>
          </button>
        </div>
      {/if}
    </div>

    <!-- Busy along the whole row: the shimmer a running card runs under its cover. -->
    {#if running}
      <div class="absolute inset-x-0 bottom-0 h-0.5 overflow-hidden bg-indigo-500/15">
        <div class="h-full w-1/3 animate-[shimmer_1.1s_ease-in-out_infinite] bg-indigo-500"></div>
      </div>
    {:else if book.ingest === "pending"}
      <div class="absolute inset-x-0 bottom-0 h-0.5 animate-pulse bg-indigo-300/70"></div>
    {/if}
  </div>

  {#if showDetails && canExpand}
    <div transition:slide={{ duration: 150 }}>
      <!-- Indented to the row's text column (px-4 + a 32px thumb + gap-3), so
           the drawer reads as this row's own, not as a block adrift under it. -->
      <CardDetails {findings} {failure} padding="py-2.5 pr-4 pl-[60px]" />
    </div>
  {/if}
</div>
