//! End-to-end regression tests for the 0.4.0/0.4.1 `:xmlns` corruption: the
//! serializer wrote html5ever's empty-prefix `xmlns` (inline `<svg>`/`<math>`
//! namespace declarations) as the malformed attribute name `:xmlns`, which
//! strict reader parsers reject as `Failed to parse QName ':xmlns'`. These
//! tests drive full conversions and assert the output is namespace-valid XML,
//! that already-corrupted books heal, and that healing is byte-stable.

mod common;

use common::build_epub;
use epub_tailor_core::{ConvertOptions, Features, Input, convert, find_invalid_qname, lint_epub};

const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol>
<li><a href="text/chapter1.xhtml">Chapter 1</a></li>
</ol></nav></body></html>"#;

const CHAPTER1: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body><h1>Chapter 1</h1><p>Text.</p></body></html>"#;

/// A titlepage carrying the exact corruption 0.4.0/0.4.1 wrote into books
/// (modelled on a real damaged file, `xmlns:xlink` and all).
const CORRUPT_TITLEPAGE: &[u8] = br##"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Die Pforte</title></head><body>
    <svg :xmlns="http://www.w3.org/2000/svg" height="100%" preserveAspectRatio="xMidYMid meet" version="1.1" viewBox="0 0 671 1024" width="100%" xmlns:xlink="http://www.w3.org/1999/xlink"><rect fill="#eee" height="1024" width="671"></rect></svg>
</body></html>"##;

const MATH_CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Math</title></head>
<body><p>Euler:</p><math xmlns="http://www.w3.org/1998/Math/MathML"><mi>e</mi></math></body></html>"#;

fn opf(manifest_extra: &str, spine_extra: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Regression</dc:title>
    <dc:language>de</dc:language>
    <dc:identifier id="pub-id">urn:uuid:00000000-0000-0000-0000-000000000042</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
    {manifest_extra}
  </manifest>
  <spine>
    {spine_extra}
    <itemref idref="ch1"/>
  </spine>
</package>"#
    )
    .into_bytes()
}

/// Extract one entry's bytes from an EPUB archive.
fn entry(epub: &[u8], name: &str) -> Vec<u8> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(epub.to_vec())).expect("output is a valid zip");
    let mut file = archive.by_name(name).unwrap_or_else(|_| {
        panic!(
            "entry {name} in output; entries: {:?}",
            (0..).map_while(|_| None::<String>).collect::<Vec<_>>()
        )
    });
    let mut data = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut data).expect("read entry");
    data
}

/// Every XHTML entry of `epub` must be well-formed, namespace-valid XML.
fn assert_all_content_valid(epub: &[u8]) {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(epub.to_vec())).expect("output is a valid zip");
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).expect("entry");
        let name = file.name().to_string();
        if !(name.ends_with(".xhtml") || name.ends_with(".html")) {
            continue;
        }
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut data).expect("read entry");
        let text = String::from_utf8(data).expect("utf8");
        let options = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        };
        roxmltree::Document::parse_with_options(&text, options)
            .unwrap_or_else(|e| panic!("{name} is not well-formed XML: {e}\n{text}"));
        if let Some((bad, line)) = find_invalid_qname(&text) {
            panic!("{name} has invalid QName '{bad}' on line {line}\n{text}");
        }
    }
}

/// The incident shape: a corrupted titlepage in a book converted with a
/// filters-only profile (`rasterize_svg` off, like the strip-watermarks run
/// that damaged the library), so the inline SVG survives to serialization.
#[test]
fn corrupted_titlepage_heals_under_a_filters_only_profile() {
    let opf = opf(
        r#"<item id="titlepage" href="titlepage.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="titlepage"/>"#,
    );
    let book = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/titlepage.xhtml", CORRUPT_TITLEPAGE),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
    ]);
    let opts = ConvertOptions {
        features: Features::repair_only(),
        ..Default::default()
    };
    let converted = convert(Input::Epub(book), &opts).expect("convert");

    let titlepage = String::from_utf8(entry(&converted.epub, "OEBPS/titlepage.xhtml")).unwrap();
    assert!(!titlepage.contains(":xmlns"), "got: {titlepage}");
    assert!(
        titlepage.contains(r#"<svg xmlns="http://www.w3.org/2000/svg""#),
        "got: {titlepage}"
    );
    assert_all_content_valid(&converted.epub);

    // The repaired book scans clean.
    let findings = lint_epub(
        &converted.epub,
        &epub_tailor_core::DeviceCaps::x4(),
        &Features::repair_only(),
    );
    assert!(
        !findings.iter().any(|f| f.code == "content-wellformed"),
        "repaired output must have no content-wellformed findings: {findings:?}"
    );

    // Healing is stable: converting the healed output again changes nothing
    // about the content documents.
    let again = convert(Input::Epub(converted.epub.clone()), &opts).expect("reconvert");
    assert_eq!(
        entry(&converted.epub, "OEBPS/titlepage.xhtml"),
        entry(&again.epub, "OEBPS/titlepage.xhtml"),
        "healed titlepage must be a fixed point of convert"
    );
}

/// MathML is never rasterized, so inline `<math xmlns=...>` reached the buggy
/// serializer even under the full default (x4) feature set.
#[test]
fn inline_math_survives_default_conversion_wellformed() {
    let opf = opf(
        r#"<item id="math" href="text/math.xhtml" media-type="application/xhtml+xml"/>"#,
        r#"<itemref idref="math"/>"#,
    );
    let book = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/math.xhtml", MATH_CHAPTER),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
    ]);
    let converted = convert(Input::Epub(book), &ConvertOptions::default()).expect("convert");
    let math = String::from_utf8(entry(&converted.epub, "OEBPS/text/math.xhtml")).unwrap();
    assert!(!math.contains(":xmlns"), "got: {math}");
    assert!(
        math.contains(r#"<math xmlns="http://www.w3.org/1998/Math/MathML""#),
        "got: {math}"
    );
    assert_all_content_valid(&converted.epub);
}

/// Non-spine XHTML documents are serialized but never rasterized, so a clean
/// inline SVG there must come out namespace-valid even with rasterize_svg on.
#[test]
fn non_spine_inline_svg_stays_wellformed_under_defaults() {
    const EXTRA: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Extra</title></head>
<body><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><circle r="1"></circle></svg></body></html>"#;
    let opf = opf(
        r#"<item id="extra" href="extra.xhtml" media-type="application/xhtml+xml"/>"#,
        "",
    );
    let book = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/extra.xhtml", EXTRA),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
    ]);
    let converted = convert(Input::Epub(book), &ConvertOptions::default()).expect("convert");
    let extra = String::from_utf8(entry(&converted.epub, "OEBPS/extra.xhtml")).unwrap();
    assert!(!extra.contains(":xmlns"), "got: {extra}");
    assert!(
        extra.contains(r#"<svg xmlns="http://www.w3.org/2000/svg""#),
        "got: {extra}"
    );
    assert_all_content_valid(&converted.epub);
}
