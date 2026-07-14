<script lang="ts">
  import { resolvePlans } from "../api/plan";
  import type { RunOptions } from "../api/argv";
  import { books, toTemplateBook } from "../stores/books.svelte";
  import { jobs } from "../stores/jobs.svelte";
  import { edits } from "../stores/edits.svelte";
  import { profiles } from "../stores/profiles.svelte";
  import { settings } from "../stores/settings.svelte";
  import Button from "./ui/Button.svelte";

  let starting = $state(false);
  let planError = $state<string | null>(null);

  // The one definition of "these books" lives in the books store; both buttons
  // and every label below read it, so they can never disagree.
  const targetBooks = $derived(books.targets);
  const targetCount = $derived(targetBooks.length);
  const epubCount = $derived(targetBooks.filter((b) => b.kind === "epub").length);
  const editedCount = $derived(targetBooks.filter((b) => edits.hasEdits(b.id)).length);
  const busy = $derived(jobs.active || starting);
  const canRun = $derived(targetCount > 0 && !busy);
  const canCheck = $derived(epubCount > 0 && !busy);
  const runLabel = $derived(
    `${settings.dryRun ? "Preview" : "Tailor"} ${targetCount} ${targetCount === 1 ? "book" : "books"}`,
  );
  const editHint = $derived(
    editedCount > 0 && !settings.dryRun
      ? `writes ${editedCount} edited ${editedCount === 1 ? "book's" : "books'"} metadata`
      : "",
  );

  function check() {
    // A copy of the derived list, so a selection change mid-run cannot reshape
    // the batch under the job store's feet.
    const items = [...targetBooks];
    if (items.length === 0) return;
    jobs.runCheck(items, profiles.activeProfileSpecs());
  }

  // Planning talks to the outside world - the CLI for a composed profile's
  // appendix, the OS for what already sits on disk - so it can fail, and a
  // Tailor click that quietly does nothing is the worst way to say so.
  async function tailor() {
    edits.flushPending();
    const items = [...targetBooks];
    if (items.length === 0) return;
    starting = true;
    planError = null;
    try {
      const appendix = await profiles.activeAppendix();
      const opts: RunOptions = {
        profiles: profiles.activeProfileSpecs(),
        quality: settings.quality,
        tables: settings.tables,
        dryRun: settings.dryRun,
        splitLevel: settings.mdSplitLevel,
      };
      const planned = items.map((b) => ({ input: b.path, kind: b.kind, template: toTemplateBook(b) }));
      const plans = await resolvePlans(planned, {
        template: settings.filenameTemplate,
        outputDir: settings.outputDir,
        inPlace: settings.inPlace,
        appendix,
      });

      jobs.runFit(items, plans, opts, edits.snapshotFor(items.map((b) => b.id)));
    } catch (err) {
      planError = `Nothing was started: we could not work out where these books would go. ${String(err)}`;
    } finally {
      starting = false;
    }
  }
</script>

<div
  class="flex items-center justify-between gap-4 border-t border-zinc-200 bg-white/80 px-5 py-3 backdrop-blur dark:border-zinc-800 dark:bg-zinc-900/80"
>
  <div class="min-w-0 text-[13px] text-zinc-500 dark:text-zinc-400">
    <span class="font-medium text-zinc-700 dark:text-zinc-200">
      {books.books.length} {books.books.length === 1 ? "book" : "books"}
    </span>
    {#if books.selected.length > 0}
      <span> · {books.selected.length} selected</span>
    {/if}
  </div>

  {#if jobs.active}
    <div class="flex flex-1 items-center justify-end gap-3">
      <div class="flex min-w-0 flex-1 items-center gap-3">
        <div class="h-1.5 w-full max-w-56 overflow-hidden rounded-full bg-zinc-200 dark:bg-zinc-700">
          <div
            class="h-full rounded-full bg-indigo-500 transition-[width] duration-300"
            style="width: {jobs.total > 0 ? (jobs.done / jobs.total) * 100 : 0}%"
          ></div>
        </div>
        <span class="shrink-0 text-[13px] tabular-nums text-zinc-600 dark:text-zinc-300">
          {jobs.done} of {jobs.total}
        </span>
      </div>
      <Button variant="secondary" onclick={() => jobs.cancelAll()}>Cancel</Button>
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
          <span class="text-[10px] leading-none text-zinc-400 dark:text-zinc-500">{editHint}</span>
        {/if}
      </div>
    </div>
  {/if}
</div>
