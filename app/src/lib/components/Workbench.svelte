<script lang="ts">
  import { books } from "../stores/books.svelte";
  import { isModalOpen, isTextField, shortcutFor } from "../api/keys";
  import BrowseButtons from "./BrowseButtons.svelte";
  import BookGrid from "./BookGrid.svelte";
  import Inspector from "./Inspector.svelte";
  import ActionBar from "./ActionBar.svelte";

  // The workbench's three shortcuts. What a chord means is decided in api/keys
  // (and unit-tested there); this only reads the event and does the deed.
  function onKeydown(event: KeyboardEvent) {
    const intent = shortcutFor({
      key: event.key,
      metaKey: event.metaKey,
      ctrlKey: event.ctrlKey,
      inTextField: isTextField(event.target),
      modalOpen: isModalOpen(),
    });
    if (intent === null) return;

    switch (intent) {
      case "select-all":
        if (books.books.length === 0) return;
        books.selectAll();
        break;
      case "clear-selection":
        if (books.selectedIds.size === 0) return;
        books.clearSelection();
        break;
      case "remove-selected": {
        // No confirmation: these are list entries, not files. Nothing on disk
        // is touched, and dropping the book back in takes one drag.
        const ids = books.selected.map((b) => b.id);
        if (ids.length === 0) return;
        books.remove(ids);
        break;
      }
    }
    event.preventDefault();
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#snippet mark(size: string)}
  <svg class={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6">
    <path d="M5 4.5A1.5 1.5 0 016.5 3H13v18H6.5A1.5 1.5 0 015 19.5v-15z" stroke-linejoin="round" />
    <path d="M13 3h4.5A1.5 1.5 0 0119 4.5v15a1.5 1.5 0 01-1.5 1.5H13" stroke-linejoin="round" />
    <path d="M16 8.5l2.5-1.2M16 12l2.5-1.2" stroke-linecap="round" />
  </svg>
{/snippet}

{#if books.books.length === 0}
  <div class="flex h-full flex-col items-center justify-center gap-7 px-8 text-center">
    <span class="text-indigo-500">{@render mark("h-14 w-14")}</span>
    <div>
      <h1 class="text-3xl font-semibold tracking-tight text-zinc-800 dark:text-zinc-100">EPUB Tailor</h1>
      <p class="mt-2 text-zinc-500 dark:text-zinc-400">Drop a book here and we will make it fit.</p>
    </div>
    <BrowseButtons />
  </div>
{:else}
  <div class="flex h-full flex-col">
    <header
      class="flex shrink-0 items-center justify-between border-b border-zinc-200 bg-white px-5 py-2.5 dark:border-zinc-800 dark:bg-zinc-900"
    >
      <div class="flex items-center gap-2 text-zinc-700 dark:text-zinc-200">
        <span class="text-indigo-500">{@render mark("h-5 w-5")}</span>
        <span class="text-sm font-semibold tracking-tight">EPUB Tailor</span>
      </div>
      <BrowseButtons size="sm" />
    </header>

    <div class="flex min-h-0 flex-1">
      <main class="min-w-0 flex-1 overflow-y-auto">
        <BookGrid />
      </main>
      <aside
        class="w-[300px] shrink-0 overflow-y-auto border-l border-zinc-200 bg-white dark:border-zinc-800 dark:bg-zinc-900"
      >
        <Inspector />
      </aside>
    </div>

    <ActionBar />
  </div>
{/if}
