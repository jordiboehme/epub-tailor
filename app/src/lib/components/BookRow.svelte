<script lang="ts">
  // One book, one row group: the same virtual folder a BookCard shows, laid
  // out as columns for the list view. The row's metadata cells come from the
  // ORIGINAL file (files[0]) and clicking the row selects exactly that file;
  // the file list beneath carries the per-file state. Everything it says
  // about a file comes from api/book-view, the same helpers the card
  // consumes, so the two views can never disagree.
  import { coverUrl } from "../api/covers";
  import { books } from "../stores/books.svelte";
  import type { Book } from "../stores/books.svelte";
  import { jobs } from "../stores/jobs.svelte";
  import { edits } from "../stores/edits.svelte";
  import { profiles } from "../stores/profiles.svelte";
  import { knownAppendixes } from "../api/copies";
  import {
    copyBadge,
    fileAuthors,
    fileInitials,
    fileSeries,
    fileTitle,
    fileYear,
    TONE_CLASS,
  } from "../api/book-view";
  import FileList from "./FileList.svelte";

  let { book }: { book: Book } = $props();

  let imgError = $state(false);

  const original = $derived(book.files[0]);

  $effect(() => {
    // Reset the load-error flag whenever this row's cover changes, same as the
    // card does: a thumbnail that failed to load once (an ingest still writing
    // the file, say) would otherwise show its initials for the rest of the
    // session, even after a cover has been fetched or picked.
    void original.coverPath;
    imgError = false;
  });

  // The body stands for the original; its selected look means "that file is
  // a target". A selected copy highlights its own row below.
  const selected = $derived(books.selectedFileIds.has(original.id));
  const job = $derived(jobs.conversionJobFor(original.id));
  const running = $derived(job?.state === "running");
  const queued = $derived(job?.state === "queued");
  const unreadable = $derived(original.ingest === "failed");
  const anyBusy = $derived(
    book.files.some((f) => {
      const state = jobs.conversionJobFor(f.id)?.state;
      return state === "running" || state === "queued";
    }),
  );

  const staged = $derived(edits.get(original.id));

  const title = $derived(fileTitle(original, staged));
  const authors = $derived(fileAuthors(original, staged));
  const series = $derived(fileSeries(original, staged));
  const year = $derived(fileYear(original, staged));

  const titleEdited = $derived(staged?.title !== undefined);
  const authorsEdited = $derived(staged !== undefined && staged.authors !== undefined);
  const seriesEdited = $derived(
    staged !== undefined && (staged.series !== undefined || staged.seriesIndex !== undefined),
  );
  const dateEdited = $derived(staged !== undefined && staged.date !== undefined);

  const authorsCleared = $derived(staged?.authors === null);
  const seriesCleared = $derived(staged?.series === null);
  const dateCleared = $derived(staged?.date === null);

  const initials = $derived(fileInitials(original));
  const hasCover = $derived(!!original.coverPath && !imgError);
  const badge = $derived(copyBadge(original, knownAppendixes(profiles.builtins)));

  function onClick(event: MouseEvent) {
    if (event.shiftKey) books.range(original.id);
    else if (event.metaKey || event.ctrlKey) books.toggle(original.id);
    else books.select(original.id);
  }

  function onKey(event: KeyboardEvent) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      books.select(original.id);
    }
  }
</script>

{#snippet cell(text: string, isEdited: boolean, isCleared: boolean)}
  {#if isCleared}
    <span class="truncate text-[12px] italic text-teal-500/70 dark:text-teal-300/60" title="will be removed">-</span>
  {:else}
    <span
      class="truncate text-[12px] {isEdited
        ? 'font-medium text-teal-700 dark:text-teal-300'
        : 'text-ink-600 dark:text-ink-400'}"
      title={text}
    >
      {text}
    </span>
  {/if}
{/snippet}

<div
  role="button"
  tabindex="0"
  aria-pressed={selected}
  onclick={onClick}
  onkeydown={onKey}
  class="group relative cursor-default text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-teal-500 dark:focus-visible:ring-teal-400 {selected
    ? 'bg-teal-50 ring-2 ring-inset ring-teal-500/60 dark:bg-teal-500/10 dark:ring-teal-400/50 dark:shadow-glow-inset'
    : unreadable
      ? 'bg-rose-50/70 ring-1 ring-inset ring-rose-200 hover:bg-rose-50 dark:bg-rose-950/20 dark:ring-rose-500/30 dark:hover:bg-rose-950/40'
      : running
        ? 'bg-teal-50/60 dark:bg-teal-500/5'
        : 'hover:bg-white dark:hover:bg-ink-900'}"
>
  <div class="relative px-4 py-2">
    <div class="book-list-grid">
      <!-- Cover thumbnail -->
      <div
        class="relative aspect-[2/3] h-12 w-8 shrink-0 overflow-hidden rounded bg-ink-100 dark:bg-ink-800"
      >
        {#if hasCover}
          <img
            src={coverUrl(original.coverPath!)}
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
            <span class="text-[11px] font-semibold tracking-wide text-ink-400 dark:text-ink-500">
              {initials || "?"}
            </span>
          </div>
        {/if}

        <!-- Running veil, scaled to the thumbnail -->
        {#if running}
          <div class="absolute inset-0 flex items-center justify-center bg-ink-950/50 backdrop-blur-[1px]">
            <svg class="h-4 w-4 animate-spin text-white dark:text-teal-300 dark:drop-shadow-glow" viewBox="0 0 24 24" fill="none">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" />
              <path class="opacity-90" d="M12 2a10 10 0 019 5.5" stroke="currentColor" stroke-width="3" stroke-linecap="round" />
            </svg>
          </div>
        {/if}
      </div>

      <!-- Title cell -->
      <div class="flex min-w-0 items-center gap-1.5">
        <p
          class="truncate text-[13px] font-medium {titleEdited
            ? 'text-teal-700 dark:text-teal-300'
            : 'text-ink-800 dark:text-ink-100'}"
          title={title}
        >
          {title}
        </p>
        {#if original.kind === "md"}
          <span
            class="shrink-0 rounded px-1 py-0.5 text-[10px] font-medium uppercase tracking-wide {TONE_CLASS.neutral}"
          >
            md
          </span>
        {/if}
        {#if badge}
          <span
            title="A copy produced by EPUB Tailor ({badge})"
            class="shrink-0 rounded px-1 py-0.5 text-[10px] font-medium {TONE_CLASS.neutral}"
          >
            copy · {badge}
          </span>
        {/if}
      </div>

      {@render cell(authors, authorsEdited, authorsCleared)}
      {@render cell(series, seriesEdited, seriesCleared)}
      {@render cell(year, dateEdited, dateCleared)}

      <!-- Status cell: the queued badge and the hover remove action -->
      <div class="flex shrink-0 items-center justify-end gap-1.5">
        {#if queued}
          <span class="rounded-md px-1.5 py-0.5 text-[11px] font-medium {TONE_CLASS.neutral}">queued</span>
        {/if}

        {#if !anyBusy}
          <div
            class="flex items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100"
          >
            <button
              type="button"
              title="Remove from the workbench (the files stay where they are)"
              onclick={(e) => {
                e.stopPropagation();
                books.remove([book.id]);
              }}
              class="rounded-md p-1 text-ink-500 transition-colors hover:bg-rose-100 hover:text-rose-600 dark:text-ink-400 dark:hover:bg-rose-500/20 dark:hover:text-rose-300"
            >
              <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M6 6l8 8M14 6l-8 8" stroke-linecap="round" />
              </svg>
            </button>
          </div>
        {/if}
      </div>
    </div>

    <!-- The book's files, always in view and individually selectable.
         Indented to the row's text column. -->
    <div class="pl-[44px] pt-1">
      <FileList {book} />
    </div>

    <!-- Busy along the whole row: the shimmer a running card runs under its cover. -->
    {#if running}
      <div class="absolute inset-x-0 bottom-0 h-0.5 overflow-hidden bg-teal-500/15">
        <div class="h-full w-1/3 animate-[shimmer_1.1s_ease-in-out_infinite] bg-teal-500 dark:bg-teal-400 dark:shadow-glow-sm"></div>
      </div>
    {:else if original.ingest === "pending"}
      <div class="absolute inset-x-0 bottom-0 h-0.5 animate-pulse bg-teal-300/70"></div>
    {/if}
  </div>
</div>
