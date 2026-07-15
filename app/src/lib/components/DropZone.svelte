<script lang="ts">
  import { fade } from "svelte/transition";
</script>

<!-- The full-window overlay shown while files are dragged over the app. -->
<div
  transition:fade={{ duration: 120 }}
  class="fixed inset-0 z-50 flex items-stretch justify-stretch bg-ink-950/30 p-6 backdrop-blur-[2px]"
>
  <div
    class="flex w-full flex-col items-center justify-center gap-4 rounded-3xl border-2 border-dashed border-teal-400 bg-white/80 dark:border-teal-400/70 dark:bg-ink-900/80 dark:shadow-glow"
  >
    <svg
      class="h-12 w-12 text-teal-500 dark:text-teal-400"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
    >
      <path d="M12 16V4m0 0L8 8m4-4l4 4" stroke-linecap="round" stroke-linejoin="round" />
      <path d="M4 15v3a2 2 0 002 2h12a2 2 0 002-2v-3" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
    <p class="text-lg font-medium text-ink-700 dark:text-ink-100">Drop them right here</p>
  </div>
</div>
