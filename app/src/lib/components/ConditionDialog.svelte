<script lang="ts">
  // What the automatic check found in one file, and what can be done about it.
  //
  // Opened from a row's condition chip. One dialog rather than a confirm per
  // action, because the chip is the only affordance a row has and it has to
  // answer both questions at once: "what is wrong?" and "what now?". Each
  // action carries its own consequence next to its button, so there is no
  // second confirm step - this dialog is the confirm. Presentational only:
  // the row runs the action, since it also owns the failure line under it.
  import { fade, scale } from "svelte/transition";
  import type { BookFile } from "../stores/books.svelte";
  import { CLEANUP_PROFILE, WATERMARK_PROFILE } from "../api/argv";
  import { conditionActions, fileCondition, findingsOf } from "../api/book-view";
  import type { ConditionAction } from "../api/book-view";
  import Button from "./ui/Button.svelte";
  import CardDetails from "./CardDetails.svelte";

  let {
    file,
    busy = false,
    onAction,
    onClose,
  }: {
    file: BookFile;
    /** A job is running: the actions stay visible but cannot start. */
    busy?: boolean;
    onAction: (action: ConditionAction) => void;
    onClose: () => void;
  } = $props();

  const condition = $derived(fileCondition(file));
  const actions = $derived(conditionActions(file));
  const blocked = $derived(condition.concerns.includes("blocked"));
  const notChecked = $derived(file.check === "failed");
  const findings = $derived(findingsOf(file));

  function onKey(event: KeyboardEvent) {
    if (event.key === "Escape") onClose();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="fixed inset-0 z-[60] flex items-center justify-center p-6">
  <div
    role="presentation"
    transition:fade={{ duration: 120 }}
    class="absolute inset-0 bg-ink-950/45 backdrop-blur-[2px]"
    onclick={onClose}
  ></div>

  <div
    role="dialog"
    aria-modal="true"
    aria-label="Condition of {file.fileName}"
    transition:scale={{ start: 0.96, duration: 140 }}
    class="relative w-full max-w-md rounded-2xl border border-ink-200 bg-white p-5 shadow-xl dark:border-ink-800 dark:bg-ink-900"
  >
    <h2 class="truncate text-base font-semibold text-ink-900 dark:text-ink-100" title={file.path}>
      {file.fileName}
    </h2>

    {#if notChecked}
      <p class="mt-2 text-[13px] leading-relaxed text-ink-600 dark:text-ink-300">
        {file.checkError?.friendly ?? "The automatic check did not complete."}
      </p>
    {:else}
      <div class="mt-2 rounded-lg border border-ink-200 dark:border-ink-800">
        <CardDetails {findings} />
      </div>
    {/if}

    {#if blocked}
      <p class="mt-3 text-[12px] leading-snug text-ink-500 dark:text-ink-400">
        This book is copy protected. We cannot open it, and nothing here will change that.
      </p>
    {/if}

    {#if actions.length > 0}
      <ul class="mt-4 flex flex-col gap-3">
        {#if actions.includes("cleanup")}
          <li class="flex items-start gap-3">
            <span class="shrink-0 whitespace-nowrap">
              <Button variant="secondary" disabled={busy} onclick={() => onAction("cleanup")}>
                Clean up
              </Button>
            </span>
            <p class="min-w-0 text-[12px] leading-snug text-ink-600 dark:text-ink-300">
              Repairs the file's structure in place, under the {CLEANUP_PROFILE} profile. The
              current version goes to the Trash first, so nothing is lost.
            </p>
          </li>
        {/if}
        {#if actions.includes("watermarks")}
          <li class="flex items-start gap-3">
            <span class="shrink-0 whitespace-nowrap">
              <Button variant="secondary" disabled={busy} onclick={() => onAction("watermarks")}>
                Remove watermarks
              </Button>
            </span>
            <p class="min-w-0 text-[12px] leading-snug text-ink-600 dark:text-ink-300">
              Strips per-copy identifiers, invisible fingerprint characters, image metadata and
              files nothing references, under {CLEANUP_PROFILE} + {WATERMARK_PROFILE}. The
              current version goes to the Trash first. It also replaces the book's unique
              identifier, so your reading position and bookmarks in this book are lost.
            </p>
          </li>
        {/if}
      </ul>
    {/if}

    <div class="mt-5 flex justify-end">
      <Button variant="secondary" onclick={onClose}>{actions.length > 0 ? "Not now" : "Close"}</Button>
    </div>
  </div>
</div>
