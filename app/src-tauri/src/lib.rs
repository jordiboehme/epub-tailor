mod commands;
mod open_files;

use tauri::{Manager, RunEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    // Must be the first plugin registered: tauri-plugin-single-instance's own
    // docs note that plugins run in registration order, and this one needs
    // to see (and, for a genuine second launch, kill) the new process before
    // anything else does its thing. Handles the Windows/Linux side of "open
    // with EPUB Tailor while it is already running" - a file association or
    // `epub-tailor-app book.epub` there spawns a full second process, whose
    // argv this callback receives before that process exits.
    //
    // The cfg matches the target gate on the dependency itself (Cargo.toml)
    // rather than tauri's broader `desktop` alias, so the two can never
    // disagree about whether the crate is even there to call.
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let paths = open_files::resolve_argv(&argv, &cwd);
            open_files::push_and_emit(app, paths);
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }));
    }

    builder
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .manage(open_files::PendingOpens::default())
        .invoke_handler(tauri::generate_handler![
            commands::expand_inputs,
            commands::paths_exist,
            commands::list_removable_volumes,
            commands::is_appimage,
            commands::ensure_covers_dir,
            open_files::drain_pending_opens,
        ])
        .setup(|app| {
            // First launch, Windows/Linux: a file-association launch (or a
            // bare `epub-tailor-app book.epub` from a shell) hands us the
            // file as our own argv, with no separate event marking it - so
            // read it once, here. macOS instead delivers this via
            // `RunEvent::Opened` below, both at launch and while running, so
            // this scan is a no-op there (its own argv never carries files).
            let args: Vec<String> = std::env::args().skip(1).collect();
            let paths = open_files::filter_existing_candidates(&args);
            open_files::push_and_emit(app.handle(), paths);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, _event| {
            // Leading underscores: both params are genuinely unused on
            // Windows/Linux, where `RunEvent::Opened` does not exist - the
            // variant, like this whole branch, is macOS/iOS-only.
            #[cfg(target_os = "macos")]
            if let RunEvent::Opened { urls } = _event {
                let paths = open_files::urls_to_paths(&urls);
                open_files::push_and_emit(_app_handle, paths);
            }
        });
}
