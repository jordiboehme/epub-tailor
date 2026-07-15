<script lang="ts">
  // The Metadata section of Edit mode's Inspector. Every field stages live to
  // the whole selection - one file or many - after a short debounce; its
  // checkbox is a pure reflection of what is staged, and unchecking it
  // reverts the field. Writing belongs to the ActionBar's "Save changes",
  // which saves staged edits into the selected files themselves.

  import { open } from "@tauri-apps/plugin-dialog";
  import { cacheCover, coverUrl } from "../api/covers";
  import { books } from "../stores/books.svelte";
  import type { BookFile } from "../stores/books.svelte";
  import { edits } from "../stores/edits.svelte";
  import { CLEARABLE_FIELDS } from "../api/edits";
  import MetadataField from "./MetadataField.svelte";
  import MetadataSearchDialog from "./MetadataSearchDialog.svelte";
  import Button from "./ui/Button.svelte";

  const selected = $derived(books.selectedFiles);
  const single = $derived(selected.length === 1 ? selected[0] : null);
  const isMulti = $derived(selected.length > 1);
  const ids = $derived(selected.map((f) => f.id));

  let searchOpen = $state(false);
  let coverError = $state(false);
  let coverFailed = $state<string | null>(null);

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

  type FieldKey =
    | "title"
    | "authors"
    | "series"
    | "seriesIndex"
    | "publisher"
    | "description"
    | "language"
    | "date"
    | "isbn"
    | "subjects";
  const LIST_KEYS: ReadonlySet<FieldKey> = new Set<FieldKey>(["authors", "subjects"]);

  /** The file's own value for a field, flattened to the textbox's shape. */
  function bookText(file: BookFile, key: FieldKey): string {
    const own = file.meta?.[key];
    return Array.isArray(own) ? own.join("\n") : (own ?? "");
  }

  /** The staged entry for a field: undefined (none), null (a clear), or text. */
  function stagedText(file: BookFile, key: FieldKey): string | null | undefined {
    const staged = edits.get(file.id)?.[key];
    if (staged === undefined || staged === null) return staged;
    return Array.isArray(staged) ? staged.join("\n") : staged;
  }

  /** Drop a field's pending debounce, so an uncheck cannot be re-staged over. */
  function cancel(key: string): void {
    const prev = timers.get(key);
    if (prev) {
      clearTimeout(prev.timer);
      timers.delete(key);
    }
  }

  /**
   * Stage what was typed to every selected book, after the debounce. The
   * target ids are captured now, at input time, so a selection change before
   * the timer fires can never stage onto the wrong books. A blank on a
   * clearable field some selected book has a value for stages a clear
   * (null); any other blank simply unstages.
   */
  function stageField(key: FieldKey, raw: string): void {
    const targets = [...ids];
    const anyOwn = selected.some((f) => bookText(f, key).trim().length > 0);
    debounce(key, () => {
      const empty = LIST_KEYS.has(key)
        ? parseLines(raw).length === 0
        : raw.trim().length === 0;
      if (!empty) {
        edits.stage(targets, { [key]: LIST_KEYS.has(key) ? parseLines(raw) : raw });
      } else if (CLEARABLE_FIELDS.has(key) && anyOwn) {
        edits.stage(targets, { [key]: null });
      } else {
        edits.stage(targets, { [key]: undefined });
      }
    });
  }

  function uncheck(key: FieldKey): void {
    cancel(key);
    edits.stage([...ids], { [key]: undefined });
  }

  /** Everything one MetadataField needs, derived from the whole selection. */
  function fieldProps(key: FieldKey) {
    const staged = selected.map((f) => stagedText(f, key));
    const stagedCount = staged.filter((v) => v !== undefined).length;
    const first = staged.find((v) => v !== undefined);
    const agree = stagedCount === selected.length && staged.every((v) => v === first);
    const check =
      stagedCount === 0
        ? ("unchecked" as const)
        : agree
          ? ("checked" as const)
          : ("indeterminate" as const);

    const shown = selected.map((f, i) => (staged[i] === undefined ? bookText(f, key) : staged[i]));
    const same = shown.every((v) => v === shown[0]);
    return {
      check,
      cleared: same && shown[0] === null,
      mixed: !same,
      value: same && typeof shown[0] === "string" ? shown[0] : "",
      oninput: (raw: string) => stageField(key, raw),
      onuncheck: () => uncheck(key),
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
    const file = single;
    coverFailed = null;
    try {
      // Staged as a copy in the cover cache, not as the path the user picked:
      // that is the only place the webview is allowed to load an image from,
      // so it is the only path that can both preview here and ride along as
      // the `--cover` flag. See api/covers.ts.
      edits.stage([file.id], { coverPath: await cacheCover(selection) });
    } catch (err) {
      coverFailed = `That image could not be read. ${String(err)}`;
    }
  }

  const mdOnlySelection = $derived(selected.length > 0 && selected.every((f) => f.kind === "md"));
</script>

{#if selected.length === 0}
  <p class="text-[12px] leading-snug text-ink-500 dark:text-ink-400">
    Select a file to edit its metadata, or fetch it from Open Library.
  </p>
{:else}
  {#key single ? single.id : `multi:${ids.join(",")}`}
    {#if isMulti}
      <p class="mb-2.5 text-[11px] text-ink-500 dark:text-ink-400">
        Editing {selected.length} files - a change stages to all of them as you type.
      </p>
    {/if}

    <div class="flex flex-col gap-2.5">
      <MetadataField label="Title" placeholder="Book title" {...fieldProps("title")} />
      <MetadataField label="Authors" placeholder="One per line" multiline {...fieldProps("authors")} />

      <div class="grid grid-cols-[1fr_5.5rem] gap-2">
        <MetadataField label="Series" placeholder="Series name" {...fieldProps("series")} />
        <MetadataField label="Index" placeholder="#" {...fieldProps("seriesIndex")} />
      </div>

      <MetadataField label="Publisher" placeholder="Publisher" {...fieldProps("publisher")} />

      <div class="grid grid-cols-2 gap-2">
        <MetadataField label="Language" placeholder="en" {...fieldProps("language")} />
        <MetadataField label="Date" placeholder="1937" {...fieldProps("date")} />
      </div>

      <MetadataField label="ISBN" placeholder="978..." {...fieldProps("isbn")} />
      <MetadataField label="Subjects" placeholder="One per line" multiline {...fieldProps("subjects")} />
      <MetadataField label="Description" placeholder="Back-cover blurb" multiline {...fieldProps("description")} />

      {#if single}
        <div class="flex flex-col gap-1.5">
          <label class="flex w-fit cursor-pointer items-center gap-1.5">
            <input
              type="checkbox"
              checked={!!coverStaged}
              onclick={(e) => {
                e.preventDefault();
                if (coverStaged && single) edits.unstage(single.id, "coverPath");
              }}
              title={coverStaged ? "Staged - uncheck to revert" : "Choose an image to stage it"}
              class="h-3 w-3 rounded accent-teal-600"
            />
            <span class="text-[11px] font-medium text-ink-500 dark:text-ink-400">Cover</span>
          </label>
          <div class="flex items-start gap-2.5">
            <div
              class="h-20 w-14 shrink-0 overflow-hidden rounded-md border border-ink-200 bg-ink-100 dark:border-ink-700 dark:bg-ink-800"
            >
              {#if coverShown && !coverError}
                <img
                  src={coverUrl(coverShown)}
                  alt="Cover"
                  onerror={() => (coverError = true)}
                  class="h-full w-full object-cover"
                />
              {:else}
                <div class="flex h-full w-full items-center justify-center text-[10px] text-ink-400">
                  {coverError ? "no preview" : "none"}
                </div>
              {/if}
            </div>
            <div class="flex min-w-0 flex-col gap-1.5">
              <Button variant="secondary" size="sm" onclick={chooseCover}>Choose image...</Button>
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
        title={selected.length !== 1 ? "Select one file to look it up" : undefined}
        onclick={() => (searchOpen = true)}
      >
        <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
          <circle cx="9" cy="9" r="5.25" />
          <path d="M13 13l3.5 3.5" stroke-linecap="round" />
        </svg>
        Find online...
      </Button>

      {#if mdOnlySelection}
        <p class="text-[11px] leading-snug text-ink-500 dark:text-ink-400">
          Markdown books get their metadata when you Fit them - there is nothing to save in place.
        </p>
      {/if}
    </div>
  {/key}
{/if}

{#if searchOpen && single}
  <MetadataSearchDialog file={single} onclose={() => (searchOpen = false)} />
{/if}
