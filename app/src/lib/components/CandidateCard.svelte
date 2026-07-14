<script lang="ts">
  // One Open Library search result. The cover comes straight off
  // covers.openlibrary.org (allowed by the app CSP); a missing or broken one
  // falls back to a neutral placeholder. The raw score is shown as a plain-word
  // match strength, never a float in the user's face.
  import type { Candidate } from "../api/contract";
  import { creatorNames } from "../api/meta";

  let { candidate, onselect }: { candidate: Candidate; onselect: () => void } = $props();

  let imgError = $state(false);

  const authors = $derived(creatorNames(candidate.metadata.authors).join(", "));
  const meta = $derived(candidate.metadata);
  const line = $derived([meta.date, meta.publisher].filter(Boolean).join(" · "));

  const strength = $derived(
    candidate.score >= 0.8 ? "Strong match" : candidate.score >= 0.5 ? "Good match" : "Loose match",
  );
  const strengthTone = $derived(
    candidate.score >= 0.8
      ? "bg-emerald-100 text-emerald-700 dark:bg-emerald-500/15 dark:text-emerald-300"
      : candidate.score >= 0.5
        ? "bg-indigo-100 text-indigo-700 dark:bg-indigo-500/15 dark:text-indigo-300"
        : "bg-zinc-200 text-zinc-600 dark:bg-zinc-700 dark:text-zinc-300",
  );
</script>

<button
  type="button"
  onclick={onselect}
  class="flex w-full items-start gap-3 rounded-xl border border-zinc-200 bg-white p-2.5 text-left transition-colors hover:border-indigo-400 hover:bg-indigo-50/40 dark:border-zinc-800 dark:bg-zinc-900 dark:hover:border-indigo-500/60 dark:hover:bg-indigo-500/5"
>
  <div class="h-16 w-11 shrink-0 overflow-hidden rounded-md bg-zinc-100 dark:bg-zinc-800">
    {#if candidate.cover_url && !imgError}
      <img
        src={candidate.cover_url}
        alt=""
        onerror={() => (imgError = true)}
        class="h-full w-full object-cover"
      />
    {:else}
      <div class="flex h-full w-full items-center justify-center text-[9px] text-zinc-400">
        no cover
      </div>
    {/if}
  </div>

  <div class="min-w-0 flex-1">
    <p class="truncate text-[13px] font-medium text-zinc-800 dark:text-zinc-100">
      {meta.title ?? "Untitled"}
    </p>
    {#if authors}
      <p class="truncate text-[12px] text-zinc-600 dark:text-zinc-300">{authors}</p>
    {/if}
    {#if line}
      <p class="truncate text-[11px] text-zinc-500 dark:text-zinc-400">{line}</p>
    {/if}
    <span class="mt-1 inline-block rounded px-1.5 py-0.5 text-[10px] font-medium {strengthTone}">
      {strength}
    </span>
  </div>
</button>
