<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import Button from "./ui/Button.svelte";
  import { books } from "../stores/books.svelte";

  let { size = "md" }: { size?: "sm" | "md" } = $props();

  async function ingest(selection: string | string[] | null) {
    if (!selection) return;
    await books.addPaths(Array.isArray(selection) ? selection : [selection]);
  }

  async function addBooks() {
    await ingest(
      await open({ multiple: true, filters: [{ name: "Books", extensions: ["epub", "md"] }] }),
    );
  }

  async function addFolder() {
    await ingest(await open({ directory: true }));
  }
</script>

<div class="flex items-center gap-2">
  <Button variant="primary" {size} onclick={addBooks}>
    <svg class="h-4 w-4" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
      <path
        d="M4 4.5A1.5 1.5 0 015.5 3H10v14H5.5A1.5 1.5 0 014 15.5v-11zM10 3h4.5A1.5 1.5 0 0116 4.5v11a1.5 1.5 0 01-1.5 1.5H10"
        stroke-linejoin="round"
      />
    </svg>
    Add books
  </Button>
  <Button variant="secondary" {size} onclick={addFolder}>
    <svg class="h-4 w-4" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
      <path
        d="M2.75 5.5A1.75 1.75 0 014.5 3.75h2.4a1.75 1.75 0 011.24.51l.9.9h6.71A1.75 1.75 0 0117.5 7.9v6.35a1.75 1.75 0 01-1.75 1.75H4.5a1.75 1.75 0 01-1.75-1.75v-8.75z"
        stroke-linejoin="round"
      />
    </svg>
    Add a folder
  </Button>
</div>
