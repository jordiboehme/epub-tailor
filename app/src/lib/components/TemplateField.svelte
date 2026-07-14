<script lang="ts">
  import { fade } from "svelte/transition";
  import { renderTemplate } from "../api/templates";
  import { books, toTemplateBook } from "../stores/books.svelte";
  import { settings } from "../stores/settings.svelte";

  let showTokens = $state(false);

  const disabled = $derived(settings.inPlace);

  const sample = $derived(books.selected[0] ?? books.books[0]);
  const preview = $derived(
    sample
      ? `${renderTemplate(settings.filenameTemplate, toTemplateBook(sample))}.epub`
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

<div class="flex flex-col gap-1.5" class:opacity-50={disabled}>
  <div class="relative">
    <input
      type="text"
      spellcheck="false"
      autocapitalize="off"
      autocorrect="off"
      {disabled}
      bind:value={settings.filenameTemplate}
      class="w-full rounded-lg border border-zinc-300 bg-white px-2.5 py-1.5 pr-9 font-mono text-[13px] text-zinc-800 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 disabled:cursor-not-allowed dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-100"
    />
    <button
      type="button"
      title="Available tokens"
      aria-label="Available tokens"
      onclick={() => (showTokens = !showTokens)}
      class="absolute right-1.5 top-1/2 -translate-y-1/2 rounded-md p-1 text-zinc-400 hover:bg-zinc-100 hover:text-zinc-600 dark:hover:bg-zinc-700"
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
        class="absolute right-0 z-20 mt-1 w-60 rounded-lg border border-zinc-200 bg-white p-2 shadow-lg dark:border-zinc-700 dark:bg-zinc-800"
      >
        <ul class="flex flex-col gap-1">
          {#each tokens as t (t.token)}
            <li class="flex items-baseline justify-between gap-2 text-[12px]">
              <code class="text-indigo-600 dark:text-indigo-400">{t.token}</code>
              <span class="text-zinc-500 dark:text-zinc-400">{t.meaning}</span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  </div>

  {#if disabled}
    <p class="text-[11px] leading-snug text-zinc-500 dark:text-zinc-400">
      Naming is off while replacing originals - each book keeps its own name.
    </p>
  {:else if preview}
    <p class="truncate text-[11px] text-zinc-500 dark:text-zinc-400" title={preview}>
      Preview: <span class="text-zinc-700 dark:text-zinc-300">{preview}</span>
    </p>
  {/if}
</div>
