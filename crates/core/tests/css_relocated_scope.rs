//! Regression test for the cross-chapter `<style>` bleed (v0.2 T3).
//!
//! The device applies every `.css` file in the book to every chapter (it scans
//! the zip and ignores `<link>`), and its selector grammar is only `tag`,
//! `.class`, `tag.class` - no descendant or multi-class scoping. So when each
//! chapter's head `<style>` is lifted into the single book-wide
//! `et-relocated.css`, an unscoped author rule like `.note { ... }` from one
//! chapter restyles every other chapter. This test pins that each contributing
//! chapter's relocated rules are scoped to a chapter-unique `cpr{k}-...` class,
//! that a chapter which contributed no CSS is left entirely untouched, and that
//! no bare (bleeding) `.note` selector survives in the relocated sheet.

mod common;

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::epub3_style_bleed;
use epub_tailor_core::{ConvertOptions, DeviceCaps, Features, Input, convert, lint_epub};
use zip::ZipArchive;

fn read_entry(epub: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    let mut file = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("output should contain {name}"));
    let mut out = String::new();
    file.read_to_string(&mut out).expect("entry is UTF-8");
    out
}

#[test]
fn relocated_head_styles_are_scoped_per_chapter() {
    let converted = convert(Input::Epub(epub3_style_bleed()), &ConvertOptions::default())
        .expect("conversion should succeed");

    // The relocated sheet keeps BOTH chapters' `.note` rules, each scoped to its
    // own chapter, so the two different margins no longer fight or bleed.
    let relocated = read_entry(&converted.epub, "OEBPS/et-relocated.css");
    assert!(
        relocated.contains(".cpr1-c-note{margin-left:2em}"),
        "chapter 1's note rule should be scoped to cpr1:\n{relocated}"
    );
    assert!(
        relocated.contains(".cpr2-c-note{margin-left:4em}"),
        "chapter 2's note rule should be scoped to cpr2:\n{relocated}"
    );

    // The bleed assertion: no bare `.note` selector survives. Pre-fix the two
    // chapters' rules concatenate into a bleeding `.note{...}.note{...}`; scoping
    // must leave no unqualified `.note{` behind.
    assert!(
        !relocated.contains(".note{"),
        "a bare (bleeding) .note selector must not survive:\n{relocated}"
    );

    // Chapter 1 tags only its own scope class; chapter 2 tags only its own.
    let chapter1 = read_entry(&converted.epub, "OEBPS/text/chapter1.xhtml");
    assert!(
        chapter1.contains("cpr1-c-note"),
        "chapter 1's note element should carry its own scope class:\n{chapter1}"
    );
    assert!(
        !chapter1.contains("cpr2"),
        "chapter 1 must not carry chapter 2's scope class:\n{chapter1}"
    );

    let chapter2 = read_entry(&converted.epub, "OEBPS/text/chapter2.xhtml");
    assert!(
        chapter2.contains("cpr2-c-note"),
        "chapter 2's note element should carry its own scope class:\n{chapter2}"
    );
    assert!(
        !chapter2.contains("cpr1"),
        "chapter 2 must not carry chapter 1's scope class:\n{chapter2}"
    );

    // Chapter 3 contributed no CSS, so it is left entirely untouched: no scope
    // class anywhere, and (since the scoped rules require a `cpr` class) the
    // relocated sheet cannot restyle its `.note` paragraph on-device.
    let chapter3 = read_entry(&converted.epub, "OEBPS/text/chapter3.xhtml");
    assert!(
        !chapter3.contains("cpr"),
        "chapter 3 contributed nothing and must carry no scope class:\n{chapter3}"
    );

    // The converted book still lints clean.
    let findings = lint_epub(&converted.epub, &DeviceCaps::x4(), &Features::all_on());
    let errors: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == epub_tailor_core::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "expected no lint errors, got {errors:#?}"
    );
}

// ---------------------------------------------------------------------
// epubcheck gate (skip-if-unavailable), mirroring the other integration tests.
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
fn scoped_relocated_output_is_epubcheck_clean() {
    let converted = convert(Input::Epub(epub3_style_bleed()), &ConvertOptions::default())
        .expect("conversion should succeed");

    let out_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("style-bleed.tailored.epub");
    std::fs::write(&out_path, &converted.epub).expect("write converted epub");

    match run_epubcheck(&out_path) {
        None => {
            eprintln!(
                "SKIP: epubcheck not found (not on PATH, EPUBCHECK_JAR unset); \
                 skipping the style-bleed validation"
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
