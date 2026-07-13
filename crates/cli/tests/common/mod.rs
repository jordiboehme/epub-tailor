//! Helpers shared by the CLI integration tests.

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_epub-tailor"))
}

pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("epub-tailor-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Build a one-chapter EPUB by running `md`, the way the other tests do.
pub fn book_in(dir: &Path, name: &str) -> PathBuf {
    let md = dir.join(format!("{name}.md"));
    std::fs::write(
        &md,
        "---\ntitle: A Book\nauthor: Jane Author\n---\n\n# One\n\nHello.\n",
    )
    .expect("write markdown");
    let out = dir.join(format!("{name}.epub"));
    let status = bin()
        .args(["md", md.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("failed to run binary");
    assert!(status.status.success(), "md should build a book");
    out
}
