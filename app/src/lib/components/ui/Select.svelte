<script lang="ts">
  interface Option {
    value: string;
    label: string;
  }

  let {
    value = $bindable(),
    options,
    disabled = false,
    ariaLabel,
    onchange,
  }: {
    value: string;
    options: Option[];
    disabled?: boolean;
    ariaLabel?: string;
    onchange?: (value: string) => void;
  } = $props();
</script>

<div class="relative">
  <select
    bind:value
    {disabled}
    aria-label={ariaLabel}
    onchange={() => onchange?.(value)}
    class="w-full appearance-none rounded-lg border border-ink-300 bg-white py-1.5 pl-2.5 pr-8 text-sm text-ink-800 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-teal-500 dark:focus-visible:ring-teal-400 disabled:cursor-not-allowed disabled:opacity-50 dark:border-ink-700 dark:bg-ink-800 dark:text-ink-100"
  >
    {#each options as option (option.value)}
      <option value={option.value}>{option.label}</option>
    {/each}
  </select>
  <svg
    class="pointer-events-none absolute right-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-ink-400"
    viewBox="0 0 20 20"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
  >
    <path d="M6 8l4 4 4-4" stroke-linecap="round" stroke-linejoin="round" />
  </svg>
</div>
