//! The generic profile: per-copy marker removal and the convergence property.

mod common;

use epub_tailor_core::generic::invisible::scrub;
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

/// A one-pixel JPEG carrying an APP1 (EXIF) segment.
fn jpeg_with_exif() -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8]; // SOI
    // APP1 with an EXIF header and an identifying payload.
    let payload = b"Exif\0\0BUYER-635962014";
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(payload);
    // A minimal but structurally valid scan: SOS then EOI.
    out.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9]);
    out
}

#[test]
fn generic_strips_exif_but_keeps_the_pixels() {
    let jpeg = jpeg_with_exif();
    let mut book = common::book_with_image("OEBPS/pic.jpg", &jpeg);
    let out = convert(
        Input::Epub(std::mem::take(&mut book)),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    let stored = common::entry(&out.epub, "OEBPS/pic.jpg").expect("image survives");
    assert!(
        !stored.windows(9).any(|w| w == b"BUYER-635"),
        "the EXIF payload must be gone"
    );
    assert_eq!(&stored[..2], &[0xFF, 0xD8], "still a JPEG");
    assert!(stored.ends_with(&[0xFF, 0xD9]), "still terminated");
}

#[test]
fn scrub_removes_the_unconditional_invisibles() {
    let (out, n) = scrub("He\u{200B}llo\u{2060} world\u{FEFF}!");
    assert_eq!(out, "Hello world!");
    assert_eq!(n, 3);
}

#[test]
fn scrub_keeps_zwnj_where_the_script_needs_it() {
    // Persian: ZWNJ between two Arabic-script letters is meaningful.
    let persian = "\u{0645}\u{06CC}\u{200C}\u{062E}\u{0648}\u{0627}\u{0645}";
    let (out, n) = scrub(persian);
    assert_eq!(out, persian, "Persian ZWNJ must survive");
    assert_eq!(n, 0);
}

#[test]
fn scrub_keeps_zwj_inside_emoji_sequences() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let (out, n) = scrub(family);
    assert_eq!(out, family, "emoji ZWJ sequences must survive");
    assert_eq!(n, 0);
}

#[test]
fn scrub_removes_zwnj_between_latin_letters() {
    // Latin has no use for ZWNJ: here it can only be a fingerprint.
    let (out, n) = scrub("wa\u{200C}termark");
    assert_eq!(out, "watermark");
    assert_eq!(n, 1);
}

#[test]
fn scrub_keeps_bidi_marks() {
    let (out, n) = scrub("a\u{200E}b\u{200F}c");
    assert_eq!(
        out, "a\u{200E}b\u{200F}c",
        "bidi marks are layout, not marks"
    );
    assert_eq!(n, 0);
}

// --- Asymmetric neighbour protection ---
//
// The five tests above are not enough on their own: in
// `scrub_keeps_zwnj_where_the_script_needs_it` and
// `scrub_keeps_zwj_inside_emoji_sequences` *both* neighbours are protected,
// so deleting either the `prev` or the `next` lookup out of `keep_mask`
// leaves the whole suite green. These two pin each side down alone.

#[test]
fn scrub_keeps_zwnj_protected_by_the_previous_neighbour_only() {
    // Persian letter before, plain Latin after: the left side alone must be
    // enough. Dropping the `prev` lookup would delete this.
    let s = "\u{0645}\u{200C}z";
    let (out, n) = scrub(s);
    assert_eq!(out, s, "left-side protection alone must be enough");
    assert_eq!(n, 0);
}

#[test]
fn scrub_keeps_zwnj_protected_by_the_next_neighbour_only() {
    // Plain Latin before, Persian letter after: the right side alone must be
    // enough. Dropping the `next` lookup would delete this.
    let s = "z\u{200C}\u{0645}";
    let (out, n) = scrub(s);
    assert_eq!(out, s, "right-side protection alone must be enough");
    assert_eq!(n, 0);
}

// --- Cross-node protection through the full `convert()` pipeline ---
//
// A per-text-node-only neighbour check sees `None` on both sides at every
// element boundary, so it deletes exactly the joiners this pass exists to
// protect once markup splits a word or an emoji sequence across elements -
// which is common, not rare: `<span epub:type="pagebreak"/>` markers are
// mandatory in print-derived EPUB 3 books, and Persian/Arabic line breaks
// tend to land inside a joined word. These exercise the real DOM pass.

/// A minimal one-chapter EPUB3 book with a caller-supplied title, TOC entry
/// title and chapter body, for exercising the invisible-character scrub
/// across markup and metadata.
fn book_with_chapter(title: &str, toc_title: &str, body: &str) -> Vec<u8> {
    let content_opf = format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="pub-id">urn:uuid:11111111-2222-4333-8444-555555555555</dc:identifier>
    <dc:title>{title}</dc:title>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch" href="chapter.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch"/></spine>
</package>"##
    );
    let nav = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body>
<nav epub:type="toc">
<ol>
<li><a href="chapter.xhtml">{toc_title}</a></li>
</ol>
</nav>
</body>
</html>"#
    );
    let chapter = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>C</title></head>
<body>{body}</body></html>"#
    );
    common::build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", common::CONTAINER_XML),
        ("OEBPS/content.opf", content_opf.as_bytes()),
        ("OEBPS/nav.xhtml", nav.as_bytes()),
        ("OEBPS/chapter.xhtml", chapter.as_bytes()),
    ])
}

fn chapter_text_of(epub: &[u8]) -> String {
    String::from_utf8(common::entry(epub, "OEBPS/chapter.xhtml").expect("chapter survives"))
        .expect("chapter is utf8")
}

fn nav_text_of(epub: &[u8]) -> String {
    String::from_utf8(common::entry(epub, "OEBPS/nav.xhtml").expect("nav survives"))
        .expect("nav is utf8")
}

#[test]
fn generic_keeps_a_zwnj_split_across_elements_by_markup() {
    // The reviewer's exact reproduction: <span>می</span>‌<span>خوام</span> -
    // the ZWNJ is its own text node, with markup as its only DOM siblings.
    let persian_left = "\u{0645}\u{06CC}";
    let persian_right = "\u{062E}\u{0648}\u{0627}\u{0645}";
    let zwnj = '\u{200C}';
    let body = format!("<p><span>{persian_left}</span>{zwnj}<span>{persian_right}</span></p>");
    let out = convert(
        Input::Epub(book_with_chapter("Book", "Chapter", &body)),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    let chapter = chapter_text_of(&out.epub);
    assert!(
        chapter.contains('\u{200C}'),
        "the mid-word ZWNJ split across <span> elements must survive:\n{chapter}"
    );
}

#[test]
fn generic_keeps_a_zwj_emoji_sequence_split_across_elements_by_markup() {
    // <span>👨</span>‍<span>👩</span> - the ZWJ is its own text node between
    // two elements, exactly like the pagebreak-marker case in real books.
    let body = "<p><span>\u{1F468}</span>\u{200D}<span>\u{1F469}</span></p>";
    let out = convert(
        Input::Epub(book_with_chapter("Book", "Chapter", body)),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    let chapter = chapter_text_of(&out.epub);
    assert!(
        chapter.contains('\u{200D}'),
        "the emoji ZWJ split across <span> elements must survive:\n{chapter}"
    );
}

#[test]
fn generic_removes_an_entity_encoded_zwsp_from_chapter_text() {
    // `&#8203;` decodes to U+200B during parsing; a raw-bytes pass would
    // never see the character at all and this would survive undetected.
    let body = "<p>foo&#8203;bar</p>";
    let out = convert(
        Input::Epub(book_with_chapter("Book", "Chapter", body)),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    let chapter = chapter_text_of(&out.epub);
    assert!(
        chapter.contains("foobar"),
        "the entity-encoded ZWSP must be scrubbed:\n{chapter}"
    );
    assert!(
        !chapter.contains('\u{200B}'),
        "no ZWSP should remain:\n{chapter}"
    );
}

#[test]
fn generic_scrubs_invisible_chars_from_the_title_and_toc() {
    // The writer regenerates the OPF title and the nav document from
    // `book.metadata`/`book.toc`, not from stored bytes, so a fingerprint
    // here survives a chapter-only scrub untouched.
    let title = "Book\u{200B}Title";
    let toc_title = "Chapter\u{200B}One";
    let out = convert(
        Input::Epub(book_with_chapter(title, toc_title, "<p>Text.</p>")),
        &opts_for(&["generic"]),
    )
    .expect("converts");

    let opf = opf_of(&out.epub);
    assert!(
        opf.contains("BookTitle"),
        "the title must be scrubbed:\n{opf}"
    );
    assert!(!opf.contains('\u{200B}'), "no ZWSP in the OPF:\n{opf}");

    let nav = nav_text_of(&out.epub);
    assert!(
        nav.contains("ChapterOne"),
        "the TOC entry title must be scrubbed:\n{nav}"
    );
    assert!(!nav.contains('\u{200B}'), "no ZWSP in the nav doc:\n{nav}");
}

#[test]
fn generic_drops_a_file_nothing_references() {
    let mut epub = common::book_with_extra_file("OEBPS/vendor-id.txt", b"SHTX001.635962014");
    let out = convert(
        Input::Epub(std::mem::take(&mut epub)),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    assert!(
        common::entry(&out.epub, "OEBPS/vendor-id.txt").is_none(),
        "an unreferenced stray file must not survive"
    );
}

#[test]
fn generic_keeps_a_file_referenced_only_from_css() {
    // Not in the manifest, reachable only through `url()`. Dropping it on
    // manifest membership would break the book; reachability keeps it.
    let mut epub = common::book_with_css_referenced_asset();
    let out = convert(
        Input::Epub(std::mem::take(&mut epub)),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    assert!(
        common::entry(&out.epub, "OEBPS/bg.png").is_some(),
        "a CSS-referenced asset must survive even when unmanifested"
    );
}

#[test]
fn the_epub_profile_keeps_unreferenced_files() {
    let mut epub = common::book_with_extra_file("OEBPS/vendor-id.txt", b"x");
    let out =
        convert(Input::Epub(std::mem::take(&mut epub)), &opts_for(&["epub"])).expect("converts");
    assert!(
        common::entry(&out.epub, "OEBPS/vendor-id.txt").is_some(),
        "repair-only must not change what survives"
    );
}

/// Mandatory regression coverage for the review's Critical 1: a null-namespace
/// attribute lookup made `xlink:href` a dead lookup, so a raster reachable
/// only through an SVG cover wrapper's `<image xlink:href>` was silently
/// deleted. Runs the real `convert()` pipeline, not just the unit-level
/// `reachable` walk, so it also proves nothing upstream (cover detection, the
/// SVG pass being off under plain `generic`) papers over the bug.
#[test]
fn generic_keeps_an_svg_cover_raster_referenced_only_via_xlink_href() {
    let mut epub = common::book_with_svg_cover_wrapper();
    let out = convert(
        Input::Epub(std::mem::take(&mut epub)),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    assert!(
        common::entry(&out.epub, "OEBPS/cover.jpg").is_some(),
        "the raster an SVG cover wraps via xlink:href must survive drop_unreferenced"
    );
}

/// Mandatory regression coverage for the review's Important 4: `<img
/// srcset>`'s target must survive `drop_unreferenced` even with no `src` at
/// all - the case where nothing else in the chapter names it once `srcset`
/// itself has been stripped. Runs the real `convert()` pipeline (not just the
/// unit-level `reachable`/`image::rewrite_refs` tests) so it also proves the
/// two passes actually wire together correctly, not just each in isolation.
#[test]
fn generic_keeps_an_img_srcset_target_with_no_src() {
    let mut epub = common::book_with_srcset_only_image();
    let out = convert(
        Input::Epub(std::mem::take(&mut epub)),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    assert!(
        common::entry(&out.epub, "OEBPS/only.png").is_some(),
        "an <img srcset> target with no src must survive drop_unreferenced"
    );
}

/// Mandatory regression coverage for a follow-up review round on Important 4:
/// a `<img srcset>` target must NOT be seeded as a global reachability root.
/// An orphan chapter (manifested, in neither spine nor nav) carrying `<img
/// srcset="wm.png 2x">` must itself be dropped as unreferenced, and `wm.png`,
/// named only through that orphan, must be dropped along with it rather than
/// surviving regardless of the orphan's own fate.
#[test]
fn generic_drops_an_img_srcset_target_owned_by_an_orphan_chapter() {
    let mut epub = common::book_with_orphan_chapter_srcset_image();
    let out = convert(
        Input::Epub(std::mem::take(&mut epub)),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    assert!(
        common::entry(&out.epub, "OEBPS/orphan.xhtml").is_none(),
        "the orphan chapter itself must still be dropped as unreferenced"
    );
    assert!(
        common::entry(&out.epub, "OEBPS/wm.png").is_none(),
        "a srcset target owned by a dropped orphan document must be dropped with it"
    );
}

/// Regression coverage for round 4's R1: `chapter_split` runs *after* the
/// srcset map is keyed by the pre-split chapter path, then `shift_remove`s
/// that path and retargets the spine to the numbered parts - so the map's key
/// became a path the reachability walk never visits, and the image it owned
/// was deleted. Fires under the `x4,generic` composition the docs recommend,
/// where `chapter_split` and `drop_unreferenced` are both on.
///
/// The fixture's nav deliberately does NOT link the oversize chapter: a stale
/// nav href pointing at the pre-split path accidentally re-reaches the map's
/// original key and masks the stranding entirely.
#[test]
fn generic_keeps_a_srcset_target_of_a_chapter_that_was_split() {
    let mut opts = opts_for(&["x4", "generic"]);
    // Well under the fixture's ~2KB chapter, so the split really happens.
    opts.max_chapter_bytes = 800;
    let mut epub = common::book_with_split_chapter_srcset_image();
    let out = convert(Input::Epub(std::mem::take(&mut epub)), &opts).expect("converts");

    assert!(
        out.report
            .transformations
            .iter()
            .any(|t| t.kind == "chapter-split"),
        "the fixture must actually split, or this test proves nothing: {:#?}",
        out.report.transformations
    );
    assert!(
        common::entry(&out.epub, "OEBPS/only.png").is_some(),
        "a srcset target must survive its owning chapter being split: {:#?}",
        out.report.transformations
    );
}

#[test]
fn a_dropped_marker_file_reports_its_payload() {
    let mut epub = common::book_with_meta_inf("META-INF/cdp.info", b"SHTX001.635962014");
    let out = convert(
        Input::Epub(std::mem::take(&mut epub)),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    let reported = out
        .report
        .transformations
        .iter()
        .any(|t| t.kind == "meta-inf-dropped" && t.detail.contains("SHTX001.635962014"));
    assert!(
        reported,
        "the payload must be reported, not just the filename: {:#?}",
        out.report.transformations
    );
}

/// The property the whole feature exists for: two copies of one edition,
/// differing only in per-copy marker channels, must converge byte for byte.
#[test]
fn two_marked_copies_converge_to_identical_bytes() {
    let copy_a = common::marked_copy(common::CopyMarks {
        cdp_info: "SHTX001.635962014",
        modified: "2026-08-02T13:18:00Z",
        exif_payload: "BUYER-A",
        invisible_payload: "\u{200B}\u{200B}\u{200C}",
        vendor_identifier: "urn:uuid:6f2a1e40-8c31-4b7e-9a55-1d0c2f9b7e31",
        doi_identifier: "doi:10.0000/TXN-A",
        // One stray, mid-archive.
        strays: vec![("OEBPS/nav.xhtml", "OEBPS/a-marker.txt", "A")],
        zip_epoch: 2026,
    });
    let copy_b = common::marked_copy(common::CopyMarks {
        cdp_info: "SHTX001.999999999",
        modified: "2026-01-09T04:55:11Z",
        exif_payload: "BUYER-B",
        invisible_payload: "\u{2060}\u{200B}",
        vendor_identifier: "urn:uuid:11111111-2222-3333-4444-555555555555",
        doi_identifier: "doi:10.0000/TXN-B",
        // A DIFFERENT count (two, not one) at DIFFERENT positions - one
        // right after the dropped META-INF file (so it lands first among
        // surviving resources), one after the image (so it lands last).
        // A single trailing stray in the same slot for both copies (the
        // original shape of this test) is exactly the position where
        // `IndexMap::swap_remove` and `shift_remove` are indistinguishable -
        // mutation testing proved that shape leaves the ordering hazard
        // `generic::reachable::prune` depends on (order-preserving removal)
        // completely unexercised. This shape catches it: see
        // `common::CopyMarks::strays`.
        strays: vec![
            ("META-INF/cdp.info", "OEBPS/b-marker-1.txt", "B1"),
            ("OEBPS/pic.jpg", "OEBPS/b-marker-2.txt", "B2"),
        ],
        zip_epoch: 2019,
    });
    assert_ne!(copy_a, copy_b, "the fixtures must actually differ");

    // Guard against a fixture regression that would make the whole test
    // vacuous by accident: each per-copy channel's raw input bytes must
    // actually differ between the two copies, not just the archive as a
    // whole (which could pass this check with six of seven channels
    // accidentally identical).
    assert_ne!(
        common::entry(&copy_a, "OEBPS/content.opf"),
        common::entry(&copy_b, "OEBPS/content.opf"),
        "the two OPFs must differ (dcterms:modified and the vendor identifier)"
    );
    assert_ne!(
        common::entry(&copy_a, "OEBPS/chapter.xhtml"),
        common::entry(&copy_b, "OEBPS/chapter.xhtml"),
        "the two chapters must differ (the invisible-character payload)"
    );
    assert_ne!(
        common::entry(&copy_a, "OEBPS/pic.jpg"),
        common::entry(&copy_b, "OEBPS/pic.jpg"),
        "the two JPEGs must differ (the EXIF payload)"
    );
    assert_ne!(
        common::entry(&copy_a, "META-INF/cdp.info"),
        common::entry(&copy_b, "META-INF/cdp.info"),
        "the two cdp.info blobs must differ"
    );

    let a = convert(Input::Epub(copy_a), &opts_for(&["generic"])).expect("converts");
    let b = convert(Input::Epub(copy_b), &opts_for(&["generic"])).expect("converts");
    assert_eq!(
        a.epub, b.epub,
        "two copies of one edition must strip to identical bytes"
    );
    // The `modified` channel (see `CopyMarks` docs) does not reach the model
    // at all, so it cannot make the assertion above catch a regression on
    // its own - assert directly that the two differing per-copy values
    // converged on the pinned epoch in the actual output.
    assert!(
        opf_of(&a.epub).contains("<meta property=\"dcterms:modified\">1970-01-01T00:00:00Z</meta>"),
        "the differing per-copy dcterms:modified values must converge on the pinned epoch:\n{}",
        opf_of(&a.epub)
    );
    // The DOI-costumed channel, asserted directly as well as through the
    // byte comparison: the fixture gives it an `identifier-type">DOI</meta>`
    // refinement, which is exactly the one OPF line a shop adds to try to
    // launder the value past the screen, so this also pins R3 end to end.
    let out_opf = opf_of(&a.epub);
    assert!(
        !out_opf.contains("TXN-A") && !out_opf.contains("TXN-B"),
        "a DOI-costumed per-copy identifier must be dropped despite its \
         identifier-type refinement:\n{out_opf}"
    );
    assert!(
        out_opf.contains("9783407868213"),
        "the edition's real ISBN must survive next to the dropped DOI:\n{out_opf}"
    );
}
