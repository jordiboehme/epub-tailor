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
  }: {
    findings?: Finding[];
    failure?: Failure;
  } = $props();

  let showStderr = $state(false);

  const dot: Record<Finding["severity"], string> = {
    error: "bg-rose-500",
    warning: "bg-amber-500",
    info: "bg-zinc-400",
  };

  const stderr = $derived((failure?.stderr ?? []).filter((line) => line.trim().length > 0));
</script>

<div class="border-t border-zinc-200 px-2.5 py-2 dark:border-zinc-800">
  {#if failure}
    <p class="text-[11px] leading-snug text-zinc-600 dark:text-zinc-300">{failure.friendly}</p>
    <div class="mt-1.5 flex items-center gap-2">
      <span class="rounded bg-zinc-100 px-1 py-0.5 font-mono text-[10px] text-zinc-600 dark:bg-zinc-800 dark:text-zinc-400">
        {failure.code}
      </span>
      {#if stderr.length > 0}
        <button
          type="button"
          onclick={(e) => {
            e.stopPropagation();
            showStderr = !showStderr;
          }}
          class="text-[10px] font-medium text-zinc-500 hover:text-zinc-700 dark:text-zinc-400 dark:hover:text-zinc-200"
        >
          {showStderr ? "Hide what it said" : "What it said"}
        </button>
      {/if}
    </div>

    {#if showStderr && stderr.length > 0}
      <pre
        class="mt-1.5 max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-md bg-zinc-100 p-1.5 font-mono text-[10px] leading-snug text-zinc-600 dark:bg-zinc-950 dark:text-zinc-400">{stderr.join(
          "\n",
        )}</pre>
    {/if}
  {:else if findings}
    {#if findings.length === 0}
      <p class="text-[11px] text-zinc-500 dark:text-zinc-400">Nothing to report.</p>
    {:else}
      <ul class="flex max-h-40 flex-col gap-1.5 overflow-y-auto">
        {#each findings as finding (finding.code + finding.message)}
          <li class="flex items-start gap-1.5 text-[11px] leading-snug">
            <span class="mt-0.5 h-1.5 w-1.5 shrink-0 rounded-full {dot[finding.severity]}"></span>
            <span class="min-w-0">
              <span class="font-mono text-zinc-500 dark:text-zinc-500">{finding.code}</span>
              <span class="text-zinc-600 dark:text-zinc-300"> {finding.message}</span>
            </span>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>
