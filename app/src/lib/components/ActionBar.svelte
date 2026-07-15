<script lang="ts">
  import { resolvePlans } from "../api/plan";
  import type { RunOptions } from "../api/argv";
  import { needsCleanup } from "../api/book-view";
  import { books, toTemplateFile } from "../stores/books.svelte";
  import type { BookFile } from "../stores/books.svelte";
  import { jobs } from "../stores/jobs.svelte";
  import { edits } from "../stores/edits.svelte";
  import { saveFilesInPlace } from "../stores/inplace";
  import { profiles } from "../stores/profiles.svelte";
  import { settings } from "../stores/settings.svelte";
  import Button from "./ui/Button.svelte";

  let starting = $state(false);
  let planError = $state<string | null>(null);
  let notice = $state<string | null>(null);

  // The one definition of "these files" lives in the books store; both buttons
  // and every label below read it, so they can never disagree. With nothing
  // selected the targets are every book's original - so the labels count
  // books; an explicit selection counts files.
  const targetFiles = $derived(books.targets);
  const targetCount = $derived(targetFiles.length);
  const hasSelection = $derived(books.selectedFiles.length > 0);
  const unit = $derived(hasSelection ? "file" : "book");
  const epubCount = $derived(targetFiles.filter((f) => f.kind === "epub").length);
  const editedCount = $derived(targetFiles.filter((f) => edits.hasEdits(f.id)).length);
  const busy = $derived(jobs.active || starting);

  // -- Fit mode ---------------------------------------------------------------

  const canRun = $derived(targetCount > 0 && !busy);
  const canCheck = $derived(epubCount > 0 && !busy);
  const runLabel = $derived(
    `${settings.dryRun ? "Preview" : "Tailor"} ${targetCount} ${unit}${targetCount === 1 ? "" : "s"}`,
  );
  const editHint = $derived(
    editedCount > 0 && !settings.dryRun
      ? `writes ${editedCount} edited ${unit}${editedCount === 1 ? "'s" : "s'"} metadata into the copies`
      : "",
  );

  // -- Edit mode --------------------------------------------------------------

  // In-place work is epub-only: Markdown files have no archive to rewrite.
  const saveTargets = $derived(targetFiles.filter((f) => f.kind === "epub" && edits.hasEdits(f.id)));
  const cleanupTargets = $derived(targetFiles.filter((f) => f.kind === "epub" && needsCleanup(f)));
  const canSave = $derived(saveTargets.length > 0 && !busy);
  const canCleanup = $derived(cleanupTargets.length > 0 && !busy);
  const saveLabel = $derived(
    saveTargets.length > 0 ? `Save changes (${saveTargets.length})` : "Save changes",
  );
  const cleanupLabel = $derived(
    cleanupTargets.length > 0 ? `Clean up (${cleanupTargets.length})` : "Clean up",
  );

  function check() {
    // A copy of the derived list, so a selection change mid-run cannot reshape
    // the batch under the job store's feet.
    const items = [...targetFiles];
    if (items.length === 0) return;
    jobs.runCheck(books.refsFor(items), profiles.activeProfileSpecs());
  }

  // Planning talks to the outside world - the CLI for a composed profile's
  // appendix, the OS for what already sits on disk - so it can fail, and a
  // Tailor click that quietly does nothing is the worst way to say so.
  async function tailor() {
    edits.flushPending();
    const items = [...targetFiles];
    if (items.length === 0) return;
    starting = true;
    planError = null;
    notice = null;
    try {
      const appendix = await profiles.activeAppendix();
      const opts: RunOptions = {
        profiles: profiles.activeProfileSpecs(),
        quality: settings.quality,
        tables: settings.tables,
        dryRun: settings.dryRun,
        splitLevel: settings.mdSplitLevel,
      };
      const planned = items.map((f) => ({ input: f.path, kind: f.kind, template: toTemplateFile(f) }));
      const plans = await resolvePlans(planned, {
        template: settings.filenameTemplate,
        outputDir: settings.outputDir,
        appendix,
      });

      // The produced copies get tracked under their books with this badge.
      // With user profile layers the CLI resolves the composed name into the
      // file's own stamp; the selected built-in is the label we can offer
      // without another CLI round-trip.
      const fitMeta = { profileName: settings.profile, appendix };
      jobs.runFit(books.refsFor(items), plans, opts, edits.snapshotFor(items.map((f) => f.id)), fitMeta);
    } catch (err) {
      planError = `Nothing was started: we could not work out where these books would go. ${String(err)}`;
    } finally {
      starting = false;
    }
  }

  /**
   * Edit mode's write path: the shared in-place flow (Trash backup, then a
   * repair-profile rewrite - see stores/inplace.ts), with this bar's error
   * and notice lines wrapped around it.
   */
  async function saveInPlace(items: BookFile[], withEdits: boolean) {
    if (items.length === 0) return;
    starting = true;
    planError = null;
    notice = null;
    try {
      const outcome = await saveFilesInPlace(items, withEdits);
      if (outcome.failures.length > 0) {
        const scope = outcome.ran === 0 ? "Nothing was written" : "Some files were skipped";
        planError = `${scope} - a safety copy could not be made. ${outcome.failures[0]}`;
      }
      if (outcome.keptAsFile > 0) {
        notice = "Backup kept next to the original - this volume has no Trash.";
      }
    } finally {
      starting = false;
    }
  }
</script>

<div
  class="flex items-center justify-between gap-4 border-t border-ink-200 bg-white/80 px-5 py-3 backdrop-blur dark:border-ink-800 dark:bg-ink-900/80"
>
  <div class="min-w-0 text-[13px] text-ink-500 dark:text-ink-400">
    <span class="font-medium text-ink-700 dark:text-ink-200">
      {books.books.length} {books.books.length === 1 ? "book" : "books"}
    </span>
    {#if books.selectedFiles.length > 0}
      <span>
        · {books.selectedFiles.length}
        {books.selectedFiles.length === 1 ? "file" : "files"} selected
      </span>
    {/if}
  </div>

  {#if jobs.active}
    <div class="flex flex-1 items-center justify-end gap-3">
      <div class="flex min-w-0 flex-1 items-center gap-3">
        <div class="h-1.5 w-full max-w-56 overflow-hidden rounded-full bg-ink-200 dark:bg-ink-700">
          <div
            class="h-full rounded-full bg-teal-500 dark:bg-teal-400 dark:shadow-glow-sm transition-[width] duration-300"
            style="width: {jobs.total > 0 ? (jobs.done / jobs.total) * 100 : 0}%"
          ></div>
        </div>
        <span class="shrink-0 text-[13px] tabular-nums text-ink-600 dark:text-ink-300">
          {jobs.done} of {jobs.total}
        </span>
      </div>
      <Button variant="secondary" onclick={() => jobs.cancelAll()}>Cancel</Button>
    </div>
  {:else if settings.mode === "edit"}
    <div class="flex items-center gap-2">
      <Button
        variant="secondary"
        disabled={!canCleanup}
        title="Repair the selected files in place (a copy goes to the Trash first)"
        onclick={() => saveInPlace([...cleanupTargets], false)}
      >
        {cleanupLabel}
      </Button>
      <div class="flex flex-col items-end gap-0.5">
        <Button variant="primary" disabled={!canSave} onclick={() => saveInPlace([...saveTargets], true)}>
          {saveLabel}
        </Button>
        {#if planError}
          <span class="max-w-80 text-right text-[10px] leading-snug text-rose-600 dark:text-rose-400">
            {planError}
          </span>
        {:else if notice}
          <span class="max-w-80 text-right text-[10px] leading-snug text-ink-400 dark:text-ink-500">
            {notice}
          </span>
        {/if}
      </div>
    </div>
  {:else}
    <div class="flex items-center gap-2">
      <Button variant="secondary" disabled={!canCheck} onclick={check}>Check</Button>
      <div class="flex flex-col items-end gap-0.5">
        <Button variant="primary" disabled={!canRun} onclick={tailor}>{runLabel}</Button>
        {#if planError}
          <span class="max-w-80 text-right text-[10px] leading-snug text-rose-600 dark:text-rose-400">
            {planError}
          </span>
        {:else if editHint}
          <span class="text-[10px] leading-none text-ink-400 dark:text-ink-500">{editHint}</span>
        {/if}
      </div>
    </div>
  {/if}
</div>
