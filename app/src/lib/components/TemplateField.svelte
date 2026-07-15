<script lang="ts">
  import { fade } from "svelte/transition";
  import { previewOutputName } from "../api/outputs";
  import { books, toTemplateFile } from "../stores/books.svelte";
  import { profiles } from "../stores/profiles.svelte";
  import { settings } from "../stores/settings.svelte";

  let showTokens = $state(false);

  const sample = $derived(books.selectedFiles[0] ?? books.books[0]?.files[0]);

  // The appendix the planner stamps on an output that would land on its own
  // input - which the defaults ({original}, alongside originals) do for every
  // book there is. Resolving it can mean asking the CLI to compose the user's
  // profile layers, so it is kept in state and refreshed when they change.
  let appendix = $state("tailored");
  $effect(() => {
    // Read what the answer depends on here, in the effect's tracking scope; the
    // resolution itself is async, and a failed composition keeps the fallback.
    void profiles.builtins;
    void settings.profile;
    void settings.userProfilePaths;
    let live = true;
    void profiles
      .activeAppendix()
      .then((next) => {
        if (live) appendix = next;
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  });

  // The real planner, run on the sample book: a preview that renders the
  // template alone would promise "Dune.epub" while the app writes
  // "Dune.tailored.epub".
  const preview = $derived(
    sample
      ? previewOutputName(
          { input: sample.path, kind: sample.kind, template: toTemplateFile(sample) },
          {
            template: settings.filenameTemplate,
            outputDir: settings.outputDir,
            appendix,
          },
        )
      : "",
  );

  const tokens: { token: string; meaning: string }[] = [
    { token: "{title}", meaning: "book title" },
    { token: "{author}", meaning: "first author" },
    { token: "{authors}", meaning: "all authors, joined" },
    { token: "{series}", meaning: "series name" },
    { token: "{series_index}", meaning: "position in the series" },
    { token: "{original}", meaning: "the file's own name" },
  ];
</script>

<div class="flex flex-col gap-1.5">
  <div class="relative">
    <input
      type="text"
      spellcheck="false"
      autocapitalize="off"
      autocorrect="off"
      bind:value={settings.filenameTemplate}
      class="w-full rounded-lg border border-ink-300 bg-white px-2.5 py-1.5 pr-9 font-mono text-[13px] text-ink-800 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-teal-500 dark:focus-visible:ring-teal-400 disabled:cursor-not-allowed dark:border-ink-700 dark:bg-ink-800 dark:text-ink-100"
    />
    <button
      type="button"
      title="Available tokens"
      aria-label="Available tokens"
      onclick={() => (showTokens = !showTokens)}
      class="absolute right-1.5 top-1/2 -translate-y-1/2 rounded-md p-1 text-ink-400 hover:bg-ink-100 hover:text-ink-600 dark:hover:bg-ink-700"
    >
      <svg class="h-4 w-4" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
        <circle cx="10" cy="10" r="7.25" />
        <path d="M10 9.5v3.5M10 6.75h.01" stroke-linecap="round" />
      </svg>
    </button>

    {#if showTokens}
      <button type="button" aria-label="Close" class="fixed inset-0 z-10 cursor-default" onclick={() => (showTokens = false)}></button>
      <div
        transition:fade={{ duration: 100 }}
        class="absolute right-0 z-20 mt-1 w-60 rounded-lg border border-ink-200 bg-white p-2 shadow-lg dark:border-ink-700 dark:bg-ink-800"
      >
        <ul class="flex flex-col gap-1">
          {#each tokens as t (t.token)}
            <li class="flex items-baseline justify-between gap-2 text-[12px]">
              <code class="text-teal-700 dark:text-teal-400">{t.token}</code>
              <span class="text-ink-500 dark:text-ink-400">{t.meaning}</span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  </div>

  {#if preview}
    <p class="truncate text-[11px] text-ink-500 dark:text-ink-400" title={preview}>
      Preview: <span class="text-ink-700 dark:text-ink-300">{preview}</span>
    </p>
  {/if}
</div>
