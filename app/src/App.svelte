<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { settings } from "./lib/stores/settings.svelte";
  import { profiles } from "./lib/stores/profiles.svelte";
  import { books } from "./lib/stores/books.svelte";
  import { restoreGeometry, trackGeometry } from "./lib/api/window";
  import Workbench from "./lib/components/Workbench.svelte";
  import DropZone from "./lib/components/DropZone.svelte";
  import UpdateBanner from "./lib/components/UpdateBanner.svelte";

  let dragging = $state(false);
  let loadWarning = $state<string | null>(null);
  let openWarning = $state<string | null>(null);

  /**
   * Settings and profiles both load from outside the app (a store file, the
   * CLI). Either can fail - a corrupt settings.json, a sidecar that will not
   * start - and neither failure is fatal: the app works on its defaults. So
   * both are awaited, not fired and forgotten, and a failure says so quietly
   * instead of becoming an unhandled rejection nobody ever sees.
   */
  async function loadStores(): Promise<UnlistenFn | undefined> {
    const failures: string[] = [];

    try {
      await settings.load();
    } catch {
      failures.push("your settings");
    }

    try {
      await profiles.load();
      // Books the OS handed us at launch can land before the profile list
      // (and with it the copy-name appendixes) is known; regroup now that it
      // is, so a copy opened next to its source still folds under it.
      books.regroup();
    } catch {
      failures.push("the device profiles");
    }

    // Cosmetic, and never worth a warning of its own: a window that opens at
    // its default size is a window, and the user has bigger news to read.
    let untrack: UnlistenFn | undefined;
    try {
      await restoreGeometry(settings.windowGeometry);
      untrack = await trackGeometry((geometry) => (settings.windowGeometry = geometry));
    } catch {
      // Ignored on purpose.
    }

    if (failures.length > 0) {
      loadWarning = `We could not load ${failures.join(" or ")}. The defaults are in play, and everything still works - but nothing you change now will be remembered.`;
    }
    return untrack;
  }

  onMount(() => {
    let untrackGeometry: UnlistenFn | undefined;
    void loadStores().then((fn) => (untrackGeometry = fn));

    let unlistenDrag: UnlistenFn | undefined;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") {
          dragging = true;
        } else if (payload.type === "drop") {
          dragging = false;
          void books.addPaths(payload.paths);
        } else {
          // "leave"
          dragging = false;
        }
      })
      .then((fn) => (unlistenDrag = fn));

    // Files the OS handed the app directly (double-click, Open With, `open
    // -a`, a bare command line). Register the live listener *first* and only
    // drain the pending-opens buffer once it is confirmed up, so there is no
    // gap between the two where an arrival could be missed by both: anything
    // from here on is caught live, and the drain call sweeps up whatever
    // arrived before that. Both feed the same books.addPaths the drop zone
    // uses, so dedupe is handled in one place.
    let unlistenOpen: UnlistenFn | undefined;
    listen<string[]>("files-opened", (event) => {
      void books.addPaths(event.payload);
    })
      .then((fn) => {
        unlistenOpen = fn;
        return invoke<string[]>("drain_pending_opens");
      })
      .then((paths) => {
        if (paths.length > 0) void books.addPaths(paths);
      })
      .catch(() => {
        // Without this listener a double-clicked book never arrives, and the
        // app would just sit there looking empty. Drag and drop is unaffected,
        // which is what the notice tells the user to fall back on.
        openWarning =
          "Books opened from Finder or the command line will not reach us this session. Drag them onto the window instead.";
      });

    return () => {
      unlistenDrag?.();
      unlistenOpen?.();
      untrackGeometry?.();
    };
  });
</script>

{#snippet notice(message: string, dismiss: () => void)}
  <div
    class="flex shrink-0 items-start gap-2 border-b border-amber-200 bg-amber-50 px-4 py-2 text-[12px] leading-snug text-amber-800 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200"
  >
    <span class="min-w-0 flex-1">{message}</span>
    <button
      type="button"
      aria-label="Dismiss"
      onclick={dismiss}
      class="shrink-0 rounded p-0.5 text-amber-600 hover:bg-amber-100 dark:text-amber-300 dark:hover:bg-amber-500/20"
    >
      <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M6 6l8 8M14 6l-8 8" stroke-linecap="round" />
      </svg>
    </button>
  </div>
{/snippet}

<main class="flex h-full flex-col bg-ink-50 text-ink-800 dark:bg-ink-950 dark:text-ink-200">
  <UpdateBanner />

  {#if loadWarning}
    {@render notice(loadWarning, () => (loadWarning = null))}
  {/if}
  {#if openWarning}
    {@render notice(openWarning, () => (openWarning = null))}
  {/if}
  {#if books.addError}
    {@render notice(books.addError, () => (books.addError = null))}
  {/if}

  <div class="min-h-0 flex-1">
    <Workbench />
  </div>

  {#if dragging}
    <DropZone />
  {/if}
</main>
