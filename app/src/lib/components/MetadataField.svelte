<script lang="ts">
  import { untrack } from "svelte";
  // One metadata field with an iTunes-style checkbox. The box is a pure
  // reflection of the staged state: editing (or emptying) the field stages it
  // through `oninput` and the box ticks itself; unchecking it unstages via
  // `onuncheck`; clicking an unticked box only focuses the input - there is
  // nothing to stage until something is typed. The parent owns all staging
  // and debouncing; this component owns the text box and the box's optics.

  let {
    label,
    value = "",
    placeholder = "",
    multiline = false,
    mixed = false,
    cleared = false,
    check = "unchecked",
    oninput,
    onuncheck,
  }: {
    label: string;
    value?: string;
    placeholder?: string;
    multiline?: boolean;
    mixed?: boolean;
    cleared?: boolean;
    check?: "checked" | "indeterminate" | "unchecked";
    oninput?: (value: string) => void;
    onuncheck?: () => void;
  } = $props();

  const external = $derived(mixed || cleared ? "" : value);
  const staged = $derived(check !== "unchecked");
  const shownPlaceholder = $derived(cleared ? "will be removed" : mixed ? "Mixed" : placeholder);

  let draft = $state("");
  let focused = $state(false);
  let box = $state<HTMLInputElement | null>(null);
  let field = $state<HTMLInputElement | HTMLTextAreaElement | null>(null);

  // Seed the box, and re-seed it when the external value changes (an uncheck
  // reset it, a search accept staged something) - but never mid-edit, so
  // typing is never yanked out from under someone. Only `external` is a
  // dependency; focused/draft are read untracked.
  $effect(() => {
    const incoming = external;
    untrack(() => {
      if (!focused && draft !== incoming) draft = incoming;
    });
  });

  // `indeterminate` is a DOM property, not an attribute.
  $effect(() => {
    if (box) box.indeterminate = check === "indeterminate";
  });

  function onInput(event: Event) {
    draft = (event.target as HTMLInputElement | HTMLTextAreaElement).value;
    oninput?.(draft);
  }

  function onBoxClick(event: MouseEvent) {
    // The box reflects staged state; it is never toggled directly.
    event.preventDefault();
    if (check === "unchecked") field?.focus();
    else onuncheck?.();
  }
</script>

<div class="flex flex-col gap-1">
  <label class="flex w-fit cursor-pointer items-center gap-1.5">
    <input
      bind:this={box}
      type="checkbox"
      checked={check === "checked"}
      onclick={onBoxClick}
      title={staged ? "Staged - uncheck to revert" : "Edit the field to stage it"}
      class="h-3 w-3 rounded accent-indigo-600"
    />
    <span class="text-[11px] font-medium text-zinc-500 dark:text-zinc-400">{label}</span>
  </label>

  {#if multiline}
    <textarea
      bind:this={field}
      rows="3"
      spellcheck="false"
      placeholder={shownPlaceholder}
      value={draft}
      oninput={onInput}
      onfocus={() => (focused = true)}
      onblur={() => (focused = false)}
      class="min-w-0 resize-y rounded-lg border bg-white px-2.5 py-1.5 text-[13px] text-zinc-800 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 dark:bg-zinc-800 dark:text-zinc-100 {staged
        ? 'border-indigo-400 dark:border-indigo-500/60'
        : 'border-zinc-300 dark:border-zinc-700'}"
    ></textarea>
  {:else}
    <input
      bind:this={field}
      type="text"
      spellcheck="false"
      placeholder={shownPlaceholder}
      value={draft}
      oninput={onInput}
      onfocus={() => (focused = true)}
      onblur={() => (focused = false)}
      class="min-w-0 rounded-lg border bg-white px-2.5 py-1.5 text-[13px] text-zinc-800 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 dark:bg-zinc-800 dark:text-zinc-100 {staged
        ? 'border-indigo-400 dark:border-indigo-500/60'
        : 'border-zinc-300 dark:border-zinc-700'}"
    />
  {/if}
</div>
