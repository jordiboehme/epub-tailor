<script lang="ts">
  // The profile stack editor: an ordered list of layers (built-in profiles
  // and user JSON files), composed left to right by the CLI with
  // last-layer-wins semantics. See settings.svelte.ts's ProfileLayer and
  // profiles.svelte.ts's stack helpers for the model this renders.
  import { fade } from "svelte/transition";
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    profiles,
    baseName,
    addBuiltinLayer,
    addFileLayer,
    describeDuplicateLayer,
    removeLayerAt,
    moveLayer,
    hasDeviceClash,
    hasScreen,
  } from "../stores/profiles.svelte";
  import type { ProfileLayer } from "../stores/settings.svelte";
  import { settings } from "../stores/settings.svelte";

  let adding = $state(false);
  let query = $state("");
  // Why a notice rather than a disabled entry like the built-in panel uses:
  // the file picker only reveals its path after the OS dialog closes, so a
  // duplicate cannot be shown as unavailable beforehand.
  let duplicateNotice = $state<string | null>(null);

  const filtered = $derived(
    profiles.builtins.filter((p) =>
      `${p.name} ${p.description}`.toLowerCase().includes(query.trim().toLowerCase()),
    ),
  );

  /** More than one layer carrying a screen means all but the last are discarded. */
  const deviceClash = $derived(hasDeviceClash(settings.profileStack, profiles.builtins));

  function layerLabel(layer: ProfileLayer): string {
    return layer.kind === "builtin" ? layer.name : baseName(layer.path);
  }

  function layerKey(layer: ProfileLayer): string {
    return layer.kind === "builtin" ? `builtin:${layer.name}` : `file:${layer.path}`;
  }

  function screenLabel(w: number, h: number): string {
    return hasScreen({ screen_w: w, screen_h: h }) ? `${w} x ${h}` : "device-neutral";
  }

  function addBuiltin(name: string) {
    settings.profileStack = addBuiltinLayer(settings.profileStack, name);
    adding = false;
    query = "";
  }

  async function addLayer() {
    const selection = await open({
      multiple: false,
      filters: [{ name: "Profile JSON", extensions: ["json"] }],
    });
    if (typeof selection !== "string") return;
    duplicateNotice = describeDuplicateLayer(settings.profileStack, selection);
    settings.profileStack = addFileLayer(settings.profileStack, selection);
  }

  function removeLayer(index: number) {
    settings.profileStack = removeLayerAt(settings.profileStack, index);
    duplicateNotice = null;
  }

  function move(index: number, delta: number) {
    settings.profileStack = moveLayer(settings.profileStack, index, delta);
  }
</script>

<div class="flex flex-col gap-1.5">
  {#each settings.profileStack as layer, i (layerKey(layer))}
    <div
      class="flex items-center gap-2 rounded-lg border border-ink-300 bg-white px-2.5 py-1.5 text-sm text-ink-800 dark:border-ink-700 dark:bg-ink-800 dark:text-ink-100"
    >
      <span class="flex-1 truncate" title={layer.kind === "file" ? layer.path : undefined}>
        {layerLabel(layer)}
      </span>
      <div class="flex shrink-0 items-center gap-0.5">
        <button
          type="button"
          aria-label={"Move " + layerLabel(layer) + " up"}
          disabled={i === 0}
          onclick={() => move(i, -1)}
          class="rounded-md p-1 text-ink-400 hover:bg-ink-100 hover:text-ink-700 disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:bg-transparent dark:hover:bg-ink-700 dark:hover:text-ink-100"
        >
          <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M6 12l4-4 4 4" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <button
          type="button"
          aria-label={"Move " + layerLabel(layer) + " down"}
          disabled={i === settings.profileStack.length - 1}
          onclick={() => move(i, 1)}
          class="rounded-md p-1 text-ink-400 hover:bg-ink-100 hover:text-ink-700 disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:bg-transparent dark:hover:bg-ink-700 dark:hover:text-ink-100"
        >
          <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M6 8l4 4 4-4" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <button
          type="button"
          aria-label={"Remove " + layerLabel(layer)}
          disabled={settings.profileStack.length === 1}
          title={settings.profileStack.length === 1 ? "At least one layer is required" : undefined}
          onclick={() => removeLayer(i)}
          class="rounded-md p-1 text-ink-400 hover:bg-ink-100 hover:text-red-600 disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-ink-400 dark:hover:bg-ink-700 dark:hover:text-red-400"
        >
          <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2.2">
            <path d="M6 6l8 8M14 6l-8 8" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    </div>
  {/each}

  <p class="text-[11px] leading-snug text-ink-500 dark:text-ink-400">Later layers win.</p>

  {#if deviceClash}
    <p
      class="inline-flex w-fit items-center gap-1 rounded-full bg-amber-100 px-2 py-0.5 text-[11px] text-amber-800 dark:bg-amber-500/10 dark:text-amber-400"
    >
      Two device profiles - only the last screen applies.
    </p>
  {/if}

  <div class="mt-1 flex flex-wrap items-center gap-3">
    <div class="relative">
      <button
        type="button"
        onclick={() => (adding = !adding)}
        class="text-[12px] font-medium text-teal-700 hover:text-teal-500 dark:text-teal-400"
      >
        + Add layer
      </button>

      {#if adding}
        <button
          type="button"
          aria-label="Close profile list"
          class="fixed inset-0 z-10 cursor-default"
          onclick={() => (adding = false)}
        ></button>
        <div
          transition:fade={{ duration: 100 }}
          class="absolute left-0 z-20 mt-1 w-64 overflow-hidden rounded-lg border border-ink-200 bg-white shadow-lg dark:border-ink-700 dark:bg-ink-800"
        >
          <div class="border-b border-ink-100 p-1.5 dark:border-ink-700">
            <!-- svelte-ignore a11y_autofocus -->
            <input
              autofocus
              placeholder="Search profiles..."
              bind:value={query}
              class="w-full rounded-md bg-ink-100 px-2 py-1 text-[13px] text-ink-800 focus-visible:outline-none dark:bg-ink-900 dark:text-ink-100"
            />
          </div>
          <ul class="max-h-56 overflow-y-auto p-1">
            {#each filtered as profile (profile.name)}
              {@const inStack = settings.profileStack.some(
                (l) => l.kind === "builtin" && l.name === profile.name,
              )}
              <li>
                <button
                  type="button"
                  disabled={inStack}
                  onclick={() => addBuiltin(profile.name)}
                  class="flex w-full flex-col items-start gap-0.5 rounded-md px-2 py-1.5 text-left hover:bg-teal-50 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent dark:hover:bg-teal-500/10"
                >
                  <span class="flex w-full items-center justify-between gap-2">
                    <span class="text-[13px] font-medium text-ink-800 dark:text-ink-100">{profile.name}</span>
                    <span class="shrink-0 text-[10px] text-ink-400">
                      {inStack ? "already added" : screenLabel(profile.caps.screen_w, profile.caps.screen_h)}
                    </span>
                  </span>
                  <span class="line-clamp-2 text-[11px] leading-snug text-ink-500 dark:text-ink-400">
                    {profile.description}
                  </span>
                </button>
              </li>
            {:else}
              <li class="px-2 py-2 text-[12px] text-ink-400">No profile matches "{query}".</li>
            {/each}
          </ul>
        </div>
      {/if}
    </div>

    <button
      type="button"
      onclick={addLayer}
      class="text-[12px] font-medium text-teal-700 hover:text-teal-500 dark:text-teal-400"
    >
      + Add profile JSON...
    </button>
  </div>

  <!-- role="status" because this is the only feedback the add action gives:
       nothing else on screen changes when the picked file is already a layer,
       so without it a screen-reader user gets silence. -->
  {#if duplicateNotice}
    <p
      role="status"
      class="inline-flex w-fit items-center gap-1 rounded-full bg-amber-100 px-2 py-0.5 text-[11px] text-amber-800 dark:bg-amber-500/10 dark:text-amber-400"
    >
      {duplicateNotice}
    </p>
  {/if}
</div>
