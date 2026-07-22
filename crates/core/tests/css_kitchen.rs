//! End-to-end test for the M4 CSS/style pipeline: convert a one-chapter book
//! that stresses head `<style>` relocation, inline-style filtering, external
//! stylesheet filtering and font stripping, assert the expected output, and
//! validate the result with epubcheck (skip-if-unavailable, the same harness
//! pattern as the other integration tests).

mod common;

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::epub3_css_kitchen;
use epub_tailor_core::{ConvertOptions, Input, convert};
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

fn entry_exists(epub: &[u8], name: &str) -> bool {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    archive.by_name(name).is_ok()
}

#[test]
fn css_kitchen_filters_relocates_and_strips_fonts() {
    let converted = convert(Input::Epub(epub3_css_kitchen()), &ConvertOptions::default())
        .expect("conversion should succeed");

    let kinds: Vec<&str> = converted
        .report
        .transformations
        .iter()
        .map(|t| t.kind.as_str())
        .collect();
    for expected in [
        "fonts-stripped",
        "head-style-relocated",
        "inline-style-filtered",
        "colors-remapped",
    ] {
        assert!(
            kinds.contains(&expected),
            "expected a `{expected}` transformation, got: {kinds:?}"
        );
    }

    let chapter = read_entry(&converted.epub, "OEBPS/text/chapter.xhtml");

    // The head <style> is gone, and its (filtered) content is relocated.
    assert!(
        !chapter.contains("<style"),
        "no <style> should remain:\n{chapter}"
    );
    assert!(
        chapter.contains("et-relocated.css"),
        "chapter should link et-relocated.css:\n{chapter}"
    );

    // The <link> to the embedded font is gone, and so is the font resource.
    assert!(
        !chapter.contains("DejaVu.ttf"),
        "the font <link> should be removed:\n{chapter}"
    );
    assert!(
        !entry_exists(&converted.epub, "OEBPS/fonts/DejaVu.ttf"),
        "the embedded font should be stripped"
    );

    // Inline styles: supported declarations kept, unsupported dropped, and
    // colors kept but remapped to the x4 panel's gray tones (red, green and
    // blue all read as one mid tone, and the 4-level panel folds them onto
    // its dark-gray level, #555).
    assert!(
        chapter.contains(r#"style="color:#555;text-align:center""#),
        "inline color remapped, sibling declaration kept:\n{chapter}"
    );
    assert!(
        !chapter.contains("color:red"),
        "no source color survives:\n{chapter}"
    );
    assert!(
        chapter.contains(r#"style="color:#555">Dropped inline."#),
        "a color-only style attr survives remapped:\n{chapter}"
    );
    assert!(
        !chapter.contains("font-size"),
        "unsupported inline decl dropped:\n{chapter}"
    );

    // The head <style> is the only contributor, so its rules are scoped to
    // chapter 1 (`cpr1-...`). `body` becomes `body.cpr1-e-body`, `.note` becomes
    // `.cpr1-c-note`, and `.s` is dead-dropped (no `.s` element in the chapter).
    // Its `color:green` survives the filter and comes out as the shared gray.
    let relocated = read_entry(&converted.epub, "OEBPS/et-relocated.css");
    assert_eq!(
        relocated,
        "body.cpr1-e-body{color:#555;text-align:justify}.cpr1-c-note{margin-left:2em}"
    );

    // The chapter's own elements carry the matching scope classes.
    assert!(
        chapter.contains(r#"<body class="cpr1-e-body">"#),
        "the body should carry the element scope class:\n{chapter}"
    );
    assert!(
        chapter.contains(r#"class="note cpr1-c-note""#),
        "the .note paragraph should carry the class scope class:\n{chapter}"
    );

    // The external stylesheet was filtered in place, its color remapped.
    let ext = read_entry(&converted.epub, "OEBPS/styles/ext.css");
    assert_eq!(
        ext,
        "p{color:#555;margin:1em}.keep{text-align:center;margin-left:2em}"
    );

    // Both generated/filtered sheets are declared in the regenerated manifest.
    let opf = read_entry(&converted.epub, "OEBPS/content.opf");
    assert!(
        opf.contains("et-relocated.css"),
        "et-relocated.css should be in the manifest:\n{opf}"
    );
    assert!(
        opf.contains("ext.css"),
        "ext.css should stay in the manifest:\n{opf}"
    );
    assert!(
        !opf.contains("DejaVu.ttf"),
        "the font should be gone from the manifest:\n{opf}"
    );
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

#[test]
fn css_kitchen_output_is_epubcheck_clean() {
    let converted = convert(Input::Epub(epub3_css_kitchen()), &ConvertOptions::default())
        .expect("conversion should succeed");

    let out_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("css-kitchen.tailored.epub");
    std::fs::write(&out_path, &converted.epub).expect("write converted epub");

    match run_epubcheck(&out_path) {
        None => {
            eprintln!(
                "SKIP: epubcheck not found (not on PATH, EPUBCHECK_JAR unset); \
                 skipping the css-kitchen validation"
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
