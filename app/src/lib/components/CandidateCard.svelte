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
        ? "bg-teal-100 text-teal-800 dark:bg-teal-500/15 dark:text-teal-300"
        : "bg-ink-200 text-ink-600 dark:bg-ink-700 dark:text-ink-300",
  );
</script>

<button
  type="button"
  onclick={onselect}
  class="flex w-full items-start gap-3 rounded-xl border border-ink-200 bg-white p-2.5 text-left transition-colors hover:border-teal-400 hover:bg-teal-50/40 dark:border-ink-800 dark:bg-ink-900 dark:hover:border-teal-500/60 dark:hover:bg-teal-500/5"
>
  <div class="h-16 w-11 shrink-0 overflow-hidden rounded-md bg-ink-100 dark:bg-ink-800">
    {#if candidate.cover_url && !imgError}
      <img
        src={candidate.cover_url}
        alt=""
        onerror={() => (imgError = true)}
        class="h-full w-full object-cover"
      />
    {:else}
      <div class="flex h-full w-full items-center justify-center text-[9px] text-ink-400">
        no cover
      </div>
    {/if}
  </div>

  <div class="min-w-0 flex-1">
    <p class="truncate text-[13px] font-medium text-ink-800 dark:text-ink-100">
      {meta.title ?? "Untitled"}
    </p>
    {#if authors}
      <p class="truncate text-[12px] text-ink-600 dark:text-ink-300">{authors}</p>
    {/if}
    {#if line}
      <p class="truncate text-[11px] text-ink-500 dark:text-ink-400">{line}</p>
    {/if}
    <span class="mt-1 inline-block rounded px-1.5 py-0.5 text-[10px] font-medium {strengthTone}">
      {strength}
    </span>
  </div>
</button>
