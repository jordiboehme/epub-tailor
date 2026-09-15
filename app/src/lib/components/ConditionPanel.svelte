<script lang="ts">
  // What the automatic check-on-add found across the files an action would
  // act on, and what to do about it.
  //
  // Lives above the mode panel rather than inside either one, because the
  // question "does this book need anything?" is not a mode: the app opens in
  // Fit, and until this existed a user could see a "watermarked" pill with no
  // way to act on it without first discovering Edit mode. The repairs are
  // in-place rewrites either way - Fit mode's own controls are about producing
  // a copy, which is a different thing entirely.
  import { books } from "../stores/books.svelte";
  import { jobs } from "../stores/jobs.svelte";
  import { saveFilesInPlace } from "../stores/inplace";
  import { CLEANUP_PROFILE } from "../api/argv";
  import { conditionActions, conditionSummary, fileCondition, repairProfiles } from "../api/book-view";
  import Button from "./ui/Button.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";

  const targets = $derived(books.targets.filter((f) => f.kind === "epub"));
  const summary = $derived(conditionSummary(targets));
  const checking = $derived(targets.some((f) => f.check === "pending"));

  // Keyed off the actions, not the verdict: an extra-files-only book is not
  // "needing attention", but it still has a button worth showing.
  const fixTargets = $derived(targets.filter((f) => conditionActions(f).includes("cleanup")));
  const markTargets = $derived(targets.filter((f) => conditionActions(f).includes("watermarks")));
  // A dead end, stated as one: nothing removes DRM, so there is no button.
  const blocked = $derived(
    targets.filter((f) => fileCondition(f).concerns.includes("blocked")).length,
  );

  const busy = $derived(jobs.active);
  let confirmFix = $state(false);
  let confirmMarks = $state(false);
  let failure = $state<string | null>(null);

  async function run(files: typeof targets, profileSpecs: string[]) {
    confirmFix = false;
    confirmMarks = false;
    failure = null;
    const outcome = await saveFilesInPlace([...files], false, profileSpecs);
    if (outcome.failures.length > 0) {
      failure = `Nothing was written - a safety copy could not be made. ${outcome.failures[0]}`;
    }
  }
</script>

{#if targets.length > 0 && !checking && (summary.attention > 0 || summary.broken > 0 || fixTargets.length > 0 || markTargets.length > 0 || blocked > 0)}
  <section class="border-b border-ink-200 px-4 py-4 dark:border-ink-800">
    <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wide text-ink-400">Condition</h3>

    <p class="text-[12px] leading-snug text-ink-600 dark:text-ink-300">
      {#if summary.broken > 0}
        <span class="font-medium text-rose-700 dark:text-rose-400"
          >{summary.broken} of {targets.length} defective</span
        >{#if summary.attention > 0}, {summary.attention} needing attention{/if}.
      {:else if summary.attention > 0}
        <span class="font-medium text-amber-700 dark:text-amber-400"
          >{summary.attention} of {targets.length}
          {summary.attention === 1 ? "needs" : "need"} attention</span
        >.
      {:else if markTargets.length > 0}
        {markTargets.length} of {targets.length}
        {markTargets.length === 1 ? "carries" : "carry"} files nothing references.
      {/if}
    </p>

    <div class="mt-2.5 flex flex-wrap items-center gap-2">
      {#if fixTargets.length > 0}
        <Button variant="secondary" disabled={busy} onclick={() => (confirmFix = true)}>
          Clean up ({fixTargets.length})
        </Button>
      {/if}
      {#if markTargets.length > 0}
        <Button variant="secondary" disabled={busy} onclick={() => (confirmMarks = true)}>
          Remove watermarks ({markTargets.length})
        </Button>
      {/if}
    </div>

    {#if blocked > 0}
      <p class="mt-2 text-[11px] leading-snug text-ink-500 dark:text-ink-400">
        {blocked}
        {blocked === 1 ? "book is" : "books are"} copy protected. We cannot open
        {blocked === 1 ? "it" : "them"}, and nothing here will change that.
      </p>
    {/if}

    {#if failure}
      <p class="mt-2 text-[11px] leading-snug text-rose-600 dark:text-rose-400">{failure}</p>
    {/if}
  </section>
{/if}

{#if confirmFix}
  <ConfirmDialog
    title="Clean up {fixTargets.length} {fixTargets.length === 1 ? 'file' : 'files'}?"
    confirmLabel="Clean up"
    cancelLabel="Not now"
    onConfirm={() => run(fixTargets, repairProfiles("cleanup"))}
    onCancel={() => (confirmFix = false)}
  >
    This repairs the files' structure in place, under the {CLEANUP_PROFILE} profile. The current
    versions go to the Trash first, so nothing is lost.
  </ConfirmDialog>
{/if}

{#if confirmMarks}
  <ConfirmDialog
    title="Remove watermarks from {markTargets.length} {markTargets.length === 1
      ? 'file'
      : 'files'}?"
    confirmLabel="Remove watermarks"
    cancelLabel="Not now"
    onConfirm={() => run(markTargets, repairProfiles("watermarks"))}
    onCancel={() => (confirmMarks = false)}
  >
    This strips per-copy identifiers, invisible fingerprint characters, image metadata and files
    nothing references. The current versions go to the Trash first, so nothing is lost.
    <br />
    It also replaces each book's unique identifier, so your reading position and bookmarks in those
    books are lost.
  </ConfirmDialog>
{/if}
