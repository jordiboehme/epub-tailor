//! Wiring for "open with EPUB Tailor": files the OS hands the app directly
//! (double-click, Open With, `open -a`, or a bare command line) rather than
//! ones the user drags onto the workbench. All three arrival paths - macOS's
//! `RunEvent::Opened`, this process's own first-launch argv, and a second
//! instance's argv relayed through `tauri-plugin-single-instance` - funnel
//! through [`push_and_emit`] so the frontend sees one shape regardless of
//! platform or timing.

use std::path::Path;
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, State, Url};

use crate::commands::classify;

/// Files the OS handed the app before the frontend had a chance to attach its
/// `files-opened` listener. Drained once on startup via
/// [`drain_pending_opens`], so a cold start racing the listener never loses a
/// file; every arrival also emits `files-opened` for an already-running app.
#[derive(Default)]
pub struct PendingOpens(Mutex<Vec<String>>);

/// Whether `arg`'s text alone makes it a plausible file to open: not a CLI
/// flag (`-`-prefixed) and an epub/md extension, ASCII case-insensitive (via
/// [`classify`], the same rule `expand_inputs` uses). Existence is
/// deliberately not checked here, so this stays pure and unit-testable;
/// [`filter_existing_candidates`] adds that filesystem check.
fn is_candidate_arg(arg: &str) -> bool {
    !arg.starts_with('-') && classify(Path::new(arg)).is_some()
}

/// `args` filtered down to the ones [`is_candidate_arg`] accepts and that
/// also exist on disk right now.
pub fn filter_existing_candidates(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|arg| is_candidate_arg(arg))
        .filter(|arg| Path::new(arg).exists())
        .cloned()
        .collect()
}

/// Resolve a second instance's argv against its cwd (a relative path only
/// means something there), then apply the same filter as the first launch.
/// `argv[0]` is that process's own executable path, which never has an
/// epub/md extension, so it drops out of [`is_candidate_arg`] naturally
/// without special-casing it.
pub fn resolve_argv(argv: &[String], cwd: &str) -> Vec<String> {
    let cwd = Path::new(cwd);
    let resolved: Vec<String> = argv
        .iter()
        .map(|arg| {
            let path = Path::new(arg);
            if path.is_absolute() {
                arg.clone()
            } else {
                cwd.join(path).display().to_string()
            }
        })
        .collect();
    filter_existing_candidates(&resolved)
}

/// Convert the `file://` URLs macOS hands the app via `RunEvent::Opened` into
/// plain paths, dropping anything that is not a `file://` URL or whose
/// extension does not qualify.
pub fn urls_to_paths(urls: &[Url]) -> Vec<String> {
    urls.iter()
        .filter_map(|url| url.to_file_path().ok())
        .map(|path| path.display().to_string())
        .filter(|path| is_candidate_arg(path))
        .collect()
}

/// Push `paths` into the pending-opens buffer (for a [`drain_pending_opens`]
/// that has not run yet) and emit `files-opened` to the main window (for one
/// that is already up and listening). A no-op if `paths` is empty, so a
/// launch with nothing to open never bothers the frontend.
pub fn push_and_emit(app: &AppHandle, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    if let Ok(mut pending) = app.state::<PendingOpens>().0.lock() {
        pending.extend(paths.iter().cloned());
    }
    // Best-effort: if the frontend is not listening yet this is simply not
    // observed, which is fine - `drain_pending_opens` is the reliable path
    // for that case.
    let _ = app.emit_to("main", "files-opened", paths);
}

/// Return and clear whatever [`push_and_emit`] has buffered so far. The
/// frontend calls this once on startup to pick up files the OS handed the
/// app before its `files-opened` listener existed.
#[tauri::command]
pub fn drain_pending_opens(state: State<PendingOpens>) -> Vec<String> {
    std::mem::take(&mut *state.0.lock().unwrap())
}

#[cfg(test)]
mod tests {
    use super::is_candidate_arg;

    #[test]
    fn accepts_epub_and_md_case_insensitively() {
        assert!(is_candidate_arg("book.epub"));
        assert!(is_candidate_arg("BOOK.EPUB"));
        assert!(is_candidate_arg("notes.md"));
        assert!(is_candidate_arg("NOTES.MD"));
        assert!(is_candidate_arg("/abs/path/to/Book.Epub"));
    }

    #[test]
    fn rejects_leading_dash_flags() {
        assert!(!is_candidate_arg("--flag"));
        assert!(!is_candidate_arg("-v"));
        // Even a flag-shaped arg that happens to end in a qualifying
        // extension is rejected - the leading `-` wins.
        assert!(!is_candidate_arg("--foo.epub"));
    }

    #[test]
    fn rejects_other_extensions_and_extensionless_names() {
        assert!(!is_candidate_arg("image.png"));
        assert!(!is_candidate_arg("archive.zip"));
        assert!(!is_candidate_arg("noext"));
        assert!(!is_candidate_arg(""));
    }
}
