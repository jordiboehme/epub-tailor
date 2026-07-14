<script lang="ts">
  import { untrack } from "svelte";
  // One metadata field, in two moods. Single-book: it shows the effective value
  // (staged over the book's own), edits stage live through `oninput`, and a
  // revert affordance appears once the field is dirty. Multi-book: it shows the
  // shared value or a "Mixed" hint, edits stay local until "Apply to N" commits
  // them to every selected book through `onapply`. The parent owns all staging;
  // this component owns only the text box.

  let {
    label,
    value = "",
    bookValue = "",
    placeholder = "",
    multiline = false,
    multi = false,
    mixed = false,
    dirty = false,
    applyCount = 0,
    oninput,
    onapply,
    onrevert,
  }: {
    label: string;
    value?: string;
    bookValue?: string;
    placeholder?: string;
    multiline?: boolean;
    multi?: boolean;
    mixed?: boolean;
    dirty?: boolean;
    applyCount?: number;
    oninput?: (value: string) => void;
    onapply?: (value: string) => void;
    onrevert?: () => void;
  } = $props();

  const external = $derived(multi ? (mixed ? "" : value) : value);

  let draft = $state("");
  let focused = $state(false);

  // Seed the box, and re-seed it when the external value changes (a search
  // accept staged something, a revert reset it) - but never mid-edit, so typing
  // is never yanked out from under someone. Only `external` is a dependency;
  // focused/draft are read untracked so a keystroke does not retrigger this.
  $effect(() => {
    const incoming = external;
    untrack(() => {
      if (!focused && draft !== incoming) draft = incoming;
    });
  });

  const canApply = $derived(multi && draft.trim().length > 0);

  function onInput(event: Event) {
    draft = (event.target as HTMLInputElement | HTMLTextAreaElement).value;
    if (!multi) oninput?.(draft);
  }

  function revert() {
    focused = false;
    draft = bookValue;
    onrevert?.();
  }

  function apply() {
    if (canApply) onapply?.(draft);
  }
</script>

<div class="flex flex-col gap-1">
  <div class="flex items-center justify-between">
    <span class="text-[11px] font-medium text-zinc-500 dark:text-zinc-400">{label}</span>
    {#if !multi && dirty}
      <button
        type="button"
        onclick={revert}
        class="text-[10px] font-medium text-zinc-400 hover:text-indigo-500 dark:text-zinc-500 dark:hover:text-indigo-400"
      >
        Revert
      </button>
    {/if}
  </div>

  <div class="flex items-start gap-1.5">
    {#if multiline}
      <textarea
        rows="3"
        spellcheck="false"
        placeholder={mixed ? "Mixed" : placeholder}
        value={draft}
        oninput={onInput}
        onfocus={() => (focused = true)}
        onblur={() => (focused = false)}
        class="min-w-0 flex-1 resize-y rounded-lg border bg-white px-2.5 py-1.5 text-[13px] text-zinc-800 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 dark:bg-zinc-800 dark:text-zinc-100 {dirty
          ? 'border-indigo-400 dark:border-indigo-500/60'
          : 'border-zinc-300 dark:border-zinc-700'}"
      ></textarea>
    {:else}
      <input
        type="text"
        spellcheck="false"
        placeholder={mixed ? "Mixed" : placeholder}
        value={draft}
        oninput={onInput}
        onfocus={() => (focused = true)}
        onblur={() => (focused = false)}
        class="min-w-0 flex-1 rounded-lg border bg-white px-2.5 py-1.5 text-[13px] text-zinc-800 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 dark:bg-zinc-800 dark:text-zinc-100 {dirty
          ? 'border-indigo-400 dark:border-indigo-500/60'
          : 'border-zinc-300 dark:border-zinc-700'}"
      />
    {/if}

    {#if multi}
      <button
        type="button"
        disabled={!canApply}
        onclick={apply}
        title={`Apply to ${applyCount} ${applyCount === 1 ? "book" : "books"}`}
        class="shrink-0 rounded-lg border border-indigo-500 bg-indigo-50 px-2 py-1.5 text-[11px] font-medium text-indigo-700 transition-colors hover:bg-indigo-100 disabled:cursor-not-allowed disabled:opacity-40 dark:border-indigo-500/50 dark:bg-indigo-500/10 dark:text-indigo-300 dark:hover:bg-indigo-500/20"
      >
        Apply to {applyCount}
      </button>
    {/if}
  </div>
</div>
