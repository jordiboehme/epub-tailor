<script lang="ts">
  // A cover, big. Opens from the Edit inspector's thumbnail. The save button
  // copies the cached file wherever the user points the save dialog; the
  // suggested extension is sniffed from the bytes because extracted covers
  // are cached as `.img` no matter what image format the EPUB held.

  import { invoke } from "@tauri-apps/api/core";
  import { save } from "@tauri-apps/plugin-dialog";
  import { fade, scale } from "svelte/transition";
  import { coverUrl } from "../api/covers";
  import Button from "./ui/Button.svelte";

  let { path, title, onclose }: { path: string; title: string; onclose: () => void } = $props();

  let saving = $state(false);
  let note = $state<string | null>(null);
  let noteIsError = $state(false);

  function onKey(event: KeyboardEvent) {
    if (event.key === "Escape") onclose();
  }

  /** A filename-safe stem from the book title, or "cover" when it yields nothing. */
  function stem(): string {
    const clean = title
      .replace(/[/\\:*?"<>|]+/g, " ")
      .replace(/\s+/g, " ")
      .trim();
    return clean || "cover";
  }

  async function saveImage() {
    saving = true;
    note = null;
    try {
      const ext = await invoke<string>("sniff_cover_extension", { path });
      const destination = await save({
        defaultPath: `${stem()} cover.${ext}`,
        filters: [{ name: "Image", extensions: [ext] }],
      });
      if (destination) {
        await invoke("export_cover", { source: path, destination });
        note = "Saved.";
        noteIsError = false;
      }
    } catch (error) {
      note = String(error);
      noteIsError = true;
    } finally {
      saving = false;
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="fixed inset-0 z-[80] flex items-center justify-center p-6">
  <div
    role="presentation"
    transition:fade={{ duration: 120 }}
    class="absolute inset-0 bg-ink-950/45 backdrop-blur-[2px]"
    onclick={onclose}
  ></div>

  <div
    role="dialog"
    aria-modal="true"
    aria-label={"Cover of " + title}
    transition:scale={{ start: 0.96, duration: 140 }}
    class="relative flex max-h-full min-h-0 flex-col items-center gap-3"
  >
    <div class="relative min-h-0">
      <img
        src={coverUrl(path)}
        alt={"Cover of " + title}
        class="max-h-[80vh] max-w-[85vw] rounded-lg object-contain shadow-xl"
      />
      <button
        type="button"
        aria-label="Close"
        onclick={onclose}
        class="absolute -right-3 -top-3 flex h-7 w-7 items-center justify-center rounded-full bg-ink-950/70 text-ink-100 shadow-md transition-colors hover:bg-ink-950/90 hover:text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-teal-400"
      >
        <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.8">
          <path d="M5 5l10 10M15 5L5 15" stroke-linecap="round" />
        </svg>
      </button>
    </div>

    <div class="flex max-w-[85vw] items-center gap-3">
      <span class="truncate text-[13px] text-ink-100">{title}</span>
      <Button variant="secondary" size="sm" onclick={saveImage} disabled={saving}>
        Save image...
      </Button>
      {#if note}
        <span class="text-[12px] {noteIsError ? 'text-rose-300' : 'text-teal-300'}">{note}</span>
      {/if}
    </div>
  </div>
</div>
