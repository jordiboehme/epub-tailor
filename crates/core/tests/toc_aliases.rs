//! Regression test: TOC hrefs must follow anchor-alias relocation.
//!
//! Phase-2 anchor aliasing (`html::apply_anchor_aliases`) rewrites `<a href>`
//! inside chapter DOMs when an inline id gets aliased onto its block
//! ancestor's existing id, but historically left `book.toc` untouched.
//! `nav.xhtml`/`toc.ncx` are regenerated verbatim from `book.toc`, so a TOC
//! entry that targeted the now-relocated inline id was left dangling. This
//! exercises the fix (`html::apply_toc_aliases`) end to end through
//! `convert`, mirroring `broken_toc_fragment_is_an_error` in `validate.rs`.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::build_epub;
use epub_tailor_core::{ConvertOptions, DeviceCaps, Features, Input, convert, lint_epub};

const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>TOC Alias</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:aaaa1111-bbbb-2222-cccc-333344445555</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

// The nav TOC targets `#toctarget`, an inline `<span>` id that conflicts with
// its block ancestor's existing `id="blk"` - `relocate_ids` drops the inline
// id and aliases it onto `blk`, so after the fix the surviving anchor is
// `id="blk"`, not `id="toctarget"`.
const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol>
<li><a href="text/chapter1.xhtml#toctarget">Note</a></li>
</ol></nav></body></html>"#;

const CHAPTER1: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body><h1>Chapter 1</h1><p id="blk"><span id="toctarget">note</span></p></body></html>"#;

fn dangling_toc_fixture() -> Vec<u8> {
    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
    ])
}

#[test]
fn toc_fragment_follows_anchor_alias_through_convert() {
    let converted = convert(
        Input::Epub(dangling_toc_fixture()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");

    let findings = lint_epub(&converted.epub, &DeviceCaps::x4(), &Features::all_on());
    assert!(
        !findings.iter().any(|f| f.code == "spine-toc-sync"),
        "TOC fragment must follow the anchor alias, got {findings:#?}"
    );
}

// ---------------------------------------------------------------------
// epubcheck gate (skip-if-unavailable), mirroring `duplicate_ids`.
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
fn toc_alias_fixture_output_is_epubcheck_clean() {
    let converted = convert(
        Input::Epub(dangling_toc_fixture()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");

    let out_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("toc-alias.tailored.epub");
    std::fs::write(&out_path, &converted.epub).expect("write converted epub");

    match run_epubcheck(&out_path) {
        None => {
            eprintln!(
                "SKIP: epubcheck not found (not on PATH, EPUBCHECK_JAR unset); \
                 skipping the TOC-alias validation"
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
