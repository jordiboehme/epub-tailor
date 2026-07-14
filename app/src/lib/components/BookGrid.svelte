<script lang="ts">
  import { flip } from "svelte/animate";
  import { scale } from "svelte/transition";
  import { books } from "../stores/books.svelte";
  import BookCard from "./BookCard.svelte";
</script>

<div
  class="grid gap-4 p-5"
  style="grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));"
>
  {#each books.books as book (book.id)}
    <div in:scale={{ start: 0.96, duration: 160 }} animate:flip={{ duration: 180 }}>
      <BookCard {book} />
    </div>
  {/each}
</div>
