<script lang="ts">
  // The field-level accept step: tick which of a candidate's fields to stage.
  // Fields the book is missing are checked by default (the common case is to
  // fill gaps); the user can tick more to overwrite. The cover is its own opt-in
  // with a preview, because Open Library's cover images are not CC0.
  //
  // The parent keys this component by the candidate ref, so it is remounted for
  // each candidate - which is why the prop-derived values below are captured
  // once (untracked) rather than kept reactive.
  import { untrack } from "svelte";
  import type { Candidate } from "../api/contract";
  import type { Book } from "../stores/books.svelte";
  import { creatorNames, stringList } from "../api/meta";
  import Button from "./ui/Button.svelte";

  let {
    candidate,
    book,
    busy = false,
    error = null,
    onaccept,
    onback,
  }: {
    candidate: Candidate;
    book: Book;
    busy?: boolean;
    /** A failed fetch, shown here: the results list this step replaced cannot. */
    error?: string | null;
    onaccept: (fields: Set<string>, includeCover: boolean) => void;
    onback: () => void;
  } = $props();

  interface Row {
    key: string;
    label: string;
    value: string;
  }

  const meta = untrack(() => candidate.metadata);

  // The fields this candidate actually carries, in a stable display order, each
  // with a short preview of what would be written.
  function buildRows(): Row[] {
    const rows: Row[] = [];
    const push = (key: string, label: string, value: string | undefined) => {
      if (value && value.trim()) rows.push({ key, label, value: value.trim() });
    };
    push("title", "Title", meta.title);
    const authors = creatorNames(meta.authors);
    if (authors.length) rows.push({ key: "authors", label: "Authors", value: authors.join(", ") });
    push("series", "Series", meta.series);
    push("seriesIndex", "Series index", meta.series_index);
    push("publisher", "Publisher", meta.publisher);
    push("language", "Language", meta.language);
    push("date", "Date", meta.date);
    push("isbn", "ISBN", meta.isbn);
    const subjects = stringList(meta.subjects);
    if (subjects.length) rows.push({ key: "subjects", label: "Subjects", value: subjects.join(", ") });
    push("description", "Description", meta.description);
    return rows;
  }

  const rows = buildRows();
  const missing = untrack(() => new Set(book.meta?.missing ?? []));

  // Default-checked: the fields the book is missing (missing_fields never names
  // language, series index or cover, so those start unticked).
  let checked = $state<Record<string, boolean>>(
    Object.fromEntries(rows.map((r) => [r.key, missing.has(r.key)])),
  );
  let includeCover = $state(false);

  const anyChecked = $derived(Object.values(checked).some(Boolean) || includeCover);

  function accept() {
    const fields = new Set(Object.keys(checked).filter((k) => checked[k]));
    onaccept(fields, includeCover);
  }
</script>

<div class="flex min-h-0 flex-col">
  <button
    type="button"
    onclick={onback}
    class="mb-2 inline-flex items-center gap-1 self-start text-[12px] font-medium text-zinc-500 hover:text-zinc-700 dark:text-zinc-400 dark:hover:text-zinc-200"
  >
    <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
      <path d="M12 5l-5 5 5 5" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
    Back to results
  </button>

  <p class="mb-2 text-[12px] text-zinc-500 dark:text-zinc-400">
    Choose what to keep. Fields your book is missing are ticked already.
  </p>

  <div class="flex max-h-[42vh] flex-col gap-1 overflow-y-auto pr-1">
    {#each rows as row (row.key)}
      <label
        class="flex cursor-pointer items-start gap-2.5 rounded-lg px-2 py-1.5 hover:bg-zinc-50 dark:hover:bg-zinc-800/60"
      >
        <input
          type="checkbox"
          bind:checked={checked[row.key]}
          class="mt-0.5 h-4 w-4 shrink-0 accent-indigo-600"
        />
        <span class="min-w-0 flex-1">
          <span class="block text-[11px] font-medium text-zinc-500 dark:text-zinc-400">{row.label}</span>
          <span class="block truncate text-[13px] text-zinc-800 dark:text-zinc-100">{row.value}</span>
        </span>
      </label>
    {/each}

    {#if candidate.cover_url}
      <label
        class="flex cursor-pointer items-center gap-2.5 rounded-lg px-2 py-1.5 hover:bg-zinc-50 dark:hover:bg-zinc-800/60"
      >
        <input type="checkbox" bind:checked={includeCover} class="h-4 w-4 shrink-0 accent-indigo-600" />
        <img
          src={candidate.cover_url}
          alt=""
          class="h-12 w-8 shrink-0 rounded object-cover"
        />
        <span class="min-w-0 flex-1">
          <span class="block text-[11px] font-medium text-zinc-500 dark:text-zinc-400">Cover</span>
          <span class="block text-[11px] text-zinc-500 dark:text-zinc-400">Not CC0 - embed only if you may.</span>
        </span>
      </label>
    {/if}
  </div>

  {#if error}
    <div
      role="alert"
      class="mt-3 rounded-xl border border-amber-200 bg-amber-50 px-3 py-2.5 text-[13px] text-amber-800 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200"
    >
      {error}
    </div>
  {/if}

  <div class="mt-3 flex justify-end gap-2">
    <Button variant="secondary" onclick={onback}>Cancel</Button>
    <Button variant="primary" disabled={!anyChecked || busy} onclick={accept}>
      {busy ? "Fetching..." : "Stage these"}
    </Button>
  </div>
</div>
