//! End-to-end gate for the Gutenberg self-closing-`<a>` duplicate-id bug.
//!
//! Project Gutenberg XHTML is full of `<a id="Pagexv"/>` page-break anchors.
//! Parsed through the HTML5 tree builder, `<a .../>` is not honored as
//! self-closing, so the anchor stays open across block boundaries and the
//! adoption agency algorithm clones it - `id` and all - onto every following
//! block until something actually closes it. One source anchor can turn into
//! several `<a id="...">` elements in the DOM, producing duplicate ids in the
//! serialized output (epubcheck RSC-005). `convert`'s per-chapter dedupe pass
//! must remove every such duplicate before serialization, and `lint_epub`
//! must catch it if it doesn't.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::epub3_gutenberg_style_ids;
use epub_tailor_core::{ConvertOptions, Input, Severity, convert, lint_epub};

/// Read a single entry from an in-memory EPUB as a UTF-8 string.
fn read_entry(epub: &[u8], name: &str) -> String {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(epub)).expect("output is a valid zip");
    let mut file = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("output should contain {name}"));
    let mut out = String::new();
    std::io::Read::read_to_string(&mut file, &mut out).expect("entry is UTF-8");
    out
}

/// Unit-level proof the dedupe pass runs inside the real pipeline: the source
/// fixture's parser-cloned `Pagexv`/`Pagexvi` ids each survive exactly once
/// in the converted chapter, the surviving occurrence is the first one (so
/// the same-document `href="#Pagexv"` back-reference still resolves), and the
/// report records one `duplicate-ids-removed` transformation.
#[test]
fn parser_cloned_ids_are_deduped_by_convert() {
    let converted = convert(
        Input::Epub(epub3_gutenberg_style_ids()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");

    let chapter = read_entry(&converted.epub, "OEBPS/text/chapter1.xhtml");
    assert_eq!(
        chapter.matches(r#"id="Pagexv""#).count(),
        1,
        "Pagexv must survive exactly once:\n{chapter}"
    );
    assert_eq!(
        chapter.matches(r#"id="Pagexvi""#).count(),
        1,
        "Pagexvi must survive exactly once:\n{chapter}"
    );
    assert!(
        chapter.contains(r##"href="#Pagexv""##),
        "the back-reference must still resolve to the surviving id:\n{chapter}"
    );

    let kinds: Vec<&str> = converted
        .report
        .transformations
        .iter()
        .map(|t| t.kind.as_str())
        .collect();
    assert!(
        kinds.contains(&"duplicate-ids-removed"),
        "expected a duplicate-ids-removed transformation, got: {kinds:?}"
    );
}

/// `lint_epub` must catch the duplicate ids in the raw (unconverted)
/// Gutenberg-style source - proving `check` on a third-party book would flag
/// exactly the epubcheck RSC-005 our own pipeline must avoid producing - and
/// must report none against `convert`'s own output.
#[test]
fn lint_flags_the_raw_fixture_and_is_clean_after_convert() {
    let raw = epub3_gutenberg_style_ids();
    let raw_findings = lint_epub(
        &raw,
        &epub_tailor_core::DeviceCaps::x4(),
        &epub_tailor_core::Features::all_on(),
    );
    assert!(
        raw_findings
            .iter()
            .any(|f| f.code == "duplicate-id" && f.severity == Severity::Error),
        "expected a duplicate-id error on the raw fixture, got {raw_findings:#?}"
    );

    let converted =
        convert(Input::Epub(raw), &ConvertOptions::default()).expect("conversion should succeed");
    let out_findings = lint_epub(
        &converted.epub,
        &epub_tailor_core::DeviceCaps::x4(),
        &epub_tailor_core::Features::all_on(),
    );
    assert!(
        !out_findings.iter().any(|f| f.code == "duplicate-id"),
        "converted output must have no duplicate-id findings, got {out_findings:#?}"
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
fn gutenberg_style_fixture_output_is_epubcheck_clean() {
    let converted = convert(
        Input::Epub(epub3_gutenberg_style_ids()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");

    let out_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gutenberg-ids.tailored.epub");
    std::fs::write(&out_path, &converted.epub).expect("write converted epub");

    match run_epubcheck(&out_path) {
        None => {
            eprintln!(
                "SKIP: epubcheck not found (not on PATH, EPUBCHECK_JAR unset); \
                 skipping the Gutenberg-style-ids validation"
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
