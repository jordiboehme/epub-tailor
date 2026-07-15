<script lang="ts">
  // Edit mode's Inspector: the metadata editor and nothing else. There are no
  // output settings here on purpose - editing works like a tag editor, and
  // saving writes into the original file itself (with a safety copy in the
  // Trash first; the ActionBar owns that flow).
  import MetadataEditor from "./MetadataEditor.svelte";
</script>

<div class="flex flex-col text-sm">
  <section class="border-b border-ink-200 px-4 py-4 dark:border-ink-800">
    <h3 class="mb-2.5 text-[11px] font-semibold uppercase tracking-wide text-ink-400">Metadata</h3>
    <MetadataEditor />
  </section>

  <section class="px-4 py-4">
    <p class="text-[11px] leading-snug text-ink-500 dark:text-ink-400">
      Saving writes into the original file. The previous version goes to the Trash.
    </p>
  </section>
</div>
