//! End-to-end tests for the T5 `--tables image` / `--tables image-all` wiring:
//! the per-table heuristic, sentinel rasterization, and the safety fallback to
//! linearization. The fixture book carries three top-level tables (a 3-col
//! table, a 2-col simple table, and a table holding a link-referenced anchor).
//! The big fixture is validated with epubcheck (skip-if-unavailable; it IS
//! available at /opt/homebrew/bin/epubcheck).

mod common;

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{epub3_nested_tables, epub3_tables};
use epub_tailor_core::{ConvertOptions, Input, TableMode, convert};
use zip::ZipArchive;

fn zip_names(epub: &[u8]) -> Vec<String> {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("valid zip");
    (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect()
}

fn read_entry(epub: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("valid zip");
    let mut file = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("output should contain {name}"));
    let mut out = String::new();
    file.read_to_string(&mut out).expect("entry is UTF-8");
    out
}

fn entry_bytes(epub: &[u8], name: &str) -> Vec<u8> {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("valid zip");
    let mut file = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("missing {name}"));
    let mut out = Vec::new();
    file.read_to_end(&mut out).expect("read entry");
    out
}

/// The PNG file signature.
const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

fn opts_with(tables: TableMode) -> ConvertOptions {
    ConvertOptions {
        tables,
        ..ConvertOptions::default()
    }
}

fn kinds(converted: &epub_tailor_core::Converted) -> Vec<&str> {
    converted
        .report
        .transformations
        .iter()
        .map(|t| t.kind.as_str())
        .collect()
}

#[test]
fn image_mode_rasterizes_complex_linearizes_simple_and_keeps_anchors() {
    let converted = convert(Input::Epub(epub3_tables()), &opts_with(TableMode::Image))
        .expect("conversion should succeed");
    let epub = &converted.epub;

    let chapter = read_entry(epub, "OEBPS/text/chapter.xhtml");

    // (a) The 3-column table became a single rasterized image; NO <table>
    // survives anywhere in the chapter.
    assert!(
        !chapter.contains("<table"),
        "no <table> may survive under --tables image:\n{chapter}"
    );
    // A rendered table is synthetic black-on-white text/lines and must encode
    // as crisp line-art PNG, never a soft photo-classified JPEG.
    assert!(
        chapter.contains("chapter-table-1.png"),
        "the 3-col table should become an <img> at chapter-table-1.png:\n{chapter}"
    );

    // The caption's `& < "` special characters survive, escaped, in the alt.
    assert!(
        chapter.contains(r#"alt="Fruit &amp; &quot;Veg&quot; &lt;prices&gt;""#),
        "the caption should survive as an escaped alt attribute:\n{chapter}"
    );

    // The rasterized resource is a real PNG, present in the zip and manifest.
    assert!(
        entry_bytes(epub, "OEBPS/text/chapter-table-1.png").starts_with(&PNG_MAGIC),
        "chapter-table-1.png must carry the PNG magic bytes"
    );
    let opf = read_entry(epub, "OEBPS/content.opf");
    assert!(
        opf.contains("chapter-table-1.png"),
        "the manifest should declare chapter-table-1.png:\n{opf}"
    );

    // (b) The 2-col simple table linearized to paragraphs (its cell text
    // survives, its markup does not).
    for text in ["alpha", "beta", "gamma", "delta"] {
        assert!(
            chapter.contains(text),
            "the simple table's cell text {text} should survive linearization:\n{chapter}"
        );
    }

    // (c) The anchor-bearing table linearized, and the referenced id survived
    // so the link still resolves.
    assert!(
        chapter.contains("reftarget"),
        "the referenced anchor id must survive linearization:\n{chapter}"
    );

    // The report records both a rasterization and a linearization.
    let kinds = kinds(&converted);
    assert!(
        kinds.contains(&"table-rasterized"),
        "expected a table-rasterized transformation: {kinds:?}"
    );
    assert!(
        kinds.contains(&"table-linearized"),
        "expected a table-linearized transformation: {kinds:?}"
    );

    assert_epubcheck_clean("tables-image", epub);
}

#[test]
fn image_all_mode_rasterizes_the_simple_table_but_still_linearizes_anchors() {
    let converted = convert(Input::Epub(epub3_tables()), &opts_with(TableMode::ImageAll))
        .expect("conversion should succeed");
    let epub = &converted.epub;

    let chapter = read_entry(epub, "OEBPS/text/chapter.xhtml");
    assert!(
        !chapter.contains("<table"),
        "no <table> may survive under --tables image-all:\n{chapter}"
    );

    // Both the 3-col table (A) and the simple 2-col table (B) rasterized, so
    // two table images exist - both crisp line-art PNGs.
    let table_imgs: Vec<String> = zip_names(epub)
        .into_iter()
        .filter(|n| n.contains("/chapter-table-"))
        .collect();
    assert_eq!(
        table_imgs.len(),
        2,
        "ImageAll should rasterize both A and B, got: {table_imgs:?}"
    );
    for name in &table_imgs {
        assert!(
            name.ends_with(".png"),
            "a rendered table must encode as PNG, got: {name}"
        );
        assert!(
            entry_bytes(epub, name).starts_with(&PNG_MAGIC),
            "{name} must carry the PNG magic bytes"
        );
    }

    // (c) The anchor-bearing table STILL linearized under ImageAll, id kept.
    assert!(
        chapter.contains("reftarget"),
        "the anchor table must still linearize under ImageAll:\n{chapter}"
    );
    let kinds = kinds(&converted);
    assert!(
        kinds.contains(&"table-rasterized") && kinds.contains(&"table-linearized"),
        "ImageAll should still record both kinds: {kinds:?}"
    );

    assert_epubcheck_clean("tables-image-all", epub);
}

#[test]
fn nested_table_rasterizes_to_one_image_marked_nested() {
    let converted = convert(
        Input::Epub(epub3_nested_tables()),
        &opts_with(TableMode::Image),
    )
    .expect("conversion should succeed");
    let epub = &converted.epub;

    let chapter = read_entry(epub, "OEBPS/text/chapter.xhtml");
    // The whole table (parent and nested) became one image; no <table> survives.
    assert!(
        !chapter.contains("<table"),
        "no <table> may survive under --tables image:\n{chapter}"
    );
    assert!(
        chapter.contains("chapter-table-1.png"),
        "the nested table should become an <img> at chapter-table-1.png:\n{chapter}"
    );
    assert!(
        entry_bytes(epub, "OEBPS/text/chapter-table-1.png").starts_with(&PNG_MAGIC),
        "chapter-table-1.png must carry the PNG magic bytes"
    );

    // The rasterization detail is marked `(nested)` because a cell has a sub-grid.
    let detail = converted
        .report
        .transformations
        .iter()
        .find(|t| t.kind == "table-rasterized")
        .map(|t| t.detail.as_str())
        .expect("a table-rasterized transformation");
    assert!(
        detail.contains("(nested)"),
        "the detail should carry the (nested) marker: {detail}"
    );

    assert_epubcheck_clean("tables-image-nested", epub);
}

// ---------------------------------------------------------------------
// epubcheck gate (skip-if-unavailable).
// ---------------------------------------------------------------------

fn run_epubcheck(path: &Path) -> Option<Output> {
    if let Ok(output) = Command::new("epubcheck").arg(path).output() {
        return Some(output);
    }
    if let Ok(jar) = std::env::var("EPUBCHECK_JAR")
        && let Ok(output) = Command::new("java").arg("-jar").arg(jar).arg(path).output()
    {
        return Some(output);
    }
    None
}

fn assert_epubcheck_clean(name: &str, epub: &[u8]) {
    let out_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.tailored.epub"));
    std::fs::write(&out_path, epub).expect("write converted epub");
    match run_epubcheck(&out_path) {
        None => eprintln!(
            "SKIP: epubcheck not found (not on PATH, EPUBCHECK_JAR unset); \
             skipping the {name} validation"
        ),
        Some(output) => {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            let offenders: Vec<&str> = combined
                .lines()
                .filter(|l| l.contains("FATAL(") || l.contains("ERROR(") || l.contains("WARNING("))
                .collect();
            assert!(
                offenders.is_empty(),
                "epubcheck reported {} problem(s) for {name} (status {:?}):\n{}",
                offenders.len(),
                output.status.code(),
                combined,
            );
        }
    }
}
