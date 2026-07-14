// The job queue: every sidecar run - a book's metadata ingestion, a check, a
// conversion - is a Job here. A pump keeps up to `settings.parallelism`
// normal-priority conversions running at once; low-priority ingestion probes
// wait in the wings and run one at a time, only when no conversion wants the
// CPU. Results are written straight back onto the Book they belong to, so the
// cards react without any wiring between the two stores.

import { spawnSidecar } from "../api/sidecar";
import type { SidecarHandle, SidecarResult } from "../api/sidecar";
import { isCliFailure, parseReport } from "../api/contract";
import type { CheckReport, CliFailure, FitReport, MetadataShowReport } from "../api/contract";
import { friendlyError } from "../api/errors";
import { fitArgv, mdArgv, checkArgv } from "../api/argv";
import type { RunOptions } from "../api/argv";
import { normalizeMeta } from "../api/meta";
import { settings } from "./settings.svelte";
import type { Book } from "./books.svelte";

/** How many trailing stderr lines a running job keeps for a details view. */
const STDERR_TAIL = 50;

/** The CLI's `check` exit code for a book it could not even read. */
const CHECK_UNREADABLE = 2;

export type JobKind = "fit" | "md" | "check" | "show";
export type JobState = "queued" | "running" | "done" | "failed" | "cancelled";

const TERMINAL: ReadonlySet<JobState> = new Set(["done", "failed", "cancelled"]);

export interface Job {
  id: string;
  bookId: string;
  kind: JobKind;
  argv: string[];
  priority: "normal" | "low";
  state: JobState;
  stderrTail: string[];
  /** The parsed success payload, once done. */
  result?: FitReport | CheckReport;
  /** The failure, once failed. */
  failure?: CliFailure;
}

class JobsStore {
  jobs = $state<Job[]>([]);

  // The ids of the jobs in the current run, so progress is per-batch and not
  // polluted by a previous run's leftovers or by background ingestion.
  #batchIds = $state<string[]>([]);

  // Live handles and book back-references are kept off the reactive array on
  // purpose: a live SidecarHandle and a foreign Book proxy have no business
  // being deep-proxied again by this store's own state.
  #handles = new Map<string, SidecarHandle>();
  #books = new Map<string, Book>();

  batchJobs = $derived(this.jobs.filter((j) => this.#batchIds.includes(j.id)));
  total = $derived(this.batchJobs.length);
  done = $derived(this.batchJobs.filter((j) => TERMINAL.has(j.state)).length);
  running = $derived(this.batchJobs.filter((j) => j.state === "running").length);
  anyFailures = $derived(this.batchJobs.some((j) => j.state === "failed"));
  /** True while the current batch still has work queued or running. */
  active = $derived(this.batchJobs.some((j) => j.state === "queued" || j.state === "running"));

  // -- enqueue ----------------------------------------------------------------

  /** Queue a low-priority metadata/cover ingestion for a book. */
  enqueueIngest(book: Book, argv: string[]): void {
    this.#enqueue(book, "show", argv, "low");
    this.#pump();
  }

  /** Start a conversion batch: one fit (or md) job per book, using its plan. */
  runFit(books: Book[], plans: { input: string; output: string | null }[], opts: RunOptions): void {
    this.#startBatch();
    const outputByInput = new Map(plans.map((p) => [p.input, p.output]));
    for (const book of books) {
      const output = outputByInput.has(book.path) ? outputByInput.get(book.path)! : null;
      if (book.kind === "md") {
        this.#enqueue(book, "md", mdArgv(book.path, output, opts), "normal");
      } else {
        this.#enqueue(book, "fit", fitArgv(book.path, output, opts), "normal");
      }
    }
    this.#pump();
  }

  /** Start a check batch: lint each book against the given profile specs. */
  runCheck(books: Book[], profileSpecs: string[]): void {
    this.#startBatch();
    for (const book of books) {
      if (book.kind === "epub") {
        this.#enqueue(book, "check", checkArgv(book.path, profileSpecs), "normal");
      }
    }
    this.#pump();
  }

  // -- cancellation -----------------------------------------------------------

  /** Cancel one job, killing it if it is already running. */
  cancel(id: string): void {
    const job = this.jobs.find((j) => j.id === id);
    if (!job || TERMINAL.has(job.state)) return;
    this.#cancelJob(job);
    this.#pump();
  }

  /** Cancel the whole conversion batch: drain the queue first, then kill runners. */
  cancelAll(): void {
    for (const job of this.jobs) {
      if (job.priority === "normal" && job.state === "queued") this.#cancelJob(job);
    }
    for (const job of this.jobs) {
      if (job.priority === "normal" && job.state === "running") this.#cancelJob(job);
    }
    this.#pump();
  }

  // -- lookups for the UI -----------------------------------------------------

  /** The most recent conversion/check job for a book, for its card's live state. */
  conversionJobFor(bookId: string): Job | undefined {
    let found: Job | undefined;
    for (const job of this.jobs) {
      if (job.bookId === bookId && job.kind !== "show") found = job;
    }
    return found;
  }

  // -- internals --------------------------------------------------------------

  #enqueue(book: Book, kind: JobKind, argv: string[], priority: "normal" | "low"): void {
    const id = crypto.randomUUID();
    this.#books.set(id, book);
    this.jobs.push({ id, bookId: book.id, kind, argv, priority, state: "queued", stderrTail: [] });
    if (priority === "normal") this.#batchIds = [...this.#batchIds, id];
  }

  #startBatch(): void {
    const kept = this.jobs.filter((j) => j.priority === "low" || !TERMINAL.has(j.state));
    const keptIds = new Set(kept.map((j) => j.id));
    for (const id of [...this.#books.keys()]) {
      if (!keptIds.has(id)) this.#books.delete(id);
    }
    this.jobs = kept;
    this.#batchIds = [];
  }

  #cancelJob(job: Job): void {
    job.state = "cancelled";
    const book = this.#books.get(job.id);
    if (book) {
      if (job.kind === "show") book.ingest = "failed";
      else book.result = { kind: "cancelled" };
    }
    const handle = this.#handles.get(job.id);
    if (handle) {
      this.#handles.delete(job.id);
      void handle.kill();
    }
  }

  #pump(): void {
    const runningNormal = this.jobs.filter(
      (j) => j.state === "running" && j.priority === "normal",
    ).length;
    let slots = settings.parallelism - runningNormal;
    for (const job of this.jobs) {
      if (slots <= 0) break;
      if (job.state === "queued" && job.priority === "normal") {
        void this.#start(job.id);
        slots -= 1;
      }
    }

    const normalActive = this.jobs.some(
      (j) => j.priority === "normal" && (j.state === "queued" || j.state === "running"),
    );
    const lowRunning = this.jobs.some((j) => j.priority === "low" && j.state === "running");
    if (!normalActive && !lowRunning) {
      const nextLow = this.jobs.find((j) => j.priority === "low" && j.state === "queued");
      if (nextLow) void this.#start(nextLow.id);
    }
  }

  async #start(id: string): Promise<void> {
    const job = this.jobs.find((j) => j.id === id);
    if (!job || job.state !== "queued") return;
    job.state = "running";
    try {
      const handle = await spawnSidecar(job.argv, {
        onStderrLine: (line) => {
          const j = this.jobs.find((x) => x.id === id);
          if (j) j.stderrTail = [...j.stderrTail, line].slice(-STDERR_TAIL);
        },
      });
      const current = this.jobs.find((j) => j.id === id);
      if (!current || current.state !== "running") {
        // Cancelled while the process was spawning: kill it now.
        void handle.kill();
        return;
      }
      this.#handles.set(id, handle);
      void handle.done.then((res) => this.#settle(id, res));
    } catch (err) {
      this.#settleSpawnError(id, String(err));
    }
  }

  #settle(id: string, res: SidecarResult): void {
    this.#handles.delete(id);
    const job = this.jobs.find((j) => j.id === id);
    if (!job || job.state === "cancelled") {
      this.#pump();
      return;
    }
    switch (job.kind) {
      case "show":
        this.#settleShow(job, res);
        break;
      case "check":
        this.#settleCheck(job, res);
        break;
      default:
        this.#settleConvert(job, res);
        break;
    }
    this.#pump();
  }

  #settleShow(job: Job, res: SidecarResult): void {
    const book = this.#books.get(job.id);
    if (res.code === 0 && book) {
      try {
        const report = parseReport<MetadataShowReport>(res.stdout, "metadata show");
        if (!isCliFailure(report)) {
          book.meta = normalizeMeta(report);
          if (report.metadata.cover) book.coverPath = report.metadata.cover;
          book.ingest = "done";
          job.state = "done";
          return;
        }
      } catch {
        // Fall through to a failed ingest below.
      }
    }
    if (book) book.ingest = "failed";
    job.state = "failed";
  }

  #settleConvert(job: Job, res: SidecarResult): void {
    if (res.code === 0) {
      try {
        const report = parseReport<FitReport>(res.stdout, job.kind);
        if (!isCliFailure(report)) {
          job.result = report;
          const book = this.#books.get(job.id);
          if (book) book.result = { kind: "fit", report };
          job.state = "done";
          return;
        }
      } catch {
        // Fall through to failure handling.
      }
    }
    this.#applyFailure(job, res);
  }

  #settleCheck(job: Job, res: SidecarResult): void {
    if (res.code !== CHECK_UNREADABLE) {
      try {
        const report = parseReport<CheckReport>(res.stdout, "check");
        if (!isCliFailure(report)) {
          job.result = report;
          const book = this.#books.get(job.id);
          if (book) book.result = { kind: "check", report };
          job.state = "done";
          return;
        }
      } catch {
        // Fall through to failure handling.
      }
    }
    this.#applyFailure(job, res);
  }

  #applyFailure(job: Job, res: SidecarResult): void {
    let failure: CliFailure;
    try {
      const parsed = parseReport<unknown>(res.stdout, job.kind);
      failure = isCliFailure(parsed)
        ? parsed
        : { code: "malformed-output", message: res.stderr || `exited with code ${res.code}` };
    } catch {
      failure = { code: "io-error", message: res.stderr || `exited with code ${res.code}` };
    }
    job.failure = failure;
    const book = this.#books.get(job.id);
    if (book) {
      book.result = {
        kind: "failed",
        failure,
        friendly: friendlyError(failure.code, failure.message),
      };
    }
    job.state = "failed";
  }

  #settleSpawnError(id: string, message: string): void {
    this.#handles.delete(id);
    const job = this.jobs.find((j) => j.id === id);
    if (!job) return;
    const failure: CliFailure = { code: "io-error", message };
    job.failure = failure;
    const book = this.#books.get(id);
    if (book) {
      if (job.kind === "show") book.ingest = "failed";
      else book.result = { kind: "failed", failure, friendly: friendlyError(failure.code, message) };
    }
    job.state = "failed";
    this.#pump();
  }
}

export const jobs = new JobsStore();
