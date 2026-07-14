<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import { settings } from "./lib/stores/settings.svelte";
  import { profiles } from "./lib/stores/profiles.svelte";
  import { books } from "./lib/stores/books.svelte";
  import Workbench from "./lib/components/Workbench.svelte";
  import DropZone from "./lib/components/DropZone.svelte";

  let dragging = $state(false);

  onMount(() => {
    void settings.load();
    void profiles.load();

    let unlisten: UnlistenFn | undefined;
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
      .then((fn) => (unlisten = fn));

    return () => unlisten?.();
  });
</script>

<main class="h-full bg-zinc-50 text-zinc-800 dark:bg-zinc-950 dark:text-zinc-200">
  <Workbench />
  {#if dragging}
    <DropZone />
  {/if}
</main>
