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
    "inline-flex items-center justify-center gap-1.5 rounded-lg font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 focus-visible:ring-offset-1 focus-visible:ring-offset-zinc-50 dark:focus-visible:ring-offset-zinc-950 disabled:cursor-not-allowed disabled:opacity-50";

  const sizes: Record<Size, string> = {
    sm: "px-2.5 py-1 text-[13px]",
    md: "px-3.5 py-2 text-sm",
  };

  const variants: Record<Variant, string> = {
    primary: "bg-indigo-600 text-white hover:bg-indigo-500 active:bg-indigo-700 shadow-sm",
    secondary:
      "border border-zinc-300 bg-white text-zinc-700 hover:bg-zinc-50 dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-100 dark:hover:bg-zinc-700",
    ghost:
      "text-zinc-600 hover:bg-zinc-200/70 dark:text-zinc-300 dark:hover:bg-zinc-800",
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
