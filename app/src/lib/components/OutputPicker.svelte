<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { listRemovableVolumes, pollVolumes } from "../api/volumes";
  import type { Volume } from "../api/volumes";
  import { settings } from "../stores/settings.svelte";

  let volumes = $state<Volume[]>([]);

  /** True while the pointer is over this picker, or something inside it has focus. */
  let atPicker = $state(false);

  // A drive already plugged in is offered from the moment the picker appears:
  // one read, on mount.
  onMount(() => {
    void listRemovableVolumes()
      .then((next) => (volumes = next))
      .catch(() => {
        // A volume list we could not read is an empty one; the two fixed
        // choices below still work, and the next poll tries again.
      });
  });

  // The live poll, though, only runs while the user is actually *at* the picker.
  // This component is mounted for the app's whole lifetime, and the poll stats
  // /Volumes every two seconds - a cost with no payoff while the user is doing
  // anything else. Reaching for the destination list is exactly the gesture that
  // follows plugging a reader in, so the list is live when it needs to be.
  $effect(() => {
    if (!atPicker) return;
    return pollVolumes((next) => (volumes = next));
  });

  function middleTruncate(path: string, max = 34): string {
    if (path.length <= max) return path;
    const head = Math.ceil((max - 1) / 2);
    const tail = Math.floor((max - 1) / 2);
    return `${path.slice(0, head)}…${path.slice(path.length - tail)}`;
  }

  const isCustom = $derived(
    settings.outputDir !== null && !volumes.some((v) => v.path === settings.outputDir),
  );

  async function chooseFolder() {
    const selection = await open({ directory: true });
    if (typeof selection === "string") settings.outputDir = selection;
  }
</script>

<div
  role="group"
  aria-label="Destination"
  class="flex flex-col gap-1.5"
  onpointerenter={() => (atPicker = true)}
  onpointerleave={() => (atPicker = false)}
  onfocusin={() => (atPicker = true)}
  onfocusout={() => (atPicker = false)}
>
  <!-- Alongside originals -->
  <button
    type="button"
    aria-pressed={settings.outputDir === null}
    onclick={() => (settings.outputDir = null)}
    class="flex w-full items-center gap-2 rounded-lg border px-2.5 py-1.5 text-left text-sm transition-colors {settings.outputDir ===
    null
      ? 'border-teal-500 bg-teal-50 text-teal-900 dark:bg-teal-500/10 dark:text-teal-200'
      : 'border-ink-200 text-ink-700 hover:bg-ink-50 dark:border-ink-800 dark:text-ink-200 dark:hover:bg-ink-800'}"
  >
    <svg class="h-4 w-4 shrink-0 text-ink-400" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
      <path d="M3 6.5A1.5 1.5 0 014.5 5h3l1.5 1.5h6A1.5 1.5 0 0116.5 8v6A1.5 1.5 0 0115 15.5H4.5A1.5 1.5 0 013 14V6.5z" stroke-linejoin="round" />
    </svg>
    Alongside originals
  </button>

  <!-- Removable volumes -->
  {#each volumes as volume (volume.path)}
    <button
      type="button"
      aria-pressed={settings.outputDir === volume.path}
      onclick={() => (settings.outputDir = volume.path)}
      class="flex w-full items-center gap-2 rounded-lg border px-2.5 py-1.5 text-left text-sm transition-colors {settings.outputDir ===
      volume.path
        ? 'border-teal-500 bg-teal-50 text-teal-900 dark:bg-teal-500/10 dark:text-teal-200'
        : 'border-ink-200 text-ink-700 hover:bg-ink-50 dark:border-ink-800 dark:text-ink-200 dark:hover:bg-ink-800'}"
      title={volume.path}
    >
      <svg class="h-4 w-4 shrink-0 text-ink-400" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
        <path d="M5 3h7l3 3v11a1 1 0 01-1 1H5a1 1 0 01-1-1V4a1 1 0 011-1z" stroke-linejoin="round" />
        <path d="M7 3v3M10 3v3M13 6v2" stroke-linecap="round" />
      </svg>
      <span class="truncate">{volume.name}</span>
    </button>
  {/each}

  <!-- Choose a folder -->
  <button
    type="button"
    aria-pressed={isCustom}
    onclick={chooseFolder}
    class="flex w-full items-center gap-2 rounded-lg border px-2.5 py-1.5 text-left text-sm transition-colors {isCustom
      ? 'border-teal-500 bg-teal-50 text-teal-900 dark:bg-teal-500/10 dark:text-teal-200'
      : 'border-ink-200 text-ink-700 hover:bg-ink-50 dark:border-ink-800 dark:text-ink-200 dark:hover:bg-ink-800'}"
    title={isCustom ? (settings.outputDir ?? undefined) : undefined}
  >
    <svg class="h-4 w-4 shrink-0 text-ink-400" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
      <path d="M2.75 5.5A1.75 1.75 0 014.5 3.75h2.4a1.75 1.75 0 011.24.51l.9.9h6.71A1.75 1.75 0 0117.5 7.9v6.35a1.75 1.75 0 01-1.75 1.75H4.5A1.75 1.75 0 012.75 14.25v-8.75z" stroke-linejoin="round" />
    </svg>
    <span class="truncate">
      {isCustom ? middleTruncate(settings.outputDir!) : "Choose folder..."}
    </span>
  </button>
</div>
