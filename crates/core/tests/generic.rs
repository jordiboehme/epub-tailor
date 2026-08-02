//! The generic profile: per-copy marker removal and the convergence property.

mod common;

use epub_tailor_core::profile::resolve;
use epub_tailor_core::{ConvertOptions, Input, convert};

/// Resolve a profile stack into options, exactly as the CLI does.
fn opts_for(specs: &[&str]) -> ConvertOptions {
    let specs: Vec<String> = specs.iter().map(|s| s.to_string()).collect();
    resolve(&specs).expect("profile resolves").to_options()
}

/// The regenerated OPF text of a converted book.
fn opf_of(epub: &[u8]) -> String {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(epub)).expect("zip");
    let name = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .find(|n| n.ends_with(".opf"))
        .expect("an OPF");
    let mut text = String::new();
    zip.by_name(&name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

#[test]
fn generic_pins_dcterms_modified_to_the_epoch() {
    let out = convert(
        Input::Epub(common::epub3_minimal()),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    assert!(
        opf_of(&out.epub)
            .contains("<meta property=\"dcterms:modified\">1970-01-01T00:00:00Z</meta>"),
        "generic must pin the modified date, got:\n{}",
        opf_of(&out.epub)
    );
}

#[test]
fn the_epub_profile_still_stamps_the_real_time() {
    let out =
        convert(Input::Epub(common::epub3_minimal()), &opts_for(&["epub"])).expect("converts");
    assert!(
        !opf_of(&out.epub).contains("1970-01-01T00:00:00Z"),
        "repair-only must not pin the date"
    );
}

/// A vendor id and a real ISBN sitting side by side in the same book.
fn book_with_vendor_id() -> Vec<u8> {
    common::build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", common::CONTAINER_XML),
        (
            "OEBPS/content.opf",
            br##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="pub-id">urn:uuid:6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31</dc:identifier>
    <dc:identifier id="isbn">9783407868213</dc:identifier>
    <meta refines="#isbn" property="identifier-type">ISBN</meta>
    <dc:identifier id="vendor">SHTX001.635962014</dc:identifier>
    <dc:title>Book</dc:title>
    <dc:language>de</dc:language>
    <meta property="dcterms:modified">2024-06-20T15:04:24Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch" href="chapter.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch"/></spine>
</package>"##,
        ),
        ("OEBPS/nav.xhtml", common::NAV_XHTML),
        (
            "OEBPS/chapter.xhtml",
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>C</title></head>
<body><p>Text.</p></body></html>"#,
        ),
    ])
}

#[test]
fn generic_drops_vendor_identifiers_but_keeps_the_isbn() {
    let out =
        convert(Input::Epub(book_with_vendor_id()), &opts_for(&["generic"])).expect("converts");
    let opf = opf_of(&out.epub);
    assert!(
        opf.contains("9783407868213"),
        "the ISBN is shared, it stays"
    );
    assert!(
        !opf.contains("SHTX001.635962014"),
        "a vendor transaction id must not survive:\n{opf}"
    );
}

#[test]
fn generic_replaces_a_uuid_unique_identifier_deterministically() {
    let out =
        convert(Input::Epub(book_with_vendor_id()), &opts_for(&["generic"])).expect("converts");
    let opf = opf_of(&out.epub);
    assert!(
        !opf.contains("6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31"),
        "a per-copy UUID must not become the book's identity:\n{opf}"
    );
    assert!(
        opf.contains("urn:epub-tailor:"),
        "replaced deterministically"
    );
}
