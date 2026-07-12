//! End-to-end test for the M3 HTML transforms: convert a one-chapter book
//! whose chapter exercises every transform, assert the expected markers in the
//! output, and validate the result with epubcheck (skip-if-unavailable, the
//! same harness pattern as `epubcheck_roundtrip`).

mod common;

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::epub3_kitchen_sink;
use epub_tailor_core::{ConvertOptions, Input, convert};
use zip::ZipArchive;

/// Read a single entry from an in-memory EPUB as a UTF-8 string.
fn read_entry(epub: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    let mut file = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("output should contain {name}"));
    let mut out = String::new();
    file.read_to_string(&mut out).expect("entry is UTF-8");
    out
}

fn entry_exists(epub: &[u8], name: &str) -> bool {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    archive.by_name(name).is_ok()
}

#[test]
fn kitchen_sink_transforms_produce_expected_markers() {
    let converted = convert(
        Input::Epub(epub3_kitchen_sink()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");

    // The report should record each kind of transform, plus the generated CSS.
    let kinds: Vec<&str> = converted
        .report
        .transformations
        .iter()
        .map(|t| t.kind.as_str())
        .collect();
    for expected in [
        "table-linearized",
        "ol-numbered",
        "code-block-preserved",
        "box-degraded",
        "link-rewritten",
        "anchor-relocated",
        "text-nfc",
        "stylesheet-added",
    ] {
        assert!(
            kinds.contains(&expected),
            "expected a `{expected}` transformation, got: {kinds:?}"
        );
    }

    // The over-long word and the dead javascript: link should each warn.
    let messages: Vec<&str> = converted
        .report
        .warnings
        .iter()
        .map(|w| w.message.as_str())
        .collect();
    assert!(
        messages.iter().any(|m| m.contains("200 bytes")),
        "expected an over-long-word warning, got: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("unwrapped a javascript")),
        "expected an unwrapped-link warning, got: {messages:?}"
    );

    let chapter = read_entry(&converted.epub, "OEBPS/text/kitchen.xhtml");

    // Constructs the firmware mishandles must be gone.
    assert!(
        !chapter.contains("<table"),
        "no table should remain:\n{chapter}"
    );
    assert!(
        !chapter.contains("<ol"),
        "no ordered list should remain:\n{chapter}"
    );
    assert!(!chapter.contains("<dl"), "no dl should remain:\n{chapter}");
    assert!(
        !chapter.contains("<figure"),
        "no figure should remain:\n{chapter}"
    );
    assert!(
        !chapter.contains("<aside"),
        "no aside should remain:\n{chapter}"
    );
    assert!(
        !chapter.contains("<pre"),
        "no pre should remain:\n{chapter}"
    );
    assert!(
        !chapter.contains("javascript:"),
        "no javascript: link should remain:\n{chapter}"
    );

    // The generated helper classes must be present.
    for marker in [
        "et-table-cell",
        "et-table-caption",
        "et-ol-item",
        "et-ol-nested",
        "et-code",
        "et-box-title",
        "et-caption",
        "et-dt",
        "et-dd",
    ] {
        assert!(chapter.contains(marker), "expected `{marker}`:\n{chapter}");
    }

    // The bullet sublist inside the ordered list survives as a real list.
    assert!(
        chapter.contains("<ul>"),
        "ul-in-ol should survive:\n{chapter}"
    );

    // Numbered paragraphs use baked-in labels (type="a", start=2 -> "b.").
    assert!(
        chapter.contains("b."),
        "baked-in ol label expected:\n{chapter}"
    );

    // The inline id was relocated and its references (both the footnote link and
    // the rewritten javascript link) point at the block's existing id.
    assert!(
        chapter.contains(r#"id="note""#),
        "block keeps its id:\n{chapter}"
    );
    assert!(
        !chapter.contains(r##"href="#fn1""##),
        "aliased fragment must be rewritten:\n{chapter}"
    );
    assert!(
        chapter.contains(r##"href="#note""##),
        "references should point at the surviving id:\n{chapter}"
    );

    // The stylesheet is a manifest resource, linked from the chapter head.
    assert!(
        entry_exists(&converted.epub, "OEBPS/et-styles.css"),
        "et-styles.css should be packaged"
    );
    let opf = read_entry(&converted.epub, "OEBPS/content.opf");
    assert!(
        opf.contains("et-styles.css"),
        "et-styles.css should be in the manifest:\n{opf}"
    );
    assert!(
        chapter.contains("et-styles.css"),
        "chapter head should link the stylesheet:\n{chapter}"
    );
}

// ---------------------------------------------------------------------
// epubcheck gate (skip-if-unavailable), mirroring `epubcheck_roundtrip`.
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

#[test]
fn kitchen_sink_output_is_epubcheck_clean() {
    let converted = convert(
        Input::Epub(epub3_kitchen_sink()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");

    let out_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("kitchen.tailored.epub");
    std::fs::write(&out_path, &converted.epub).expect("write converted epub");

    match run_epubcheck(&out_path) {
        None => {
            eprintln!(
                "SKIP: epubcheck not found (not on PATH, EPUBCHECK_JAR unset); \
                 skipping the kitchen-sink validation"
            );
        }
        Some(output) => {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            let offenders: Vec<&str> = combined
                .lines()
                .filter(|line| {
                    line.contains("FATAL(") || line.contains("ERROR(") || line.contains("WARNING(")
                })
                .collect();
            assert!(
                offenders.is_empty(),
                "epubcheck reported {} problem(s) (status {:?}):\n{}",
                offenders.len(),
                output.status.code(),
                combined,
            );
        }
    }
}
