//! Integration tests for `epub_tailor_core::read_epub`, driven entirely by
//! in-code fixtures (see `tests/common/mod.rs`) — no binary EPUBs checked in.

mod common;

use std::path::PathBuf;

use common::{build_epub, epub2_minimal, epub3_minimal, run_epubcheck};
use epub_tailor_core::{
    ConvertError, ConvertOptions, DeviceCaps, Features, Input, Severity, convert, lint_epub,
    read_epub,
};

const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

const CHAPTER1: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body><h1>Chapter 1</h1><p>Text.</p></body></html>"#;

fn minimal_opf(chapter_href: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Minimal</dc:title>
    <dc:creator>Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:00000000-0000-0000-0000-000000000000</dc:identifier>
  </metadata>
  <manifest>
    <item id="ch1" href="{chapter_href}" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
    )
}

#[test]
fn epub3_parses_metadata_spine_toc_cover_and_resources() {
    let bytes = epub3_minimal();
    let result = epub_tailor_core::read_epub(&bytes).expect("valid epub3 should read");

    assert!(
        result.warnings.is_empty(),
        "expected no warnings, got {:?}",
        result.warnings
    );

    let book = result.book;
    assert_eq!(book.metadata.title, "Sample Book");
    assert_eq!(book.metadata.authors[0].name, "Jane Author");
    assert_eq!(book.metadata.language, "en");
    assert_eq!(
        book.metadata.identifier.as_deref(),
        Some("urn:uuid:12345678-1234-1234-1234-123456789012")
    );

    assert_eq!(book.opf_path, "OEBPS/content.opf");
    assert_eq!(book.nav_path.as_deref(), Some("OEBPS/nav.xhtml"));
    assert_eq!(book.ncx_path, None);

    assert_eq!(
        book.spine,
        vec![
            "OEBPS/text/chapter1.xhtml".to_string(),
            "OEBPS/text/chapter2.xhtml".to_string(),
        ]
    );

    assert_eq!(book.cover.as_deref(), Some("OEBPS/images/cover.jpg"));

    assert_eq!(book.toc.len(), 3);
    assert_eq!(book.toc[0].title, "Chapter 1");
    assert_eq!(book.toc[0].href, "OEBPS/text/chapter1.xhtml");
    assert_eq!(book.toc[0].level, 1);
    assert_eq!(book.toc[1].title, "Section 1.1");
    assert_eq!(book.toc[1].href, "OEBPS/text/chapter1.xhtml#s2");
    assert_eq!(book.toc[1].level, 2);
    assert_eq!(book.toc[2].title, "Chapter 2");
    assert_eq!(book.toc[2].href, "OEBPS/text/chapter2.xhtml");
    assert_eq!(book.toc[2].level, 1);

    let keys: Vec<&str> = book.resources.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec![
            "OEBPS/content.opf",
            "OEBPS/nav.xhtml",
            "OEBPS/text/chapter1.xhtml",
            "OEBPS/text/chapter2.xhtml",
            "OEBPS/styles/main.css",
            "OEBPS/images/cover.jpg",
        ]
    );
    assert_eq!(
        book.resources["OEBPS/content.opf"].media_type,
        "application/oebps-package+xml"
    );
    assert_eq!(
        book.resources["OEBPS/nav.xhtml"].media_type,
        "application/xhtml+xml"
    );
    assert_eq!(
        book.resources["OEBPS/text/chapter1.xhtml"].media_type,
        "application/xhtml+xml"
    );
    assert_eq!(
        book.resources["OEBPS/styles/main.css"].media_type,
        "text/css"
    );
    assert_eq!(
        book.resources["OEBPS/images/cover.jpg"].media_type,
        "image/jpeg"
    );
}

#[test]
fn epub2_falls_back_to_ncx_toc() {
    let bytes = epub2_minimal();
    let result = epub_tailor_core::read_epub(&bytes).expect("valid epub2 should read");

    let book = result.book;
    assert_eq!(book.nav_path, None);
    assert_eq!(book.ncx_path.as_deref(), Some("OEBPS/toc.ncx"));

    assert_eq!(book.toc.len(), 2);
    assert_eq!(book.toc[0].title, "Chapter 1");
    assert_eq!(book.toc[0].href, "OEBPS/text/chapter1.xhtml");
    assert_eq!(book.toc[0].level, 1);
    assert_eq!(book.toc[1].title, "Chapter 2");
    assert_eq!(book.toc[1].href, "OEBPS/text/chapter2.xhtml");
    assert_eq!(book.toc[1].level, 1);
}

#[test]
fn adept_style_drm_is_rejected() {
    const ENCRYPTION_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#">
    <EncryptionMethod Algorithm="http://www.adobe.com/adept"/>
    <CipherData><CipherReference URI="text/chapter1.xhtml"/></CipherData>
  </EncryptedData>
</encryption>"#;

    let opf = minimal_opf("text/chapter1.xhtml");
    let bytes = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("META-INF/encryption.xml", ENCRYPTION_XML),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
    ]);

    let err = epub_tailor_core::read_epub(&bytes).expect_err("ADEPT DRM should be rejected");
    assert!(
        matches!(err, ConvertError::DrmProtected),
        "expected DrmProtected, got {err:?}"
    );
}

#[test]
fn font_obfuscation_only_is_a_warning_not_an_error() {
    // The `CipherReference` URI deliberately names no resource in this
    // fixture: this test only exercises `check_drm`'s classification warning
    // (font-obfuscation-only is not DRM), not de-obfuscation itself - that is
    // covered end to end by `font_obfuscation_is_undone_and_encryption_xml_is_dropped`
    // and `adobe_font_obfuscation_is_undone` below. The missing-resource skip
    // is asserted explicitly (no `font-deobfuscated` transformation) so it
    // reads as an intentional part of this fixture, not an unstated side
    // effect of a stale path.
    const ENCRYPTION_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#">
    <EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/>
    <CipherData><CipherReference URI="fonts/embedded.ttf"/></CipherData>
  </EncryptedData>
</encryption>"#;

    let opf = minimal_opf("text/chapter1.xhtml");
    let bytes = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("META-INF/encryption.xml", ENCRYPTION_XML),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
    ]);

    let result =
        epub_tailor_core::read_epub(&bytes).expect("font-obfuscation-only should not error");
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.message.to_lowercase().contains("font")),
        "expected a warning mentioning fonts, got {:?}",
        result.warnings
    );
    assert!(
        !result
            .transformations
            .iter()
            .any(|t| t.kind == "font-deobfuscated"),
        "the CipherReference URI names nothing in this fixture, so nothing should have been \
         de-obfuscated: {:?}",
        result.transformations
    );
}

/// The IDPF font-obfuscation key for `unique_id`: SHA-1 of the identifier
/// with whitespace stripped (there is none here), matching
/// `epub_tailor_core`'s own `fonts::idpf_key` - kept independent (computed by
/// hand from the `sha1` crate directly) so the test does not just assert the
/// implementation agrees with itself.
fn idpf_obfuscate(data: &[u8], unique_id: &str) -> Vec<u8> {
    use sha1::{Digest, Sha1};
    let key = Sha1::digest(unique_id.as_bytes());
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

#[test]
fn font_obfuscation_is_undone_and_encryption_xml_is_dropped() {
    const UNIQUE_ID: &str = "urn:uuid:00000000-0000-0000-0000-000000000000";
    let original_font: Vec<u8> = (0..300u32).map(|i| (i % 250) as u8).collect();
    let obfuscated_font = idpf_obfuscate(&original_font, UNIQUE_ID);
    // The obfuscation must actually have changed the bytes, or this fixture
    // would not exercise de-obfuscation at all.
    assert_ne!(obfuscated_font, original_font);

    const ENCRYPTION_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#">
    <EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/>
    <CipherData><CipherReference URI="OEBPS/fonts/embedded.ttf"/></CipherData>
  </EncryptedData>
</encryption>"#;

    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Fonted</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">{UNIQUE_ID}</dc:identifier>
  </metadata>
  <manifest>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="font" href="fonts/embedded.ttf" media-type="application/vnd.ms-opentype"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
    );

    let bytes = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("META-INF/encryption.xml", ENCRYPTION_XML),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
        ("OEBPS/fonts/embedded.ttf", &obfuscated_font),
    ]);

    // Reading alone must de-obfuscate the font in the `Book` model and record
    // the transformation.
    let result = epub_tailor_core::read_epub(&bytes).expect("font-obfuscated epub should read");
    let font_resource = &result.book.resources["OEBPS/fonts/embedded.ttf"];
    assert_eq!(
        font_resource.data, original_font,
        "the font bytes must be de-obfuscated back to the original"
    );
    assert!(
        result
            .transformations
            .iter()
            .any(|t| t.kind == "font-deobfuscated"
                && t.file.as_deref() == Some("OEBPS/fonts/embedded.ttf")),
        "expected a font-deobfuscated transformation naming the font, got {:?}",
        result.transformations
    );

    // End to end through `convert()` (the `epub` profile: repair-only, so the
    // font is not stripped and would otherwise reach the output still
    // scrambled): the output carries the original font bytes and no longer
    // declares `META-INF/encryption.xml`.
    let converted = convert(
        Input::Epub(bytes),
        &ConvertOptions {
            features: Features::repair_only(),
            ..ConvertOptions::default()
        },
    )
    .expect("font-obfuscated epub should convert");

    assert!(
        common::entry(&converted.epub, "META-INF/encryption.xml").is_none(),
        "the converted output must not declare META-INF/encryption.xml"
    );
    let out_font = common::entry(&converted.epub, "OEBPS/fonts/embedded.ttf")
        .expect("the font resource must survive conversion");
    assert_eq!(
        out_font, original_font,
        "the converted output's font bytes must be the de-obfuscated original"
    );
}

/// The Adobe font-obfuscation key for a canonical `urn:uuid:` identifier: the
/// 16 raw bytes of its UUID. Computed independently of `epub_tailor_core`'s
/// own key derivation (plain hex parsing, no shared code) so this test does
/// not just assert the implementation agrees with itself.
fn adobe_obfuscate(data: &[u8], unique_id: &str) -> Vec<u8> {
    let hex: String = unique_id
        .strip_prefix("urn:uuid:")
        .expect("test identifier must be a urn:uuid: value")
        .chars()
        .filter(|c| *c != '-')
        .collect();
    assert_eq!(hex.len(), 32, "test identifier must have 32 hex digits");
    let key: Vec<u8> = (0..16)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("valid hex digit pair"))
        .collect();
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

/// Adobe's algorithm (`http://ns.adobe.com/pdf/enc#RC`) is only otherwise
/// named in `read.rs` itself - never exercised end to end - so this pins it
/// separately from the IDPF coverage above. It also directly covers the
/// off-by-one `adobe_key` bug a review caught: a `urn:uuid:` identifier is
/// the shape EPUB 3 recommends for `dc:identifier`, so this is the common
/// case, not an edge case.
#[test]
fn adobe_font_obfuscation_is_undone() {
    const UNIQUE_ID: &str = "urn:uuid:12345678-1234-1234-1234-123456789012";
    let original_font: Vec<u8> = (0..300u32).map(|i| (i % 250) as u8).collect();
    let obfuscated_font = adobe_obfuscate(&original_font, UNIQUE_ID);
    assert_ne!(obfuscated_font, original_font);

    const ENCRYPTION_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#">
    <EncryptionMethod Algorithm="http://ns.adobe.com/pdf/enc#RC"/>
    <CipherData><CipherReference URI="OEBPS/fonts/embedded.ttf"/></CipherData>
  </EncryptedData>
</encryption>"#;

    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Fonted</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">{UNIQUE_ID}</dc:identifier>
  </metadata>
  <manifest>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="font" href="fonts/embedded.ttf" media-type="application/vnd.ms-opentype"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
    );

    let bytes = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("META-INF/encryption.xml", ENCRYPTION_XML),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
        ("OEBPS/fonts/embedded.ttf", &obfuscated_font),
    ]);

    let result = epub_tailor_core::read_epub(&bytes).expect("adobe-obfuscated epub should read");
    let font_resource = &result.book.resources["OEBPS/fonts/embedded.ttf"];
    assert_eq!(
        font_resource.data, original_font,
        "the font bytes must be de-obfuscated back to the original"
    );
    assert!(
        result
            .transformations
            .iter()
            .any(|t| t.kind == "font-deobfuscated"
                && t.file.as_deref() == Some("OEBPS/fonts/embedded.ttf")),
        "expected a font-deobfuscated transformation naming the font, got {:?}",
        result.transformations
    );
}

/// Adobe's scheme needs a `urn:uuid:` identifier; an ISBN-only identifier
/// yields no usable key. The font must be left exactly as scrambled as it
/// arrived (not further corrupted by XORing with a bogus key), and the read
/// must warn that it could not be de-obfuscated rather than record a
/// `font-deobfuscated` transformation that would be a false claim.
#[test]
fn adobe_obfuscation_without_a_uuid_identifier_warns_instead_of_claiming_success() {
    const ENCRYPTION_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#">
    <EncryptionMethod Algorithm="http://ns.adobe.com/pdf/enc#RC"/>
    <CipherData><CipherReference URI="OEBPS/fonts/embedded.ttf"/></CipherData>
  </EncryptedData>
</encryption>"#;

    const OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Fonted</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">9783407868213</dc:identifier>
  </metadata>
  <manifest>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="font" href="fonts/embedded.ttf" media-type="application/vnd.ms-opentype"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

    let scrambled: Vec<u8> = (0..300u32).map(|i| (i % 250) as u8).collect();
    let bytes = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("META-INF/encryption.xml", ENCRYPTION_XML),
        ("OEBPS/content.opf", OPF),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
        ("OEBPS/fonts/embedded.ttf", &scrambled),
    ]);

    let result = epub_tailor_core::read_epub(&bytes).expect("should still read");
    assert_eq!(
        result.book.resources["OEBPS/fonts/embedded.ttf"].data, scrambled,
        "without a usable key the font must be left untouched, not further corrupted"
    );
    assert!(
        !result
            .transformations
            .iter()
            .any(|t| t.kind == "font-deobfuscated"),
        "must not claim success when no key could be derived, got {:?}",
        result.transformations
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.message.contains("could not derive")),
        "expected a warning about the missing key, got {:?}",
        result.warnings
    );
}

/// Two `EncryptedData` entries name the SAME resource - one via a plain
/// path, one via a `./`-prefixed path that `normalize_href` resolves to the
/// same key - so this proves de-obfuscation runs at most once per resource.
/// XOR is its own inverse: applying it twice would silently restore the
/// scrambled bytes while still reporting two successes.
#[test]
fn duplicate_cipher_references_deobfuscate_only_once() {
    const UNIQUE_ID: &str = "urn:uuid:00000000-0000-0000-0000-000000000000";
    let original_font: Vec<u8> = (0..300u32).map(|i| (i % 250) as u8).collect();
    let obfuscated_font = idpf_obfuscate(&original_font, UNIQUE_ID);

    const ENCRYPTION_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#">
    <EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/>
    <CipherData><CipherReference URI="OEBPS/fonts/embedded.ttf"/></CipherData>
  </EncryptedData>
  <EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#">
    <EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/>
    <CipherData><CipherReference URI="./OEBPS/fonts/embedded.ttf"/></CipherData>
  </EncryptedData>
</encryption>"#;

    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Fonted</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">{UNIQUE_ID}</dc:identifier>
  </metadata>
  <manifest>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="font" href="fonts/embedded.ttf" media-type="application/vnd.ms-opentype"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
    );

    let bytes = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("META-INF/encryption.xml", ENCRYPTION_XML),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
        ("OEBPS/fonts/embedded.ttf", &obfuscated_font),
    ]);

    let result = epub_tailor_core::read_epub(&bytes).expect("should read despite the duplicate");
    assert_eq!(
        result.book.resources["OEBPS/fonts/embedded.ttf"].data, original_font,
        "a duplicate CipherReference must not XOR the font a second time"
    );
    assert_eq!(
        result
            .transformations
            .iter()
            .filter(|t| t.kind == "font-deobfuscated")
            .count(),
        1,
        "exactly one font-deobfuscated transformation, not one per duplicate entry"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.message.contains("more than once")),
        "expected a warning about the duplicate reference, got {:?}",
        result.warnings
    );
}

/// A one-chapter EPUB3 whose sole spine itemref is `linear="no"`: `parse_spine`
/// rescues it (rather than skipping every itemref and leaving `book.spine`
/// empty) so the book still has readable content, warning once with the OPF
/// path.
fn all_linear_no_fixture() -> Vec<u8> {
    const OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>All Linear No</dc:title>
    <dc:creator>Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:99999999-9999-9999-9999-999999999999</dc:identifier>
  </metadata>
  <manifest>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1" linear="no"/>
  </spine>
</package>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", OPF),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
    ])
}

#[test]
fn converting_an_all_linear_no_book_rescues_the_spine_and_stays_clean() {
    // RED before the rescue: convert() panics in debug builds because the
    // synthesized nav/NCX point at a non-spine resource (the spine is empty),
    // tripping the spine-toc-sync self-check.
    let converted = convert(
        Input::Epub(all_linear_no_fixture()),
        &ConvertOptions::default(),
    )
    .expect("an all-linear=\"no\" book should convert once its spine is rescued");

    let out = read_epub(&converted.epub)
        .expect("converted output should read")
        .book;
    let spine_path = out
        .spine
        .first()
        .expect("the rescued output must have a spine chapter");
    let body = String::from_utf8_lossy(&out.resources[spine_path].data);
    assert!(
        body.contains("Text."),
        "the rescued chapter's body must appear in the output, got: {body}"
    );
    assert!(
        converted.report.warnings.iter().any(|w| w
            .message
            .contains("keeping the non-linear ones so the book is not empty")),
        "the rescue warning must be surfaced in the report, got: {:?}",
        converted.report.warnings
    );
    assert_epubcheck_clean("all-linear-no", &converted.epub);
}

#[test]
fn all_linear_no_spine_is_rescued_and_warns_with_the_opf_path() {
    let result = epub_tailor_core::read_epub(&all_linear_no_fixture())
        .expect("an all-linear=\"no\" book should still read");

    // Every resolvable itemref is kept in document order so the book is not
    // empty (rescued) rather than skipped.
    assert_eq!(
        result.book.spine,
        vec!["OEBPS/text/chapter1.xhtml".to_string()],
        "expected the linear=\"no\" itemref to be rescued into the spine, got {:?}",
        result.book.spine
    );
    // Exactly one rescue warning, naming the OPF path.
    assert!(
        result.warnings.iter().any(|w| w
            .message
            .contains("keeping the non-linear ones so the book is not empty")
            && w.file.as_deref() == Some(result.book.opf_path.as_str())),
        "expected the rescue warning naming the OPF path, got {:?}",
        result.warnings
    );
    // The per-item "skipped" warning must not appear in the rescue case.
    assert!(
        !result
            .warnings
            .iter()
            .any(|w| w.message.contains("linear=\"no\"; skipped")),
        "the per-item skip warning must not appear in the rescue case, got {:?}",
        result.warnings
    );
}

/// A two-chapter EPUB3 whose first itemref is linear (default) and whose second
/// is `linear="no"`: because a linear itemref exists, the non-linear one is
/// skipped exactly as before (no rescue).
fn mixed_linear_fixture() -> Vec<u8> {
    const OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Mixed Linear</dc:title>
    <dc:creator>Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:88888888-8888-8888-8888-888888888888</dc:identifier>
  </metadata>
  <manifest>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="text/chapter2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="ch2" linear="no"/>
  </spine>
</package>"#;

    const CHAPTER2: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 2</title></head>
<body><h1>Chapter 2</h1><p>Text.</p></body></html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", OPF),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
        ("OEBPS/text/chapter2.xhtml", CHAPTER2),
    ])
}

#[test]
fn mixed_linear_spine_skips_only_the_non_linear_item_with_its_per_item_warning() {
    let result =
        epub_tailor_core::read_epub(&mixed_linear_fixture()).expect("a mixed spine should read");

    // Only the linear chapter is kept; the non-linear one is skipped.
    assert_eq!(
        result.book.spine,
        vec!["OEBPS/text/chapter1.xhtml".to_string()],
        "the non-linear chapter must be skipped when a linear one exists, got {:?}",
        result.book.spine
    );
    // The per-item skip warning names the skipped chapter's href.
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.message.contains("linear=\"no\"; skipped")
                && w.file.as_deref() == Some("OEBPS/text/chapter2.xhtml")),
        "expected the per-item skip warning naming the skipped chapter, got {:?}",
        result.warnings
    );
    // No rescue warning in the mixed case.
    assert!(
        !result.warnings.iter().any(|w| w
            .message
            .contains("keeping the non-linear ones so the book is not empty")),
        "the rescue warning must not appear when a linear itemref exists, got {:?}",
        result.warnings
    );
}

/// A two-item spine whose only LINEAR itemref dangles (its idref names no
/// manifest item) while a resolvable `linear="no"` itemref exists: no linear
/// itemref resolves, so the rescue still fires - and its message must not claim
/// "every spine itemref is linear='no'" when a linear one was in fact present
/// but unresolvable.
fn dangling_linear_plus_nonlinear_fixture() -> Vec<u8> {
    const OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Dangling Linear</dc:title>
    <dc:creator>Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:77777777-7777-7777-7777-777777777777</dc:identifier>
  </metadata>
  <manifest>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ghost"/>
    <itemref idref="ch1" linear="no"/>
  </spine>
</package>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", OPF),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
    ])
}

#[test]
fn dangling_linear_itemref_still_rescues_with_the_precise_message() {
    let result = epub_tailor_core::read_epub(&dangling_linear_plus_nonlinear_fixture())
        .expect("a book with a dangling linear itemref should still read");

    // No linear itemref resolves, so the resolvable non-linear chapter is
    // rescued rather than left out.
    assert_eq!(
        result.book.spine,
        vec!["OEBPS/text/chapter1.xhtml".to_string()],
        "the resolvable non-linear itemref must be rescued into the spine, got {:?}",
        result.book.spine
    );
    // The rescue message must be the precise wording (it does not claim every
    // itemref was linear="no" - one was linear but dangled).
    assert!(
        result.warnings.iter().any(|w| w.message
            == "the spine has no readable linear items - keeping the non-linear ones so the book is not empty"),
        "expected the precise rescue message, got {:?}",
        result.warnings
    );
}

/// A one-chapter EPUB3 whose spine has no itemrefs at all: there is genuinely
/// nothing to convert.
fn empty_spine_fixture() -> Vec<u8> {
    const OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>No Itemrefs</dc:title>
    <dc:creator>Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:77777777-7777-7777-7777-777777777777</dc:identifier>
  </metadata>
  <manifest>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
  </spine>
</package>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", OPF),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
    ])
}

#[test]
fn converting_a_book_with_no_spine_itemrefs_errors_cleanly() {
    // A spine with no itemrefs has nothing to convert: a clean error, not a
    // panic and not a broken book. (`Converted` is not `Debug`, so match rather
    // than `expect_err`.)
    match convert(
        Input::Epub(empty_spine_fixture()),
        &ConvertOptions::default(),
    ) {
        Err(ConvertError::EmptySpine) => {}
        Err(other) => panic!("expected EmptySpine, got {other:?}"),
        Ok(_) => panic!("a spine with no itemrefs must error, not produce a broken book"),
    }
}

#[test]
fn windows_1252_chapter_is_normalized_to_utf8() {
    let mut chapter = Vec::new();
    chapter.extend_from_slice(
        br#"<?xml version="1.0" encoding="windows-1252"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Caf"#,
    );
    chapter.push(0xE9); // 'é' in windows-1252
    chapter.extend_from_slice(b"</title></head><body><p>Caf");
    chapter.push(0xE9);
    chapter.extend_from_slice(b"</p></body></html>");

    let opf = minimal_opf("text/chapter1.xhtml");
    let bytes = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/text/chapter1.xhtml", &chapter),
    ]);

    let result = epub_tailor_core::read_epub(&bytes).expect("should read despite windows-1252");
    let resource = &result.book.resources["OEBPS/text/chapter1.xhtml"];
    let text = std::str::from_utf8(&resource.data).expect("resource must be valid utf-8");

    assert!(
        text.contains('é'),
        "expected 'é' in normalized text: {text}"
    );
    assert!(
        text.contains(r#"encoding="UTF-8""#),
        "expected xml decl to declare UTF-8, got: {text}"
    );
    assert!(
        !text.to_lowercase().contains("windows-1252"),
        "xml decl should no longer mention windows-1252: {text}"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.message.contains("windows-1252")
                && w.file.as_deref() == Some("OEBPS/text/chapter1.xhtml")),
        "expected a warning naming the file and source encoding, got {:?}",
        result.warnings
    );
}

#[test]
fn windows_1252_opf_is_normalized_exactly_once() {
    // The OPF is decoded once (~line 96, to parse the package document) and
    // would previously be decoded again while building the final resources
    // map (~line 125, since its guessed media type is also a "text" type).
    // Assert that only a single transcode warning is produced for it and
    // that the stored OPF resource is valid, rewritten UTF-8.
    let mut opf = Vec::new();
    opf.extend_from_slice(
        br#"<?xml version="1.0" encoding="windows-1252"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Caf"#,
    );
    opf.push(0xE9); // 'é' in windows-1252
    opf.extend_from_slice(
        br#"</dc:title>
    <dc:creator>Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:00000000-0000-0000-0000-000000000000</dc:identifier>
  </metadata>
  <manifest>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#,
    );

    let bytes = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", &opf),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
    ]);

    let result = epub_tailor_core::read_epub(&bytes).expect("should read despite windows-1252 opf");

    assert_eq!(result.book.metadata.title, "Café");

    let resource = &result.book.resources["OEBPS/content.opf"];
    let text = std::str::from_utf8(&resource.data).expect("resource must be valid utf-8");
    assert!(text.contains("Café"), "expected 'Café' in OPF: {text}");
    assert!(
        text.contains(r#"encoding="UTF-8""#),
        "expected xml decl to declare UTF-8, got: {text}"
    );
    assert!(
        !text.to_lowercase().contains("windows-1252"),
        "xml decl should no longer mention windows-1252: {text}"
    );

    let matching: Vec<_> = result
        .warnings
        .iter()
        .filter(|w| {
            w.message.contains("windows-1252") && w.file.as_deref() == Some("OEBPS/content.opf")
        })
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one windows-1252 transcode warning for the OPF, got {:?}",
        result.warnings
    );
}

#[test]
fn nested_percent_encoded_href_resolves() {
    let opf = minimal_opf("text/ch%201.xhtml");
    let bytes = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/text/ch 1.xhtml", CHAPTER1),
    ]);

    let result = epub_tailor_core::read_epub(&bytes).expect("percent-encoded href should resolve");
    assert_eq!(result.book.spine, vec!["OEBPS/text/ch 1.xhtml".to_string()]);
    assert!(result.book.resources.contains_key("OEBPS/text/ch 1.xhtml"));
}

#[test]
fn not_a_zip_file_is_invalid_epub() {
    let err = epub_tailor_core::read_epub(b"this is definitely not a zip file")
        .expect_err("garbage bytes should not parse as an epub");
    assert!(
        matches!(err, ConvertError::InvalidEpub(_)),
        "expected InvalidEpub, got {err:?}"
    );
}

// ---------------------------------------------------------------------
// Zip entry name normalization (T4): raw zip names are normalized into
// resource keys with the same shape `normalize_href` gives manifest hrefs,
// so a `./`-prefixed or percent-encoded entry no longer misses lookups.
// ---------------------------------------------------------------------

/// A small, valid 2x2 grayscale PNG, so the fixture's cover decodes cleanly
/// through both the lint's image-format check and the convert image pipeline.
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x00, 0x00, 0x00, 0x00, 0x57, 0xDD, 0x52,
    0xF8, 0x00, 0x00, 0x00, 0x0E, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xF8, 0xFF, 0x9F, 0xE1,
    0xFF, 0x7F, 0x00, 0x0B, 0xFA, 0x03, 0xFD, 0xFD, 0x4D, 0xC4, 0x66, 0x00, 0x00, 0x00, 0x00, 0x49,
    0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// A unique marker string embedded in the fixture's spine chapter body, so a
/// convert can assert the chapter survived into the output.
const BODY_MARKER: &str = "NORMALIZEDBODYMARKER";

/// A one-chapter EPUB3 whose ZIP entry names are deliberately "messy" while the
/// manifest hrefs are plain: the chapter entry is `./`-prefixed and the cover
/// image entry is percent-encoded, but the manifest references them by their
/// plain, decoded, dot-free hrefs. Every entry's normalized key must reconcile
/// with the corresponding normalized manifest href.
fn messy_entry_names_fixture() -> Vec<u8> {
    const CONTAINER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    // The image manifest href is plain and declared image/png though the entry
    // is named `.bin`, so the read pipeline must take the declared type over the
    // extension guess.
    const OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Messy Entry Names</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:44444444-4444-4444-4444-444444444444</dc:identifier>
    <meta name="cover" content="cover-img"/>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="cover-img" href="images/cover.bin" media-type="image/png" properties="cover-image"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

    const NAV: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol>
<li><a href="text/chapter1.xhtml">Chapter 1</a></li>
</ol></nav></body></html>"#;

    let chapter = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body><h1>Chapter 1</h1><p>{BODY_MARKER}</p></body></html>"#
    );

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", OPF),
        ("OEBPS/nav.xhtml", NAV),
        // `./`-prefixed chapter entry: normalizes to OEBPS/text/chapter1.xhtml.
        ("./OEBPS/text/chapter1.xhtml", chapter.as_bytes()),
        // Percent-encoded image entry (%76 is 'v'): the raw name
        // OEBPS/images/co%76er.bin percent-decodes to OEBPS/images/cover.bin,
        // matching the plain manifest href once the entry name is normalized.
        ("OEBPS/images/co%76er.bin", TINY_PNG),
    ])
}

#[test]
fn messy_entry_names_are_normalized_into_resource_keys() {
    let book = read_epub(&messy_entry_names_fixture())
        .expect("messy but well-formed epub should read")
        .book;

    // The `./`-prefixed chapter and the percent-encoded image are both keyed by
    // their normalized paths, matching the (already normalized) manifest hrefs.
    assert!(
        book.resources.contains_key("OEBPS/text/chapter1.xhtml"),
        "chapter should be keyed by its normalized path, got keys {:?}",
        book.resources.keys().collect::<Vec<_>>()
    );
    assert!(
        book.resources.contains_key("OEBPS/images/cover.bin"),
        "image should be keyed by its normalized (percent-decoded) path, got keys {:?}",
        book.resources.keys().collect::<Vec<_>>()
    );
    assert_eq!(book.spine, vec!["OEBPS/text/chapter1.xhtml".to_string()]);
    assert_eq!(book.cover.as_deref(), Some("OEBPS/images/cover.bin"));
}

#[test]
fn declared_media_type_wins_over_extension_guess_for_normalized_entry() {
    let book = read_epub(&messy_entry_names_fixture())
        .expect("messy but well-formed epub should read")
        .book;

    // The image entry is named `.bin` (would guess application/octet-stream)
    // but declared image/png; the declared type must win now that the
    // normalized resource key matches the normalized manifest href.
    assert_eq!(
        book.resources["OEBPS/images/cover.bin"].media_type,
        "image/png"
    );
}

#[test]
fn converting_messy_entry_names_keeps_the_spine_chapter() {
    let converted = convert(
        Input::Epub(messy_entry_names_fixture()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");

    // Re-read the output: the chapter must still be in the spine and its body
    // text must have survived (RED today: the spine chapter is silently skipped
    // because its `./`-prefixed key never matches the normalized spine path).
    let out = read_epub(&converted.epub)
        .expect("converted output should read")
        .book;
    let spine_path = out
        .spine
        .first()
        .expect("converted output must have a spine chapter");
    let body = String::from_utf8_lossy(&out.resources[spine_path].data);
    assert!(
        body.contains(BODY_MARKER),
        "the spine chapter's body must appear in the output, got: {body}"
    );
}

#[test]
fn linting_messy_entry_names_has_no_false_manifest_sync_findings() {
    let findings = lint_epub(
        &messy_entry_names_fixture(),
        &DeviceCaps::x4(),
        &Features::all_on(),
    );
    let manifest_sync: Vec<&_> = findings
        .iter()
        .filter(|f| f.code == "manifest-sync")
        .collect();
    assert!(
        manifest_sync.is_empty(),
        "normalized entry keys must reconcile with normalized manifest hrefs, got: {manifest_sync:#?}"
    );
}

#[test]
fn converting_messy_entry_names_roundtrips_clean_through_epubcheck() {
    let converted = convert(
        Input::Epub(messy_entry_names_fixture()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");
    assert_epubcheck_clean("messy-entry-names", &converted.epub);
}

/// Two ZIP entries whose raw names normalize to the same key: the first must
/// win, the second is dropped, and both `read_epub` and `lint_epub` report the
/// collision.
fn colliding_entry_names_fixture() -> Vec<u8> {
    const CONTAINER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Colliding Entry Names</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:55555555-5555-5555-5555-555555555555</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

    const NAV: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol>
<li><a href="text/chapter1.xhtml">Chapter 1</a></li>
</ol></nav></body></html>"#;

    const CHAPTER_FIRST: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body><h1>Chapter 1</h1><p>FIRSTWINS</p></body></html>"#;

    const CHAPTER_SECOND: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body><h1>Chapter 1</h1><p>SECONDDROP</p></body></html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", OPF),
        ("OEBPS/nav.xhtml", NAV),
        // Both normalize to OEBPS/text/chapter1.xhtml; the first must win. The
        // MESSY entry is deliberately first, so the kept and dropped raw names
        // differ and the collision message must name both distinctly.
        ("./OEBPS/text/chapter1.xhtml", CHAPTER_FIRST),
        ("OEBPS/text/chapter1.xhtml", CHAPTER_SECOND),
    ])
}

#[test]
fn colliding_entry_names_keep_the_first_and_warn() {
    let result = read_epub(&colliding_entry_names_fixture()).expect("collision should still read");

    let body = String::from_utf8_lossy(&result.book.resources["OEBPS/text/chapter1.xhtml"].data);
    assert!(
        body.contains("FIRSTWINS") && !body.contains("SECONDDROP"),
        "the first colliding entry's bytes must be kept, got: {body}"
    );
    assert!(
        result.warnings.iter().any(|w| {
            w.message.contains("normalize to the same path")
                // The messy entry won: kept and dropped raw names must both be
                // named, and distinctly - not the same string twice.
                && w.message.contains("'./OEBPS/text/chapter1.xhtml'")
                && w.message.contains("'OEBPS/text/chapter1.xhtml'")
        }),
        "the collision warning must name both distinct raw entries, got: {:?}",
        result.warnings
    );
}

#[test]
fn colliding_entry_names_produce_an_entry_collision_lint_finding() {
    let findings = lint_epub(
        &colliding_entry_names_fixture(),
        &DeviceCaps::x4(),
        &Features::all_on(),
    );
    assert!(
        findings
            .iter()
            .any(|f| f.code == "entry-collision" && f.severity == Severity::Warning),
        "expected an entry-collision Warning finding, got: {findings:#?}"
    );
}

// ---------------------------------------------------------------------
// epubcheck gate (skip-if-unavailable), mirroring the other test binaries.
// ---------------------------------------------------------------------

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
