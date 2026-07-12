//! The metadata a book arrives with must survive the rewrite.
//!
//! Until 0.2 it did not. The reader looked up four element names and the OPF
//! template emitted four; a book with a publisher, a blurb, subjects, a date
//! and an ISBN came back with none of them, and no warning either. Nothing
//! noticed because **no fixture in the suite had any of those fields** - so the
//! first thing here is a fixture that does.

mod common;

use std::io::{Cursor, Read};

use common::build_epub;
use epub_tailor_core::metadata::{MergeMode, MetadataDoc};
use epub_tailor_core::profile::resolve;
use epub_tailor_core::{ConvertOptions, Input, convert, read_epub};
use zip::ZipArchive;

/// A book whose OPF carries everything the old reader threw away: a publisher,
/// a description, three subjects, a date, rights, a second creator with a sort
/// key and a role, an ISBN with a scheme, and a series.
fn epub_with_rich_metadata() -> Vec<u8> {
    const CONTAINER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

    const OPF: &[u8] = br##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" xmlns:dc="http://purl.org/dc/elements/1.1/"
         version="3.0" unique-identifier="pub-id">
  <metadata>
    <dc:identifier id="pub-id">urn:uuid:aaaaaaaa-1111-2222-3333-444444444444</dc:identifier>
    <dc:identifier id="isbn">9780261102217</dc:identifier>
    <meta refines="#isbn" property="identifier-type">ISBN</meta>
    <dc:title>The Rich Book</dc:title>
    <dc:creator id="a1">J. R. R. Tolkien</dc:creator>
    <meta refines="#a1" property="file-as">Tolkien, J. R. R.</meta>
    <meta refines="#a1" property="role" scheme="marc:relators">aut</meta>
    <dc:contributor id="c1">Christopher Tolkien</dc:contributor>
    <meta refines="#c1" property="role" scheme="marc:relators">edt</meta>
    <dc:language>en</dc:language>
    <dc:publisher>Allen &amp; Unwin</dc:publisher>
    <dc:description>In a hole in the ground there lived a hobbit.</dc:description>
    <dc:subject>Fantasy</dc:subject>
    <dc:subject>Adventure</dc:subject>
    <dc:subject>Middle-earth</dc:subject>
    <dc:date>1937-09-21</dc:date>
    <dc:rights>Public domain in some jurisdictions.</dc:rights>
    <meta property="belongs-to-collection" id="series">The Hobbit Cycle</meta>
    <meta refines="#series" property="group-position">1</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"##;

    const NAV: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol><li><a href="chapter1.xhtml">One</a></li></ol></nav></body>
</html>"#;

    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>One</title></head>
<body><h1>One</h1><p>Body.</p></body></html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", OPF),
        ("OEBPS/nav.xhtml", NAV),
        ("OEBPS/chapter1.xhtml", CHAPTER),
    ])
}

/// A minimal book with nothing but the four fields the old model knew.
fn epub_with_bare_metadata() -> Vec<u8> {
    common::epub3_minimal()
}

fn opf_of(epub: &[u8]) -> String {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    let mut out = String::new();
    archive
        .by_name("OEBPS/content.opf")
        .expect("output has an OPF")
        .read_to_string(&mut out)
        .expect("OPF is UTF-8");
    out
}

fn convert_with(profile: &str, epub: Vec<u8>) -> Vec<u8> {
    let resolved = resolve(&[profile.to_string()]).expect("profile resolves");
    convert(Input::Epub(epub), &resolved.to_options())
        .expect("conversion succeeds")
        .epub
}

#[test]
fn every_metadata_field_survives_the_rewrite() {
    // The whole point. Before 0.2 all of this vanished, silently.
    let opf = opf_of(&convert_with("epub", epub_with_rich_metadata()));

    for expected in [
        // The ampersand comes back as the numeric reference `&#38;`, which is
        // the same character to any XML parser, so match on the element instead
        // of pinning one of the two legal spellings.
        "<dc:publisher>Allen ",
        "<dc:description>In a hole in the ground there lived a hobbit.</dc:description>",
        "<dc:subject>Fantasy</dc:subject>",
        "<dc:subject>Adventure</dc:subject>",
        "<dc:subject>Middle-earth</dc:subject>",
        "<dc:date>1937-09-21</dc:date>",
        "<dc:rights>Public domain in some jurisdictions.</dc:rights>",
        "9780261102217",
        "The Hobbit Cycle",
    ] {
        assert!(opf.contains(expected), "lost {expected:?} in:\n{opf}");
    }
    // ...and the round-trip test below proves the ampersand really is an
    // ampersand when it is read back.
}

#[test]
fn author_sort_keys_and_roles_survive() {
    // `file-as` is what a Kobo sorts your shelf by; losing it reshelves the book
    // under "J".
    let opf = opf_of(&convert_with("epub", epub_with_rich_metadata()));
    assert!(opf.contains("Tolkien, J. R. R."), "lost file-as:\n{opf}");
    assert!(opf.contains("Christopher Tolkien"), "lost contributor");
    assert!(opf.contains(r#"property="role""#), "lost role");
}

#[test]
fn the_isbn_stays_a_secondary_identifier_and_the_unique_one_is_untouched() {
    let out = convert_with("epub", epub_with_rich_metadata());
    let book = read_epub(&out).expect("reads back").book;
    assert_eq!(
        book.metadata.identifier.as_deref(),
        Some("urn:uuid:aaaaaaaa-1111-2222-3333-444444444444"),
        "the unique identifier must not move"
    );
    assert_eq!(book.metadata.identifiers.len(), 1);
    assert_eq!(book.metadata.identifiers[0].value, "9780261102217");
    assert_eq!(
        book.metadata.identifiers[0].scheme.as_deref(),
        Some("ISBN"),
        "the identifier-type refinement must round-trip"
    );
}

#[test]
fn a_rich_book_round_trips_through_read_convert_read() {
    // Convert twice: the second pass must see everything the first wrote.
    let once = convert_with("epub", epub_with_rich_metadata());
    let twice = convert_with("epub", once);
    let book = read_epub(&twice).expect("reads back").book;
    let m = &book.metadata;

    assert_eq!(m.title, "The Rich Book");
    assert_eq!(m.publisher.as_deref(), Some("Allen & Unwin"));
    assert_eq!(
        m.description.as_deref(),
        Some("In a hole in the ground there lived a hobbit.")
    );
    assert_eq!(m.subjects, vec!["Fantasy", "Adventure", "Middle-earth"]);
    assert_eq!(m.date.as_deref(), Some("1937-09-21"));
    assert_eq!(m.authors[0].name, "J. R. R. Tolkien");
    assert_eq!(m.authors[0].file_as.as_deref(), Some("Tolkien, J. R. R."));
    assert_eq!(m.authors[0].role.as_deref(), Some("aut"));
    assert_eq!(m.contributors[0].name, "Christopher Tolkien");
    let series = m.series.as_ref().expect("series survives");
    assert_eq!(series.name, "The Hobbit Cycle");
    assert_eq!(series.index.as_deref(), Some("1"));
}

#[test]
fn a_calibre_series_is_understood_too() {
    // A large share of sideloaded EPUBs come out of Calibre, which writes its
    // own spelling instead of the EPUB3 collection refinement.
    const OPF: &[u8] = br##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" xmlns:dc="http://purl.org/dc/elements/1.1/"
         version="3.0" unique-identifier="pub-id">
  <metadata>
    <dc:identifier id="pub-id">urn:uuid:cal</dc:identifier>
    <dc:title>Calibre Book</dc:title>
    <dc:language>en</dc:language>
    <meta name="calibre:series" content="Discworld"/>
    <meta name="calibre:series_index" content="8"/>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"##;
    const NAV: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol><li><a href="chapter1.xhtml">One</a></li></ol></nav></body></html>"#;
    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>One</title></head>
<body><p>Body.</p></body></html>"#;
    const CONTAINER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

    let epub = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER),
        ("OEBPS/content.opf", OPF),
        ("OEBPS/nav.xhtml", NAV),
        ("OEBPS/chapter1.xhtml", CHAPTER),
    ]);
    let book = read_epub(&convert_with("epub", epub))
        .expect("reads back")
        .book;
    let series = book.metadata.series.expect("calibre series is read");
    assert_eq!(series.name, "Discworld");
    assert_eq!(series.index.as_deref(), Some("8"));
}

#[test]
fn a_supplied_document_fills_the_gaps_without_touching_what_is_there() {
    // The user's actual complaint: the book is missing a description and a
    // publisher. Fill them, and leave everything else exactly as it was.
    let mut opts = ConvertOptions {
        features: epub_tailor_core::profile::Features::repair_only(),
        ..ConvertOptions::default()
    };
    opts.metadata = MetadataDoc::parse(
        "publisher: Supplied Press\ndescription: A supplied blurb.\nsubjects: [Supplied]\n",
    )
    .expect("doc parses");
    opts.metadata_merge = MergeMode::Fill;

    let converted =
        convert(Input::Epub(epub_with_bare_metadata()), &opts).expect("conversion succeeds");
    let book = read_epub(&converted.epub).expect("reads back").book;

    assert_eq!(book.metadata.publisher.as_deref(), Some("Supplied Press"));
    assert_eq!(
        book.metadata.description.as_deref(),
        Some("A supplied blurb.")
    );
    assert_eq!(book.metadata.subjects, vec!["Supplied"]);
    // The book's own title and author are left alone.
    assert_eq!(book.metadata.title, "Sample Book");
    assert_eq!(book.metadata.authors[0].name, "Jane Author");
}

/// Run epubcheck on `epub`, skipping (not failing) when it is not installed -
/// the same harness pattern the other suites use.
fn assert_epubcheck_clean(epub: &[u8], name: &str) {
    use std::path::PathBuf;
    use std::process::Command;

    let out_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.epub"));
    std::fs::write(&out_path, epub).expect("write epub");

    let output = match Command::new("epubcheck").arg(&out_path).output() {
        Ok(output) => output,
        Err(_) => match std::env::var("EPUBCHECK_JAR") {
            Ok(jar) => Command::new("java")
                .args(["-jar", &jar])
                .arg(&out_path)
                .output()
                .expect("run epubcheck jar"),
            Err(_) => {
                eprintln!("SKIP: epubcheck not found; skipping {name}");
                return;
            }
        },
    };
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let offenders: Vec<&str> = combined
        .lines()
        .filter(|l| l.contains("FATAL(") || l.contains("ERROR(") || l.contains("WARNING("))
        .collect();
    assert!(
        offenders.is_empty(),
        "epubcheck rejected the metadata we emit:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_metadata_we_emit_is_valid_epub3() {
    // The refinements, the collection block and the second identifier are all
    // new markup. epubcheck is the only thing that will tell us whether the
    // `<metadata>` we now write is actually legal, so the rich fixture goes
    // through it.
    assert_epubcheck_clean(
        &convert_with("epub", epub_with_rich_metadata()),
        "rich-metadata",
    );
}

#[test]
fn a_supplied_cover_is_embedded_and_stays_valid() {
    // A 1x1 PNG is enough to prove the resource, the manifest properties and the
    // cover hint all line up.
    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x3A,
        0x7E, 0x9B, 0x55, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    let opts = ConvertOptions {
        features: epub_tailor_core::profile::Features::repair_only(),
        cover_image: Some(epub_tailor_core::CoverImage {
            data: PNG_1X1.to_vec(),
            media_type: "image/png".to_string(),
            file_name: "cover.png".to_string(),
        }),
        ..ConvertOptions::default()
    };
    // epub3_minimal already has a cover; this must replace it, not duplicate it.
    let converted =
        convert(Input::Epub(epub_with_bare_metadata()), &opts).expect("conversion succeeds");
    let book = read_epub(&converted.epub).expect("reads back").book;
    assert_eq!(book.cover.as_deref(), Some("OEBPS/cover.png"));
    assert_epubcheck_clean(&converted.epub, "supplied-cover");
}

#[test]
fn fill_mode_will_not_overwrite_a_publisher_the_book_already_has() {
    let mut opts = ConvertOptions {
        features: epub_tailor_core::profile::Features::repair_only(),
        ..ConvertOptions::default()
    };
    opts.metadata = MetadataDoc::parse("publisher: A Worse Guess").expect("doc parses");

    let converted =
        convert(Input::Epub(epub_with_rich_metadata()), &opts).expect("conversion succeeds");
    let book = read_epub(&converted.epub).expect("reads back").book;
    assert_eq!(
        book.metadata.publisher.as_deref(),
        Some("Allen & Unwin"),
        "fill mode must not clobber the book's own publisher"
    );
}
