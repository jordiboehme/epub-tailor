<script lang="ts">
  // The Inspector's Metadata section. One selected book: fields prefilled from
  // staged edits over the book's own, edits stage live with a per-field revert.
  // Many selected: each field shows the shared value or "Mixed", and an
  // "Apply to N" commits it to the whole selection. Plus the two escape hatches -
  // "Find online" (single book) and "Write metadata only" (epub books with edits).

  import { open } from "@tauri-apps/plugin-dialog";
  import { cacheCover, coverUrl } from "../api/covers";
  import type { RunOptions } from "../api/argv";
  import { resolvePlans } from "../api/plan";
  import { books, toTemplateBook } from "../stores/books.svelte";
  import type { Book } from "../stores/books.svelte";
  import { edits } from "../stores/edits.svelte";
  import type { StagedEdits } from "../api/edits";
  import { jobs } from "../stores/jobs.svelte";
  import { profiles } from "../stores/profiles.svelte";
  import { settings } from "../stores/settings.svelte";
  import MetadataField from "./MetadataField.svelte";
  import MetadataSearchDialog from "./MetadataSearchDialog.svelte";
  import Button from "./ui/Button.svelte";

  type TextKey = "title" | "series" | "seriesIndex" | "publisher" | "language" | "date" | "isbn";
  type ListKey = "authors" | "subjects";

  const selected = $derived(books.selected);
  const single = $derived(selected.length === 1 ? selected[0] : null);
  const isMulti = $derived(selected.length > 1);
  const ids = $derived(selected.map((b) => b.id));

  let searchOpen = $state(false);
  let writing = $state(false);
  let coverError = $state(false);
  let coverFailed = $state<string | null>(null);
  let writeFailed = $state<string | null>(null);

  // Debounced live staging in single-book mode, one timer per field so fast
  // typing in one box never delays another. Each pending timer keeps its own
  // apply function too, so flushPending() can settle it immediately instead
  // of losing it - to a Tailor/write click that reads the edits store before
  // the timer fires, or to the same field key being reused for a different
  // book before its 200ms is up.
  const timers = new Map<string, { timer: ReturnType<typeof setTimeout>; apply: () => void }>();
  function debounce(key: string, fn: () => void) {
    const prev = timers.get(key);
    if (prev) clearTimeout(prev.timer);
    timers.set(key, {
      apply: fn,
      timer: setTimeout(() => {
        timers.delete(key);
        fn();
      }, 200),
    });
  }

  /** Run and clear every pending debounced stage right now, in field order. */
  function flushPending(): void {
    for (const { timer, apply } of timers.values()) {
      clearTimeout(timer);
      apply();
    }
    timers.clear();
  }

  // Let Tailor (in ActionBar) and "write metadata only" (below) settle a
  // pending keystroke before either reads a snapshot of the edits store. Also
  // flush on every selection change, so a pending edit for the book being
  // left never gets cancelled by the same field key being reused on the book
  // being switched to.
  $effect(() => edits.onFlush(flushPending));
  $effect(() => {
    void ids;
    flushPending();
  });

  function parseLines(value: string): string[] {
    return value.split("\n").map((line) => line.trim()).filter(Boolean);
  }

  function metaText(book: Book, key: TextKey): string {
    return book.meta?.[key] ?? "";
  }
  function metaList(book: Book, key: ListKey): string[] {
    return book.meta?.[key] ?? [];
  }

  // -- single-book field props ------------------------------------------------

  function textProps(book: Book, key: TextKey) {
    const staged = edits.get(book.id)?.[key];
    const bookValue = metaText(book, key);
    return {
      value: staged ?? bookValue,
      bookValue,
      dirty: staged !== undefined,
      oninput: (v: string) => debounce(key, () => edits.stage([book.id], { [key]: v })),
      onrevert: () => edits.unstage(book.id, key),
    };
  }

  function listProps(book: Book, key: ListKey) {
    const staged = edits.get(book.id)?.[key];
    const bookValue = metaList(book, key).join("\n");
    return {
      multiline: true,
      value: (staged ?? metaList(book, key)).join("\n"),
      bookValue,
      dirty: staged !== undefined,
      oninput: (v: string) => debounce(key, () => edits.stage([book.id], { [key]: parseLines(v) })),
      onrevert: () => edits.unstage(book.id, key),
    };
  }

  // -- multi-book field props -------------------------------------------------

  function commonText(key: TextKey): { value: string; mixed: boolean } {
    const values = selected.map((b) => edits.get(b.id)?.[key] ?? metaText(b, key));
    const first = values[0] ?? "";
    const mixed = values.some((v) => v !== first);
    return { value: mixed ? "" : first, mixed };
  }

  function commonList(key: ListKey): { value: string; mixed: boolean } {
    const values = selected.map((b) => (edits.get(b.id)?.[key] ?? metaList(b, key)).join("\n"));
    const first = values[0] ?? "";
    const mixed = values.some((v) => v !== first);
    return { value: mixed ? "" : first, mixed };
  }

  function multiText(key: TextKey) {
    const { value, mixed } = commonText(key);
    return {
      multi: true,
      value,
      mixed,
      applyCount: selected.length,
      onapply: (v: string) => edits.stage(ids, { [key]: v }),
    };
  }

  function multiList(key: ListKey) {
    const { value, mixed } = commonList(key);
    return {
      multi: true,
      multiline: true,
      value,
      mixed,
      applyCount: selected.length,
      onapply: (v: string) => edits.stage(ids, { [key]: parseLines(v) }),
    };
  }

  // Description is a plain text field that happens to be multiline.
  function descSingle(book: Book) {
    const staged = edits.get(book.id)?.description;
    const bookValue = book.meta?.description ?? "";
    return {
      multiline: true,
      value: staged ?? bookValue,
      bookValue,
      dirty: staged !== undefined,
      oninput: (v: string) =>
        debounce("description", () => edits.stage([book.id], { description: v })),
      onrevert: () => edits.unstage(book.id, "description"),
    };
  }

  function descMulti() {
    const values = selected.map((b) => edits.get(b.id)?.description ?? b.meta?.description ?? "");
    const first = values[0] ?? "";
    const mixed = values.some((v) => v !== first);
    return {
      multi: true,
      multiline: true,
      value: mixed ? "" : first,
      mixed,
      applyCount: selected.length,
      onapply: (v: string) => edits.stage(ids, { description: v }),
    };
  }

  // -- cover (single book only) -----------------------------------------------

  const coverStaged = $derived(single ? edits.get(single.id)?.coverPath : undefined);
  const coverShown = $derived(single ? (coverStaged ?? single.coverPath) : undefined);

  $effect(() => {
    // Reset the load-error flag whenever the shown cover changes.
    void coverShown;
    coverError = false;
  });

  async function chooseCover() {
    if (!single) return;
    const selection = await open({
      multiple: false,
      filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "gif", "webp"] }],
    });
    if (typeof selection !== "string") return;
    const book = single;
    coverFailed = null;
    try {
      // Staged as a copy in the cover cache, not as the path the user picked:
      // that is the only place the webview is allowed to load an image from,
      // so it is the only path that can both preview here and ride along as
      // the `--cover` flag. See api/covers.ts.
      edits.stage([book.id], { coverPath: await cacheCover(selection) });
    } catch (err) {
      coverFailed = `That image could not be read. ${String(err)}`;
    }
  }

  // -- write metadata only ----------------------------------------------------

  // Metadata is written under the plain `epub` profile - a repair, not a device
  // conversion - so both the run and the name it may have to fall back on come
  // from that profile and not from whatever device is selected. With `x4`
  // active, an appendix taken from the picker would write "Book.x4.epub": a
  // filename promising a conversion the file did not get.
  const WRITE_PROFILE = "epub";

  const epubEditable = $derived(selected.filter((b) => b.kind === "epub" && edits.hasEdits(b.id)));
  const mdOnlySelection = $derived(selected.length > 0 && selected.every((b) => b.kind === "md"));
  const canWrite = $derived(epubEditable.length > 0 && !writing && !jobs.active);

  async function writeMetadataOnly() {
    edits.flushPending();
    const items = epubEditable;
    if (items.length === 0) return;
    writing = true;
    writeFailed = null;
    try {
      const appendix = profiles.builtinAppendix(WRITE_PROFILE);
      const opts: RunOptions = {
        profiles: [WRITE_PROFILE],
        quality: null,
        tables: null,
        dryRun: false,
      };
      const planned = items.map((b) => ({ input: b.path, kind: b.kind, template: toTemplateBook(b) }));
      const plans = await resolvePlans(planned, {
        template: settings.filenameTemplate,
        outputDir: settings.outputDir,
        inPlace: settings.inPlace,
        appendix,
      });
      jobs.runFit(items, plans, opts, edits.snapshotFor(items.map((b) => b.id)));
    } catch (err) {
      // Planning asks the OS what already sits on disk; a rejection there used
      // to leave the button unpressed-looking and the edits unwritten, silently.
      writeFailed = `Nothing was written: we could not work out where these books would go. ${String(err)}`;
    } finally {
      writing = false;
    }
  }

  const writeLabel = $derived(
    epubEditable.length > 0 ? `Write metadata (${epubEditable.length})` : "Write metadata",
  );
</script>

{#if selected.length === 0}
  <p class="text-[12px] leading-snug text-zinc-500 dark:text-zinc-400">
    Select a book to edit its metadata, or fetch it from Open Library.
  </p>
{:else}
  {#key single ? single.id : `multi:${ids.join(",")}`}
    {#if isMulti}
      <p class="mb-2.5 text-[11px] text-zinc-500 dark:text-zinc-400">
        Editing {selected.length} books - set a field once, apply it to all.
      </p>
    {/if}

    <div class="flex flex-col gap-2.5">
      <MetadataField
        label="Title"
        placeholder="Book title"
        {...single ? textProps(single, "title") : multiText("title")}
      />
      <MetadataField
        label="Authors"
        placeholder="One per line"
        {...single ? listProps(single, "authors") : multiList("authors")}
      />

      <div class="grid grid-cols-[1fr_5.5rem] gap-2">
        <MetadataField
          label="Series"
          placeholder="Series name"
          {...single ? textProps(single, "series") : multiText("series")}
        />
        <MetadataField
          label="Index"
          placeholder="#"
          {...single ? textProps(single, "seriesIndex") : multiText("seriesIndex")}
        />
      </div>

      <MetadataField
        label="Publisher"
        placeholder="Publisher"
        {...single ? textProps(single, "publisher") : multiText("publisher")}
      />

      <div class="grid grid-cols-2 gap-2">
        <MetadataField
          label="Language"
          placeholder="en"
          {...single ? textProps(single, "language") : multiText("language")}
        />
        <MetadataField
          label="Date"
          placeholder="1937"
          {...single ? textProps(single, "date") : multiText("date")}
        />
      </div>

      <MetadataField
        label="ISBN"
        placeholder="978..."
        {...single ? textProps(single, "isbn") : multiText("isbn")}
      />
      <MetadataField
        label="Subjects"
        placeholder="One per line"
        {...single ? listProps(single, "subjects") : multiList("subjects")}
      />
      <MetadataField
        label="Description"
        placeholder="Back-cover blurb"
        {...single ? descSingle(single) : descMulti()}
      />

      {#if single}
        <div class="flex flex-col gap-1.5">
          <span class="text-[11px] font-medium text-zinc-500 dark:text-zinc-400">Cover</span>
          <div class="flex items-start gap-2.5">
            <div
              class="h-20 w-14 shrink-0 overflow-hidden rounded-md border border-zinc-200 bg-zinc-100 dark:border-zinc-700 dark:bg-zinc-800"
            >
              {#if coverShown && !coverError}
                <img
                  src={coverUrl(coverShown)}
                  alt="Cover"
                  onerror={() => (coverError = true)}
                  class="h-full w-full object-cover"
                />
              {:else}
                <div class="flex h-full w-full items-center justify-center text-[10px] text-zinc-400">
                  {coverError ? "no preview" : "none"}
                </div>
              {/if}
            </div>
            <div class="flex min-w-0 flex-col gap-1.5">
              <Button variant="secondary" size="sm" onclick={chooseCover}>Choose image...</Button>
              {#if coverStaged}
                <button
                  type="button"
                  onclick={() => single && edits.unstage(single.id, "coverPath")}
                  class="text-left text-[10px] font-medium text-zinc-400 hover:text-indigo-500 dark:text-zinc-500 dark:hover:text-indigo-400"
                >
                  Revert cover
                </button>
              {/if}
            </div>
          </div>
          {#if coverFailed}
            <p class="text-[11px] leading-snug text-rose-600 dark:text-rose-400">{coverFailed}</p>
          {/if}
        </div>
      {/if}
    </div>

    <div class="mt-3 flex flex-col gap-2">
      <Button
        variant="secondary"
        size="sm"
        disabled={selected.length !== 1}
        title={selected.length !== 1 ? "Select one book to look it up" : undefined}
        onclick={() => (searchOpen = true)}
      >
        <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
          <circle cx="9" cy="9" r="5.25" />
          <path d="M13 13l3.5 3.5" stroke-linecap="round" />
        </svg>
        Find online...
      </Button>

      <Button variant="primary" size="sm" disabled={!canWrite} onclick={writeMetadataOnly}>
        {writeLabel}
      </Button>

      {#if writeFailed}
        <p class="text-[11px] leading-snug text-rose-600 dark:text-rose-400">{writeFailed}</p>
      {/if}

      {#if mdOnlySelection}
        <p class="text-[11px] leading-snug text-zinc-500 dark:text-zinc-400">
          Markdown books get their metadata when you Tailor them - there is nothing to write in place.
        </p>
      {:else if epubEditable.length === 0}
        <p class="text-[11px] leading-snug text-zinc-500 dark:text-zinc-400">
          Edit a field, or fetch a record, and this writes it without a device conversion.
        </p>
      {/if}
    </div>
  {/key}
{/if}

{#if searchOpen && single}
  <MetadataSearchDialog book={single} onclose={() => (searchOpen = false)} />
{/if}
