//! Filesystem discovery for batch mode: which files inside a scanned folder
//! a subcommand should process.

use std::io;
use std::path::{Path, PathBuf};

/// A file found under a scanned root, with its path relative to that root so
/// an output tree can mirror the input tree.
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub relative: PathBuf,
}

/// Walk `root` for files with `extension` (ASCII case-insensitive, given
/// without the dot). Entries are visited in file-name order, depth first, so
/// every run sees the same sequence. Dot entries are skipped (`.git`,
/// `.DS_Store`, AppleDouble `._*` files), symlinks are never followed, and
/// subfolders are entered only when `recursive`.
pub fn discover(root: &Path, extension: &str, recursive: bool) -> io::Result<Vec<DiscoveredFile>> {
    let mut found = Vec::new();
    walk(root, root, extension, recursive, &mut found)?;
    Ok(found)
}

fn walk(
    root: &Path,
    dir: &Path,
    extension: &str,
    recursive: bool,
    found: &mut Vec<DiscoveredFile>,
) -> io::Result<()> {
    let mut entries = std::fs::read_dir(dir)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        // `file_type()` does not traverse links, so a symlink is reported as
        // one rather than as its target - exactly what "never follow" needs.
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            if recursive {
                walk(root, &path, extension, recursive, found)?;
            }
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(extension))
        {
            let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            found.push(DiscoveredFile { path, relative });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "epub-tailor-discover-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).expect("create parent");
        std::fs::write(path, b"x").expect("write file");
    }

    fn names(found: &[DiscoveredFile]) -> Vec<String> {
        found
            .iter()
            .map(|f| f.relative.to_string_lossy().replace('\\', "/"))
            .collect()
    }

    #[test]
    fn finds_matching_files_in_name_order() {
        let dir = scratch("order");
        touch(&dir.join("b.epub"));
        touch(&dir.join("a.epub"));
        touch(&dir.join("notes.txt"));
        touch(&dir.join("c.md"));

        let found = discover(&dir, "epub", false).expect("discover");
        assert_eq!(names(&found), ["a.epub", "b.epub"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        let dir = scratch("case");
        touch(&dir.join("SHOUTY.EPUB"));

        let found = discover(&dir, "epub", false).expect("discover");
        assert_eq!(names(&found), ["SHOUTY.EPUB"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn descends_only_when_recursive_and_reports_relative_paths() {
        let dir = scratch("recurse");
        touch(&dir.join("top.epub"));
        touch(&dir.join("sub/deep.epub"));

        let flat = discover(&dir, "epub", false).expect("discover");
        assert_eq!(names(&flat), ["top.epub"]);

        let deep = discover(&dir, "epub", true).expect("discover");
        assert_eq!(names(&deep), ["sub/deep.epub", "top.epub"]);
        let nested = deep
            .iter()
            .find(|f| f.relative == Path::new("sub/deep.epub"))
            .expect("nested file");
        assert_eq!(nested.path, dir.join("sub/deep.epub"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn skips_dot_files_and_dot_dirs() {
        let dir = scratch("dots");
        touch(&dir.join("._resource-fork.epub"));
        touch(&dir.join(".hidden/secret.epub"));
        touch(&dir.join("visible.epub"));

        let found = discover(&dir, "epub", true).expect("discover");
        assert_eq!(names(&found), ["visible.epub"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn never_follows_symlinks() {
        let dir = scratch("symlinks");
        touch(&dir.join("real/linked.epub"));
        touch(&dir.join("plain.epub"));
        std::os::unix::fs::symlink(dir.join("real"), dir.join("linked-dir")).expect("dir link");
        std::os::unix::fs::symlink(dir.join("plain.epub"), dir.join("alias.epub"))
            .expect("file link");

        let found = discover(&dir, "epub", true).expect("discover");
        assert_eq!(names(&found), ["plain.epub", "real/linked.epub"]);

        std::fs::remove_dir_all(&dir).ok();
    }
}
