//! End-to-end tests for the M6 SVG pass: unwrapping `<image>`-wrapper SVGs
//! (href and base64 data URI, resource and inline) and rasterizing real vector
//! art (resource and inline). Fixtures are built in code; the big fixture is
//! validated with epubcheck (skip-if-unavailable, same harness as the other
//! integration tests).

mod common;

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::build_epub;
use epub_tailor_core::{ConvertOptions, Input, convert};
use image::codecs::png::PngEncoder;
use image::{DynamicImage, ExtendedColorType, ImageEncoder, RgbImage};
use zip::ZipArchive;

// ---------------------------------------------------------------------
// Fixture image helpers (no binary files).
// ---------------------------------------------------------------------

/// A smooth grayscale gradient: classified as a photo, stays JPEG.
fn photo(w: u32, h: u32) -> RgbImage {
    let mut img = RgbImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let v = (((x + y) * 255) / (w + h).max(1)) as u8;
        *px = image::Rgb([v, v, v]);
    }
    img
}

fn png_of(img: &RgbImage) -> Vec<u8> {
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            ExtendedColorType::Rgb8,
        )
        .unwrap();
    out
}

fn jpeg_of(img: &RgbImage) -> Vec<u8> {
    let mut out = Vec::new();
    DynamicImage::ImageRgb8(img.clone())
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg)
        .unwrap();
    out
}

/// A small two-tone PNG for a data-URI payload: line art, kept as PNG by M5.
fn line_art_png() -> Vec<u8> {
    let mut img = RgbImage::new(16, 16);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let v = if (x / 2 + y / 2) % 2 == 0 { 0 } else { 255 };
        *px = image::Rgb([v, v, v]);
    }
    png_of(&img)
}

/// Standard-alphabet base64 encoder, for building `data:` URIs.
fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

// ---------------------------------------------------------------------
// Zip helpers.
// ---------------------------------------------------------------------

fn zip_names(epub: &[u8]) -> Vec<String> {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("valid zip");
    (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect()
}

fn read_entry(epub: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("valid zip");
    let mut file = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("output should contain {name}"));
    let mut out = String::new();
    file.read_to_string(&mut out).expect("entry is UTF-8");
    out
}

fn entry_bytes(epub: &[u8], name: &str) -> Vec<u8> {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("valid zip");
    let mut file = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("missing {name}"));
    let mut out = Vec::new();
    file.read_to_end(&mut out).expect("read entry");
    out
}

const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol><li><a href="text/chapter.xhtml">Chapter</a></li></ol></nav></body>
</html>"#;

/// A real vector diagram: rectangles, a circle and text.
const DIAGRAM_SVG: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 600" width="800" height="600">
  <rect x="40" y="40" width="320" height="220" fill="black"/>
  <circle cx="600" cy="300" r="120" fill="rgb(40,40,40)"/>
  <text x="60" y="520" font-size="90" fill="black">Diagram</text>
</svg>"#;

// ---------------------------------------------------------------------
// Big integration fixture: cover wrapper + inline vector + vector resource.
// ---------------------------------------------------------------------

fn epub_svg_full() -> Vec<u8> {
    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>SVG Book</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:aaaa1111-2222-3333-4444-555566667777</dc:identifier>
    <meta name="cover" content="cover-svg"/>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="cover-svg" href="images/cover.svg" media-type="image/svg+xml"/>
    <item id="cover-raster" href="images/cover.jpg" media-type="image/jpeg"/>
    <item id="diagram" href="images/diagram.svg" media-type="image/svg+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

    // Chapter references the vector .svg resource (with stale width/height) and
    // carries an inline vector <svg> of its own.
    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title></head>
<body>
<h1>Chapter</h1>
<p><img src="../images/diagram.svg" width="120" height="90" alt="diagram"/></p>
<p>An inline chart:</p>
<p><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 120" width="200" height="120"><rect x="10" y="10" width="80" height="90" fill="black"/><circle cx="150" cy="60" r="40" fill="rgb(20,20,20)"/><text x="20" y="115" font-size="18">chart</text></svg></p>
</body></html>"#;

    // A wrapper SVG framing the raster cover next to it.
    const COVER_SVG: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 480 800"><title>Cover</title><image width="480" height="800" xlink:href="cover.jpg"/></svg>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
        ("OEBPS/images/cover.svg", COVER_SVG),
        ("OEBPS/images/cover.jpg", &jpeg_of(&photo(480, 800))),
        ("OEBPS/images/diagram.svg", DIAGRAM_SVG),
    ])
}

#[test]
fn full_svg_book_leaves_no_svg_and_is_epubcheck_clean() {
    let converted = convert(Input::Epub(epub_svg_full()), &ConvertOptions::default())
        .expect("conversion should succeed");
    let epub = &converted.epub;

    // No SVG survives, anywhere: not as a resource, not in the manifest.
    for name in zip_names(epub) {
        assert!(
            !name.to_ascii_lowercase().ends_with(".svg"),
            "an SVG resource survived: {name}"
        );
    }
    let opf = read_entry(epub, "OEBPS/content.opf");
    assert!(
        !opf.contains("image/svg+xml"),
        "manifest still declares an SVG:\n{opf}"
    );

    // The chapter has no inline <svg> and no reference to a .svg file left.
    let chapter = read_entry(epub, "OEBPS/text/chapter.xhtml");
    assert!(
        !chapter.contains("<svg"),
        "an inline <svg> survived:\n{chapter}"
    );
    assert!(!chapter.contains(".svg"), "a .svg ref survived:\n{chapter}");
    // The diagram resource ref was repointed to its raster, w/h stripped. Pure
    // vector art (no image/gradient/pattern/filter in the source) must ship
    // as a crisp PNG, never a blurry JPEG.
    assert!(
        chapter.contains("../images/diagram.png"),
        "diagram ref not repointed to a PNG:\n{chapter}"
    );
    assert!(!chapter.contains("width="), "width stripped:\n{chapter}");
    // The inline vector became an <img> pointing at a new sibling PNG resource.
    assert!(
        chapter.contains("chapter-svg-1.png"),
        "inline vector not rasterized to a new PNG resource:\n{chapter}"
    );

    // The cover now points at the unwrapped raster and keeps its role.
    assert!(
        opf.contains(r#"href="images/cover.jpg""#) && opf.contains("cover-image"),
        "cover not unwrapped to the raster:\n{opf}"
    );

    // Transformations recorded both kinds.
    let kinds: Vec<&str> = converted
        .report
        .transformations
        .iter()
        .map(|t| t.kind.as_str())
        .collect();
    assert!(
        kinds.contains(&"svg-unwrapped"),
        "no unwrap recorded: {kinds:?}"
    );
    assert!(
        kinds.contains(&"svg-rasterized"),
        "no rasterize recorded: {kinds:?}"
    );

    assert_epubcheck_clean("svg-full", epub);
}

// ---------------------------------------------------------------------
// A real vector SVG declared as the cover must be rasterized into the COVER
// box (480x800 on x4), not the inline box (480x730) - a 600x1000 source has
// the same aspect ratio as the cover box, so a correct fit lands exactly on
// (480, 800); fit into the inline box instead yields (438, 730).
// ---------------------------------------------------------------------

fn epub_vector_cover() -> Vec<u8> {
    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Vector Cover</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:99991111-2222-3333-4444-555566667777</dc:identifier>
    <meta name="cover" content="cover-svg"/>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="cover-svg" href="images/cover.svg" media-type="image/svg+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title></head>
<body><p>Text.</p></body></html>"#;

    // A real vector cover (rect + circle - not a single-<image> wrapper),
    // 600x1000: the same aspect ratio as the x4 cover box (480x800).
    const COVER_SVG: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 1000" width="600" height="1000">
  <rect x="0" y="0" width="600" height="1000" fill="rgb(20,20,60)"/>
  <circle cx="300" cy="500" r="150" fill="white"/>
</svg>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
        ("OEBPS/images/cover.svg", COVER_SVG),
    ])
}

/// Pull the `href` off the manifest `<item>` carrying `properties="cover-image"`.
fn cover_href(opf: &str) -> String {
    let marker = r#"properties="cover-image""#;
    let marker_idx = opf.find(marker).expect("a cover-image manifest item");
    let item_start = opf[..marker_idx]
        .rfind("<item")
        .expect("the enclosing <item start");
    let line = &opf[item_start..];
    let href_idx = line.find("href=\"").expect("an href attribute") + "href=\"".len();
    let rest = &line[href_idx..];
    let end = rest.find('"').expect("closing quote on href");
    rest[..end].to_string()
}

#[test]
fn vector_cover_rasterizes_into_the_cover_box_not_inline() {
    let converted = convert(Input::Epub(epub_vector_cover()), &ConvertOptions::default())
        .expect("conversion should succeed");
    let epub = &converted.epub;

    let opf = read_entry(epub, "OEBPS/content.opf");
    let href = cover_href(&opf);
    let cover_bytes = entry_bytes(epub, &format!("OEBPS/{href}"));
    let cover_image =
        image::load_from_memory(&cover_bytes).expect("cover resource decodes as an image");

    assert_eq!(
        (cover_image.width(), cover_image.height()),
        (480, 800),
        "a vector cover must be rasterized into the cover box (480x800), not the inline box"
    );

    assert_epubcheck_clean("svg-vector-cover", epub);
}

// ---------------------------------------------------------------------
// Wrapper with a base64 PNG data URI -> unwrapped, payload processed by M5.
// ---------------------------------------------------------------------

fn epub_data_uri_wrapper() -> Vec<u8> {
    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Data URI</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:bbbb1111-2222-3333-4444-555566667777</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="wrap" href="images/wrap.svg" media-type="image/svg+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    let wrap_svg = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="data:image/png;base64,{}"/></svg>"#,
        base64(&line_art_png())
    );
    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title></head>
<body><p><img src="../images/wrap.svg" alt="wrap"/></p></body></html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
        ("OEBPS/images/wrap.svg", wrap_svg.as_bytes()),
    ])
}

#[test]
fn data_uri_wrapper_is_unwrapped_and_payload_processed() {
    let converted = convert(
        Input::Epub(epub_data_uri_wrapper()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");
    let epub = &converted.epub;

    for name in zip_names(epub) {
        assert!(!name.ends_with(".svg"), "SVG survived: {name}");
    }
    // The decoded PNG payload became a device raster (line art -> PNG).
    assert!(
        zip_names(epub).iter().any(|n| n == "OEBPS/images/wrap.png"),
        "payload not extracted: {:?}",
        zip_names(epub)
    );
    assert!(
        entry_bytes(epub, "OEBPS/images/wrap.png").starts_with(&[0x89, 0x50, 0x4E, 0x47]),
        "payload is not a PNG"
    );
    let chapter = read_entry(epub, "OEBPS/text/chapter.xhtml");
    assert!(
        chapter.contains("../images/wrap.png") && !chapter.contains(".svg"),
        "ref not updated to the raster:\n{chapter}"
    );
    assert_epubcheck_clean("svg-datauri", epub);
}

// ---------------------------------------------------------------------
// Wrapper with a relative href -> SVG dropped, chapter <img> at the raster.
// ---------------------------------------------------------------------

fn epub_href_wrapper() -> Vec<u8> {
    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Href Wrapper</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:cccc1111-2222-3333-4444-555566667777</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="wrap" href="images/wrap.svg" media-type="image/svg+xml"/>
    <item id="pic" href="images/pic.jpg" media-type="image/jpeg"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    const WRAP_SVG: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 300 200"><image width="300" height="200" xlink:href="pic.jpg"/></svg>"#;
    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title></head>
<body><p><img src="../images/wrap.svg" alt="wrap"/></p></body></html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
        ("OEBPS/images/wrap.svg", WRAP_SVG),
        ("OEBPS/images/pic.jpg", &jpeg_of(&photo(300, 200))),
    ])
}

#[test]
fn href_wrapper_is_dropped_and_ref_points_at_the_raster() {
    let converted = convert(Input::Epub(epub_href_wrapper()), &ConvertOptions::default())
        .expect("conversion should succeed");
    let epub = &converted.epub;

    assert!(
        !zip_names(epub).iter().any(|n| n.ends_with(".svg")),
        "wrapper SVG not dropped: {:?}",
        zip_names(epub)
    );
    let chapter = read_entry(epub, "OEBPS/text/chapter.xhtml");
    assert!(
        chapter.contains("../images/pic.jpg") && !chapter.contains(".svg"),
        "ref not repointed at the raster:\n{chapter}"
    );
    assert_epubcheck_clean("svg-href", epub);
}

// ---------------------------------------------------------------------
// Wrapper with the <image> nested inside a <g> (the classic "cover page
// wrapped in a positioning group" pattern) -> still unwrapped, not
// rasterized to a blank frame (resvg cannot load an external raster href).
// ---------------------------------------------------------------------

fn epub_g_wrapped_href() -> Vec<u8> {
    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>G Wrapper</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:a1b2c3d4-1111-2222-3333-444455556666</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="wrap" href="images/wrap.svg" media-type="image/svg+xml"/>
    <item id="pic" href="images/pic.jpg" media-type="image/jpeg"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    const WRAP_SVG: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 300 200"><g transform="translate(0,0)"><image width="300" height="200" xlink:href="pic.jpg"/></g></svg>"#;
    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title></head>
<body><p><img src="../images/wrap.svg" alt="wrap"/></p></body></html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
        ("OEBPS/images/wrap.svg", WRAP_SVG),
        ("OEBPS/images/pic.jpg", &jpeg_of(&photo(300, 200))),
    ])
}

#[test]
fn g_wrapped_href_is_unwrapped_and_ref_points_at_the_raster() {
    let converted = convert(
        Input::Epub(epub_g_wrapped_href()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");
    let epub = &converted.epub;

    assert!(
        !zip_names(epub).iter().any(|n| n.ends_with(".svg")),
        "g-wrapped SVG not dropped: {:?}",
        zip_names(epub)
    );
    let chapter = read_entry(epub, "OEBPS/text/chapter.xhtml");
    assert!(
        chapter.contains("../images/pic.jpg") && !chapter.contains(".svg"),
        "ref not repointed at the raster:\n{chapter}"
    );
    assert_epubcheck_clean("svg-g-wrapped-href", epub);
}

// ---------------------------------------------------------------------
// Wrapper href pointing at nothing the book holds -> rasterized (likely
// blank) with a warning naming the SVG and the missing target, instead of
// silently producing a blank raster with no diagnostic.
// ---------------------------------------------------------------------

fn epub_dangling_href_wrapper() -> Vec<u8> {
    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Dangling Wrapper</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:ffff1111-2222-3333-4444-555566667777</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="wrap" href="images/wrap.svg" media-type="image/svg+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    // The href points at an image the book never carries (neither a sibling
    // resource nor fetchable), so the wrapper cannot be unwrapped.
    const WRAP_SVG: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 300 200"><image width="300" height="200" xlink:href="missing.jpg"/></svg>"#;
    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title></head>
<body><p><img src="../images/wrap.svg" alt="wrap"/></p></body></html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
        ("OEBPS/images/wrap.svg", WRAP_SVG),
    ])
}

#[test]
fn dangling_href_wrapper_rasterizes_with_a_warning() {
    let converted = convert(
        Input::Epub(epub_dangling_href_wrapper()),
        &ConvertOptions::default(),
    )
    .expect("conversion should still succeed");
    let epub = &converted.epub;

    // The wrapper is gone, replaced by a rasterized resource, same as any
    // other vector SVG.
    assert!(
        !zip_names(epub).iter().any(|n| n.ends_with(".svg")),
        "wrapper SVG not dropped: {:?}",
        zip_names(epub)
    );

    // A warning names both the SVG and the missing target, instead of
    // silently producing a blank raster.
    assert!(
        converted
            .report
            .warnings
            .iter()
            .any(|w| w.message.contains("wrap.svg") && w.message.contains("missing.jpg")),
        "expected a warning naming the SVG and the missing target: {:?}",
        converted.report.warnings
    );

    assert_epubcheck_clean("svg-dangling-href", epub);
}

// ---------------------------------------------------------------------
// Inline <svg><image .../></svg> wrapper -> <img>.
// ---------------------------------------------------------------------

fn epub_inline_wrapper() -> Vec<u8> {
    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Inline Wrapper</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:dddd1111-2222-3333-4444-555566667777</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="pic" href="images/cover.jpg" media-type="image/jpeg"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title></head>
<body><p><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="../images/cover.jpg"/></svg></p></body></html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
        ("OEBPS/images/cover.jpg", &jpeg_of(&photo(300, 400))),
    ])
}

#[test]
fn inline_wrapper_becomes_an_img() {
    let converted = convert(
        Input::Epub(epub_inline_wrapper()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");
    let epub = &converted.epub;
    let chapter = read_entry(epub, "OEBPS/text/chapter.xhtml");
    assert!(
        !chapter.contains("<svg"),
        "inline <svg> not replaced:\n{chapter}"
    );
    assert!(
        !chapter.contains("<image"),
        "inline <image> survived:\n{chapter}"
    );
    assert!(
        chapter.contains(r#"<img src="../images/cover.jpg""#),
        "not replaced with the expected <img>:\n{chapter}"
    );
    assert_epubcheck_clean("svg-inline-wrapper", epub);
}

// ---------------------------------------------------------------------
// Malformed SVG -> kept byte-identical, warning, conversion still succeeds.
// ---------------------------------------------------------------------

fn epub_malformed_svg() -> (Vec<u8>, Vec<u8>) {
    let broken: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><this is not <valid xml";
    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Broken</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:eeee1111-2222-3333-4444-555566667777</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="broken" href="images/broken.svg" media-type="image/svg+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title></head>
<body><p>text only</p></body></html>"#;

    let epub = build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
        ("OEBPS/images/broken.svg", broken),
    ]);
    (epub, broken.to_vec())
}

#[test]
fn malformed_svg_is_kept_unchanged_with_a_warning() {
    let (epub_in, original) = epub_malformed_svg();
    let converted =
        convert(Input::Epub(epub_in), &ConvertOptions::default()).expect("conversion must succeed");
    let epub = &converted.epub;

    // The broken resource is echoed byte-for-byte.
    assert_eq!(
        entry_bytes(epub, "OEBPS/images/broken.svg"),
        original,
        "malformed SVG must be preserved byte-for-byte"
    );
    assert!(
        converted
            .report
            .warnings
            .iter()
            .any(|w| w.message.contains("broken.svg")),
        "a warning about the broken SVG should be recorded: {:?}",
        converted.report.warnings
    );
}

// ---------------------------------------------------------------------
// svg_render_hint: PNG/JPEG chosen from the SVG source, not just the
// rendered pixel histogram.
// ---------------------------------------------------------------------

/// ~20 full-canvas-width gray bands, each a distinct tone spanning no two
/// dominant: the rendered pixel histogram alone (more than 16 distinct tones,
/// no top two tones covering 85% or more) would classify this as a photo, but
/// the source is pure vector art (no `image`/gradient/pattern/filter), so it
/// must ship as a crisp PNG.
fn many_grays_svg() -> Vec<u8> {
    let band_h = 600.0 / 20.0;
    let mut bands = String::new();
    for i in 0..20u32 {
        let v = 8 + i * 12;
        bands.push_str(&format!(
            r#"<rect x="0" y="{:.2}" width="800" height="{:.2}" fill="rgb({v},{v},{v})"/>"#,
            i as f32 * band_h,
            band_h,
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 600" width="800" height="600">{bands}</svg>"#
    )
    .into_bytes()
}

/// A full-canvas `linearGradient` fill: continuous-tone by construction, so
/// the Auto (histogram) path must still classify it as a photo and keep it a
/// JPEG - pinning that the source-based hint does not overreach into real
/// photographic/gradient content.
const GRADIENT_SVG: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 600" width="800" height="600">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0%" stop-color="black"/>
      <stop offset="100%" stop-color="white"/>
    </linearGradient>
  </defs>
  <rect x="0" y="0" width="800" height="600" fill="url(#g)"/>
</svg>"#;

/// A minimal book with a single SVG resource ("images/diagram.svg")
/// referenced from the chapter, for pinning what extension it rasterizes to.
fn epub_single_svg_resource(title: &str, uuid: &str, svg: &[u8]) -> Vec<u8> {
    let content_opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>{title}</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:{uuid}</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="diagram" href="images/diagram.svg" media-type="image/svg+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#
    );
    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title></head>
<body><p><img src="../images/diagram.svg" alt="diagram"/></p></body></html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", content_opf.as_bytes()),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
        ("OEBPS/images/diagram.svg", svg),
    ])
}

/// The extension of the rasterized "images/diagram.*" resource, or panics if
/// none is present.
fn diagram_resource_ext(epub: &[u8]) -> String {
    zip_names(epub)
        .into_iter()
        .find_map(|n| n.strip_prefix("OEBPS/images/diagram.").map(str::to_string))
        .unwrap_or_else(|| panic!("no images/diagram.* resource found: {:?}", zip_names(epub)))
}

#[test]
fn many_distinct_grays_ship_as_crisp_png_not_blurry_jpeg() {
    let epub_in = epub_single_svg_resource(
        "Many Grays",
        "11112222-3333-4444-5555-666677778888",
        &many_grays_svg(),
    );
    let converted = convert(Input::Epub(epub_in), &ConvertOptions::default())
        .expect("conversion should succeed");
    let epub = &converted.epub;

    let ext = diagram_resource_ext(epub);
    assert_eq!(
        ext, "png",
        "pure vector art with >16 rendered tones must still ship as PNG, not JPEG"
    );
    let bytes = entry_bytes(epub, &format!("OEBPS/images/diagram.{ext}"));
    assert!(
        bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]),
        "resource is not a PNG"
    );
    assert_epubcheck_clean("svg-many-grays", epub);
}

#[test]
fn gradient_fill_stays_jpeg() {
    let epub_in = epub_single_svg_resource(
        "Gradient",
        "22223333-4444-5555-6666-777788889999",
        GRADIENT_SVG,
    );
    let converted = convert(Input::Epub(epub_in), &ConvertOptions::default())
        .expect("conversion should succeed");
    let epub = &converted.epub;

    let ext = diagram_resource_ext(epub);
    assert_eq!(
        ext, "jpg",
        "a gradient fill is continuous-tone and must still be classified as a photo"
    );
    let bytes = entry_bytes(epub, &format!("OEBPS/images/diagram.{ext}"));
    assert!(
        bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "resource is not a JPEG"
    );
    assert_epubcheck_clean("svg-gradient", epub);
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
