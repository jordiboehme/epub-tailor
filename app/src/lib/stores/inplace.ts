// The one in-place write path: rewrite files under the repair profile, each
// preceded by a safety copy parked in the OS Trash. Shared by the ActionBar's
// "Save changes" / "Clean up" buttons and the file rows' "needs cleanup"
// chip, so the safety rules cannot drift between entry points. Lives beside
// the stores (not in api/) because it orchestrates them and talks to Tauri.

import { invoke } from "@tauri-apps/api/core";
import { CLEANUP_PROFILE } from "../api/argv";
import type { RunOptions } from "../api/argv";
import { books } from "./books.svelte";
import type { BookFile } from "./books.svelte";
import { edits } from "./edits.svelte";
import { jobs } from "./jobs.svelte";

/** What `backup_to_trash` reports back (see src-tauri/commands.rs). */
interface BackupOutcome {
  method: "trash" | "file";
  backup_path: string;
}

export interface InPlaceOutcome {
  /** Per-file backup failures ("name: reason"); these files were NOT written. */
  failures: string[];
  /** Backups kept as sibling files because the volume has no trash. */
  keptAsFile: number;
  /** How many files actually went into the run. */
  ran: number;
}

/**
 * Rewrite `files` in place under `profileSpecs`, with staged edits
 * (`withEdits`) or without (a pure cleanup). Before any file is touched, a
 * copy of it goes to the OS Trash; a file whose backup cannot be made is
 * dropped from the batch - no backup, no overwrite.
 *
 * `profileSpecs` defaults to the bare repair profile, and the metadata-save
 * path MUST keep that default. Composing `generic` in here would strip
 * content and rewrite the book's identifier off a button labelled "Save
 * changes" - the widening only ever belongs to an action the user picked by
 * name (see `WATERMARK_PROFILE`).
 */
export async function saveFilesInPlace(
  files: BookFile[],
  withEdits: boolean,
  profileSpecs: string[] = [CLEANUP_PROFILE],
): Promise<InPlaceOutcome> {
  edits.flushPending();
  const outcome: InPlaceOutcome = { failures: [], keptAsFile: 0, ran: 0 };
  if (files.length === 0) return outcome;

  const kept: BookFile[] = [];
  for (const file of files) {
    try {
      const backup = await invoke<BackupOutcome>("backup_to_trash", { path: file.path });
      if (backup.method === "file") outcome.keptAsFile += 1;
      kept.push(file);
    } catch (err) {
      outcome.failures.push(`${file.fileName}: ${String(err)}`);
    }
  }
  if (kept.length === 0) return outcome;

  const opts: RunOptions = {
    profiles: profileSpecs,
    quality: null,
    tables: null,
    dryRun: false,
  };
  const plans = kept.map((f) => ({ input: f.path, output: null }));
  const snapshot = withEdits ? edits.snapshotFor(kept.map((f) => f.id)) : {};
  jobs.runFit(books.refsFor(kept), plans, opts, snapshot);
  outcome.ran = kept.length;
  return outcome;
}
