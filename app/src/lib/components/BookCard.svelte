<script lang="ts">
  // One book, one card: a virtual folder of files. The header - cover, title,
  // author - comes from the ORIGINAL file (files[0]), and clicking the body
  // selects exactly that file; the file list below carries the per-file
  // state (chips, details, actions). Everything it says about a file comes
  // from api/book-view, the same helpers the list row consumes.
  import { coverUrl } from "../api/covers";
  import { books, stemOf } from "../stores/books.svelte";
  import type { Book } from "../stores/books.svelte";
  import { jobs } from "../stores/jobs.svelte";
  import { edits } from "../stores/edits.svelte";
  import { profiles } from "../stores/profiles.svelte";
  import { countEdits } from "../api/edits";
  import { knownAppendixes } from "../api/copies";
  import { copyBadge, fileInitials, fileSubtitle, fileTitle } from "../api/book-view";
  import FileList from "./FileList.svelte";

  let { book }: { book: Book } = $props();

  let imgError = $state(false);

  const original = $derived(book.files[0]);

  $effect(() => {
    // Reset the load-error flag whenever this card's cover changes. Without it,
    // a card whose cover failed to load once (an ingest still writing the file,
    // say) shows its initials for the rest of the session, even after a new
    // cover has been fetched or picked. Same pattern as MetadataEditor's preview.
    void original.coverPath;
    imgError = false;
  });

  // The body stands for the original: its selected look means "the file this
  // body represents is a target", never "something inside is selected" - the
  // highlighted file row carries that signal.
  const selected = $derived(books.selectedFileIds.has(original.id));
  const staged = $derived(edits.get(original.id));
  const edited = $derived(staged !== undefined);
  const editCount = $derived(staged ? countEdits(staged) : 0);
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

  const stem = $derived(stemOf(original.fileName));
  const title = $derived(fileTitle(original, staged));
  const subtitle = $derived(fileSubtitle(original, staged));
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
        src={coverUrl(original.coverPath!)}
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
      {#if original.kind === "md"}
        <span class="rounded bg-zinc-900/70 px-1 py-0.5 text-[10px] font-medium uppercase tracking-wide text-white">
          md
        </span>
      {/if}
      {#if badge}
        <span
          title="A copy produced by EPUB Tailor ({badge})"
          class="rounded bg-zinc-900/70 px-1 py-0.5 text-[10px] font-medium text-white"
        >
          copy · {badge}
        </span>
      {/if}
      {#if edited}
        <span
          title="Has staged metadata edits, written on the next save"
          class="inline-flex items-center gap-0.5 rounded bg-indigo-600/85 px-1 py-0.5 text-[10px] font-medium text-white"
        >
          <svg class="h-2.5 w-2.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M13.5 4.5l2 2L8 14l-3 1 1-3 7.5-7.5z" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
          edited · {editCount}
        </span>
      {/if}
    </div>

    <!-- Running veil (the original's own job; per-file jobs show on their rows) -->
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
    {:else if original.ingest === "pending"}
      <div class="absolute inset-x-0 bottom-0 h-0.5 animate-pulse bg-indigo-300/70"></div>
    {/if}

    <!-- Hover action: remove the book. Hidden while any of its files is busy. -->
    {#if !anyBusy}
      <div class="absolute right-1.5 top-1.5 hidden gap-1 group-hover:flex">
        <button
          type="button"
          title="Remove from the workbench (the files stay where they are)"
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

    <div class="mt-1.5 border-t border-zinc-100 pt-1.5 dark:border-zinc-800">
      <FileList {book} />
    </div>
  </div>
</div>
