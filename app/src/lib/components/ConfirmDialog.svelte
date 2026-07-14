<script lang="ts">
  import type { Snippet } from "svelte";
  import { fade, scale } from "svelte/transition";
  import Button from "./ui/Button.svelte";

  let {
    title,
    confirmLabel,
    cancelLabel,
    confirmVariant = "primary",
    onConfirm,
    onCancel,
    children,
  }: {
    title: string;
    confirmLabel: string;
    cancelLabel: string;
    confirmVariant?: "primary" | "danger";
    onConfirm: () => void;
    onCancel: () => void;
    children: Snippet;
  } = $props();

  function onKey(event: KeyboardEvent) {
    if (event.key === "Escape") onCancel();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="fixed inset-0 z-[60] flex items-center justify-center p-6">
  <div
    role="presentation"
    transition:fade={{ duration: 120 }}
    class="absolute inset-0 bg-zinc-950/45 backdrop-blur-[2px]"
    onclick={onCancel}
  ></div>

  <div
    role="dialog"
    aria-modal="true"
    aria-label={title}
    transition:scale={{ start: 0.96, duration: 140 }}
    class="relative w-full max-w-sm rounded-2xl border border-zinc-200 bg-white p-5 shadow-xl dark:border-zinc-800 dark:bg-zinc-900"
  >
    <h2 class="text-base font-semibold text-zinc-900 dark:text-zinc-100">{title}</h2>
    <div class="mt-2 text-[13px] leading-relaxed text-zinc-600 dark:text-zinc-300">
      {@render children()}
    </div>
    <div class="mt-5 flex justify-end gap-2">
      <Button variant="secondary" onclick={onCancel}>{cancelLabel}</Button>
      <Button variant={confirmVariant} onclick={onConfirm}>{confirmLabel}</Button>
    </div>
  </div>
</div>
