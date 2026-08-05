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

/// A chapter narrated by a SMIL media overlay, plus an unsupported-type item
/// with a fallback - both link their target by manifest id, which the writer
/// must remap through its own id reassignment (`IdAllocator`) rather than
/// copy verbatim or silently drop.
///
/// The linking item's `media-overlay`/`fallback` idref (`narration-id`,
/// `fallback-id`) is deliberately a *different* string from its target's
/// `href` (`mo1`, `fb`), so a reader that just echoed the idref back as if it
/// were already a path - instead of actually resolving id -> href through the
/// manifest, the way `generic::reachable::opf_refs` does - could not pass
/// this test by coincidence.
///
/// The package document sits at the zip root (not the usual `OEBPS/`), and
/// the SMIL/fallback targets are given bare, extension-less hrefs (`mo1`,
/// `fb`) that are also valid XML-id shapes: `IdAllocator::allocate` derives
/// the *regenerated* id from a resource's path, and a path that already
/// looks like an id round-trips through it unchanged. That lets this fixture
/// pin the regenerated ids by literal string, without needing to know the
/// allocator's internals from the test side.
fn book_with_media_overlay() -> Vec<u8> {
    common::build_epub(&[
        ("mimetype", b"application/epub+zip"),
        (
            "META-INF/container.xml",
            br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
        ),
        (
            "content.opf",
            br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="pub-id">urn:uuid:2f6b7e2a-4c2d-4c2d-8a3e-9b6f9e6a1a10</dc:identifier>
    <dc:title>Narrated Book</dc:title>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="chapter.xhtml" media-type="application/xhtml+xml" media-overlay="narration-id"/>
    <item id="narration-id" href="mo1" media-type="application/smil+xml"/>
    <item id="weird" href="weird.dat" media-type="application/x-weird+xml" fallback="fallback-id"/>
    <item id="fallback-id" href="fb" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#,
        ),
        ("nav.xhtml", common::NAV_XHTML),
        (
            "chapter.xhtml",
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>C</title></head>
<body><p>Text.</p></body></html>"#,
        ),
        (
            "mo1",
            br#"<smil xmlns="http://www.w3.org/ns/SMIL" xmlns:epub="http://www.idpf.org/2007/ops" version="3.0">
<body><seq id="s1" epub:textref="chapter.xhtml">
<par id="p1"><text src="chapter.xhtml"/><audio src="track.mp3"/></par>
</seq></body></smil>"#,
        ),
        ("weird.dat", b"weird payload"),
        (
            "fb",
            br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Fallback</title></head>
<body><p>Fallback.</p></body></html>"#,
        ),
    ])
}

#[test]
fn a_media_overlay_and_fallback_survive_the_rebuild() {
    let out =
        convert(Input::Epub(book_with_media_overlay()), &opts_for(&["epub"])).expect("converts");
    let opf = opf_of(&out.epub);
    assert!(
        opf.contains("media-overlay=\"mo1\""),
        "narration linkage lost:\n{opf}"
    );
    assert!(
        opf.contains("fallback=\"fb\""),
        "fallback chain lost:\n{opf}"
    );
}

/// A one-pixel JPEG carrying an APP1 (EXIF) segment.
#[test]
fn generic_strips_exif_but_keeps_the_pixels() {
    let jpeg = common::jpeg_with_exif();
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
fn generic_keeps_an_import_target_reached_only_past_a_malformed_url_rule() {
    // Regression a code review caught: swapping `css_refs`'s naive `url()`
    // scanner for a real AST walk silently dropped the raw `@import`
    // string-literal recovery the old scanner used to provide. lightningcss
    // discards a misplaced `@import` per grammar (never valid after another
    // rule) and, separately, a malformed `url(` ahead of it can make the
    // tokenizer itself swallow everything up to the next `)` - either way
    // the AST walk alone never reaches `sub.css`. Proven end to end through
    // `convert()`, not just the unit-level `reachable` walk.
    let mut epub = common::book_with_css_import_after_a_malformed_rule();
    let out = convert(
        Input::Epub(std::mem::take(&mut epub)),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    assert!(
        common::entry(&out.epub, "OEBPS/sub.css").is_some(),
        "an @import target reached only past a malformed rule must survive"
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

/// Regression coverage for the stranding found while verifying round 4: the
/// image optimizer re-encodes `wm.png` to `wm.jpg` and drops the old path
/// from the book, but `rewrite_refs` reported the srcset edge under the
/// PRE-rename path, so `prune` never reached the renamed resource and deleted
/// it. `<img src>` never hit this because that branch already followed the
/// rename map. Every earlier srcset fixture used an undecodable 8-byte PNG
/// stub, so no rename ever happened and no test could see it.
#[test]
fn generic_keeps_a_srcset_target_that_the_optimizer_renamed() {
    let opts = opts_for(&["x4", "generic"]);
    let out = convert(
        Input::Epub(common::book_with_srcset_image_that_gets_re_encoded()),
        &opts,
    )
    .expect("converts");
    assert!(
        out.report
            .transformations
            .iter()
            .any(|t| t.kind == "image-optimized"),
        "the fixture must really be re-encoded, or this test proves nothing: {:#?}",
        out.report.transformations
    );
    assert!(
        common::entry(&out.epub, "OEBPS/wm.jpg").is_some(),
        "a srcset target must survive being re-encoded and renamed: {:#?}",
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

/// The same property for the channel a declared `identifier-type` opens.
///
/// Screening only the shapes that are per-copy *regardless* of scheme (a
/// UUID, an email address, a DOI whose own parts betray it) left every other
/// per-copy shape laundered by one OPF line: with
/// `<meta refines="#vendor" property="identifier-type">ISBN</meta>` present,
/// `SHTX001.635962014` was measured surviving a `generic` conversion intact
/// and two copies of one edition failed to converge. This pins the fix end to
/// end for each declarable scheme, not just at the classifier.
#[test]
fn two_copies_marked_only_by_a_scheme_refined_identifier_converge() {
    for scheme in ["ISBN", "ISSN", "DOI"] {
        let copy_a = common::book_with_refined_identifier("SHTX001.635962014", scheme);
        let copy_b = common::book_with_refined_identifier("SHTX001.999999999", scheme);
        assert_ne!(
            copy_a, copy_b,
            "the fixtures must actually differ: {scheme}"
        );

        let a = convert(Input::Epub(copy_a), &opts_for(&["generic"])).expect("converts");
        let b = convert(Input::Epub(copy_b), &opts_for(&["generic"])).expect("converts");
        assert_eq!(
            a.epub, b.epub,
            "two copies differing only in a {scheme}-refined watermark must converge"
        );

        let opf = opf_of(&a.epub);
        assert!(
            !opf.contains("SHTX001"),
            "a declared {scheme} refinement must not launder a vendor id:\n{opf}"
        );
        assert!(
            opf.contains("9783407868213"),
            "the edition's real ISBN must survive the {scheme} case:\n{opf}"
        );
    }
}

/// The other direction of the same gate: the refinement must still rescue a
/// value that is genuinely shaped like the type it declares but fails its
/// checksum, which is the only reason the shortcut exists. `9783407868214` is
/// the fixture edition's ISBN with a mistyped check digit.
#[test]
fn a_scheme_refined_mistyped_isbn_survives_a_generic_conversion() {
    let book = common::book_with_refined_identifier("9783407868214", "ISBN");
    let out = convert(Input::Epub(book), &opts_for(&["generic"])).expect("converts");
    let opf = opf_of(&out.epub);
    assert!(
        opf.contains("9783407868214"),
        "an ISBN-13-shaped value behind an ISBN refinement must be kept:\n{opf}"
    );
}

/// The nav guard must key on the path the writer will actually use, not on
/// `book.nav_path` alone: when that is `None` the writer emits
/// `<opf_dir>/nav.xhtml` and discards whatever was stored there.
#[test]
fn a_nav_path_the_writer_will_synthesize_is_not_scrubbed_as_content() {
    let out = convert(
        Input::Epub(common::epub2_with_a_stray_nav_xhtml()),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    assert!(
        !out.report.transformations.iter().any(|t| {
            t.kind == "generic-invisible" && t.file.as_deref() == Some("OEBPS/nav.xhtml")
        }),
        "must not report work on a file the writer replaces: {:#?}",
        out.report.transformations
    );
}

/// The companion to `a_nav_path_the_writer_will_synthesize_is_not_scrubbed_as_content`
/// one layer down: the prune walk must also root on the path the writer will
/// use, or it drops the stray nav and reports "nothing references" it while
/// the writer ships a regenerated nav at that exact path.
#[test]
fn a_nav_path_the_writer_will_synthesize_is_not_reported_as_unreferenced() {
    let out = convert(
        Input::Epub(common::epub2_with_a_stray_nav_xhtml()),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    assert!(
        !out.report.transformations.iter().any(|t| {
            t.kind == "generic-unreferenced" && t.file.as_deref() == Some("OEBPS/nav.xhtml")
        }),
        "must not report dropping a path the writer then writes: {:#?}",
        out.report.transformations
    );
    assert!(
        common::entry(&out.epub, "OEBPS/nav.xhtml").is_some(),
        "the writer ships a nav at that path either way"
    );
}

/// The narrated fixture is otherwise reached only through the epubcheck-gated
/// round-trip, so a local run without `EPUBCHECK_FORCE` never checked that the
/// media-overlay linkage and both `media:duration` values survive a rebuild.
/// This pins them with plain assertions, no external tool.
#[test]
fn a_narrated_book_keeps_its_overlay_and_both_durations() {
    let out = convert(
        Input::Epub(common::epub3_narrated()),
        &ConvertOptions::default(),
    )
    .expect("converts");
    let opf = opf_of(&out.epub);

    // The SMIL's regenerated manifest id, read back out of the OPF rather than
    // hardcoded, so this does not also pin `IdAllocator`'s naming scheme. Both
    // the chapter's linkage and the duration refinement must name it: asserting
    // on `media-overlay=` alone passes even when the linkage points at the
    // wrong item, which is the failure worth catching.
    let smil_id = opf
        .split("<item ")
        .find(|item| item.contains(r#"href="chapter1.smil""#))
        .and_then(|item| item.split_once(r#"id=""#))
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(id, _)| id.to_string())
        .expect("the SMIL item must still be in the manifest");
    assert!(
        opf.contains(&format!("media-overlay=\"{smil_id}\"")),
        "the chapter must still declare its overlay by the SMIL's id:\n{opf}"
    );
    assert!(
        opf.contains(r#"<meta property="media:duration">0:00:05.000</meta>"#),
        "the book-wide duration must survive:\n{opf}"
    );
    // `r##` because the refinement's value starts with `"#`, which would close
    // an `r#` raw string.
    assert!(
        opf.contains(&format!(
            r##"property="media:duration" refines="#{smil_id}""##
        )),
        "the per-overlay duration must still refine the SMIL item:\n{opf}"
    );
    assert!(
        common::entry(&out.epub, "OEBPS/chapter1.smil").is_some(),
        "the SMIL itself must ship"
    );
}

/// A narrated book carrying the book-wide `media:duration` but no per-item
/// `refines` refinement - legal EPUB 3, and the shape a gate keyed on the
/// refinements rather than on the surviving overlays silently strips the total
/// from. EPUB 3 requires the total precisely because an item carries
/// `media-overlay`, so that is what it must key on.
#[test]
fn a_narrated_book_without_per_item_refinements_keeps_its_total_duration() {
    const OPF: &[u8] = br##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Narrated, unrefined</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:44444444-4444-4444-4444-444444444444</dc:identifier>
    <meta property="dcterms:modified">2024-01-01T00:00:00Z</meta>
    <meta property="media:duration">0:00:05.000</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="chapter1.xhtml" media-type="application/xhtml+xml" media-overlay="mo1"/>
    <item id="mo1" href="chapter1.smil" media-type="application/smil+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"##;

    const NAV: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol><li><a href="chapter1.xhtml">Chapter 1</a></li></ol></nav></body></html>"#;

    const CHAPTER1: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body><p id="c1">Text.</p></body></html>"#;

    const SMIL: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<smil xmlns="http://www.w3.org/ns/SMIL" version="3.0">
<body><seq id="s1"><par id="p1"><text src="chapter1.xhtml#c1"/></par></seq></body>
</smil>"#;

    let epub = common::build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", common::CONTAINER_XML),
        ("OEBPS/content.opf", OPF),
        ("OEBPS/nav.xhtml", NAV),
        ("OEBPS/chapter1.xhtml", CHAPTER1),
        ("OEBPS/chapter1.smil", SMIL),
    ]);

    let out = convert(Input::Epub(epub), &ConvertOptions::default()).expect("converts");
    let opf = opf_of(&out.epub);
    assert!(
        opf.contains("media-overlay="),
        "the overlay must survive, or this test proves nothing:\n{opf}"
    );
    assert!(
        opf.contains(r#"<meta property="media:duration">0:00:05.000</meta>"#),
        "a narrated book must keep the total EPUB 3 requires of it:\n{opf}"
    );
}

/// The synthesized nav path is protected from pruning, but its stored bytes
/// are discarded and regenerated, so its links must not keep anything else
/// alive. `stray-only.png` is referenced by nothing but that discarded
/// document, so it has to go - rooting the path and WALKING it would retain
/// the image, which is the difference this pins.
#[test]
fn the_synthesized_nav_path_is_protected_without_its_links_being_followed() {
    let out = convert(
        Input::Epub(common::epub2_with_a_stray_nav_xhtml()),
        &opts_for(&["generic"]),
    )
    .expect("converts");
    assert!(
        common::entry(&out.epub, "OEBPS/stray-only.png").is_none(),
        "an image only the discarded nav bytes referenced must not be retained"
    );
    assert!(
        common::entry(&out.epub, "OEBPS/nav.xhtml").is_some(),
        "the nav path itself still ships, regenerated by the writer"
    );
}
