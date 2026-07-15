<script lang="ts">
  // A small group of mutually exclusive choices - one of them is always on.
  // Icon-sized by default; `labeled` renders icon + text for a control that
  // has to read as a first-class concept (the Edit/Fit mode switch), not a
  // toolbar affordance. Button has neither a pressed state nor an icon-only
  // size, and bending it into both would cost more than this. Controlled,
  // like Toggle: it renders `value` and asks the parent to change it.
  import type { Snippet } from "svelte";

  interface Option {
    value: string;
    /** The accessible name and tooltip; also rendered as text when `labeled`. */
    label: string;
    icon: Snippet;
  }

  let {
    value,
    options,
    onchange,
    labeled = false,
  }: {
    value: string;
    options: Option[];
    onchange: (value: string) => void;
    labeled?: boolean;
  } = $props();
</script>

<div class="inline-flex rounded-lg border border-zinc-300 p-0.5 dark:border-zinc-700">
  {#each options as option (option.value)}
    <button
      type="button"
      title={option.label}
      aria-label={option.label}
      aria-pressed={option.value === value}
      onclick={() => onchange(option.value)}
      class="flex items-center gap-1.5 rounded-md transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 {labeled
        ? 'px-3 py-1.5 text-[13px] font-medium'
        : 'px-2 py-1'} {option.value === value
        ? 'bg-indigo-600 text-white'
        : 'text-zinc-600 hover:bg-zinc-200/70 dark:text-zinc-300 dark:hover:bg-zinc-800'}"
    >
      {@render option.icon()}
      {#if labeled}
        <span>{option.label}</span>
      {/if}
    </button>
  {/each}
</div>
