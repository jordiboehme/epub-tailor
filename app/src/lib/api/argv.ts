// Pure command-line builders for the `epub-tailor` sidecar. No Tauri import:
// the argv is a plain string array the jobs store hands to `spawnSidecar`, and
// every rule here is unit-tested without a window in sight. Flag order is
// fixed so the tests can pin it and a reader can predict it.

export interface RunOptions {
  /** Resolved profile specs, composed left to right - one `--profile` pair each. */
  profiles: string[];
  /** `low`/`std`/`high` or a raw number as a string; `null` leaves the profile default. */
  quality: string | null;
  /** `text`/`image`/`image-all`; `null` leaves the profile default. */
  tables: string | null;
  /** Analyze without writing anything. */
  dryRun: boolean;
}

/**
 * The shared body of `fit` and `md`: both take the same flags in the same
 * order. `output === null` means an in-place run (`--lets-get-dangerous`);
 * any other value is written with `-o`.
 */
function convertArgv(
  command: "fit" | "md",
  input: string,
  output: string | null,
  opts: RunOptions,
): string[] {
  const argv = [command, input, "--report", "json"];

  for (const profile of opts.profiles) {
    argv.push("--profile", profile);
  }
  if (opts.quality !== null) {
    argv.push("--quality", opts.quality);
  }
  if (opts.tables !== null) {
    argv.push("--tables", opts.tables);
  }
  if (opts.dryRun) {
    argv.push("--dry-run");
  }

  if (output === null) {
    argv.push("--lets-get-dangerous");
  } else {
    argv.push("-o", output);
  }

  return argv;
}

/** `epub-tailor fit <input> ...`. `output === null` runs in place. */
export function fitArgv(input: string, output: string | null, opts: RunOptions): string[] {
  return convertArgv("fit", input, output, opts);
}

/**
 * `epub-tailor md <input> ...`: same shape as {@link fitArgv}, different verb.
 * Markdown never runs in place, so `output` is expected to be a real path -
 * the planner never hands this a `null`.
 */
export function mdArgv(input: string, output: string | null, opts: RunOptions): string[] {
  return convertArgv("md", input, output, opts);
}

/** `epub-tailor check <input> --report json` plus one `--profile` pair per spec. */
export function checkArgv(input: string, profiles: string[]): string[] {
  const argv = ["check", input, "--report", "json"];
  for (const profile of profiles) {
    argv.push("--profile", profile);
  }
  return argv;
}

/** `epub-tailor metadata show <input> --report json --cover-out <coverOut>`. */
export function showArgv(input: string, coverOut: string): string[] {
  return ["metadata", "show", input, "--report", "json", "--cover-out", coverOut];
}
