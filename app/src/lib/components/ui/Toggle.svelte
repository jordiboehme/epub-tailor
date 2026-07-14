<script lang="ts">
  // A controlled switch: it renders `checked` and asks the parent to change it
  // via `onchange`. Keeping it controlled lets a caller intercept the flip -
  // the in-place toggle, for instance, holds off until a confirm dialog says so.
  let {
    checked = false,
    disabled = false,
    label,
    onchange,
  }: {
    checked?: boolean;
    disabled?: boolean;
    label?: string;
    onchange?: (checked: boolean) => void;
  } = $props();
</script>

<button
  type="button"
  role="switch"
  aria-checked={checked}
  aria-label={label}
  {disabled}
  onclick={() => onchange?.(!checked)}
  class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 focus-visible:ring-offset-1 focus-visible:ring-offset-zinc-50 disabled:cursor-not-allowed disabled:opacity-50 dark:focus-visible:ring-offset-zinc-950 {checked
    ? 'bg-indigo-600'
    : 'bg-zinc-300 dark:bg-zinc-700'}"
>
  <span
    class="inline-block h-4 w-4 transform rounded-full bg-white shadow-sm transition-transform {checked
      ? 'translate-x-4'
      : 'translate-x-0.5'}"
  ></span>
</button>
