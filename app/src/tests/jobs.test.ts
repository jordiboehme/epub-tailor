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
import type { Book, BookFile } from "../lib/stores/books.svelte";
import type { RunOptions } from "../lib/api/argv";
import type { CheckReport, FitReport } from "../lib/api/contract";

function makeFile(overrides: Partial<BookFile> = {}): BookFile {
  return {
    id: crypto.randomUUID(),
    path: "/tmp/book.epub",
    kind: "epub",
    fileName: "book.epub",
    role: "original",
    profile: null,
    appendix: null,
    size: 100,
    modifiedMs: 0,
    ingest: "done",
    ...overrides,
  };
}

function makeBook(file: BookFile): Book {
  return { id: crypto.randomUUID(), files: [file] };
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

/** Push raw stdout and an exit code through the most recently spawned command. */
function fireClose(stdout: string, code: number): void {
  const cmd = spawned[spawned.length - 1];
  if (stdout) cmd.handlers["stdout:data"]?.forEach((cb) => cb(stdout));
  cmd.handlers["cmd:close"]?.forEach((cb) => cb({ code }));
}

function checkReport(findings: CheckReport["findings"]): CheckReport {
  return {
    schema: 1,
    findings,
    errors: findings.filter((f) => f.severity === "error").length,
    warnings: findings.filter((f) => f.severity === "warning").length,
  };
}

const A_FINDING = {
  severity: "warning" as const,
  code: "junk-meta",
  message: "carries junk metadata",
  path: null,
};

async function waitForJob(fileId: string, kind: string, state: string): Promise<void> {
  await vi.waitFor(() => {
    const job = jobs.jobs.find((j) => j.fileId === fileId && j.kind === kind);
    expect(job?.state).toBe(state);
  });
}

async function waitForDone(fileId: string): Promise<void> {
  await vi.waitFor(() => {
    const job = jobs.jobs.find(
      (j) => j.fileId === fileId && j.kind !== "show" && j.kind !== "autocheck",
    );
    expect(job?.state).toBe("done");
  });
}

beforeEach(() => {
  // The store is a singleton: cancel whatever a previous test left queued or
  // running (e.g. the re-probe an in-place write enqueues), or it blocks the
  // low-priority lane for the next test.
  for (const job of [...jobs.jobs]) jobs.cancel(job.id);
  spawned.length = 0;
  edits.clear();
  jobs.onCopyTracked = null;
});

afterEach(() => {
  edits.clear();
  jobs.onCopyTracked = null;
});

describe("JobsStore / runFit dry run", () => {
  it("does not consume staged edits when the settled run was a dry run", async () => {
    const file = makeFile();
    const book = makeBook(file);
    edits.stage([file.id], { title: "New Title" });
    const staged = edits.get(file.id)!;

    jobs.runFit(
      [{ book, file }],
      [{ input: file.path, output: null }],
      baseOpts({ dryRun: true }),
      { [file.id]: staged },
    );
    fireSuccess(baseReport({ dry_run: true, output: null }));
    await waitForDone(file.id);

    // A preview writes nothing: the staged edits must still be there, and the
    // row's metadata must not have been rewritten as though they were.
    expect(edits.get(file.id)).toEqual(staged);
    expect(file.meta).toBeUndefined();
  });

  it("still consumes staged edits when a real (non-dry-run) run wrote them", async () => {
    const file = makeFile();
    const book = makeBook(file);
    edits.stage([file.id], { title: "New Title" });
    const staged = edits.get(file.id)!;

    jobs.runFit(
      [{ book, file }],
      [{ input: file.path, output: null }],
      baseOpts({ dryRun: false }),
      { [file.id]: staged },
    );
    fireSuccess(baseReport({ dry_run: false, output: file.path }));
    await waitForDone(file.id);

    expect(edits.get(file.id)).toBeUndefined();
    expect(file.meta?.title).toBe("New Title");
  });
});

describe("JobsStore / automatic check", () => {
  it("settles onto file.cleanup and leaves file.result alone", async () => {
    const file = makeFile();
    const book = makeBook(file);
    jobs.enqueueAutoCheck(book, file, ["check", file.path, "--report", "json"]);
    // `check` exits 1 when it found something - still a valid report.
    fireClose(JSON.stringify(checkReport([A_FINDING])), 1);
    await waitForJob(file.id, "autocheck", "done");

    expect(file.cleanup?.findings).toEqual([A_FINDING]);
    expect(file.result).toBeUndefined();
  });

  it("stays silent on the row when the probe itself fails", async () => {
    const file = makeFile();
    const book = makeBook(file);
    jobs.enqueueAutoCheck(book, file, ["check", file.path, "--report", "json"]);
    // Exit 2 = the CLI could not even read the file.
    fireClose("", 2);
    await waitForJob(file.id, "autocheck", "failed");

    expect(file.cleanup).toBeUndefined();
    expect(file.result).toBeUndefined();
    expect(file.ingest).toBe("done");
  });
});

describe("JobsStore / copy tracking", () => {
  it("tracks a written copy on the book, tagged with the run's profile", async () => {
    const file = makeFile();
    const book = makeBook(file);
    jobs.runFit(
      [{ book, file }],
      [{ input: file.path, output: "/tmp/book.x4.epub" }],
      baseOpts(),
      {},
      { profileName: "x4", appendix: "x4" },
    );
    fireSuccess(baseReport({ output: "/tmp/book.x4.epub" }));
    await waitForDone(file.id);

    expect(book.files).toHaveLength(2);
    expect(book.files[1]).toEqual(
      expect.objectContaining({
        path: "/tmp/book.x4.epub",
        fileName: "book.x4.epub",
        kind: "epub",
        role: "copy",
        profile: "x4",
        appendix: "x4",
        origin: "fit",
        ingest: "pending",
      }),
    );
    // The size chips describe the OUTPUT: the report lands on the produced
    // file's row, and the input - which did not change - stays unmarked.
    expect(book.files[1].result?.kind).toBe("fit");
    expect(file.result).toBeUndefined();
  });

  it("keeps the copy's id stable when the same output is rewritten", async () => {
    const file = makeFile();
    const book = makeBook(file);
    for (let run = 0; run < 2; run += 1) {
      jobs.runFit(
        [{ book, file }],
        [{ input: file.path, output: "/tmp/book.x4.epub" }],
        baseOpts(),
        {},
        { profileName: "x4", appendix: "x4" },
      );
      fireSuccess(baseReport({ output: "/tmp/book.x4.epub" }));
      await waitForDone(file.id);
      if (run === 0) book.files[1].id = "stable-id";
    }
    expect(book.files).toHaveLength(2);
    // A fresh id on re-fit would orphan the copy's selection and staged edits.
    expect(book.files[1].id).toBe("stable-id");
    expect(book.files[1].ingest).toBe("pending");
  });

  it("tracks nothing for a dry run", async () => {
    const file = makeFile();
    const book = makeBook(file);
    jobs.runFit(
      [{ book, file }],
      [{ input: file.path, output: "/tmp/book.x4.epub" }],
      baseOpts({ dryRun: true }),
      {},
      { profileName: "x4", appendix: "x4" },
    );
    fireSuccess(baseReport({ dry_run: true, output: null }));
    await waitForDone(file.id);

    expect(book.files).toHaveLength(1);
    // A preview describes what would happen to the input, so it chips there.
    expect(file.result?.kind).toBe("fit");
  });

  it("clears the stale cleanup verdict and re-probes after an in-place write", async () => {
    const file = makeFile({ cleanup: checkReport([A_FINDING]) });
    const book = makeBook(file);
    jobs.runFit([{ book, file }], [{ input: file.path, output: null }], baseOpts());
    // An in-place run reports the input path as where the file landed.
    fireSuccess(baseReport({ output: file.path }));
    await waitForDone(file.id);

    expect(file.cleanup).toBeUndefined();
    // An in-place rewrite changed the input itself, so the result chips there.
    expect(file.result?.kind).toBe("fit");
    expect(
      jobs.jobs.some((j) => j.fileId === file.id && j.kind === "autocheck"),
    ).toBe(true);
  });

  it("hands a tracked copy to onCopyTracked, but never for dry-run or in-place", async () => {
    const spy = vi.fn();
    jobs.onCopyTracked = spy;

    const file = makeFile();
    const book = makeBook(file);
    jobs.runFit(
      [{ book, file }],
      [{ input: file.path, output: "/tmp/book.x4.epub" }],
      baseOpts(),
      {},
      { profileName: "x4", appendix: "x4" },
    );
    fireSuccess(baseReport({ output: "/tmp/book.x4.epub" }));
    await waitForDone(file.id);

    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy).toHaveBeenCalledWith(book, book.files[1]);

    // In place: the callback stays quiet (nothing new to stat or ingest).
    spy.mockClear();
    jobs.runFit([{ book, file }], [{ input: file.path, output: null }], baseOpts());
    fireSuccess(baseReport({ output: file.path }));
    await waitForDone(file.id);
    expect(spy).not.toHaveBeenCalled();
  });
});
