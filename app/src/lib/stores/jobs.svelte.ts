// The job queue: every sidecar run - a file's metadata ingestion, a check, a
// conversion - is a Job here. A pump keeps up to `settings.parallelism`
// normal-priority conversions running at once; low-priority ingestion probes
// wait in the wings and run one at a time, only when no conversion wants the
// CPU. Results are written straight back onto the BookFile they belong to, so
// the rows react without any wiring between the two stores.

import { spawnSidecar } from "../api/sidecar";
import type { SidecarHandle, SidecarResult } from "../api/sidecar";
import { isCliFailure, parseReport } from "../api/contract";
import type { CheckReport, CliFailure, FitReport, MetadataShowReport } from "../api/contract";
import { friendlyError } from "../api/errors";
import { fitArgv, mdArgv, checkArgv, CLEANUP_PROFILE } from "../api/argv";
import type { RunOptions } from "../api/argv";
import { samePath } from "../api/copies";
import { normalizeMeta } from "../api/meta";
import { mergeEditsIntoMeta } from "../api/edits";
import type { StagedEdits } from "../api/edits";
import { settings } from "./settings.svelte";
import { edits } from "./edits.svelte";
import type { Book, BookFile, FileTarget } from "./books.svelte";

/** How many trailing stderr lines a running job keeps for a details view. */
const STDERR_TAIL = 50;

/** The CLI's `check` exit code for a book it could not even read. */
const CHECK_UNREADABLE = 2;

export type JobKind = "fit" | "md" | "check" | "show" | "autocheck";
export type JobState = "queued" | "running" | "done" | "failed" | "cancelled";

const TERMINAL: ReadonlySet<JobState> = new Set(["done", "failed", "cancelled"]);

export interface Job {
  id: string;
  fileId: string;
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

  /**
   * Set by the books store: stat + ingest + check a copy `#trackOutput` just
   * upserted. A callback, not an import, so this store stays free of
   * `invoke`/covers plumbing and its tests stay free of the mocks that would
   * drag in.
   */
  onCopyTracked: ((book: Book, file: BookFile) => void) | null = null;

  // The ids of the jobs in the current run, so progress is per-batch and not
  // polluted by a previous run's leftovers or by background ingestion.
  #batchIds = $state<string[]>([]);

  // Live handles and file back-references are kept off the reactive array on
  // purpose: a live SidecarHandle and a foreign proxy have no business being
  // deep-proxied again by this store's own state. Every Book/BookFile in
  // #refs must be the reactive array element, never a local literal, or the
  // write-back misses the graph.
  #handles = new Map<string, SidecarHandle>();
  #refs = new Map<string, FileTarget>();
  // The staged edits a convert job carried, so a successful run can clear them
  // and (for an in-place write) refresh the row without re-ingesting. Never
  // populated for a dry run: it wrote nothing, so there is nothing here for a
  // settled job to consume. Kept off the reactive array for the same reason
  // as #refs.
  #applied = new Map<string, { edits: StagedEdits; inPlace: boolean }>();
  // What a fit job knew about the profile it ran under, so a settled copy can
  // be tracked on its book with a profile badge. Same lifecycle rules as
  // #applied.
  #fitMeta = new Map<string, { profileName: string; appendix: string }>();

  batchJobs = $derived(this.jobs.filter((j) => this.#batchIds.includes(j.id)));
  total = $derived(this.batchJobs.length);
  done = $derived(this.batchJobs.filter((j) => TERMINAL.has(j.state)).length);
  running = $derived(this.batchJobs.filter((j) => j.state === "running").length);
  anyFailures = $derived(this.batchJobs.some((j) => j.state === "failed"));
  /** True while the current batch still has work queued or running. */
  active = $derived(this.batchJobs.some((j) => j.state === "queued" || j.state === "running"));

  // -- enqueue ----------------------------------------------------------------

  /** Queue a low-priority metadata/cover ingestion for a file. */
  enqueueIngest(book: Book, file: BookFile, argv: string[]): void {
    this.#enqueue(book, file, "show", argv, "low");
    this.#pump();
  }

  /**
   * Queue the automatic epub-profile check behind the "needs cleanup"
   * indicator. Low priority like an ingestion, and deliberately quieter: its
   * outcome lands on `file.cleanup`, never on `file.result`, and a failure
   * says nothing at all - an unasked-for probe must not paint a row red.
   */
  enqueueAutoCheck(book: Book, file: BookFile, argv: string[]): void {
    this.#enqueue(book, file, "autocheck", argv, "low");
    this.#pump();
  }

  /**
   * Start a conversion batch: one fit (or md) job per file, using its plan.
   * `editsByFile` (already de-proxied to plain objects by the caller)
   * supplies per-file staged metadata that is spliced into that file's argv.
   */
  runFit(
    targets: FileTarget[],
    plans: { input: string; output: string | null }[],
    opts: RunOptions,
    editsByFile: Record<string, StagedEdits> = {},
    fitMeta?: { profileName: string; appendix: string },
  ): void {
    this.#startBatch();
    const outputByInput = new Map(plans.map((p) => [p.input, p.output]));
    for (const { book, file } of targets) {
      // `null` means "in place" - a decision the caller makes, never a
      // fallback we invent. A file with no plan at all is skipped rather than
      // run: reading a missing entry as `null` would turn a planner bug into a
      // `--lets-get-dangerous` rewrite of an original the user never offered
      // up, and for a Markdown file into a flag the `md` command does not even
      // have. Neither is a mistake worth making quietly.
      if (!outputByInput.has(file.path)) continue;
      const output = outputByInput.get(file.path)!;
      if (output === null && file.kind !== "epub") continue;

      const fileEdits = editsByFile[file.id];
      const perFile: RunOptions = fileEdits ? { ...opts, edits: fileEdits } : opts;
      if (file.kind === "md") {
        this.#enqueue(book, file, "md", mdArgv(file.path, output, perFile), "normal");
      } else {
        this.#enqueue(book, file, "fit", fitArgv(file.path, output, perFile), "normal");
      }
      // A dry run writes nothing, so there is nothing to consume once it
      // settles: tracking it here would make #consumeEdits unstage (and, for
      // an in-place plan, fold into the row) edits the CLI was never asked
      // to write. Gating on `opts.dryRun` at enqueue time - rather than on
      // the settled report's own `dry_run` - keeps the one flag that decided
      // whether `--dry-run` went into this job's argv also the one flag that
      // decides whether its edits are ever tracked as applied.
      if (fileEdits && !opts.dryRun) {
        const jobId = this.#batchIds[this.#batchIds.length - 1];
        this.#applied.set(jobId, { edits: fileEdits, inPlace: output === null });
      }
      if (fitMeta && !opts.dryRun) {
        const jobId = this.#batchIds[this.#batchIds.length - 1];
        this.#fitMeta.set(jobId, fitMeta);
      }
    }
    this.#pump();
  }

  /** Start a check batch: lint each epub file against the given profile specs. */
  runCheck(targets: FileTarget[], profileSpecs: string[]): void {
    this.#startBatch();
    for (const { book, file } of targets) {
      if (file.kind === "epub") {
        this.#enqueue(book, file, "check", checkArgv(file.path, profileSpecs), "normal");
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

  /** The most recent conversion/check job for a file, for its row's live state. */
  conversionJobFor(fileId: string): Job | undefined {
    let found: Job | undefined;
    for (const job of this.jobs) {
      // Background probes (ingest, the automatic check) never drive a row's
      // live state - only work the user asked for does.
      if (job.fileId === fileId && job.kind !== "show" && job.kind !== "autocheck") found = job;
    }
    return found;
  }

  // -- internals --------------------------------------------------------------

  #enqueue(
    book: Book,
    file: BookFile,
    kind: JobKind,
    argv: string[],
    priority: "normal" | "low",
  ): void {
    const id = crypto.randomUUID();
    this.#refs.set(id, { book, file });
    this.jobs.push({ id, fileId: file.id, kind, argv, priority, state: "queued", stderrTail: [] });
    if (priority === "normal") this.#batchIds = [...this.#batchIds, id];
  }

  /**
   * Clear the decks for a new batch: keep only what is still queued or
   * running - a background ingestion included, since one may well be in flight
   * - and drop every job that has already settled, along with its file
   * back-reference and its applied-edits entry. Nothing needs them once they
   * are done: a settled job has already written its outcome onto the file
   * itself.
   */
  #startBatch(): void {
    const kept = this.jobs.filter((j) => !TERMINAL.has(j.state));
    const keptIds = new Set(kept.map((j) => j.id));
    for (const id of [...this.#refs.keys()]) {
      if (!keptIds.has(id)) this.#refs.delete(id);
    }
    for (const id of [...this.#applied.keys()]) {
      if (!keptIds.has(id)) this.#applied.delete(id);
    }
    for (const id of [...this.#fitMeta.keys()]) {
      if (!keptIds.has(id)) this.#fitMeta.delete(id);
    }
    this.jobs = kept;
    this.#batchIds = [];
  }

  #cancelJob(job: Job): void {
    job.state = "cancelled";
    this.#applied.delete(job.id);
    this.#fitMeta.delete(job.id);
    const file = this.#refs.get(job.id)?.file;
    if (file) {
      if (job.kind === "show") {
        file.ingest = "failed";
        file.ingestError = this.#ingestError(job, {
          code: "cancelled",
          message: "Reading this book's metadata was cancelled.",
        });
      } else if (job.kind !== "autocheck") {
        // A cancelled background probe just goes away; only user-asked work
        // leaves a "cancelled" mark on the row.
        file.result = { kind: "cancelled" };
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
      case "autocheck":
        this.#settleAutoCheck(job, res);
        break;
      default:
        this.#settleConvert(job, res);
        break;
    }
    this.#pump();
  }

  #settleShow(job: Job, res: SidecarResult): void {
    const file = this.#refs.get(job.id)?.file;
    if (res.code === 0 && file) {
      try {
        const report = parseReport<MetadataShowReport>(res.stdout, "metadata show");
        if (!isCliFailure(report)) {
          file.meta = normalizeMeta(report);
          if (report.metadata.cover) file.coverPath = report.metadata.cover;
          file.fitted = report.fitted ?? undefined;
          // A recognized copy is ingested after the fold, so the stamp
          // arrives too late for the regroup to read: the "stamp beats
          // appendix" refinement of its badge lands here instead.
          if (file.role === "copy" && report.fitted?.profile) {
            if (report.fitted.profile !== CLEANUP_PROFILE) file.profile = report.fitted.profile;
          }
          file.ingest = "done";
          file.ingestError = undefined;
          job.state = "done";
          return;
        }
      } catch {
        // Fall through to a failed ingest below.
      }
    }
    if (file) {
      file.ingest = "failed";
      file.ingestError = this.#ingestError(job, this.#failureOf(job, res));
    }
    job.state = "failed";
  }

  /**
   * The reason an ingestion failed, in the shape the row's details drawer
   * reads. Written onto the file (not left on the job) because the job is
   * pruned at the next batch while the row keeps saying "could not read" for
   * as long as the file is on the workbench - and a row that says so has to
   * be able to say why.
   */
  #ingestError(job: Job, failure: CliFailure): BookFile["ingestError"] {
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
          const ref = this.#refs.get(job.id);
          if (ref) {
            this.#consumeEdits(job.id, ref.file);
            this.#routeResult(job.id, ref.book, ref.file, report);
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
   * Land a settled fit where its report actually applies. The size chips
   * describe the OUTPUT, so: a preview (nothing written) and an in-place
   * rewrite mark the INPUT - it is what the report speaks about - while a
   * copy run marks the PRODUCED file and leaves the untouched input alone.
   * An in-place rewrite also invalidates the automatic check (the bytes just
   * changed, so the "needs cleanup" indicator is re-probed rather than left
   * lying), and a copy gets tracked in the book's file list with the profile
   * the run was made under. Whether the run was in place is read off the
   * report's own output path (the CLI reports where the file landed), not
   * re-derived from the plan.
   */
  #routeResult(jobId: string, book: Book, input: BookFile, report: FitReport): void {
    const meta = this.#fitMeta.get(jobId);
    this.#fitMeta.delete(jobId);
    if (report.output === null || report.dry_run) {
      input.result = { kind: "fit", report };
      return;
    }

    if (samePath(report.output, input.path)) {
      input.result = { kind: "fit", report };
      input.cleanup = undefined;
      if (input.kind === "epub") {
        this.#enqueue(book, input, "autocheck", checkArgv(input.path, [CLEANUP_PROFILE]), "low");
      }
      return;
    }

    const output = report.output;
    const existing = book.files.find((f) => samePath(f.path, output));
    let file: BookFile;
    if (existing) {
      // A re-fit of the same output: update the entry in place and KEEP ITS
      // ID - a fresh id would silently orphan the file's selection and any
      // staged edits. The bytes just changed, so everything read from the
      // old bytes is reset and re-probed.
      existing.profile = meta?.profileName ?? existing.profile;
      existing.appendix = meta?.appendix ?? existing.appendix;
      existing.origin = "fit";
      existing.cleanup = undefined;
      existing.fitted = undefined;
      existing.ingest = "pending";
      file = existing;
    } else {
      book.files.push({
        id: crypto.randomUUID(),
        path: output,
        fileName: output.slice(Math.max(output.lastIndexOf("/"), output.lastIndexOf("\\")) + 1),
        kind: "epub",
        role: "copy",
        profile: meta?.profileName ?? null,
        appendix: meta?.appendix ?? null,
        origin: "fit",
        size: 0,
        modifiedMs: 0,
        ingest: "pending",
      });
      // Hand the callback the proxied element, not the literal, so the stat
      // and ingest write-backs reach the reactive graph.
      file = book.files[book.files.length - 1];
    }
    file.result = { kind: "fit", report };
    this.onCopyTracked?.(book, file);
  }

  /**
   * A successful convert consumes its staged edits: unstage the fields it
   * actually wrote (the badge goes away once nothing is left) and, when the
   * file was rewritten in place, fold the written fields back into its row
   * so the title/cover reflect what is now on disk. A copy run leaves the
   * input file's own metadata untouched, so only the edits are unstaged
   * there. Unstaging by field rather than clearing the whole entry matters
   * for a file late in a batch: the user may have staged (or retyped) more
   * on it while this job's argv snapshot - taken back when the run was
   * clicked - sat in the queue, and none of that ever ran through the CLI.
   */
  #consumeEdits(jobId: string, file: BookFile): void {
    const applied = this.#applied.get(jobId);
    if (!applied) return;
    this.#applied.delete(jobId);
    edits.unstageApplied(file.id, applied.edits);
    if (applied.inPlace) {
      file.meta = mergeEditsIntoMeta(file.meta, applied.edits);
      if (applied.edits.coverPath) file.coverPath = applied.edits.coverPath;
    }
  }

  #settleCheck(job: Job, res: SidecarResult): void {
    if (res.code !== CHECK_UNREADABLE) {
      try {
        const report = parseReport<CheckReport>(res.stdout, "check");
        if (!isCliFailure(report)) {
          job.result = report;
          const file = this.#refs.get(job.id)?.file;
          if (file) file.result = { kind: "check", report };
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
   * The automatic check settles onto `file.cleanup` and nowhere else: it
   * never touches `file.result`, and any failure - unreadable file, garbled
   * output - marks only the job and leaves the row silent. The file's real
   * problems surface from the paths the user actually runs.
   */
  #settleAutoCheck(job: Job, res: SidecarResult): void {
    if (res.code !== CHECK_UNREADABLE) {
      try {
        const report = parseReport<CheckReport>(res.stdout, "check");
        if (!isCliFailure(report)) {
          job.result = report;
          const file = this.#refs.get(job.id)?.file;
          if (file) file.cleanup = report;
          job.state = "done";
          return;
        }
      } catch {
        // Fall through to the silent failure below.
      }
    }
    job.failure = this.#failureOf(job, res);
    job.state = "failed";
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
    this.#fitMeta.delete(job.id);
    const failure = this.#failureOf(job, res);
    job.failure = failure;
    const file = this.#refs.get(job.id)?.file;
    if (file) {
      file.result = {
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
    this.#fitMeta.delete(id);
    const job = this.jobs.find((j) => j.id === id);
    if (!job) return;
    const failure: CliFailure = { code: "io-error", message };
    job.failure = failure;
    const file = this.#refs.get(id)?.file;
    if (file) {
      if (job.kind === "show") {
        file.ingest = "failed";
        file.ingestError = this.#ingestError(job, failure);
      } else if (job.kind !== "autocheck") {
        // The automatic probe stays silent even here; see #settleAutoCheck.
        file.result = {
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
