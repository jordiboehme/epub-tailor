<script lang="ts">
  // The files behind one book card: the original itself, then every tracked
  // copy. Always visible - a card reads as "this book, these files" - and
  // each row is a selectable target in its own right (see FileRow).
  import type { Book } from "../stores/books.svelte";
  import FileRow from "./FileRow.svelte";

  let { book }: { book: Book } = $props();
</script>

<div class="flex flex-col gap-0.5">
  {#each book.files as file (file.id)}
    <FileRow {book} {file} />
  {/each}
</div>
