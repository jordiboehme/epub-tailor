//! Tauri command handlers: the few things the frontend cannot do for itself.
//!
//! All of them are registered in `lib.rs` via `tauri::generate_handler!`, and
//! they fall into three groups: expanding what was dropped or browsed in
//! (`expand_inputs`), answering the filesystem questions the planner and the
//! destination picker need answered (`paths_exist`, `list_removable_volumes`,
//! `is_appimage`), and the cover cache (`ensure_covers_dir`, `cache_cover`,
//! `sniff_cover_extension`, `export_cover`) - the only place the app writes to
//! disk itself, which is why the fs plugin is not installed at all.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tauri::Manager;

/// One file the workbench can queue: an EPUB or a Markdown source.
#[derive(serde::Serialize)]
pub struct InputEntry {
    pub path: String,
    /// `"epub"` or `"md"`.
    pub kind: String,
    pub size: u64,
    pub modified_ms: u64,
}

/// Whether `path`'s extension is one this app converts, ASCII case-insensitive.
pub(crate) fn classify(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    if ext.eq_ignore_ascii_case("epub") {
        Some("epub")
    } else if ext.eq_ignore_ascii_case("md") {
        Some("md")
    } else {
        None
    }
}

/// Whether an entry name starts a dotfile/dotdir - `.git`, `.DS_Store`, and
/// AppleDouble `._*` siblings all start with `.`, so one check catches all
/// three, matching `crates/cli/src/discover.rs`'s convention.
fn is_dotted(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

/// Canonicalize, dedupe and, if `path` still qualifies, push an [`InputEntry`]
/// for it. Any failure along the way (the entry vanished, permissions,
/// whatever) just skips this one file rather than failing the batch.
fn add_entry(path: &Path, seen: &mut HashSet<PathBuf>, out: &mut Vec<InputEntry>) {
    let Some(kind) = classify(path) else {
        return;
    };
    let Ok(canonical) = std::fs::canonicalize(path) else {
        return;
    };
    if !seen.insert(canonical.clone()) {
        return;
    }
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    out.push(InputEntry {
        path: canonical.display().to_string(),
        kind: kind.to_string(),
        size: metadata.len(),
        modified_ms,
    });
}

/// Walk `dir` for epub/md files: name-sorted, depth-first, dot entries and
/// symlinks skipped, subfolders entered only when `recursive`. Mirrors
/// `crates/cli/src/discover.rs`'s conventions so a folder dropped on the app
/// discovers the same files the CLI's own batch mode would.
fn walk_dir(dir: &Path, recursive: bool, seen: &mut HashSet<PathBuf>, out: &mut Vec<InputEntry>) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read_dir.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        if is_dotted(&entry.file_name()) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        // `file_type()` does not traverse links, so a symlink is reported as
        // one rather than as its target - exactly what "never follow" needs.
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            if recursive {
                walk_dir(&path, recursive, seen, out);
            }
        } else if file_type.is_file() {
            add_entry(&path, seen, out);
        }
    }
}

/// Expand `paths` (files and/or folders, e.g. from a drag-and-drop) into the
/// concrete epub/md files they name: a file passes through if its extension
/// matches; a folder is scanned top-level only, unless `recursive`. A
/// nonexistent path is skipped rather than failing the whole batch - the UI
/// is expected to have validated the drop already. Entries are deduped by
/// canonicalized path, so the same file passed twice (directly, and again via
/// its containing folder) appears once.
#[tauri::command]
pub fn expand_inputs(paths: Vec<String>, recursive: bool) -> Result<Vec<InputEntry>, String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for raw in paths {
        let path = PathBuf::from(raw);
        // `symlink_metadata` does not follow a symlink, so a nonexistent
        // target and a symlink are both visible here rather than silently
        // resolved through.
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            walk_dir(&path, recursive, &mut seen, &mut out);
        } else if meta.is_file() {
            add_entry(&path, &mut seen, &mut out);
        }
    }

    Ok(out)
}

/// Whether a file (or anything) already sits at each of `paths`, positionally.
/// The output planner needs this to number a name collision (`Book (2).epub`)
/// or dodge overwriting an existing file, and it needs it as one batched call
/// rather than a chatty round-trip per candidate. Existence follows symlinks -
/// a link pointing at a real file counts as occupied, because writing there
/// would still clobber the target.
#[tauri::command]
pub fn paths_exist(paths: Vec<String>) -> Vec<bool> {
    paths.iter().map(|p| Path::new(p).exists()).collect()
}

/// A removable volume the workbench can offer as a conversion destination.
#[derive(serde::Serialize)]
pub struct Volume {
    pub name: String,
    pub path: String,
}

/// macOS: every entry of `/Volumes` except the boot volume, which is really a
/// symlink back to `/`.
#[cfg(target_os = "macos")]
fn list_removable_volumes_impl() -> Vec<Volume> {
    let Ok(read_dir) = std::fs::read_dir("/Volumes") else {
        return Vec::new();
    };
    let mut out: Vec<Volume> = read_dir
        .filter_map(Result::ok)
        .filter(|entry| {
            std::fs::canonicalize(entry.path())
                .map(|canonical| canonical != Path::new("/"))
                .unwrap_or(false)
        })
        .map(|entry| Volume {
            name: entry.file_name().to_string_lossy().to_string(),
            path: entry.path().display().to_string(),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Linux: the removable-media conventions used by udisks2/most desktop
/// environments, deduped by canonical path since a distro may populate more
/// than one of these for the same mount.
#[cfg(target_os = "linux")]
fn list_removable_volumes_impl() -> Vec<Volume> {
    let user = std::env::var("USER").unwrap_or_default();
    let mut roots = Vec::new();
    if !user.is_empty() {
        roots.push(PathBuf::from(format!("/run/media/{user}")));
        roots.push(PathBuf::from(format!("/media/{user}")));
    }
    roots.push(PathBuf::from("/media"));

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for root in roots {
        let Ok(read_dir) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in read_dir.filter_map(Result::ok) {
            let path = entry.path();
            let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !seen.insert(canonical) {
                continue;
            }
            out.push(Volume {
                name: entry.file_name().to_string_lossy().to_string(),
                path: path.display().to_string(),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Windows: every drive letter whose type the firmware reports as removable.
#[cfg(target_os = "windows")]
fn list_removable_volumes_impl() -> Vec<Volume> {
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows::Win32::System::WindowsProgramming::DRIVE_REMOVABLE;
    use windows::core::PCWSTR;

    let mut out = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string that outlives
        // this call; `GetDriveTypeW` only reads through the pointer.
        let drive_type = unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) };
        if drive_type == DRIVE_REMOVABLE {
            out.push(Volume {
                name: format!("{}:", letter as char),
                path: root,
            });
        }
    }
    out
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn list_removable_volumes_impl() -> Vec<Volume> {
    Vec::new()
}

/// List volumes a book could be copied to (a plugged-in e-reader, an SD card,
/// ...). Never panics: any per-platform lookup failure just yields an empty
/// list rather than crashing the app.
#[tauri::command]
pub fn list_removable_volumes() -> Vec<Volume> {
    list_removable_volumes_impl()
}

/// Whether this build is running as an AppImage (Linux). Relevant to the
/// updater: an AppImage replaces itself in place rather than going through an
/// OS installer.
#[tauri::command]
pub fn is_appimage() -> bool {
    std::env::var_os("APPIMAGE").is_some()
}

/// The cover cache directory (`<app cache dir>/covers`), created if it does
/// not exist yet. TypeScript's `covers.ts` calls this instead of the app
/// pulling in the (otherwise unused) fs plugin just to create one directory.
#[tauri::command]
pub fn ensure_covers_dir(app: tauri::AppHandle) -> Result<String, String> {
    let dir = covers_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.display().to_string())
}

fn covers_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("covers"))
}

/// FNV-1a (32-bit) over `input`'s UTF-8 bytes - the same small, dependency-free
/// hash `covers.ts` uses, but deliberately *not* an agreement between the two:
/// this one keys the covers the user picks (`picked-<hash>.<ext>`) off a source
/// path, that one keys the covers an ingest writes off `path|size|mtime`, and
/// it hashes UTF-16 code units, so the two diverge on any non-ASCII input. The
/// two filename namespaces are disjoint, which is what makes that harmless -
/// neither side ever has to reproduce the other's key.
fn fnv1a(input: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in input.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// The extension a cached copy should carry, lowercased and reduced to plain
/// ASCII alphanumerics (an extension only ever reaches a filename we build, so
/// anything stranger is dropped rather than escaped). `img` when the source has
/// no usable one - the CLI re-encodes a cover under the device profile anyway,
/// and the app's ingested covers already use that name.
fn cover_extension(source: &Path) -> String {
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            e.to_ascii_lowercase()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
        })
        .unwrap_or_default();
    if ext.is_empty() {
        "img".to_string()
    } else {
        ext
    }
}

/// Copy a user-chosen cover image into the cover cache and return the copy's
/// path.
///
/// The webview may only load images from `$APPCACHE/covers/**` (the asset
/// protocol's scope in tauri.conf.json), so a cover picked from anywhere else
/// on disk would stage fine and then silently fail to render - the card would
/// quietly fall back to its initials. Copying it into the cache first means the
/// one path the app then carries around (the `--cover` flag's argument, the
/// staged edit, the card's thumbnail) is a path everything can actually read.
///
/// The name is derived from the source path, size and mtime, so re-picking the
/// same unchanged image lands on the same cached file instead of littering the
/// cache.
#[tauri::command]
pub fn cache_cover(app: tauri::AppHandle, source: String) -> Result<String, String> {
    let source = PathBuf::from(source);
    let metadata =
        std::fs::metadata(&source).map_err(|e| format!("cannot read {}: {e}", source.display()))?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let key = fnv1a(&format!(
        "{}|{}|{}",
        source.display(),
        metadata.len(),
        modified_ms
    ));

    let dir = covers_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let target = dir.join(format!("picked-{key:08x}.{}", cover_extension(&source)));
    std::fs::copy(&source, &target)
        .map_err(|e| format!("cannot cache {}: {e}", source.display()))?;
    Ok(target.display().to_string())
}

/// The image extension a cached cover actually is, sniffed from its first
/// bytes. Extracted covers are cached as `<hash>.img` no matter what the EPUB
/// held, so the save dialog would otherwise suggest a filename no image viewer
/// wants to claim. Falls back to the file's own extension, then to `jpg`.
#[tauri::command]
pub fn sniff_cover_extension(path: String) -> Result<String, String> {
    let path = PathBuf::from(path);
    let mut head = [0u8; 12];
    let n = std::fs::File::open(&path)
        .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let head = &head[..n];
    let sniffed = if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if head.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if head.starts_with(b"GIF8") {
        Some("gif")
    } else if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    };
    if let Some(ext) = sniffed {
        return Ok(ext.to_string());
    }
    let fallback = cover_extension(&path);
    Ok(if fallback == "img" {
        "jpg".to_string()
    } else {
        fallback
    })
}

/// Copy a cached cover to a destination the user chose in a save dialog. The
/// inverse of `cache_cover`, with the same narrow contract: the source must
/// live inside the cover cache, so the frontend can never use this to copy
/// arbitrary files around.
#[tauri::command]
pub fn export_cover(
    app: tauri::AppHandle,
    source: String,
    destination: String,
) -> Result<(), String> {
    let source = PathBuf::from(source)
        .canonicalize()
        .map_err(|e| format!("cannot resolve source: {e}"))?;
    let dir = covers_dir(&app)?
        .canonicalize()
        .map_err(|e| format!("cannot resolve cover cache: {e}"))?;
    if !source.starts_with(&dir) {
        return Err("source is not a cached cover".to_string());
    }
    std::fs::copy(&source, Path::new(&destination))
        .map_err(|e| format!("cannot save to {destination}: {e}"))?;
    Ok(())
}

/// The sibling path an in-place write's safety copy is staged at:
/// `<stem> (backup).<ext>`, numbered `(backup 2)`, `(backup 3)`, ... until a
/// free name is found. The copy stays on the original's volume so the OS
/// trash can take it (and restore it next to the original). Pure - existence
/// is injected - so the naming is testable without touching disk.
fn backup_sibling(path: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let ext = path.extension().and_then(|e| e.to_str());
    let sibling = |marker: &str| {
        let name = match ext {
            Some(ext) => format!("{stem} ({marker}).{ext}"),
            None => format!("{stem} ({marker})"),
        };
        path.with_file_name(name)
    };
    let mut candidate = sibling("backup");
    let mut n = 2u32;
    while exists(&candidate) {
        candidate = sibling(&format!("backup {n}"));
        n += 1;
    }
    candidate
}

/// Move a file to the OS trash. On macOS this deliberately uses the
/// NSFileManager API instead of the crate's default Finder AppleScript: the
/// Finder route pops a "wants to control Finder" permission dialog the first
/// time - a scary ask in the middle of a save - and all it buys is the
/// Trash's "Put Back" menu entry. Dragging the file back out works either way.
fn move_to_trash(path: &Path) -> Result<(), trash::Error> {
    #[cfg(target_os = "macos")]
    {
        use trash::TrashContext;
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut ctx = TrashContext::default();
        ctx.set_delete_method(DeleteMethod::NsFileManager);
        ctx.delete(path)
    }
    #[cfg(not(target_os = "macos"))]
    trash::delete(path)
}

/// How [`backup_to_trash`] preserved the safety copy.
#[derive(serde::Serialize)]
pub struct BackupOutcome {
    /// `"trash"` - the copy is in the OS trash - or `"file"` - the volume has
    /// no trash, so the copy stays as a plain sibling file.
    pub method: String,
    /// The sibling name the copy was staged under (its restore name in the
    /// trash, or where it still sits when `method` is `"file"`).
    pub backup_path: String,
}

/// Preserve a copy of `path` before an in-place write rewrites it: copy it to
/// a `<stem> (backup).epub` sibling, then move that sibling to the OS trash.
/// On a volume without a trash the sibling is kept in place and reported as
/// `method: "file"`. An `Err` means nothing was preserved - the caller must
/// not proceed with the overwrite.
#[tauri::command]
pub fn backup_to_trash(path: String) -> Result<BackupOutcome, String> {
    let original = PathBuf::from(&path);
    let backup = backup_sibling(&original, |p| p.exists());
    std::fs::copy(&original, &backup)
        .map_err(|e| format!("cannot back up {}: {e}", original.display()))?;
    let method = match move_to_trash(&backup) {
        Ok(()) => "trash",
        // Typically a volume with no trash directory (some network/exFAT
        // mounts): the sibling copy is still a valid backup, just visible.
        Err(_) => "file",
    };
    Ok(BackupOutcome {
        method: method.to_string(),
        backup_path: backup.display().to_string(),
    })
}

/// Move a file to the OS trash - used when the user deletes a tracked copy
/// from a book card. Never deletes permanently.
#[tauri::command]
pub fn trash_file(path: String) -> Result<(), String> {
    move_to_trash(Path::new(&path)).map_err(|e| format!("cannot move {path} to the trash: {e}"))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{InputEntry, expand_inputs, is_appimage, paths_exist};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "epub-tailor-app-commands-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).expect("create parent");
        std::fs::write(path, b"hello").expect("write file");
    }

    /// The entries' file names, sorted - for the assertions that only care
    /// *which* files were found. Traversal order has its own test below.
    fn file_names(entries: &[InputEntry]) -> Vec<String> {
        let mut names = names_in_order(entries);
        names.sort();
        names
    }

    /// The entries' file names exactly as `expand_inputs` returned them.
    fn names_in_order(entries: &[InputEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| {
                Path::new(&e.path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn walks_a_folder_in_name_order_depth_first() {
        // Mirrors `crates/cli/src/discover.rs`'s
        // `finds_matching_files_in_name_order`: the app's own expansion must
        // hand the workbench the same books in the same order the CLI's batch
        // mode would, so a dropped folder is not shuffled by whatever order
        // the filesystem happens to hand back.
        let dir = scratch("order");
        touch(&dir.join("b.epub"));
        touch(&dir.join("a.epub"));
        touch(&dir.join("c.md"));
        touch(&dir.join("notes.txt"));
        // `alpha` sorts before every top-level file, and is descended into
        // where it sits - depth-first, not after everything else.
        touch(&dir.join("alpha/inner-b.epub"));
        touch(&dir.join("alpha/inner-a.epub"));

        let entries = expand_inputs(vec![dir.display().to_string()], true).expect("expand_inputs");
        assert_eq!(
            names_in_order(&entries),
            vec!["a.epub", "inner-a.epub", "inner-b.epub", "b.epub", "c.md"]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preserves_the_order_of_the_paths_it_was_given() {
        let dir = scratch("arg-order");
        touch(&dir.join("z.epub"));
        touch(&dir.join("a.epub"));

        let entries = expand_inputs(
            vec![
                dir.join("z.epub").display().to_string(),
                dir.join("a.epub").display().to_string(),
            ],
            false,
        )
        .expect("expand_inputs");
        assert_eq!(names_in_order(&entries), vec!["z.epub", "a.epub"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finds_top_level_files_case_insensitively_by_extension() {
        let dir = scratch("flat");
        touch(&dir.join("a.epub"));
        touch(&dir.join("B.MD"));
        touch(&dir.join("SHOUTY.EPUB"));
        touch(&dir.join("notes.txt"));

        let entries = expand_inputs(vec![dir.display().to_string()], false).expect("expand_inputs");
        let mut kinds: Vec<(String, String)> = entries
            .iter()
            .map(|e| {
                (
                    Path::new(&e.path)
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                    e.kind.clone(),
                )
            })
            .collect();
        kinds.sort();
        assert_eq!(
            kinds,
            vec![
                ("B.MD".to_string(), "md".to_string()),
                ("SHOUTY.EPUB".to_string(), "epub".to_string()),
                ("a.epub".to_string(), "epub".to_string()),
            ]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn skips_dotfiles_dotdirs_and_appledouble_files() {
        let dir = scratch("dots");
        touch(&dir.join(".hidden/secret.epub"));
        touch(&dir.join("._resource.epub"));
        touch(&dir.join("visible.epub"));

        let entries = expand_inputs(vec![dir.display().to_string()], true).expect("expand_inputs");
        assert_eq!(file_names(&entries), vec!["visible.epub"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn recursive_walks_nested_dirs_non_recursive_does_not() {
        let dir = scratch("recurse");
        touch(&dir.join("top.epub"));
        touch(&dir.join("sub/deep/nested.epub"));

        let flat = expand_inputs(vec![dir.display().to_string()], false).expect("flat scan");
        assert_eq!(file_names(&flat), vec!["top.epub"]);

        let deep = expand_inputs(vec![dir.display().to_string()], true).expect("recursive scan");
        assert_eq!(file_names(&deep), vec!["nested.epub", "top.epub"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn never_follows_symlinked_dirs_or_files() {
        let dir = scratch("symlinks");
        touch(&dir.join("real/linked.epub"));
        touch(&dir.join("plain.epub"));
        std::os::unix::fs::symlink(dir.join("real"), dir.join("linked-dir")).expect("dir link");
        std::os::unix::fs::symlink(dir.join("plain.epub"), dir.join("alias.epub"))
            .expect("file link");

        let entries = expand_inputs(vec![dir.display().to_string()], true).expect("expand_inputs");
        assert_eq!(file_names(&entries), vec!["linked.epub", "plain.epub"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dedupes_a_file_passed_directly_and_via_its_folder() {
        let dir = scratch("dedupe");
        touch(&dir.join("dup.epub"));

        let entries = expand_inputs(
            vec![
                dir.join("dup.epub").display().to_string(),
                dir.display().to_string(),
            ],
            false,
        )
        .expect("expand_inputs");
        assert_eq!(file_names(&entries), vec!["dup.epub"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn skips_nonexistent_paths_silently() {
        let dir = scratch("missing");

        let entries = expand_inputs(vec![dir.join("nope.epub").display().to_string()], false)
            .expect("expand_inputs should not error on a missing path");
        assert!(entries.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn collects_size_and_a_recent_modified_time() {
        let dir = scratch("stat");
        touch(&dir.join("book.epub"));

        let entries = expand_inputs(vec![dir.display().to_string()], false).expect("expand_inputs");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].size, 5); // "hello"
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        // Written moments ago; allow generous slack for a slow CI disk.
        assert!(entries[0].modified_ms > 0);
        assert!(entries[0].modified_ms <= now_ms);
        assert!(now_ms - entries[0].modified_ms < 60_000);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_appimage_reflects_the_env_var() {
        // SAFETY: this test crate is single-threaded per test binary process
        // for env var mutation purposes here; no other test reads APPIMAGE.
        unsafe {
            std::env::remove_var("APPIMAGE");
        }
        assert!(!is_appimage());

        unsafe {
            std::env::set_var("APPIMAGE", "/path/to/App.AppImage");
        }
        assert!(is_appimage());

        unsafe {
            std::env::remove_var("APPIMAGE");
        }
    }

    #[test]
    fn list_removable_volumes_does_not_panic() {
        // A smoke test: whatever this machine has plugged in, the call must
        // return rather than panic.
        let _ = super::list_removable_volumes();
    }

    #[test]
    fn cover_extension_normalizes_or_falls_back_to_img() {
        use super::cover_extension;
        assert_eq!(cover_extension(Path::new("/a/cover.PNG")), "png");
        assert_eq!(cover_extension(Path::new("/a/cover.jpeg")), "jpeg");
        // No extension, and an extension made of nothing we would put in a
        // filename, both land on the neutral name the ingested covers use.
        assert_eq!(cover_extension(Path::new("/a/cover")), "img");
        assert_eq!(cover_extension(Path::new("/a/cover. ./")), "img");
    }

    #[test]
    fn fnv1a_matches_the_canonical_reference_vectors() {
        // The canonical FNV-1a 32-bit vectors. What is being pinned is that
        // this side is a real, stable FNV-1a - not that it agrees with the
        // TypeScript one, which hashes UTF-16 code units of a different input
        // into a different set of filenames (see the note on `fnv1a`).
        assert_eq!(super::fnv1a("a"), 0xe40c_292c);
        assert_eq!(super::fnv1a(""), 0x811c_9dc5);
    }

    #[test]
    fn backup_sibling_names_the_copy_next_to_the_original() {
        let name = super::backup_sibling(Path::new("/books/Dune.epub"), |_| false);
        assert_eq!(name, PathBuf::from("/books/Dune (backup).epub"));
    }

    #[test]
    fn backup_sibling_numbers_taken_names() {
        let taken = ["/books/Dune (backup).epub", "/books/Dune (backup 2).epub"];
        let name = super::backup_sibling(Path::new("/books/Dune.epub"), |p| {
            taken.contains(&p.to_str().unwrap())
        });
        assert_eq!(name, PathBuf::from("/books/Dune (backup 3).epub"));
    }

    #[test]
    fn backup_sibling_keeps_multi_dot_stems_and_odd_extensions() {
        let name = super::backup_sibling(Path::new("/b/My.Novel.x4.epub"), |_| false);
        assert_eq!(name, PathBuf::from("/b/My.Novel.x4 (backup).epub"));

        // No extension: the marker just lands at the end of the name.
        let bare = super::backup_sibling(Path::new("/b/README"), |_| false);
        assert_eq!(bare, PathBuf::from("/b/README (backup)"));
    }

    #[test]
    fn paths_exist_reports_presence_positionally() {
        let dir = scratch("exists");
        let here = dir.join("here.epub");
        touch(&here);
        let gone = dir.join("gone.epub");

        let result = paths_exist(vec![
            here.display().to_string(),
            gone.display().to_string(),
            here.display().to_string(),
        ]);
        assert_eq!(result, vec![true, false, true]);

        std::fs::remove_dir_all(&dir).ok();
    }
}
