<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { settings } from "./lib/stores/settings.svelte";
  import { profiles } from "./lib/stores/profiles.svelte";
  import { books } from "./lib/stores/books.svelte";
  import Workbench from "./lib/components/Workbench.svelte";
  import DropZone from "./lib/components/DropZone.svelte";

  let dragging = $state(false);

  onMount(() => {
    void settings.load();
    void profiles.load();

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
      });

    return () => {
      unlistenDrag?.();
      unlistenOpen?.();
    };
  });
</script>

<main class="h-full bg-zinc-50 text-zinc-800 dark:bg-zinc-950 dark:text-zinc-200">
  <Workbench />
  {#if dragging}
    <DropZone />
  {/if}
</main>
