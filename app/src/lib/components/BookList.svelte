<script lang="ts">
  import { flip } from "svelte/animate";
  import { fade } from "svelte/transition";
  import { books } from "../stores/books.svelte";
  import BookRow from "./BookRow.svelte";

  // A click that lands on the list itself - the space below the last row - and
  // not on a row, is a click on nothing: deselect. Rows stop nothing; the check
  // is simply whether the event started here. Same contract as BookGrid.
  function onBackgroundClick(event: MouseEvent) {
    if (event.target === event.currentTarget) books.clearSelection();
  }
</script>

<!--
  A background click target, not a control: the list has no keyboard role of its
  own (Escape already clears the selection from anywhere, see api/keys) and
  every row in it is focusable in its own right.
-->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  onclick={onBackgroundClick}
  class="flex min-h-full flex-col divide-y divide-zinc-200 dark:divide-zinc-800"
>
  {#each books.books as book (book.id)}
    <!-- The rows keep the store's order, which is what makes a shift-click
         range (books.range walks that same array) select what it looks like
         it selects. A fade suits a row better than the grid's pop-in scale. -->
    <div in:fade={{ duration: 140 }} animate:flip={{ duration: 180 }}>
      <BookRow {book} />
    </div>
  {/each}
</div>
