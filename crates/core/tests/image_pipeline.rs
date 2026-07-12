//! End-to-end test for the M5 image pipeline: convert a book carrying a PNG
//! photo, a GIF, a WebP and a cover, assert every raster came out as a device
//! decodable JPEG/PNG with references and manifest updated, and validate the
//! result with epubcheck (skip-if-unavailable, the same harness as the other
//! integration tests). A second book exercises the opt-in tall-image split.

mod common;

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::build_epub;
use epub_tailor_core::{ConvertOptions, Input, convert};
use image::codecs::gif::GifEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, Frame, ImageEncoder, RgbImage};
use zip::ZipArchive;

// ---------------------------------------------------------------------
// Image fixture generators (no binary files: everything is built in code).
// ---------------------------------------------------------------------

/// A smooth grayscale gradient in RGB: many tones, classified as a photo.
fn photo(w: u32, h: u32) -> RgbImage {
    let mut img = RgbImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let v = (((x + y) * 255) / (w + h).max(1)) as u8;
        *px = image::Rgb([v, v, v]);
    }
    img
}

/// A two-tone image: classified as line art.
fn line_art(w: u32, h: u32) -> RgbImage {
    let mut img = RgbImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let v = if (x / 4 + y / 4) % 2 == 0 { 0 } else { 255 };
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

fn gif_of(img: &RgbImage) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        enc.encode_frame(Frame::new(
            image::DynamicImage::ImageRgb8(img.clone()).into_rgba8(),
        ))
        .unwrap();
    }
    out
}

fn webp_of(img: &RgbImage) -> Vec<u8> {
    let mut out = Vec::new();
    WebPEncoder::new_lossless(&mut out)
        .encode(
            img.as_raw(),
            img.width(),
            img.height(),
            ExtendedColorType::Rgb8,
        )
        .unwrap();
    out
}

// ---------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------

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

/// A book with a PNG photo, a GIF, a WebP and a PNG cover, referenced from one
/// chapter (the photo carrying stale width/height attributes).
fn epub_images() -> Vec<u8> {
    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Images</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:11112222-3333-4444-5555-666677778888</dc:identifier>
    <meta name="cover" content="cover-img"/>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="cover-img" href="images/cover.png" media-type="image/png" properties="cover-image"/>
    <item id="photo" href="images/photo.png" media-type="image/png"/>
    <item id="diagram" href="images/diagram.gif" media-type="image/gif"/>
    <item id="scene" href="images/scene.webp" media-type="image/webp"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title></head>
<body>
<h1>Chapter</h1>
<p><img src="../images/photo.png" width="800" height="600" alt="photo"/></p>
<p><img src="../images/diagram.gif" alt="diagram"/></p>
<p><img src="../images/scene.webp" alt="scene"/></p>
</body></html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
        ("OEBPS/images/cover.png", &png_of(&photo(600, 900))),
        ("OEBPS/images/photo.png", &png_of(&photo(800, 600))),
        ("OEBPS/images/diagram.gif", &gif_of(&line_art(40, 40))),
        ("OEBPS/images/scene.webp", &webp_of(&photo(300, 200))),
    ])
}

/// A book with one very tall image, for the split test.
fn epub_tall() -> Vec<u8> {
    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Tall</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:99998888-7777-6666-5555-444433332222</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="tall" href="images/tall.png" media-type="image/png"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

    // The tall image is referenced in the three common source shapes that
    // must not become `<p><p class="et-img">...</p></p>` (block-in-p) or
    // block-in-<a> once split into tiles: a lone `<p>` wrapper, a linked
    // image alone in a `<p>`, and a linked image sharing its `<p>` with text.
    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Tall</title></head>
<body>
<h1>Tall</h1>
<p><img src="../images/tall.png" alt="tall"/></p>
<p><a href="../images/tall.png"><img src="../images/tall.png" alt="tall linked"/></a></p>
<p>See <a href="../images/tall.png"><img src="../images/tall.png" alt="tall in text"/></a> here</p>
</body></html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
        ("OEBPS/images/tall.png", &png_of(&photo(480, 3000))),
    ])
}

// ---------------------------------------------------------------------
// Zip helpers
// ---------------------------------------------------------------------

fn read_entry(epub: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    let mut file = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("output should contain {name}"));
    let mut out = String::new();
    file.read_to_string(&mut out).expect("entry is UTF-8");
    out
}

fn entry_exists(epub: &[u8], name: &str) -> bool {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    archive.by_name(name).is_ok()
}

fn entry_bytes(epub: &[u8], name: &str) -> Vec<u8> {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    let mut file = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("missing {name}"));
    let mut out = Vec::new();
    file.read_to_end(&mut out).expect("read entry");
    out
}

/// Assert no `<p>` in `xhtml` contains another `<p>` (attribute-robust: scans
/// open and close tags instead of matching a literal `<p><p` substring).
fn assert_no_nested_p(xhtml: &str) {
    let mut depth = 0i32;
    for (idx, _) in xhtml.match_indices('<') {
        let rest = &xhtml[idx..];
        if rest.starts_with("</p>") {
            depth -= 1;
        } else if let Some(after) = rest.strip_prefix("<p")
            && after.starts_with([' ', '>', '/', '\t', '\n'])
        {
            let tag = &rest[..rest.find('>').map_or(rest.len(), |e| e + 1)];
            if !tag.ends_with("/>") {
                depth += 1;
                assert!(depth <= 1, "found a <p> nested inside a <p>:\n{xhtml}");
            }
        }
    }
    assert_eq!(depth, 0, "unbalanced <p> tags:\n{xhtml}");
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[test]
fn every_raster_is_transcoded_and_references_updated() {
    let converted = convert(Input::Epub(epub_images()), &ConvertOptions::default())
        .expect("conversion should succeed");
    let epub = &converted.epub;

    // Four source images were processed.
    assert_eq!(converted.report.stats.images_processed, 4);

    // No dead-format resource survives; the live replacements are all present.
    for gone in [
        "OEBPS/images/cover.png",
        "OEBPS/images/photo.png",
        "OEBPS/images/diagram.gif",
        "OEBPS/images/scene.webp",
    ] {
        assert!(
            !entry_exists(epub, gone),
            "{gone} should have been renamed away"
        );
    }
    for present in [
        "OEBPS/images/cover.jpg",
        "OEBPS/images/photo.jpg",
        "OEBPS/images/diagram.png",
        "OEBPS/images/scene.jpg",
    ] {
        assert!(entry_exists(epub, present), "{present} should exist");
    }

    // Every image entry begins with a live format's magic bytes.
    for (name, magic) in [
        ("OEBPS/images/cover.jpg", &[0xFF, 0xD8, 0xFF][..]),
        ("OEBPS/images/photo.jpg", &[0xFF, 0xD8, 0xFF][..]),
        ("OEBPS/images/scene.jpg", &[0xFF, 0xD8, 0xFF][..]),
        ("OEBPS/images/diagram.png", &[0x89, 0x50, 0x4E, 0x47][..]),
    ] {
        assert!(
            entry_bytes(epub, name).starts_with(magic),
            "{name} must start with its format magic"
        );
    }

    // Chapter references were repointed, and width/height attributes stripped.
    let chapter = read_entry(epub, "OEBPS/text/chapter.xhtml");
    assert!(
        chapter.contains("../images/photo.jpg"),
        "photo ref:\n{chapter}"
    );
    assert!(
        chapter.contains("../images/diagram.png"),
        "gif ref:\n{chapter}"
    );
    assert!(
        chapter.contains("../images/scene.jpg"),
        "webp ref:\n{chapter}"
    );
    assert!(
        !chapter.contains("photo.png"),
        "old png ref gone:\n{chapter}"
    );
    assert!(
        !chapter.contains("width="),
        "width attr stripped:\n{chapter}"
    );
    assert!(
        !chapter.contains("height="),
        "height attr stripped:\n{chapter}"
    );

    // Manifest media types are correct and the cover points at the JPEG.
    let opf = read_entry(epub, "OEBPS/content.opf");
    assert!(
        opf.contains(r#"href="images/cover.jpg""#),
        "cover renamed:\n{opf}"
    );
    assert!(
        opf.contains(r#"media-type="image/png""#),
        "diagram.png keeps image/png:\n{opf}"
    );
    assert!(
        opf.contains(r#"href="images/diagram.png""#),
        "diagram is a manifest item:\n{opf}"
    );
    assert!(
        !opf.contains(".gif") && !opf.contains(".webp"),
        "no dead format in manifest:\n{opf}"
    );
    assert!(
        opf.contains("cover-image"),
        "cover-image property kept:\n{opf}"
    );
}

#[test]
fn tall_image_splits_into_ordered_page_tiles() {
    let opts = ConvertOptions {
        split_tall_images: true,
        ..ConvertOptions::default()
    };
    let converted = convert(Input::Epub(epub_tall()), &opts).expect("conversion should succeed");
    let epub = &converted.epub;

    assert_eq!(converted.report.stats.images_processed, 1);
    assert!(
        converted
            .report
            .transformations
            .iter()
            .any(|t| t.kind == "image-split"),
        "a split transformation should be recorded"
    );

    assert!(
        !entry_exists(epub, "OEBPS/images/tall.png"),
        "original removed"
    );
    assert!(entry_exists(epub, "OEBPS/images/tall-p1.jpg"), "first tile");
    assert!(
        entry_exists(epub, "OEBPS/images/tall-p2.jpg"),
        "second tile"
    );

    // Each chapter <img> (a bare <p> wrapper, a linked image and a linked
    // image with surrounding text) became one et-img paragraph per tile, in
    // order, never nested inside a <p> or an <a>.
    let chapter = read_entry(epub, "OEBPS/text/chapter.xhtml");
    assert!(
        chapter.contains(r#"class="et-img""#),
        "tile wrappers:\n{chapter}"
    );
    assert_no_nested_p(&chapter);
    assert!(
        !chapter.contains("<a"),
        "the emptied links must be dropped:\n{chapter}"
    );
    assert!(
        chapter.contains("See") && chapter.contains("here"),
        "the text paragraph keeps its text:\n{chapter}"
    );
    let p1 = chapter.find("tall-p1.jpg").expect("p1 present");
    let p2 = chapter.find("tall-p2.jpg").expect("p2 present");
    assert!(p1 < p2, "tiles must appear in order:\n{chapter}");
}

#[test]
fn tall_split_output_is_epubcheck_clean() {
    let opts = ConvertOptions {
        split_tall_images: true,
        ..ConvertOptions::default()
    };
    let converted = convert(Input::Epub(epub_tall()), &opts).expect("conversion should succeed");

    let out_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("tall-split.tailored.epub");
    std::fs::write(&out_path, &converted.epub).expect("write converted epub");

    match run_epubcheck(&out_path) {
        None => {
            eprintln!(
                "SKIP: epubcheck not found (not on PATH, EPUBCHECK_JAR unset); \
                 skipping the tall-split validation"
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

#[test]
fn image_pipeline_output_is_epubcheck_clean() {
    let converted = convert(Input::Epub(epub_images()), &ConvertOptions::default())
        .expect("conversion should succeed");

    let out_path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("images.tailored.epub");
    std::fs::write(&out_path, &converted.epub).expect("write converted epub");

    match run_epubcheck(&out_path) {
        None => {
            eprintln!(
                "SKIP: epubcheck not found (not on PATH, EPUBCHECK_JAR unset); \
                 skipping the image-pipeline validation"
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
