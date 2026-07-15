<script lang="ts">
  import type { Snippet } from "svelte";

  type Variant = "primary" | "secondary" | "ghost" | "danger";
  type Size = "sm" | "md";

  let {
    variant = "secondary",
    size = "md",
    type = "button",
    disabled = false,
    title,
    onclick,
    children,
  }: {
    variant?: Variant;
    size?: Size;
    type?: "button" | "submit";
    disabled?: boolean;
    title?: string;
    onclick?: (event: MouseEvent) => void;
    children: Snippet;
  } = $props();

  const base =
    "inline-flex items-center justify-center gap-1.5 rounded-lg font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-teal-500 dark:focus-visible:ring-teal-400 focus-visible:ring-offset-1 focus-visible:ring-offset-ink-50 dark:focus-visible:ring-offset-ink-950 disabled:cursor-not-allowed disabled:opacity-50";

  const sizes: Record<Size, string> = {
    sm: "px-2.5 py-1 text-[13px]",
    md: "px-3.5 py-2 text-sm",
  };

  const variants: Record<Variant, string> = {
    primary:
      "bg-teal-700 text-white hover:bg-teal-600 active:bg-teal-800 shadow-sm dark:bg-teal-400 dark:text-teal-950 dark:hover:bg-teal-300 dark:active:bg-teal-500 dark:shadow-glow-sm",
    secondary:
      "border border-ink-300 bg-white text-ink-700 hover:bg-ink-50 dark:border-ink-700 dark:bg-ink-800 dark:text-ink-100 dark:hover:bg-ink-700",
    ghost:
      "text-ink-600 hover:bg-ink-200/70 dark:text-ink-300 dark:hover:bg-ink-800",
    danger: "bg-rose-600 text-white hover:bg-rose-500 active:bg-rose-700 shadow-sm",
  };
</script>

<button
  {type}
  {title}
  {disabled}
  {onclick}
  class="{base} {sizes[size]} {variants[variant]}"
>
  {@render children()}
</button>
