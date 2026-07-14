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
import { mergeEditsIntoMeta } from "../api/edits";
import type { StagedEdits } from "../api/edits";
import { settings } from "./settings.svelte";
import { edits } from "./edits.svelte";
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
  // The staged edits a convert job carried, so a successful run can clear them
  // and (for an in-place write) refresh the card without re-ingesting. Never
  // populated for a dry run: it wrote nothing, so there is nothing here for a
  // settled job to consume. Kept off the reactive array for the same reason
  // as #books.
  #applied = new Map<string, { edits: StagedEdits; inPlace: boolean }>();

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

  /**
   * Start a conversion batch: one fit (or md) job per book, using its plan.
   * `editsByBook` (already de-proxied to plain objects by the caller) supplies
   * per-book staged metadata that is spliced into that book's argv.
   */
  runFit(
    books: Book[],
    plans: { input: string; output: string | null }[],
    opts: RunOptions,
    editsByBook: Record<string, StagedEdits> = {},
  ): void {
    this.#startBatch();
    const outputByInput = new Map(plans.map((p) => [p.input, p.output]));
    for (const book of books) {
      // `null` means "in place" - a decision the planner makes, never a
      // fallback we invent. A book with no plan at all is skipped rather than
      // run: reading a missing entry as `null` would turn a planner bug into a
      // `--lets-get-dangerous` rewrite of an original the user never offered
      // up, and for a Markdown book into a flag the `md` command does not even
      // have. Neither is a mistake worth making quietly.
      if (!outputByInput.has(book.path)) continue;
      const output = outputByInput.get(book.path)!;
      if (output === null && book.kind !== "epub") continue;

      const bookEdits = editsByBook[book.id];
      const perBook: RunOptions = bookEdits ? { ...opts, edits: bookEdits } : opts;
      if (book.kind === "md") {
        this.#enqueue(book, "md", mdArgv(book.path, output, perBook), "normal");
      } else {
        this.#enqueue(book, "fit", fitArgv(book.path, output, perBook), "normal");
      }
      // A dry run writes nothing, so there is nothing to consume once it
      // settles: tracking it here would make #consumeEdits unstage (and, for
      // an in-place plan, fold into the card) edits the CLI was never asked
      // to write. Gating on `opts.dryRun` at enqueue time - rather than on
      // the settled report's own `dry_run` - keeps the one flag that decided
      // whether `--dry-run` went into this job's argv also the one flag that
      // decides whether its edits are ever tracked as applied.
      if (bookEdits && !opts.dryRun) {
        const jobId = this.#batchIds[this.#batchIds.length - 1];
        this.#applied.set(jobId, { edits: bookEdits, inPlace: output === null });
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

  /**
   * Clear the decks for a new batch: keep only what is still queued or
   * running - a background ingestion included, since one may well be in flight
   * - and drop every job that has already settled, along with its book
   * back-reference and its applied-edits entry. Ingestion jobs used to be kept
   * regardless of state, which meant a long session's `show` jobs (and the
   * `#books` entries pinning their books) piled up for as long as the app was
   * open. Nothing needs them once they are done: a settled ingestion has
   * already written its meta, cover and failure onto the book itself.
   */
  #startBatch(): void {
    const kept = this.jobs.filter((j) => !TERMINAL.has(j.state));
    const keptIds = new Set(kept.map((j) => j.id));
    for (const id of [...this.#books.keys()]) {
      if (!keptIds.has(id)) this.#books.delete(id);
    }
    for (const id of [...this.#applied.keys()]) {
      if (!keptIds.has(id)) this.#applied.delete(id);
    }
    this.jobs = kept;
    this.#batchIds = [];
  }

  #cancelJob(job: Job): void {
    job.state = "cancelled";
    this.#applied.delete(job.id);
    const book = this.#books.get(job.id);
    if (book) {
      if (job.kind === "show") {
        book.ingest = "failed";
        book.ingestError = this.#ingestError(job, {
          code: "cancelled",
          message: "Reading this book's metadata was cancelled.",
        });
      } else {
        book.result = { kind: "cancelled" };
      }
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
          book.ingestError = undefined;
          job.state = "done";
          return;
        }
      } catch {
        // Fall through to a failed ingest below.
      }
    }
    if (book) {
      book.ingest = "failed";
      book.ingestError = this.#ingestError(job, this.#failureOf(job, res));
    }
    job.state = "failed";
  }

  /**
   * The reason an ingestion failed, in the shape the card's details drawer
   * reads. Written onto the book (not left on the job) because the job is
   * pruned at the next batch while the card keeps saying "could not read" for
   * as long as the book is on the workbench - and a card that says so has to
   * be able to say why.
   */
  #ingestError(job: Job, failure: CliFailure): Book["ingestError"] {
    return {
      friendly: friendlyError(failure.code, failure.message),
      code: failure.code,
      stderr: [...job.stderrTail],
    };
  }

  #settleConvert(job: Job, res: SidecarResult): void {
    if (res.code === 0) {
      try {
        const report = parseReport<FitReport>(res.stdout, job.kind);
        if (!isCliFailure(report)) {
          job.result = report;
          const book = this.#books.get(job.id);
          if (book) {
            book.result = { kind: "fit", report };
            this.#consumeEdits(job.id, book);
          }
          job.state = "done";
          return;
        }
      } catch {
        // Fall through to failure handling.
      }
    }
    this.#applyFailure(job, res);
  }

  /**
   * A successful convert consumes its staged edits: unstage the fields it
   * actually wrote (the badge goes away once nothing is left) and, when the
   * book was rewritten in place, fold the written fields back into its card
   * so the title/cover reflect what is now on disk. A copy run leaves the
   * input book's own metadata untouched, so only the edits are unstaged
   * there. Unstaging by field rather than clearing the whole entry matters
   * for a book late in a batch: the user may have staged (or retyped) more
   * on it while this job's argv snapshot - taken back when Tailor was
   * clicked - sat in the queue, and none of that ever ran through the CLI.
   */
  #consumeEdits(jobId: string, book: Book): void {
    const applied = this.#applied.get(jobId);
    if (!applied) return;
    this.#applied.delete(jobId);
    edits.unstageApplied(book.id, applied.edits);
    if (applied.inPlace) {
      book.meta = mergeEditsIntoMeta(book.meta, applied.edits);
      if (applied.edits.coverPath) book.coverPath = applied.edits.coverPath;
    }
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

  /**
   * The `{code, message}` behind a non-zero exit: the CLI's own failure
   * payload when it printed one, and an honest guess from the exit code and
   * stderr when it did not (a crash, a killed process, output we could not
   * parse).
   */
  #failureOf(job: Job, res: SidecarResult): CliFailure {
    try {
      const parsed = parseReport<unknown>(res.stdout, job.kind);
      if (isCliFailure(parsed)) return parsed;
      return { code: "malformed-output", message: res.stderr || `exited with code ${res.code}` };
    } catch {
      return { code: "io-error", message: res.stderr || `exited with code ${res.code}` };
    }
  }

  #applyFailure(job: Job, res: SidecarResult): void {
    this.#applied.delete(job.id);
    const failure = this.#failureOf(job, res);
    job.failure = failure;
    const book = this.#books.get(job.id);
    if (book) {
      book.result = {
        kind: "failed",
        failure,
        friendly: friendlyError(failure.code, failure.message),
        stderr: [...job.stderrTail],
      };
    }
    job.state = "failed";
  }

  #settleSpawnError(id: string, message: string): void {
    this.#handles.delete(id);
    this.#applied.delete(id);
    const job = this.jobs.find((j) => j.id === id);
    if (!job) return;
    const failure: CliFailure = { code: "io-error", message };
    job.failure = failure;
    const book = this.#books.get(id);
    if (book) {
      if (job.kind === "show") {
        book.ingest = "failed";
        book.ingestError = this.#ingestError(job, failure);
      } else {
        book.result = {
          kind: "failed",
          failure,
          friendly: friendlyError(failure.code, message),
          stderr: [...job.stderrTail],
        };
      }
    }
    job.state = "failed";
    this.#pump();
  }
}

export const jobs = new JobsStore();
