<script lang="ts">
  import { flip } from "svelte/animate";
  import { scale } from "svelte/transition";
  import { books } from "../stores/books.svelte";
  import BookCard from "./BookCard.svelte";

  // A click that lands on the grid itself - the gaps between the cards, the
  // space below them - and not on a card, is a click on nothing: deselect.
  // Cards stop nothing; the check is simply whether the event started here.
  function onBackgroundClick(event: MouseEvent) {
    if (event.target === event.currentTarget) books.clearSelection();
  }
</script>

<!--
  A background click target, not a control: the grid has no keyboard role of its
  own (Escape already clears the selection from anywhere, see api/keys) and
  every card in it is focusable in its own right.
-->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  onclick={onBackgroundClick}
  class="grid min-h-full content-start gap-4 p-5"
  style="grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));"
>
  {#each books.books as book (book.id)}
    <div in:scale={{ start: 0.96, duration: 160 }} animate:flip={{ duration: 180 }}>
      <BookCard {book} />
    </div>
  {/each}
</div>
