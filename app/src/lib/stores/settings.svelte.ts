// The user's persistent preferences, backed by `@tauri-apps/plugin-store`
// (`settings.json` in the app data dir). Loaded once at startup; changes to the
// persisted fields are written back on a debounce by the store's own autoSave.
//
// Two fields are deliberately session-only - `dryRun` and `inPlace` always
// start off, because "preview" and "replace originals" are decisions you make
// per session, not defaults you want silently remembered into the next launch.

import { Store } from "@tauri-apps/plugin-store";
import type { WindowGeometry } from "../api/geometry";

export type { WindowGeometry };

const STORE_FILE = "settings.json";
const AUTOSAVE_DEBOUNCE_MS = 300;

// The numeric settings, with the range each one is only ever allowed to hold.
// settings.json is a file: it can be hand-edited, truncated by a bad shutdown
// or simply carry a value from a future version. A parallelism of 0 read back
// unchecked hands the job pump zero slots forever - a batch stuck at "0 of N"
// with nothing ever starting - and a split level of 7 is a CLI argument error
// per Markdown book. Neither is worth a dialog; both are worth a clamp.
const DEFAULT_PARALLELISM = 3;
const MIN_PARALLELISM = 1;
const MAX_PARALLELISM = 8;

const DEFAULT_SPLIT_LEVEL = 1;
const MIN_SPLIT_LEVEL = 1;
const MAX_SPLIT_LEVEL = 2;

/** A persisted number, forced into `[min, max]`, or `fallback` if it is not a number at all. */
function clampInt(value: unknown, min: number, max: number, fallback: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, Math.round(value)));
}

/** How the workbench shows its books: the cover gallery, or the denser list. */
export type ViewMode = "grid" | "list";

const DEFAULT_VIEW_MODE: ViewMode = "grid";

/**
 * A persisted view mode, or the gallery when the file holds anything else -
 * same reasoning as the clamps above: settings.json is hand-editable, and a
 * `"View.LIST"` read back unchecked would render neither view.
 */
function toViewMode(value: unknown): ViewMode {
  return value === "grid" || value === "list" ? value : DEFAULT_VIEW_MODE;
}

class SettingsStore {
  // -- persisted --------------------------------------------------------------
  /** Selected built-in profile name. */
  profile = $state("epub");
  /** Paths to user profile JSON layers, composed on top of the built-in. */
  userProfilePaths = $state<string[]>([]);
  /** Destination folder, or `null` to write alongside each original. */
  outputDir = $state<string | null>(null);
  /** Filename template; `{original}` keeps each book's own stem. */
  filenameTemplate = $state("{original}");
  /** JPEG quality override (`low`/`std`/`high` or a number), or `null` for the profile default. */
  quality = $state<string | null>(null);
  /** Table handling override, or `null` for the profile default. */
  tables = $state<string | null>(null);
  /** Heading level a Markdown book splits chapters on: 1 or 2 (the CLI's default is 1). */
  mdSplitLevel = $state(DEFAULT_SPLIT_LEVEL);
  /** Walk subfolders when a dropped folder is expanded. */
  recursive = $state(true);
  /** How many conversions run at once. */
  parallelism = $state(DEFAULT_PARALLELISM);
  /** Whether the books show as a cover gallery or as a list. */
  viewMode = $state<ViewMode>(DEFAULT_VIEW_MODE);
  /** Where the window was, and how big, when it was last moved or resized. */
  windowGeometry = $state<WindowGeometry | null>(null);

  // -- session-only (never persisted) -----------------------------------------
  /** Analyze without writing (Preview only). Always starts off. */
  dryRun = $state(false);
  /** Rewrite originals in place. Always starts off. */
  inPlace = $state(false);

  /** True once `load()` has read the persisted values. */
  ready = $state(false);

  #store: Store | null = null;

  /** Read persisted settings, then wire up write-back for future changes. */
  async load(): Promise<void> {
    const store = await Store.load(STORE_FILE, { defaults: {}, autoSave: AUTOSAVE_DEBOUNCE_MS });
    this.profile = (await store.get<string>("profile")) ?? this.profile;
    this.userProfilePaths = (await store.get<string[]>("userProfilePaths")) ?? this.userProfilePaths;
    this.outputDir = (await store.get<string | null>("outputDir")) ?? this.outputDir;
    this.filenameTemplate = (await store.get<string>("filenameTemplate")) ?? this.filenameTemplate;
    this.quality = (await store.get<string | null>("quality")) ?? this.quality;
    this.tables = (await store.get<string | null>("tables")) ?? this.tables;
    this.mdSplitLevel = clampInt(
      await store.get<number>("mdSplitLevel"),
      MIN_SPLIT_LEVEL,
      MAX_SPLIT_LEVEL,
      DEFAULT_SPLIT_LEVEL,
    );
    this.recursive = (await store.get<boolean>("recursive")) ?? this.recursive;
    this.parallelism = clampInt(
      await store.get<number>("parallelism"),
      MIN_PARALLELISM,
      MAX_PARALLELISM,
      DEFAULT_PARALLELISM,
    );
    this.viewMode = toViewMode(await store.get("viewMode"));
    this.windowGeometry = (await store.get<WindowGeometry>("windowGeometry")) ?? null;
    this.#store = store;
    this.ready = true;

    // One effect per persisted field: each writes only its own key when it
    // changes, and the store debounces the actual disk write. Set up after the
    // reads above so the initial values are not clobbered before they load.
    $effect.root(() => {
      $effect(() => void store.set("profile", this.profile));
      $effect(() => void store.set("userProfilePaths", $state.snapshot(this.userProfilePaths)));
      $effect(() => void store.set("outputDir", this.outputDir));
      $effect(() => void store.set("filenameTemplate", this.filenameTemplate));
      $effect(() => void store.set("quality", this.quality));
      $effect(() => void store.set("tables", this.tables));
      $effect(() => void store.set("mdSplitLevel", this.mdSplitLevel));
      $effect(() => void store.set("recursive", this.recursive));
      $effect(() => void store.set("parallelism", this.parallelism));
      $effect(() => void store.set("viewMode", this.viewMode));
      $effect(() => void store.set("windowGeometry", $state.snapshot(this.windowGeometry)));
    });
  }
}

export const settings = new SettingsStore();
