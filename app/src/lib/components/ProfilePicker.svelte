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
    layerKey,
    addBuiltinLayer,
    addFileLayer,
    describeDuplicateLayer,
    removeLayerAt,
    replaceLayerAt,
    moveLayer,
    hasDeviceClash,
    hasScreen,
  } from "../stores/profiles.svelte";
  import type { ProfileLayer } from "../stores/settings.svelte";
  import { settings } from "../stores/settings.svelte";

  // Which dropdown is open, if any. `replace` targets one existing layer:
  // without it a stack of one is unchangeable, since move and remove are all
  // necessarily disabled there and adding a second layer is a different
  // intent than swapping the one you have.
  type Menu = { mode: "add" } | { mode: "replace"; index: number };
  let menu = $state<Menu | null>(null);
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

  function screenLabel(w: number, h: number): string {
    return hasScreen({ screen_w: w, screen_h: h }) ? `${w} x ${h}` : "device-neutral";
  }

  /** Open a dropdown, or close it if that same one is already open. */
  function toggleMenu(next: Menu) {
    const same =
      menu?.mode === next.mode &&
      (next.mode === "add" || (menu.mode === "replace" && menu.index === next.index));
    menu = same ? null : next;
    query = "";
  }

  function closeMenu() {
    menu = null;
    query = "";
  }

  /** The layer a replace dropdown is aimed at, or `null` in add mode. */
  const targetKey = $derived(
    menu?.mode === "replace" ? layerKey(settings.profileStack[menu.index]) : null,
  );

  function addBuiltin(name: string) {
    settings.profileStack = addBuiltinLayer(settings.profileStack, name);
    closeMenu();
  }

  function replaceWithBuiltin(index: number, name: string) {
    settings.profileStack = replaceLayerAt(settings.profileStack, index, { kind: "builtin", name });
    duplicateNotice = null;
    closeMenu();
  }

  /** The OS file dialog, shared by the add and replace paths. */
  async function pickProfileJson(): Promise<string | null> {
    const selection = await open({
      multiple: false,
      filters: [{ name: "Profile JSON", extensions: ["json"] }],
    });
    return typeof selection === "string" ? selection : null;
  }

  async function addLayer() {
    const selection = await pickProfileJson();
    if (selection === null) return;
    duplicateNotice = describeDuplicateLayer(settings.profileStack, selection);
    settings.profileStack = addFileLayer(settings.profileStack, selection);
  }

  async function replaceWithFile(index: number) {
    const selection = await pickProfileJson();
    if (selection === null) return;
    // Ignore the layer being replaced: re-picking the file it already holds is
    // a no-op, and reporting it as a duplicate would read as a refusal.
    duplicateNotice = describeDuplicateLayer(settings.profileStack, selection, index);
    settings.profileStack = replaceLayerAt(settings.profileStack, index, {
      kind: "file",
      path: selection,
    });
    closeMenu();
  }

  function removeLayer(index: number) {
    settings.profileStack = removeLayerAt(settings.profileStack, index);
    duplicateNotice = null;
    closeMenu();
  }

  function move(index: number, delta: number) {
    settings.profileStack = moveLayer(settings.profileStack, index, delta);
  }
</script>

<!-- The built-in list, shared by the add and replace dropdowns. `currentKey`
     is the layer a replace dropdown would overwrite: its own entry reads
     "current" rather than "already added", because picking it is a harmless
     no-op and not the clash the other in-stack entries are.

     `top-full` is load-bearing, not decoration: the replace dropdown's anchor
     is the layer row, a flex container, so an absolutely positioned child
     with no `top` falls back to its static position inside the flex line and
     lands over the row instead of under it. -->
{#snippet profileMenu(currentKey: string | null, pick: (name: string) => void, fileAction: (() => void) | null, fileLabel: string)}
  <button
    type="button"
    aria-label="Close profile list"
    class="fixed inset-0 z-10 cursor-default"
    onclick={closeMenu}
  ></button>
  <div
    transition:fade={{ duration: 100 }}
    class="absolute left-0 top-full z-20 mt-1 w-64 overflow-hidden rounded-lg border border-ink-200 bg-white shadow-lg dark:border-ink-700 dark:bg-ink-800"
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
        {@const isCurrent = currentKey === `builtin:${profile.name}`}
        {@const inStack =
          !isCurrent &&
          settings.profileStack.some((l) => l.kind === "builtin" && l.name === profile.name)}
        <li>
          <button
            type="button"
            disabled={inStack}
            onclick={() => pick(profile.name)}
            class="flex w-full flex-col items-start gap-0.5 rounded-md px-2 py-1.5 text-left hover:bg-teal-50 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent dark:hover:bg-teal-500/10"
          >
            <span class="flex w-full items-center justify-between gap-2">
              <span class="text-[13px] font-medium text-ink-800 dark:text-ink-100">{profile.name}</span>
              <span
                class="shrink-0 text-[10px] {isCurrent
                  ? 'font-medium text-teal-600 dark:text-teal-400'
                  : 'text-ink-400'}"
              >
                {isCurrent
                  ? "current"
                  : inStack
                    ? "already added"
                    : screenLabel(profile.caps.screen_w, profile.caps.screen_h)}
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
    {#if fileAction}
      <div class="border-t border-ink-100 p-1 dark:border-ink-700">
        <button
          type="button"
          onclick={fileAction}
          class="w-full rounded-md px-2 py-1.5 text-left text-[12px] font-medium text-teal-700 hover:bg-teal-50 dark:text-teal-400 dark:hover:bg-teal-500/10"
        >
          {fileLabel}
        </button>
      </div>
    {/if}
  </div>
{/snippet}

<div class="flex flex-col gap-1.5">
  {#each settings.profileStack as layer, i (layerKey(layer))}
    <div
      class="relative flex items-center gap-2 rounded-lg border border-ink-300 bg-white px-2.5 py-1.5 text-sm text-ink-800 dark:border-ink-700 dark:bg-ink-800 dark:text-ink-100"
    >
      <!-- The name is the swap control. Without it a stack of one is frozen:
           move and remove are both necessarily disabled there, so there would
           be no way to change the profile short of adding a second layer and
           deleting the first. -->
      <button
        type="button"
        aria-haspopup="listbox"
        aria-expanded={menu?.mode === "replace" && menu.index === i}
        aria-label={"Change profile " + layerLabel(layer)}
        title={layer.kind === "file" ? layer.path : "Change this profile"}
        onclick={() => toggleMenu({ mode: "replace", index: i })}
        class="flex min-w-0 flex-1 items-center gap-1 rounded-md px-1 py-0.5 text-left hover:bg-ink-100 dark:hover:bg-ink-700"
      >
        <span class="truncate">{layerLabel(layer)}</span>
        <svg
          class="h-3 w-3 shrink-0 text-ink-400"
          viewBox="0 0 20 20"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <path d="M6 8l4 4 4-4" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>

      {#if menu?.mode === "replace" && menu.index === i}
        {@render profileMenu(
          targetKey,
          (name) => replaceWithBuiltin(i, name),
          () => replaceWithFile(i),
          "Replace with profile JSON...",
        )}
      {/if}

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
        aria-haspopup="listbox"
        aria-expanded={menu?.mode === "add"}
        onclick={() => toggleMenu({ mode: "add" })}
        class="text-[12px] font-medium text-teal-700 hover:text-teal-500 dark:text-teal-400"
      >
        + Add layer
      </button>

      {#if menu?.mode === "add"}
        {@render profileMenu(null, addBuiltin, null, "")}
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
       nothing else on screen changes when the picked file is already a layer.
       Note the region is mounted with its text already in place rather than
       filled after mounting, which not every screen reader announces - this
       raises the odds, it does not guarantee them. -->
  {#if duplicateNotice}
    <p
      role="status"
      class="inline-flex w-fit items-center gap-1 rounded-full bg-amber-100 px-2 py-0.5 text-[11px] text-amber-800 dark:bg-amber-500/10 dark:text-amber-400"
    >
      {duplicateNotice}
    </p>
  {/if}
</div>
