<script lang="ts">
  // "Find online": search Open Library for a book, then stage the fields you
  // pick off one candidate. Search and fetch both go through runSidecar with the
  // app's user-agent (see api/lookup). The only proxy-safe values crossing into
  // the sidecar are the plain query strings and the reference, so nothing here
  // needs a snapshot dance - argv.ts hands the sidecar flat strings either way.
  import { untrack } from "svelte";
  import { fade, scale } from "svelte/transition";
  import type { Candidate } from "../api/contract";
  import { fetchRecord, searchOnline } from "../api/lookup";
  import { coverCachePath } from "../api/covers";
  import type { Book } from "../stores/books.svelte";
  import { edits } from "../stores/edits.svelte";
  import Button from "./ui/Button.svelte";
  import CandidateCard from "./CandidateCard.svelte";
  import AcceptStep from "./AcceptStep.svelte";

  let { book, onclose }: { book: Book; onclose: () => void } = $props();

  const isEpub = $derived(book.kind === "epub");

  // Seed the query once from the book (epub only); the fields are the user's to
  // edit afterwards, so this never re-runs when the book's own metadata changes.
  let query = $state(
    untrack(() => ({
      title: book.kind === "epub" ? (book.meta?.title ?? "") : "",
      author: book.kind === "epub" ? (book.meta?.authors?.[0] ?? "") : "",
      isbn: book.kind === "epub" ? (book.meta?.isbn ?? "") : "",
    })),
  );

  let loading = $state(false);
  let error = $state<string | null>(null);
  let candidates = $state<Candidate[] | null>(null);
  let licence = $state<string | null>(null);
  let chosen = $state<Candidate | null>(null);
  let accepting = $state(false);

  const canSearch = $derived(
    !loading && Boolean(isEpub || query.title.trim() || query.author.trim() || query.isbn.trim()),
  );

  async function runSearch() {
    loading = true;
    error = null;
    candidates = null;
    chosen = null;
    const outcome = await searchOnline({
      input: isEpub ? book.path : undefined,
      title: query.title.trim() || undefined,
      author: query.author.trim() || undefined,
      isbn: query.isbn.trim() || undefined,
    });
    loading = false;
    if (outcome.ok) {
      candidates = outcome.report.candidates;
      licence = outcome.report.source_licence;
    } else {
      error = outcome.message;
    }
  }

  /** A filesystem-safe cache key for a candidate's downloaded cover. */
  function coverKey(ref: string): string {
    return `fetched-${ref.replace(/[^a-zA-Z0-9]+/g, "-")}`;
  }

  async function accept(fields: Set<string>, includeCover: boolean) {
    if (!chosen) return;
    accepting = true;
    error = null;
    try {
      const coverOut = includeCover ? await coverCachePath(coverKey(chosen.ref)) : undefined;
      const outcome = await fetchRecord(chosen.ref, coverOut);
      if (!outcome.ok) {
        error = outcome.message;
        return;
      }
      const fieldSet = new Set(fields);
      if (includeCover) fieldSet.add("cover");
      edits.applyDoc(book.id, outcome.doc, fieldSet);
      onclose();
    } finally {
      accepting = false;
    }
  }

  function onKey(event: KeyboardEvent) {
    if (event.key === "Escape") onclose();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="fixed inset-0 z-[70] flex items-center justify-center p-6">
  <div
    role="presentation"
    transition:fade={{ duration: 120 }}
    class="absolute inset-0 bg-zinc-950/45 backdrop-blur-[2px]"
    onclick={onclose}
  ></div>

  <div
    role="dialog"
    aria-modal="true"
    aria-label="Find metadata online"
    transition:scale={{ start: 0.96, duration: 140 }}
    class="relative flex max-h-[80vh] w-full max-w-lg flex-col rounded-2xl border border-zinc-200 bg-white p-5 shadow-xl dark:border-zinc-800 dark:bg-zinc-900"
  >
    <div class="mb-3 flex items-center justify-between">
      <h2 class="text-base font-semibold text-zinc-900 dark:text-zinc-100">Find metadata online</h2>
      <button
        type="button"
        aria-label="Close"
        onclick={onclose}
        class="rounded-md p-1 text-zinc-400 hover:bg-zinc-100 hover:text-zinc-600 dark:hover:bg-zinc-800"
      >
        <svg class="h-4 w-4" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6">
          <path d="M6 6l8 8M14 6l-8 8" stroke-linecap="round" />
        </svg>
      </button>
    </div>

    {#if chosen}
      {#key chosen.ref}
        <AcceptStep
          candidate={chosen}
          {book}
          busy={accepting}
          onaccept={accept}
          onback={() => (chosen = null)}
        />
      {/key}
    {:else}
      <!-- Query row -->
      <div class="grid grid-cols-[1fr_1fr] gap-2">
        <input
          type="text"
          placeholder="Title"
          spellcheck="false"
          bind:value={query.title}
          onkeydown={(e) => e.key === "Enter" && canSearch && runSearch()}
          class="rounded-lg border border-zinc-300 bg-white px-2.5 py-1.5 text-[13px] text-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-100"
        />
        <input
          type="text"
          placeholder="Author"
          spellcheck="false"
          bind:value={query.author}
          onkeydown={(e) => e.key === "Enter" && canSearch && runSearch()}
          class="rounded-lg border border-zinc-300 bg-white px-2.5 py-1.5 text-[13px] text-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-100"
        />
      </div>
      <div class="mt-2 flex gap-2">
        <input
          type="text"
          placeholder="ISBN (most precise)"
          spellcheck="false"
          bind:value={query.isbn}
          onkeydown={(e) => e.key === "Enter" && canSearch && runSearch()}
          class="min-w-0 flex-1 rounded-lg border border-zinc-300 bg-white px-2.5 py-1.5 text-[13px] text-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-100"
        />
        <Button variant="primary" disabled={!canSearch} onclick={runSearch}>Search</Button>
      </div>

      <!-- Results -->
      <div class="mt-3 min-h-0 flex-1 overflow-y-auto">
        {#if loading}
          <div class="flex items-center justify-center gap-2 py-10 text-[13px] text-zinc-500 dark:text-zinc-400">
            <svg class="h-4 w-4 animate-spin" viewBox="0 0 24 24" fill="none">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" />
              <path class="opacity-90" d="M12 2a10 10 0 019 5.5" stroke="currentColor" stroke-width="3" stroke-linecap="round" />
            </svg>
            Asking Open Library...
          </div>
        {:else if error}
          <div class="rounded-xl border border-amber-200 bg-amber-50 px-3 py-3 text-[13px] text-amber-800 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200">
            {error}
          </div>
        {:else if candidates === null}
          <p class="py-8 text-center text-[13px] text-zinc-500 dark:text-zinc-400">
            {isEpub
              ? "Search with the fields above - we seeded them from your book."
              : "Type a title, author or ISBN, then Search."}
          </p>
        {:else if candidates.length === 0}
          <p class="py-8 text-center text-[13px] text-zinc-500 dark:text-zinc-400">
            Nothing found. Try a different title, author or an ISBN.
          </p>
        {:else}
          <div class="flex flex-col gap-2">
            {#each candidates as candidate (candidate.ref)}
              <CandidateCard {candidate} onselect={() => (chosen = candidate)} />
            {/each}
          </div>
        {/if}
      </div>
    {/if}

    <!-- Footer: licence -->
    <div class="mt-3 border-t border-zinc-200 pt-2.5 text-[10px] leading-relaxed text-zinc-400 dark:border-zinc-800 dark:text-zinc-500">
      {licence ?? "Open Library metadata is CC0; cover images are not."}
      <br />
      Cover images come from many sources and are not CC0 - only embed one you may use.
    </div>
  </div>
</div>
