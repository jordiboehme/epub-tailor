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
    class="flex w-full items-center justify-between rounded-lg border border-ink-300 bg-white px-2.5 py-1.5 text-left text-sm text-ink-800 transition-colors hover:bg-ink-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-teal-500 dark:focus-visible:ring-teal-400 dark:border-ink-700 dark:bg-ink-800 dark:text-ink-100 dark:hover:bg-ink-700"
  >
    <span class="truncate">{current?.name ?? settings.profile}</span>
    <svg class="h-4 w-4 shrink-0 text-ink-400" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
      <path d="M6 8l4 4 4-4" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  </button>

  {#if current?.description}
    <p class="mt-1 text-[11px] leading-snug text-ink-500 dark:text-ink-400">{current.description}</p>
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
      class="absolute left-0 right-0 z-20 mt-1 overflow-hidden rounded-lg border border-ink-200 bg-white shadow-lg dark:border-ink-700 dark:bg-ink-800"
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
          <li>
            <button
              type="button"
              onclick={() => pick(profile.name)}
              class="flex w-full flex-col items-start gap-0.5 rounded-md px-2 py-1.5 text-left hover:bg-teal-50 dark:hover:bg-teal-500/10 {profile.name ===
              settings.profile
                ? 'bg-teal-50 dark:bg-teal-500/10'
                : ''}"
            >
              <span class="flex w-full items-center justify-between gap-2">
                <span class="text-[13px] font-medium text-ink-800 dark:text-ink-100">{profile.name}</span>
                <span class="shrink-0 text-[10px] text-ink-400">
                  {screenLabel(profile.caps.screen_w, profile.caps.screen_h)}
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

  <!-- User profile layers -->
  {#if settings.userProfilePaths.length > 0}
    <div class="mt-2 flex flex-wrap gap-1.5">
      {#each settings.userProfilePaths as path (path)}
        <span
          class="inline-flex items-center gap-1 rounded-full bg-ink-200 py-0.5 pl-2 pr-1 text-[11px] text-ink-700 dark:bg-ink-700 dark:text-ink-200"
          title={path}
        >
          {baseName(path)}
          <button
            type="button"
            aria-label={"Remove " + baseName(path)}
            onclick={() => removeLayer(path)}
            class="rounded-full p-0.5 text-ink-500 hover:bg-ink-300 hover:text-ink-800 dark:hover:bg-ink-600 dark:hover:text-white"
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
    class="mt-2 text-[12px] font-medium text-teal-700 hover:text-teal-500 dark:text-teal-400"
  >
    + Add profile JSON...
  </button>
</div>
