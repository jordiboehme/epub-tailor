<script lang="ts">
  // The drawer under a card: what the check found, or why the thing failed.
  // Lives apart from BookCard because a card should be a card, and because a
  // failure worth showing at all is worth showing properly - the friendly
  // line, the CLI's own error code, and the last of what it said on stderr.
  import type { Finding } from "../api/contract";
  import type { Failure } from "../api/book-view";

  let {
    findings,
    failure,
    padding = "px-2.5 py-2",
  }: {
    findings?: Finding[];
    failure?: Failure;
    /**
     * The drawer's own padding. The default is tuned for a 150px card; a list
     * row overrides it to indent the text under the row's title column, so the
     * drawer reads as part of the row rather than as a stray block under it.
     */
    padding?: string;
  } = $props();

  let showStderr = $state(false);

  const dot: Record<Finding["severity"], string> = {
    error: "bg-rose-500",
    warning: "bg-amber-500",
    info: "bg-ink-400",
  };

  const stderr = $derived((failure?.stderr ?? []).filter((line) => line.trim().length > 0));
</script>

<div class="border-t border-ink-200 {padding} dark:border-ink-800">
  {#if failure}
    <p class="text-[11px] leading-snug text-ink-600 dark:text-ink-300">{failure.friendly}</p>
    <div class="mt-1.5 flex items-center gap-2">
      <span class="rounded bg-ink-100 px-1 py-0.5 font-mono text-[10px] text-ink-600 dark:bg-ink-800 dark:text-ink-400">
        {failure.code}
      </span>
      {#if stderr.length > 0}
        <button
          type="button"
          onclick={(e) => {
            e.stopPropagation();
            showStderr = !showStderr;
          }}
          class="text-[10px] font-medium text-ink-500 hover:text-ink-700 dark:text-ink-400 dark:hover:text-ink-200"
        >
          {showStderr ? "Hide what it said" : "What it said"}
        </button>
      {/if}
    </div>

    {#if showStderr && stderr.length > 0}
      <pre
        class="mt-1.5 max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-md bg-ink-100 p-1.5 font-mono text-[10px] leading-snug text-ink-600 dark:bg-ink-950 dark:text-ink-400">{stderr.join(
          "\n",
        )}</pre>
    {/if}
  {:else if findings}
    {#if findings.length === 0}
      <p class="text-[11px] text-ink-500 dark:text-ink-400">Nothing to report.</p>
    {:else}
      <ul class="flex max-h-40 flex-col gap-1.5 overflow-y-auto">
        {#each findings as finding (finding.code + finding.message)}
          <li class="flex items-start gap-1.5 text-[11px] leading-snug">
            <span class="mt-0.5 h-1.5 w-1.5 shrink-0 rounded-full {dot[finding.severity]}"></span>
            <span class="min-w-0">
              {#if finding.code}
                <span class="font-mono text-ink-500 dark:text-ink-500">{finding.code}</span>
              {/if}
              <span class="text-ink-600 dark:text-ink-300"> {finding.message}</span>
            </span>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>
