<script lang="ts">
  import { books } from "../stores/books.svelte";
  import { jobs } from "../stores/jobs.svelte";
  import { settings } from "../stores/settings.svelte";
  import type { AppMode } from "../stores/settings.svelte";
  import { isModalOpen, isTextField, shortcutFor } from "../api/keys";
  import BrowseButtons from "./BrowseButtons.svelte";
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
  <!-- The app icon's e-reader, minus its backdrop: same geometry and lettering
       paths as icons-src/icon.svg, colored through theme classes so it sits on
       any surface. At small sizes the text degrades to a glowing screen, just
       like the real icon does. -->
  <svg class={size} viewBox="248 148 528 728" fill="none">
    <rect x="262" y="162" width="500" height="700" rx="56" class="fill-ink-800" />
    <rect x="262" y="162" width="500" height="700" rx="56" class="stroke-teal-500 dark:stroke-teal-400" stroke-width="12" />
    <rect x="302" y="210" width="420" height="540" rx="18" class="fill-ink-100" />
    <g transform="translate(368,350) scale(0.62)" class="stroke-ink-900" stroke-width="30" stroke-linecap="round" stroke-linejoin="round">
      <path d="M15,15 L15,105 L42,105 C68,105 75,84 75,60 C75,36 68,15 42,15 L15,15 Z" />
      <g transform="translate(110,0)"><ellipse cx="45" cy="60" rx="30" ry="45" /></g>
      <g transform="translate(220,0)"><path d="M15,105 L15,15 L75,105 L75,15" /></g>
      <g transform="translate(330,0)"><path d="M14,12 L8,40" stroke-width="26" /></g>
      <g transform="translate(374,0)"><path d="M8,15 L82,15 M45,15 L45,105" /></g>
    </g>
    <g transform="translate(348,466) scale(0.62)" class="stroke-ink-900" stroke-width="30" stroke-linecap="round" stroke-linejoin="round">
      <path d="M15,105 L15,15 L46,15 C68,15 74,28 74,42 C74,56 68,69 46,69 L15,69" />
      <g transform="translate(110,0)"><path d="M10,105 L45,15 L80,105 M25,72 L65,72" /></g>
      <g transform="translate(220,0)"><path d="M15,105 L15,15 L75,105 L75,15" /></g>
      <g transform="translate(330,0)"><path d="M25,15 L65,15 M45,15 L45,105 M25,105 L65,105" /></g>
      <g transform="translate(440,0)"><path d="M73,32 C66,20 57,15 45,15 C22,15 15,36 15,60 C15,84 22,105 45,105 C57,105 66,100 73,88" /></g>
    </g>
    <g class="fill-ink-900" opacity="0.3">
      <rect x="362" y="596" width="300" height="16" rx="8" />
      <rect x="382" y="634" width="260" height="16" rx="8" />
    </g>
    <circle cx="512" cy="806" r="9" class="fill-teal-500 dark:fill-teal-400" />
  </svg>
{/snippet}

{#if books.books.length === 0}
  <div class="flex h-full flex-col items-center justify-center gap-7 px-8 text-center">
    <span class="text-teal-500 dark:text-teal-400 dark:drop-shadow-glow">{@render mark("h-24 w-24")}</span>
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
        <BrowseButtons size="sm" />
      </div>
    </header>

    <div class="flex min-h-0 flex-1">
      <main class="min-w-0 flex-1 overflow-y-auto">
        <BookList />
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
