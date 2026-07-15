<script lang="ts">
  import { books } from "../stores/books.svelte";
  import { jobs } from "../stores/jobs.svelte";
  import { settings } from "../stores/settings.svelte";
  import type { AppMode, ViewMode } from "../stores/settings.svelte";
  import { isModalOpen, isTextField, shortcutFor } from "../api/keys";
  import BrowseButtons from "./BrowseButtons.svelte";
  import BookGrid from "./BookGrid.svelte";
  import BookList from "./BookList.svelte";
  import Inspector from "./Inspector.svelte";
  import ActionBar from "./ActionBar.svelte";
  import SegmentedControl from "./ui/SegmentedControl.svelte";

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
        if (books.selectedFileIds.size === 0) return;
        books.clearSelection();
        break;
      case "remove-selected": {
        // No confirmation: these are list entries, and nothing on disk is
        // touched - dropping the book back in takes one drag. Removal is
        // book-granular (a selected copy takes its whole card along), and a
        // book with any file mid-conversion is left alone, exactly as its
        // card's own remove button is while it is busy.
        const ids = books.books
          .filter((b) => b.files.some((f) => books.selectedFileIds.has(f.id)))
          .filter((b) =>
            b.files.every((f) => {
              const state = jobs.conversionJobFor(f.id)?.state;
              return state !== "running" && state !== "queued";
            }),
          )
          .map((b) => b.id);
        if (ids.length === 0) return;
        books.remove(ids);
        break;
      }
    }
    event.preventDefault();
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#snippet galleryIcon()}
  <svg class="h-4 w-4" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
    <rect x="3" y="3" width="6" height="6" rx="1.2" />
    <rect x="11" y="3" width="6" height="6" rx="1.2" />
    <rect x="3" y="11" width="6" height="6" rx="1.2" />
    <rect x="11" y="11" width="6" height="6" rx="1.2" />
  </svg>
{/snippet}

{#snippet listIcon()}
  <svg class="h-4 w-4" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
    <rect x="3" y="4" width="3.5" height="3.5" rx="0.8" />
    <rect x="3" y="12.5" width="3.5" height="3.5" rx="0.8" />
    <path d="M9 5.75h8M9 14.25h8" stroke-linecap="round" />
  </svg>
{/snippet}

{#snippet editIcon()}
  <svg class="h-4 w-4" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
    <path d="M13.8 3.7l2.5 2.5L7 15.5l-3.4 0.9 0.9-3.4 9.3-9.3z" stroke-linecap="round" stroke-linejoin="round" />
    <path d="M12 5.5l2.5 2.5" stroke-linecap="round" />
  </svg>
{/snippet}

{#snippet fitIcon()}
  <svg class="h-4 w-4" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
    <path d="M11.5 8.5L16.5 3.5M16.5 3.5h-3.5M16.5 3.5V7" stroke-linecap="round" stroke-linejoin="round" />
    <path d="M8.5 11.5L3.5 16.5M3.5 16.5H7M3.5 16.5V13" stroke-linecap="round" stroke-linejoin="round" />
  </svg>
{/snippet}

{#snippet mark(size: string)}
  <svg class={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6">
    <path d="M5 4.5A1.5 1.5 0 016.5 3H13v18H6.5A1.5 1.5 0 015 19.5v-15z" stroke-linejoin="round" />
    <path d="M13 3h4.5A1.5 1.5 0 0119 4.5v15a1.5 1.5 0 01-1.5 1.5H13" stroke-linejoin="round" />
    <path d="M16 8.5l2.5-1.2M16 12l2.5-1.2" stroke-linecap="round" />
  </svg>
{/snippet}

{#if books.books.length === 0}
  <div class="flex h-full flex-col items-center justify-center gap-7 px-8 text-center">
    <span class="text-teal-500 dark:text-teal-400 dark:drop-shadow-glow">{@render mark("h-14 w-14")}</span>
    <div>
      <h1 class="text-3xl font-semibold tracking-tight text-ink-800 dark:text-ink-100">EPUB Tailor</h1>
      <p class="mt-2 text-ink-500 dark:text-ink-400">Drop a book here and we will make it fit.</p>
    </div>
    <BrowseButtons />
  </div>
{:else}
  <div class="flex h-full flex-col">
    <header
      class="grid shrink-0 grid-cols-[1fr_auto_1fr] items-center gap-3 border-b border-ink-200 bg-white px-5 py-2.5 dark:border-ink-800 dark:bg-ink-900"
    >
      <div class="flex items-center gap-2 text-ink-700 dark:text-ink-200">
        <span class="text-teal-500 dark:text-teal-400">{@render mark("h-5 w-5")}</span>
        <span class="text-sm font-semibold tracking-tight">EPUB Tailor</span>
      </div>
      <!-- The app's central concept sits front and center, spelled out: what
           you do in Edit stays in the file, what you do in Fit makes copies. -->
      <SegmentedControl
        labeled
        value={settings.mode}
        options={[
          { value: "edit", label: "Edit metadata", icon: editIcon },
          { value: "fit", label: "Fit for device", icon: fitIcon },
        ]}
        onchange={(value) => (settings.mode = value as AppMode)}
      />
      <div class="flex items-center justify-end gap-3">
        <SegmentedControl
          value={settings.viewMode}
          options={[
            { value: "grid", label: "Gallery", icon: galleryIcon },
            { value: "list", label: "List", icon: listIcon },
          ]}
          onchange={(value) => (settings.viewMode = value as ViewMode)}
        />
        <BrowseButtons size="sm" />
      </div>
    </header>

    <div class="flex min-h-0 flex-1">
      <main class="min-w-0 flex-1 overflow-y-auto">
        {#if settings.viewMode === "list"}
          <BookList />
        {:else}
          <BookGrid />
        {/if}
      </main>
      <aside
        class="w-[300px] shrink-0 overflow-y-auto border-l border-ink-200 bg-white dark:border-ink-800 dark:bg-ink-900"
      >
        <Inspector />
      </aside>
    </div>

    <ActionBar />
  </div>
{/if}
