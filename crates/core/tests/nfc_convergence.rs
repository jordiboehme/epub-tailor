//! Cleanup must *converge* on decomposed Unicode: `lint_epub`'s `encoding`
//! check reads whole file bytes, and the writer regenerates the OPF, nav doc
//! and NCX from `book.metadata` / `book.toc` - so a decomposed TOC title or
//! `dc:title` that only chapter-text hygiene would never touch must still come
//! out precomposed, or the same warning reappears after every repair run.

mod common;

use epub_tailor_core::profile::{DeviceCaps, Features};
use epub_tailor_core::{ConvertOptions, Input, convert, lint_epub};

use std::io::{Cursor, Read};
use zip::ZipArchive;

/// The resolved built-in `epub` profile as ConvertOptions.
fn repair_only_opts() -> ConvertOptions {
    ConvertOptions {
        device: DeviceCaps::permissive(),
        features: Features::repair_only(),
        ..ConvertOptions::default()
    }
}

fn entry(epub: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    let mut file = archive.by_name(name).expect("entry exists");
    let mut data = String::new();
    file.read_to_string(&mut data).expect("entry is UTF-8");
    data
}

/// An EPUB3 carrying both a nav doc and an NCX whose `dc:title` and TOC
/// titles spell every umlaut decomposed (`u` + U+0308 COMBINING DIAERESIS),
/// the way some tooling emits them. The chapters themselves are plain ASCII,
/// so only the metadata/TOC path is exercised.
fn epub3_decomposed_titles() -> Vec<u8> {
    // "Die Rückkehr der Jediritter" / "Über Endor" / "Für die Rebellion",
    // each with a decomposed umlaut.
    let title = "Die Ru\u{308}ckkehr der Jediritter";
    let toc1 = "U\u{308}ber Endor";
    let toc2 = "Fu\u{308}r die Rebellion";

    const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    let content_opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>{title}</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>de</dc:language>
    <dc:identifier id="pub-id">urn:uuid:0dec0dec-1111-2222-3333-444455556666</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="text/chapter2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>"#
    );

    let nav_xhtml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body>
<nav epub:type="toc">
<ol>
<li><a href="text/chapter1.xhtml">{toc1}</a></li>
<li><a href="text/chapter2.xhtml">{toc2}</a></li>
</ol>
</nav>
</body>
</html>"#
    );

    let toc_ncx = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:0dec0dec-1111-2222-3333-444455556666"/></head>
  <docTitle><text>{title}</text></docTitle>
  <navMap>
    <navPoint id="np1" playOrder="1">
      <navLabel><text>{toc1}</text></navLabel>
      <content src="text/chapter1.xhtml"/>
    </navPoint>
    <navPoint id="np2" playOrder="2">
      <navLabel><text>{toc2}</text></navLabel>
      <content src="text/chapter2.xhtml"/>
    </navPoint>
  </navMap>
</ncx>"#
    );

    const CHAPTER1: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body><h1>Chapter 1</h1><p>Text.</p></body></html>"#;

    const CHAPTER2: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 2</title></head>
<body><h1>Chapter 2</h1><p>Text.</p></body></html>"#;

    common::build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", content_opf.as_bytes()),
        ("OEBPS/nav.xhtml", nav_xhtml.as_bytes()),
        ("OEBPS/toc.ncx", toc_ncx.as_bytes()),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
        ("OEBPS/text/chapter2.xhtml", CHAPTER2),
    ])
}

#[test]
fn cleanup_converges_on_decomposed_toc_titles() {
    let source = epub3_decomposed_titles();

    // Fixture sanity: the raw book reproduces the bug - `check` flags the
    // OPF, nav doc and NCX as not NFC-normalized.
    let raw_findings = lint_epub(&source, &DeviceCaps::permissive(), &Features::repair_only());
    for name in ["OEBPS/content.opf", "OEBPS/nav.xhtml", "OEBPS/toc.ncx"] {
        assert!(
            raw_findings
                .iter()
                .any(|f| f.code == "encoding" && f.path.as_deref() == Some(name)),
            "the raw fixture must be flagged for {name}, got: {raw_findings:?}"
        );
    }

    let converted = convert(Input::Epub(source), &repair_only_opts()).expect("conversion succeeds");

    // The whole point: after one repair run, the check comes back clean.
    let out_findings = lint_epub(
        &converted.epub,
        &DeviceCaps::permissive(),
        &Features::repair_only(),
    );
    assert!(
        out_findings.is_empty(),
        "cleanup must converge (no findings on its own output), got: {out_findings:?}"
    );

    // The regenerated OPF, nav and NCX carry the precomposed form.
    for name in ["OEBPS/content.opf", "OEBPS/nav.xhtml", "OEBPS/toc.ncx"] {
        let text = entry(&converted.epub, name);
        assert!(
            !text.contains("u\u{308}") && !text.contains("U\u{308}"),
            "{name} must not keep decomposed umlauts"
        );
        assert!(
            text.contains('\u{fc}') || text.contains('\u{dc}'),
            "{name} must carry the precomposed umlaut"
        );
    }

    // The change is reported, so a repair fit no longer claims it did nothing.
    let kinds: Vec<&str> = converted
        .report
        .transformations
        .iter()
        .map(|t| t.kind.as_str())
        .collect();
    assert!(
        kinds.contains(&"metadata-nfc"),
        "a metadata-nfc transformation must be recorded, got: {kinds:?}"
    );
    assert!(
        kinds.contains(&"toc-nfc"),
        "a toc-nfc transformation must be recorded, got: {kinds:?}"
    );
}

#[test]
fn refit_of_normalized_output_records_no_nfc_change() {
    let source = epub3_decomposed_titles();
    let first = convert(Input::Epub(source), &repair_only_opts()).expect("first pass succeeds");
    let second =
        convert(Input::Epub(first.epub), &repair_only_opts()).expect("second pass succeeds");

    let nfc_kinds: Vec<&str> = second
        .report
        .transformations
        .iter()
        .map(|t| t.kind.as_str())
        .filter(|k| k.ends_with("-nfc"))
        .collect();
    assert!(
        nfc_kinds.is_empty(),
        "a second repair pass must find nothing left to normalize, got: {nfc_kinds:?}"
    );
}
