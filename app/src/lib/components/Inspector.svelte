<script lang="ts">
  import { slide } from "svelte/transition";
  import { books } from "../stores/books.svelte";
  import { settings } from "../stores/settings.svelte";
  import ProfilePicker from "./ProfilePicker.svelte";
  import OutputPicker from "./OutputPicker.svelte";
  import TemplateField from "./TemplateField.svelte";
  import MetadataEditor from "./MetadataEditor.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import Toggle from "./ui/Toggle.svelte";
  import Select from "./ui/Select.svelte";

  let confirmInPlace = $state(false);
  let advancedOpen = $state(false);

  const qualityOptions = [
    { value: "", label: "Profile default" },
    { value: "low", label: "Low" },
    { value: "std", label: "Standard" },
    { value: "high", label: "High" },
  ];
  const tablesOptions = [
    { value: "", label: "Profile default" },
    { value: "text", label: "Text" },
    { value: "image", label: "Image" },
    { value: "image-all", label: "Image (all)" },
  ];
  const splitOptions = [
    { value: "1", label: "Heading 1" },
    { value: "2", label: "Heading 2" },
  ];

  // Markdown-only, so the control only shows up when a Markdown book is
  // actually in the firing line.
  const hasMarkdown = $derived(books.targets.some((b) => b.kind === "md"));

  function onInPlaceToggle(next: boolean) {
    if (next) confirmInPlace = true;
    else settings.inPlace = false;
  }
</script>

<div class="flex flex-col text-sm">
  <section class="border-b border-zinc-200 px-4 py-4 dark:border-zinc-800">
    <h3 class="mb-2.5 text-[11px] font-semibold uppercase tracking-wide text-zinc-400">Metadata</h3>
    <MetadataEditor />
  </section>

  <section class="border-b border-zinc-200 px-4 py-4 dark:border-zinc-800">
    <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-zinc-400">Profile</h3>
    <ProfilePicker />
  </section>

  <section class="border-b border-zinc-200 px-4 py-4 dark:border-zinc-800">
    <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-zinc-400">Destination</h3>
    <OutputPicker />
  </section>

  <section class="border-b border-zinc-200 px-4 py-4 dark:border-zinc-800">
    <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-zinc-400">Naming</h3>
    <TemplateField />
  </section>

  {#if hasMarkdown}
    <section transition:slide={{ duration: 150 }} class="border-b border-zinc-200 px-4 py-4 dark:border-zinc-800">
      <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-zinc-400">Markdown</h3>
      <span class="mb-1 block text-[12px] text-zinc-500 dark:text-zinc-400">Split chapters at</span>
      <Select
        value={String(settings.mdSplitLevel)}
        options={splitOptions}
        ariaLabel="Split chapters at"
        onchange={(v) => (settings.mdSplitLevel = Number(v))}
      />
      <p class="mt-1.5 text-[11px] leading-snug text-zinc-500 dark:text-zinc-400">
        Every heading at this level starts a new chapter.
      </p>
    </section>
  {/if}

  <section class="px-4 py-4">
    <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-zinc-400">Options</h3>

    <div class="flex flex-col gap-3">
      <label class="flex cursor-pointer items-center justify-between gap-3">
        <span>
          <span class="block text-[13px] font-medium text-zinc-700 dark:text-zinc-200">Preview only</span>
          <span class="block text-[11px] text-zinc-500 dark:text-zinc-400">See what would change, write nothing.</span>
        </span>
        <Toggle checked={settings.dryRun} onchange={(v) => (settings.dryRun = v)} label="Preview only" />
      </label>

      <label class="flex cursor-pointer items-center justify-between gap-3">
        <span>
          <span class="block text-[13px] font-medium text-zinc-700 dark:text-zinc-200">Replace originals</span>
          <span class="block text-[11px] text-zinc-500 dark:text-zinc-400">Rewrite each book in place.</span>
        </span>
        <Toggle checked={settings.inPlace} onchange={onInPlaceToggle} label="Replace originals" />
      </label>
    </div>

    <!-- Advanced -->
    <button
      type="button"
      onclick={() => (advancedOpen = !advancedOpen)}
      class="mt-3 flex items-center gap-1 text-[12px] font-medium text-zinc-500 hover:text-zinc-700 dark:text-zinc-400 dark:hover:text-zinc-200"
    >
      <svg
        class="h-3.5 w-3.5 transition-transform {advancedOpen ? 'rotate-90' : ''}"
        viewBox="0 0 20 20"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
      >
        <path d="M8 6l4 4-4 4" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
      Advanced
    </button>

    {#if advancedOpen}
      <div transition:slide={{ duration: 150 }} class="mt-3 flex flex-col gap-3">
        <div>
          <span class="mb-1 block text-[12px] text-zinc-500 dark:text-zinc-400">Image quality</span>
          <Select
            value={settings.quality ?? ""}
            options={qualityOptions}
            ariaLabel="Image quality"
            onchange={(v) => (settings.quality = v || null)}
          />
        </div>
        <div>
          <span class="mb-1 block text-[12px] text-zinc-500 dark:text-zinc-400">Tables</span>
          <Select
            value={settings.tables ?? ""}
            options={tablesOptions}
            ariaLabel="Tables"
            onchange={(v) => (settings.tables = v || null)}
          />
        </div>
      </div>
    {/if}
  </section>
</div>

{#if confirmInPlace}
  <ConfirmDialog
    title="Lets. Get. Dangerous."
    confirmLabel="Replace originals"
    cancelLabel="Keep copies"
    confirmVariant="danger"
    onConfirm={() => {
      settings.inPlace = true;
      confirmInPlace = false;
    }}
    onCancel={() => (confirmInPlace = false)}
  >
    This rewrites your originals in place. A failed write cannot eat a book - the new file is staged
    and swapped in atomically - but there is no undo. Your call.
  </ConfirmDialog>
{/if}
