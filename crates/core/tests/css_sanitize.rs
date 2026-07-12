//! End-to-end tests for the Adobe RMSDK escape hatch (`sanitize_css`).
//!
//! RMSDK - the engine behind a plain `.epub` on a Kobo, PocketBook's EPUB2 path
//! and tolino's RMSDK mode - has a CSS parser with no fault tolerance: one
//! construct it cannot read and it discards the *entire* stylesheet, or refuses
//! the book outright. These tests drive a real conversion through a real device
//! profile and assert on the bytes that come out the other end, because "the
//! stylesheet survived" is the only thing that actually matters here.

mod common;

use std::io::{Cursor, Read};

use common::build_epub;
use epub_tailor_core::{Input, convert, profile};
use zip::{CompressionMethod, ZipArchive};

/// A one-chapter EPUB whose stylesheet mixes RMSDK-hostile constructs with
/// perfectly ordinary rules that must survive.
fn epub_with_modern_css() -> Vec<u8> {
    const CONTAINER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

    const OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Modern CSS</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:00000000-0000-0000-0000-00000000rmsd</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="css" href="main.css" media-type="text/css"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;

    const NAV: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol><li><a href="chapter1.xhtml">One</a></li></ol></nav></body>
</html>"#;

    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>One</title><link rel="stylesheet" type="text/css" href="main.css"/></head>
<body><h1>One</h1><p>Body text.</p></body>
</html>"#;

    // The two `calc()` declarations and the `@supports` block are what RMSDK
    // chokes on. Everything else is ordinary and must come out the far side.
    const CSS: &[u8] = br#":root { --accent: #336699; }
body { margin: 0; font-size: 16px; line-height: 1.4; }
p { text-indent: 1em; width: calc(100% - 2em); }
h1 { text-align: center; font-weight: 700; color: var(--accent); }
blockquote { margin-left: 2em; font-style: italic; }
@supports (display: grid) { .layout { display: grid; } }
@media screen { .note { font-size: 14px; width: calc(50% + 1px); } }
"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", OPF),
        ("OEBPS/nav.xhtml", NAV),
        ("OEBPS/chapter1.xhtml", CHAPTER),
        ("OEBPS/main.css", CSS),
    ])
}

fn convert_with(profile_name: &str, epub: Vec<u8>) -> Vec<u8> {
    let resolved = profile::resolve(&[profile_name.to_string()]).expect("profile resolves");
    convert(Input::Epub(epub), &resolved.to_options())
        .expect("conversion should succeed")
        .epub
}

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
fn a_kobo_book_loses_its_modern_css_and_keeps_everything_else() {
    let out = convert_with("kobo-clara-bw", epub_with_modern_css());
    let css = read_entry(&out, "OEBPS/main.css");

    // The constructs that would make RMSDK discard the whole sheet are gone...
    for hostile in ["calc(", "var(", "@supports"] {
        assert!(
            !css.to_ascii_lowercase().contains(hostile),
            "{hostile} must not survive for RMSDK, got: {css}"
        );
    }

    // ...and everything else is still there. This is the half that separates
    // sanitize_css from filter_css: it is not allowed to demolish the sheet.
    for kept in [
        "margin",
        "font-size",
        "line-height",
        "text-indent",
        "text-align",
        "font-weight",
        "font-style",
        "margin-left",
    ] {
        assert!(css.contains(kept), "{kept} must survive, got: {css}");
    }

    // The @media block loses only its calc() declaration, not the block.
    assert!(css.contains("@media"), "the media block survives: {css}");
}

#[test]
fn the_crosspoint_profile_is_unaffected_by_the_rmsdk_pass() {
    // x4 runs filter_css, not sanitize_css: its output is the tiny CrossPoint
    // subset, and calc() is gone because the subset never had it.
    let out = convert_with("x4", epub_with_modern_css());
    let css = read_entry(&out, "OEBPS/main.css");
    assert!(!css.contains("calc("), "got: {css}");
    // line-height is not in CrossPoint's grammar, so filter_css drops it -
    // proving this book went through the demolition pass, not the gentle one.
    assert!(
        !css.contains("line-height"),
        "x4 should have filtered, not sanitized: {css}"
    );
}

#[test]
fn the_repair_profile_leaves_the_stylesheet_completely_alone() {
    let out = convert_with("epub", epub_with_modern_css());
    let css = read_entry(&out, "OEBPS/main.css");
    assert!(
        css.contains("calc(100% - 2em)"),
        "repair-only must not touch CSS: {css}"
    );
}

#[test]
fn the_output_has_the_ocf_zip_shape_rmsdk_requires() {
    // RMSDK is strict about the container: `mimetype` must be the first entry
    // and STORED (uncompressed). Get this wrong and the book does not open.
    // The writer already does it; this pins it so it stays done.
    let out = convert_with("kobo-clara-bw", epub_with_modern_css());
    let mut archive = ZipArchive::new(Cursor::new(&out[..])).expect("output is a valid zip");

    let first = archive.by_index(0).expect("the archive has entries");
    assert_eq!(first.name(), "mimetype", "mimetype must be the first entry");
    assert_eq!(
        first.compression(),
        CompressionMethod::Stored,
        "mimetype must be STORED, not deflated"
    );
    drop(first);

    let mut mimetype = String::new();
    archive
        .by_name("mimetype")
        .expect("mimetype entry")
        .read_to_string(&mut mimetype)
        .expect("mimetype is UTF-8");
    assert_eq!(mimetype, "application/epub+zip");
}

#[test]
fn every_device_profile_still_emits_both_a_nav_document_and_an_ncx() {
    // Kobo ignores the NCX entirely in EPUB3 and needs the nav doc; CrossPoint
    // reads the NCX. We emit both, unconditionally, so one book serves both.
    for name in ["kobo-clara-bw", "pocketbook-era", "boox-page", "x4"] {
        let out = convert_with(name, epub_with_modern_css());
        let mut archive = ZipArchive::new(Cursor::new(&out[..])).expect("valid zip");
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).expect("entry").name().to_string())
            .collect();
        assert!(
            names.iter().any(|n| n.ends_with(".ncx")),
            "{name}: an NCX must be emitted, got {names:?}"
        );
        assert!(
            names.iter().any(|n| n.ends_with("nav.xhtml")),
            "{name}: a nav document must be emitted, got {names:?}"
        );
    }
}
