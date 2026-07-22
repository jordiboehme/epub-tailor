<script lang="ts">
  // One file of a book: a selectable row. Files are the unit Edit and Fit act
  // on, so a row takes the same selection gestures as a card body (which
  // stands for the book's original) and carries everything that happened to
  // its file - staged-edit marker, status chips, findings/failure details,
  // live job state - plus reveal and, for copies, move-to-Trash.
  import { slide } from "svelte/transition";
  import { invoke } from "@tauri-apps/api/core";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { books } from "../stores/books.svelte";
  import type { Book, BookFile } from "../stores/books.svelte";
  import { jobs } from "../stores/jobs.svelte";
  import { edits } from "../stores/edits.svelte";
  import { saveFilesInPlace } from "../stores/inplace";
  import { countEdits } from "../api/edits";
  import { chipsFor, failureOf, fileBadge, findingsOf, TONE_CLASS } from "../api/book-view";
  import type { Chip } from "../api/book-view";
  import CardDetails from "./CardDetails.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";

  let { book, file }: { book: Book; file: BookFile } = $props();

  let showDetails = $state(false);
  let trashFailed = $state<string | null>(null);
  let confirmCleanup = $state(false);
  let cleanupFailed = $state<string | null>(null);

  const selected = $derived(books.selectedFileIds.has(file.id));
  const staged = $derived(edits.get(file.id));
  const editCount = $derived(staged ? countEdits(staged) : 0);
  const job = $derived(jobs.conversionJobFor(file.id));
  const running = $derived(job?.state === "running");
  const queued = $derived(job?.state === "queued");

  const failure = $derived(failureOf(file));
  const findings = $derived(findingsOf(file));
  const canExpand = $derived(Boolean(failure || findings));
  const chips = $derived(chipsFor(file));

  function onClick(event: MouseEvent) {
    event.stopPropagation();
    if (event.shiftKey) books.range(file.id);
    else if (event.metaKey || event.ctrlKey) books.toggle(file.id);
    else books.select(file.id);
  }

  function onKey(event: KeyboardEvent) {
    // A chip or Details button inside the row handles its own Enter/Space;
    // only a keypress on the row itself selects the file.
    if (event.target !== event.currentTarget) return;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      event.stopPropagation();
      books.select(file.id);
    }
  }

  function reveal(event: MouseEvent) {
    event.stopPropagation();
    void revealItemInDir(file.path);
  }

  async function trash(event: MouseEvent) {
    event.stopPropagation();
    trashFailed = null;
    try {
      await invoke("trash_file", { path: file.path });
      books.removeFile(book.id, file.id);
    } catch (err) {
      trashFailed = `That copy could not be moved to the Trash. ${String(err)}`;
    }
  }

  // The "needs cleanup" chip doubles as the fix: click, confirm, and the
  // file is repaired in place through the same Trash-backed flow the
  // ActionBar's Clean up uses.
  const canCleanup = $derived(file.kind === "epub" && !jobs.active);

  async function runCleanup() {
    confirmCleanup = false;
    cleanupFailed = null;
    const outcome = await saveFilesInPlace([file], false);
    if (outcome.failures.length > 0) {
      cleanupFailed = `Nothing was written - a safety copy could not be made. ${outcome.failures[0]}`;
    }
  }
</script>

{#snippet chip(c: Chip)}
  {#if canExpand && (c.tone === "bad" || c.tone === "warn")}
    <!-- A chip that stands for expandable information (findings, a failure)
         is itself the toggle - clicking "2 errors" should show the errors. -->
    <button
      type="button"
      aria-expanded={showDetails}
      title={c.title ?? (showDetails ? "Hide the details" : "Show the details")}
      onclick={(e) => {
        e.stopPropagation();
        showDetails = !showDetails;
      }}
      class="shrink-0 cursor-pointer rounded px-1 py-0.5 text-[10px] font-medium ring-1 ring-current/30 ring-inset transition-shadow hover:ring-current/70 focus-visible:ring-2 focus-visible:ring-teal-500 focus-visible:outline-none dark:focus-visible:ring-teal-400 {TONE_CLASS[c.tone]}"
    >
      {c.label}
    </button>
  {:else}
    <span class="shrink-0 rounded px-1 py-0.5 text-[10px] font-medium {TONE_CLASS[c.tone]}" title={c.title}>
      {c.label}
    </span>
  {/if}
{/snippet}

<div
  role="button"
  tabindex="0"
  aria-pressed={selected}
  onclick={onClick}
  onkeydown={onKey}
  class="group/file relative cursor-default rounded-md px-1 py-0.5 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-teal-500 dark:focus-visible:ring-teal-400 {selected
    ? 'bg-teal-100/70 ring-1 ring-inset ring-teal-400/50 dark:bg-teal-500/15 dark:ring-teal-400/40'
    : 'hover:bg-ink-100 dark:hover:bg-ink-800/60'}"
>
  <div class="flex min-w-0 items-center gap-1.5">
    <span
      class="shrink-0 rounded px-1 py-0.5 text-[10px] font-medium {file.role === 'original'
        ? 'bg-ink-200 text-ink-600 dark:bg-ink-700 dark:text-ink-300'
        : 'bg-teal-100 text-teal-800 dark:bg-teal-500/15 dark:text-teal-300'}"
    >
      {fileBadge(file)}
    </span>

    <span class="min-w-0 truncate text-[11px] text-ink-600 dark:text-ink-300" title={file.path}>
      {file.fileName}
    </span>

    {#if staged}
      <span
        title="Has staged metadata edits, written on the next save"
        class="inline-flex shrink-0 items-center gap-0.5 rounded bg-teal-700/85 px-1 py-0.5 text-[10px] font-medium text-white"
      >
        <svg class="h-2.5 w-2.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M13.5 4.5l2 2L8 14l-3 1 1-3 7.5-7.5z" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
        {editCount}
      </span>
    {/if}

    <div class="ml-auto flex shrink-0 items-center gap-1">
      {#each chips as c}
        {#if c.id === "needs-cleanup" && canCleanup}
          <button
            type="button"
            title="Clean up this file in place - a safety copy goes to the Trash first"
            onclick={(e) => {
              e.stopPropagation();
              confirmCleanup = true;
            }}
            class="shrink-0 rounded px-1 py-0.5 text-[10px] font-medium ring-1 ring-inset ring-amber-400/50 transition-shadow hover:ring-amber-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-teal-500 dark:focus-visible:ring-teal-400 {TONE_CLASS[c.tone]}"
          >
            {c.label}
          </button>
        {:else}
          {@render chip(c)}
        {/if}
      {/each}

      {#if running}
        <svg class="h-3 w-3 animate-spin text-teal-500 dark:text-teal-400" viewBox="0 0 24 24" fill="none">
          <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" />
          <path class="opacity-90" d="M12 2a10 10 0 019 5.5" stroke="currentColor" stroke-width="3" stroke-linecap="round" />
        </svg>
      {:else if queued}
        <span class="rounded px-1 py-0.5 text-[10px] font-medium {TONE_CLASS.neutral}">queued</span>
      {/if}

      {#if canExpand}
        <button
          type="button"
          onclick={(e) => {
            e.stopPropagation();
            showDetails = !showDetails;
          }}
          class="rounded px-1 py-0.5 text-[10px] font-medium text-ink-500 hover:text-ink-700 dark:text-ink-400 dark:hover:text-ink-200"
        >
          {showDetails ? "Less" : "Details"}
        </button>
      {/if}

      {#if !running && !queued}
        <div
          class="flex items-center gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover/file:opacity-100"
        >
          <button
            type="button"
            title="Show this file in the file manager"
            onclick={reveal}
            class="rounded p-0.5 text-ink-500 transition-colors hover:bg-teal-100 hover:text-teal-800 dark:text-ink-400 dark:hover:bg-teal-500/20 dark:hover:text-teal-300"
          >
            <svg class="h-3 w-3" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.7">
              <path d="M2.5 6.5A1.5 1.5 0 014 5h3.2l1.4 1.8H16A1.5 1.5 0 0117.5 8.3v6.2A1.5 1.5 0 0116 16H4a1.5 1.5 0 01-1.5-1.5v-8z" stroke-linejoin="round" />
            </svg>
          </button>
          {#if file.role === "copy"}
            <button
              type="button"
              title="Move this copy to the Trash"
              onclick={trash}
              class="rounded p-0.5 text-ink-500 transition-colors hover:bg-rose-100 hover:text-rose-600 dark:text-ink-400 dark:hover:bg-rose-500/20 dark:hover:text-rose-300"
            >
              <svg class="h-3 w-3" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.7">
                <path d="M4 6h12M8.5 6V4.5A1 1 0 019.5 3.5h1a1 1 0 011 1V6M6 6l.7 9.2a1.5 1.5 0 001.5 1.3h3.6a1.5 1.5 0 001.5-1.3L14 6" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            </button>
          {/if}
        </div>
      {/if}
    </div>
  </div>

  {#if file.ingest === "pending"}
    <div class="absolute inset-x-0 bottom-0 h-0.5 animate-pulse bg-teal-300/70"></div>
  {/if}

  {#if failure && !showDetails}
    <p class="line-clamp-1 text-[10px] leading-snug text-rose-600 dark:text-rose-400" title={failure.friendly}>
      {failure.friendly}
    </p>
  {/if}

  {#if trashFailed}
    <p class="text-[10px] leading-snug text-rose-600 dark:text-rose-400">{trashFailed}</p>
  {/if}

  {#if cleanupFailed}
    <p class="text-[10px] leading-snug text-rose-600 dark:text-rose-400">{cleanupFailed}</p>
  {/if}

  {#if showDetails && canExpand}
    <!-- Once the slide has settled (final height known), make sure the
         details are actually visible: a row near the bottom of the list
         would otherwise expand below the fold. -->
    <div
      transition:slide={{ duration: 150 }}
      onintroend={(e) => e.currentTarget.scrollIntoView({ block: "nearest", behavior: "smooth" })}
    >
      <CardDetails {findings} {failure} padding="py-1.5 px-1" />
    </div>
  {/if}
</div>

{#if confirmCleanup}
  <ConfirmDialog
    title="Clean up {file.fileName}?"
    confirmLabel="Clean up"
    cancelLabel="Not now"
    onConfirm={runCleanup}
    onCancel={() => (confirmCleanup = false)}
  >
    This repairs the file in place under the epub profile. The current version goes to the Trash
    first, so nothing is lost.
  </ConfirmDialog>
{/if}
