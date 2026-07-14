// jobs.svelte.ts drives conversions through the real `@tauri-apps/plugin-shell`
// IPC surface, so exercising its settle path means faking that surface the same
// way sidecar.test.ts does - but capturing the registered stdout/close handlers
// too, so a test can push a report through them and drive a job all the way to
// "done" without a Tauri runtime.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

interface FakeCommand {
  handlers: Record<string, Array<(...args: unknown[]) => void>>;
  spawn: ReturnType<typeof vi.fn>;
  stdout: { on: (event: string, cb: (...args: unknown[]) => void) => void };
  stderr: { on: (event: string, cb: (...args: unknown[]) => void) => void };
  on: (event: string, cb: (...args: unknown[]) => void) => void;
}

const spawned: FakeCommand[] = [];

function makeFakeCommand(): FakeCommand {
  const handlers: Record<string, Array<(...args: unknown[]) => void>> = {};
  const on =
    (bucket: string) =>
    (event: string, cb: (...args: unknown[]) => void): void => {
      (handlers[`${bucket}:${event}`] ??= []).push(cb);
    };
  return {
    handlers,
    spawn: vi.fn().mockResolvedValue({ kill: vi.fn() }),
    stdout: { on: on("stdout") },
    stderr: { on: on("stderr") },
    on: on("cmd"),
  };
}

vi.mock("@tauri-apps/plugin-shell", () => {
  return {
    Command: {
      sidecar: vi.fn(() => {
        const cmd = makeFakeCommand();
        spawned.push(cmd);
        return cmd;
      }),
    },
  };
});

import { jobs } from "../lib/stores/jobs.svelte";
import { edits } from "../lib/stores/edits.svelte";
import type { Book } from "../lib/stores/books.svelte";
import type { RunOptions } from "../lib/api/argv";
import type { FitReport } from "../lib/api/contract";

function makeBook(overrides: Partial<Book> = {}): Book {
  return {
    id: crypto.randomUUID(),
    path: "/tmp/book.epub",
    kind: "epub",
    fileName: "book.epub",
    size: 100,
    modifiedMs: 0,
    ingest: "done",
    ...overrides,
  };
}

function baseReport(overrides: Partial<FitReport> = {}): FitReport {
  return {
    schema: 1,
    output: null,
    dry_run: false,
    transformations: [],
    warnings: [],
    stats: {
      bytes_in: 0,
      bytes_out: 0,
      images_processed: 0,
      chapters: 0,
      chapters_split: 0,
      warnings: 0,
    },
    ...overrides,
  };
}

function baseOpts(overrides: Partial<RunOptions> = {}): RunOptions {
  return { profiles: [], quality: null, tables: null, dryRun: false, ...overrides };
}

/** Push a successful report through the most recently spawned fake command. */
function fireSuccess(report: FitReport): void {
  const cmd = spawned[spawned.length - 1];
  cmd.handlers["stdout:data"]?.forEach((cb) => cb(JSON.stringify(report)));
  cmd.handlers["cmd:close"]?.forEach((cb) => cb({ code: 0 }));
}

async function waitForDone(bookId: string): Promise<void> {
  await vi.waitFor(() => {
    const job = jobs.jobs.find((j) => j.bookId === bookId && j.kind !== "show");
    expect(job?.state).toBe("done");
  });
}

beforeEach(() => {
  spawned.length = 0;
  edits.clear();
});

afterEach(() => {
  edits.clear();
});

describe("JobsStore / runFit dry run", () => {
  it("does not consume staged edits when the settled run was a dry run", async () => {
    const book = makeBook();
    edits.stage([book.id], { title: "New Title" });
    const staged = edits.get(book.id)!;

    jobs.runFit(
      [book],
      [{ input: book.path, output: null }],
      baseOpts({ dryRun: true }),
      { [book.id]: staged },
    );
    fireSuccess(baseReport({ dry_run: true, output: null }));
    await waitForDone(book.id);

    // A preview writes nothing: the staged edits must still be there, and the
    // card's metadata must not have been rewritten as though they were.
    expect(edits.get(book.id)).toEqual(staged);
    expect(book.meta).toBeUndefined();
  });

  it("still consumes staged edits when a real (non-dry-run) run wrote them", async () => {
    const book = makeBook();
    edits.stage([book.id], { title: "New Title" });
    const staged = edits.get(book.id)!;

    jobs.runFit(
      [book],
      [{ input: book.path, output: null }],
      baseOpts({ dryRun: false }),
      { [book.id]: staged },
    );
    fireSuccess(baseReport({ dry_run: false, output: book.path }));
    await waitForDone(book.id);

    expect(edits.get(book.id)).toBeUndefined();
    expect(book.meta?.title).toBe("New Title");
  });
});
