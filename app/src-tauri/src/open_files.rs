//! Wiring for "open with EPUB Tailor": files the OS hands the app directly
//! (double-click, Open With, `open -a`, or a bare command line) rather than
//! ones the user drags onto the workbench. All three arrival paths - macOS's
//! `RunEvent::Opened`, this process's own first-launch argv, and a second
//! instance's argv relayed through `tauri-plugin-single-instance` - funnel
//! through [`push_and_emit`] so the frontend sees one shape regardless of
//! platform or timing.

use std::path::Path;
use std::sync::{Mutex, PoisonError};

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
/// A poisoned lock (some other code panicked while holding it) is recovered
/// from rather than propagated: the buffer is a plain `Vec<String>` that a
/// panic cannot leave half-written into an invalid state, and dropping a file
/// the user asked us to open - or panicking the whole app over it - is a worse
/// outcome than reading a list that is exactly as valid as it was before.
/// [`drain_pending_opens`] takes the same view, and the two must agree.
fn lock_pending(pending: &Mutex<Vec<String>>) -> std::sync::MutexGuard<'_, Vec<String>> {
    pending.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn push_and_emit(app: &AppHandle, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    {
        let state = app.state::<PendingOpens>();
        lock_pending(&state.0).extend(paths.iter().cloned());
    }
    // Best-effort: if the frontend is not listening yet this is simply not
    // observed, which is fine - `drain_pending_opens` is the reliable path
    // for that case.
    let _ = app.emit_to("main", "files-opened", paths);
}

/// Return and clear whatever [`push_and_emit`] has buffered so far. The
/// frontend calls this once on startup to pick up files the OS handed the
/// app before its `files-opened` listener existed. Poison-tolerant, for the
/// reason [`lock_pending`] gives.
#[tauri::command]
pub fn drain_pending_opens(state: State<PendingOpens>) -> Vec<String> {
    let mut pending = lock_pending(&state.0);
    std::mem::take(&mut *pending)
}

#[cfg(test)]
mod tests {
    use super::{filter_existing_candidates, is_candidate_arg, lock_pending, resolve_argv};
    use std::panic::AssertUnwindSafe;
    use std::path::PathBuf;
    use std::sync::Mutex;

    #[test]
    fn a_poisoned_lock_still_yields_the_buffered_paths() {
        let pending: Mutex<Vec<String>> = Mutex::new(vec!["book.epub".to_string()]);

        // Poison it the only way a mutex gets poisoned: panic while holding the
        // guard. The hook is muted for the duration so the deliberate panic
        // does not print a scary backtrace into an otherwise clean test run.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = pending.lock().expect("lock");
            panic!("poison the mutex");
        }));
        std::panic::set_hook(hook);
        assert!(pending.is_poisoned());

        let mut guard = lock_pending(&pending);
        guard.push("more.md".to_string());
        assert_eq!(*guard, vec!["book.epub".to_string(), "more.md".to_string()]);
    }

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

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "epub-tailor-app-open-files-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn touch(path: &std::path::Path) {
        std::fs::write(path, b"hello").expect("write file");
    }

    #[test]
    fn filter_existing_candidates_drops_qualifying_paths_that_do_not_exist() {
        let dir = scratch("exists");
        let here = dir.join("here.epub");
        touch(&here);
        let gone = dir.join("gone.epub").display().to_string();

        let result = filter_existing_candidates(&[here.display().to_string(), gone]);
        assert_eq!(result, vec![here.display().to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_argv_resolves_relative_paths_against_cwd_and_drops_argv0() {
        let dir = scratch("argv");
        touch(&dir.join("book.epub"));

        let argv = vec![
            "/usr/bin/epub-tailor-app".to_string(), // argv[0]: no qualifying extension
            "book.epub".to_string(),                // relative: resolved against cwd
            "--flag".to_string(),                   // rejected outright
            "missing.epub".to_string(),             // qualifies but does not exist
        ];
        let result = resolve_argv(&argv, &dir.display().to_string());
        assert_eq!(result, vec![dir.join("book.epub").display().to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_argv_passes_through_an_existing_absolute_path() {
        let dir = scratch("argv-abs");
        let book = dir.join("book.md");
        touch(&book);

        let result = resolve_argv(&[book.display().to_string()], "/some/other/cwd");
        assert_eq!(result, vec![book.display().to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }
}
