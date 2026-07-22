// What to ask before the window closes. The decision and the wording are pure
// and live here; the Tauri glue (window handle, close event, native dialog)
// is guardClose in api/window.ts.

/** What is at stake if the window closes now. All primitives, IPC-safe. */
export interface WorkbenchLoad {
  /** Books on the workbench. */
  books: number;
  /** Files with staged metadata edits that have not been written. */
  editedFiles: number;
  /** Whether a conversion batch is still queued or running. */
  jobsRunning: boolean;
}

export interface ClosePrompt {
  title: string;
  message: string;
}

/** The question to ask before closing, or null when nothing is at stake. */
export function closePrompt(load: WorkbenchLoad): ClosePrompt | null {
  if (load.books === 0) return null;

  const lines = [
    load.books === 1
      ? "You still have 1 book on the workbench."
      : `You still have ${load.books} books on the workbench.`,
  ];
  if (load.editedFiles > 0) {
    lines.push(
      load.editedFiles === 1
        ? "1 file has staged metadata edits that have not been written."
        : `${load.editedFiles} files have staged metadata edits that have not been written.`,
    );
  }
  if (load.jobsRunning) {
    lines.push("A conversion is still running and will be stopped.");
  }

  return { title: "Close EPUB Tailor?", message: lines.join("\n\n") };
}
