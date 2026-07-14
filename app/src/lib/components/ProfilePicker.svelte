<script lang="ts">
  import { fade } from "svelte/transition";
  import { open } from "@tauri-apps/plugin-dialog";
  import { profiles } from "../stores/profiles.svelte";
  import { settings } from "../stores/settings.svelte";

  let expanded = $state(false);
  let query = $state("");

  const current = $derived(profiles.builtins.find((p) => p.name === settings.profile));
  const filtered = $derived(
    profiles.builtins.filter((p) =>
      `${p.name} ${p.description}`.toLowerCase().includes(query.trim().toLowerCase()),
    ),
  );

  function pick(name: string) {
    settings.profile = name;
    expanded = false;
    query = "";
  }

  function screenLabel(w: number, h: number): string {
    return w > 0 && h > 0 ? `${w} x ${h}` : "device-neutral";
  }

  function baseName(path: string): string {
    const slash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    return slash >= 0 ? path.slice(slash + 1) : path;
  }

  async function addLayer() {
    const selection = await open({
      multiple: false,
      filters: [{ name: "Profile JSON", extensions: ["json"] }],
    });
    if (typeof selection !== "string") return;
    if (settings.userProfilePaths.includes(selection)) return;
    settings.userProfilePaths = [...settings.userProfilePaths, selection];
  }

  function removeLayer(path: string) {
    settings.userProfilePaths = settings.userProfilePaths.filter((p) => p !== path);
  }
</script>

<div class="relative">
  <button
    type="button"
    onclick={() => (expanded = !expanded)}
    class="flex w-full items-center justify-between rounded-lg border border-zinc-300 bg-white px-2.5 py-1.5 text-left text-sm text-zinc-800 transition-colors hover:bg-zinc-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-100 dark:hover:bg-zinc-700"
  >
    <span class="truncate">{current?.name ?? settings.profile}</span>
    <svg class="h-4 w-4 shrink-0 text-zinc-400" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
      <path d="M6 8l4 4 4-4" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  </button>

  {#if current?.description}
    <p class="mt-1 text-[11px] leading-snug text-zinc-500 dark:text-zinc-400">{current.description}</p>
  {/if}

  {#if expanded}
    <button
      type="button"
      aria-label="Close profile list"
      class="fixed inset-0 z-10 cursor-default"
      onclick={() => (expanded = false)}
    ></button>
    <div
      transition:fade={{ duration: 100 }}
      class="absolute left-0 right-0 z-20 mt-1 overflow-hidden rounded-lg border border-zinc-200 bg-white shadow-lg dark:border-zinc-700 dark:bg-zinc-800"
    >
      <div class="border-b border-zinc-100 p-1.5 dark:border-zinc-700">
        <!-- svelte-ignore a11y_autofocus -->
        <input
          autofocus
          placeholder="Search profiles..."
          bind:value={query}
          class="w-full rounded-md bg-zinc-100 px-2 py-1 text-[13px] text-zinc-800 focus-visible:outline-none dark:bg-zinc-900 dark:text-zinc-100"
        />
      </div>
      <ul class="max-h-56 overflow-y-auto p-1">
        {#each filtered as profile (profile.name)}
          <li>
            <button
              type="button"
              onclick={() => pick(profile.name)}
              class="flex w-full flex-col items-start gap-0.5 rounded-md px-2 py-1.5 text-left hover:bg-indigo-50 dark:hover:bg-indigo-500/10 {profile.name ===
              settings.profile
                ? 'bg-indigo-50 dark:bg-indigo-500/10'
                : ''}"
            >
              <span class="flex w-full items-center justify-between gap-2">
                <span class="text-[13px] font-medium text-zinc-800 dark:text-zinc-100">{profile.name}</span>
                <span class="shrink-0 text-[10px] text-zinc-400">
                  {screenLabel(profile.caps.screen_w, profile.caps.screen_h)}
                </span>
              </span>
              <span class="line-clamp-2 text-[11px] leading-snug text-zinc-500 dark:text-zinc-400">
                {profile.description}
              </span>
            </button>
          </li>
        {:else}
          <li class="px-2 py-2 text-[12px] text-zinc-400">No profile matches "{query}".</li>
        {/each}
      </ul>
    </div>
  {/if}

  <!-- User profile layers -->
  {#if settings.userProfilePaths.length > 0}
    <div class="mt-2 flex flex-wrap gap-1.5">
      {#each settings.userProfilePaths as path (path)}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-zinc-200 py-0.5 pl-2 pr-1 text-[11px] text-zinc-700 dark:bg-zinc-700 dark:text-zinc-200"
          title={path}
        >
          {baseName(path)}
          <button
            type="button"
            aria-label={"Remove " + baseName(path)}
            onclick={() => removeLayer(path)}
            class="rounded-full p-0.5 text-zinc-500 hover:bg-zinc-300 hover:text-zinc-800 dark:hover:bg-zinc-600 dark:hover:text-white"
          >
            <svg class="h-3 w-3" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2.2">
              <path d="M6 6l8 8M14 6l-8 8" stroke-linecap="round" />
            </svg>
          </button>
        </span>
      {/each}
    </div>
  {/if}

  <button
    type="button"
    onclick={addLayer}
    class="mt-2 text-[12px] font-medium text-indigo-600 hover:text-indigo-500 dark:text-indigo-400"
  >
    + Add profile JSON...
  </button>
</div>
