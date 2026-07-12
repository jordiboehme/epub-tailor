//! Shared helpers for building minimal, in-code EPUB fixtures for integration
//! tests. No binary fixture files: every archive is assembled at test time
//! with the same `zip` crate the reader uses.
//!
//! This module is `mod`-included by several test binaries, each of which uses
//! only a subset of the fixtures, so unused-in-one-binary helpers are expected.
#![allow(dead_code)]

use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Build a ZIP archive from `entries` (path, raw bytes), in the given order.
/// `mimetype` (if present) is written STORED (uncompressed); everything else
/// is written DEFLATE. Callers are responsible for ordering `entries` so that
/// `mimetype` comes first, matching the EPUB OCF requirement.
pub fn build_epub(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    for (name, data) in entries {
        let options = if *name == "mimetype" {
            stored
        } else {
            deflated
        };
        writer.start_file(*name, options).expect("start_file");
        writer.write_all(data).expect("write entry data");
    }
    writer.finish().expect("finish zip").into_inner()
}

/// A minimal, well-formed EPUB3 book: two chapters, a nav doc with a nested
/// table of contents (to exercise TOC levels), one stylesheet, one cover
/// image referenced both via `meta[name=cover]` and `properties=cover-image`.
pub fn epub3_minimal() -> Vec<u8> {
    const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Sample Book</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:12345678-1234-1234-1234-123456789012</dc:identifier>
    <meta name="cover" content="cover-img"/>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="text/chapter2.xhtml" media-type="application/xhtml+xml"/>
    <item id="css" href="styles/main.css" media-type="text/css"/>
    <item id="cover-img" href="images/cover.jpg" media-type="image/jpeg" properties="cover-image"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>"#;

    const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body>
<nav epub:type="toc">
<ol>
<li><a href="text/chapter1.xhtml">Chapter 1</a>
  <ol>
  <li><a href="text/chapter1.xhtml#s2">Section 1.1</a></li>
  </ol>
</li>
<li><a href="text/chapter2.xhtml">Chapter 2</a></li>
</ol>
</nav>
</body>
</html>"#;

    const CHAPTER1: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body><h1>Chapter 1</h1><p>Text.</p><h2 id="s2">Section 1.1</h2><p>More text.</p></body></html>"#;

    const CHAPTER2: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 2</title></head>
<body><h1>Chapter 2</h1><p>Text.</p></body></html>"#;

    const MAIN_CSS: &[u8] = b"body { font-family: serif; }\n";

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
        ("OEBPS/text/chapter2.xhtml", CHAPTER2),
        ("OEBPS/styles/main.css", MAIN_CSS),
        ("OEBPS/images/cover.jpg", COVER_JPG),
    ])
}

/// A real, minimal baseline grayscale JPEG (generated once, in-code so no
/// binary fixtures are checked in). A valid image is required so the epubcheck
/// gates see a decodable cover.
const COVER_JPG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x06, 0x04, 0x05, 0x06, 0x05, 0x04, 0x06,
    0x06, 0x05, 0x06, 0x07, 0x07, 0x06, 0x08, 0x0A, 0x10, 0x0A, 0x0A, 0x09, 0x09, 0x0A, 0x14, 0x0E,
    0x0F, 0x0C, 0x10, 0x17, 0x14, 0x18, 0x18, 0x17, 0x14, 0x16, 0x16, 0x1A, 0x1D, 0x25, 0x1F, 0x1A,
    0x1B, 0x23, 0x1C, 0x16, 0x16, 0x20, 0x2C, 0x20, 0x23, 0x26, 0x27, 0x29, 0x2A, 0x29, 0x19, 0x1F,
    0x2D, 0x30, 0x2D, 0x28, 0x30, 0x25, 0x28, 0x29, 0x28, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x02,
    0x00, 0x02, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00, 0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01,
    0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04,
    0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03,
    0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00,
    0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32,
    0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72,
    0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35,
    0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55,
    0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75,
    0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94,
    0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2,
    0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9,
    0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6,
    0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xDA,
    0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0xF6, 0x0A, 0xFF, 0xD9,
];

/// A one-chapter EPUB3 whose single chapter is a "kitchen sink" exercising every
/// M3 transform: a table with a caption and header, ordered lists (typed,
/// nested, with a bulleted sublist), a `<pre>` code block, an `<aside>`, a
/// `<figure>` with caption, a `<dl>`, a `javascript:` footnote link (with and
/// without a recoverable target), an inline id that must be relocated (and
/// aliased onto a block that already has one), a decomposed-Unicode string, and
/// an over-long word.
pub fn epub3_kitchen_sink() -> Vec<u8> {
    const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Kitchen Sink</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:abcdef01-2345-6789-abcd-ef0123456789</dc:identifier>
    <meta name="cover" content="cover-img"/>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/kitchen.xhtml" media-type="application/xhtml+xml"/>
    <item id="cover-img" href="images/cover.jpg" media-type="image/jpeg" properties="cover-image"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

    const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body>
<nav epub:type="toc">
<ol>
<li><a href="text/kitchen.xhtml">Kitchen Sink</a></li>
</ol>
</nav>
</body>
</html>"#;

    let chapter = kitchen_chapter();
    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/kitchen.xhtml", chapter.as_bytes()),
        ("OEBPS/images/cover.jpg", COVER_JPG),
    ])
}

/// The kitchen-sink chapter XHTML. Built as a `String` so combining-mark,
/// tab and over-long-word content can be spelled with escapes and interpolated.
fn kitchen_chapter() -> String {
    let long_word = "x".repeat(250);
    // "cafe" + U+0301 COMBINING ACUTE ACCENT: decomposed, so NFC has work to do.
    let decomposed = "cafe\u{301}";
    let code = "def f():\n\treturn 1\n";
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Kitchen Sink</title></head>
<body>
<h1 id="top">Kitchen Sink</h1>
<p>A {decomposed} with a decomposed name.</p>
<table>
<caption>Prices</caption>
<thead><tr><th>Item</th><th>Cost</th></tr></thead>
<tbody>
<tr><td>Pen</td><td>1</td></tr>
<tr><td>Ink</td><td>2</td></tr>
</tbody>
</table>
<ol type="a" start="2">
<li>alpha<ol><li>nested one</li><li>nested two</li></ol></li>
<li>beta<ul><li>bullet</li></ul></li>
</ol>
<pre><code>{code}</code></pre>
<aside><strong>Note:</strong> mind the gap.</aside>
<figure><img src="../images/cover.jpg" alt="cover"/><figcaption>The cover</figcaption></figure>
<dl><dt>Term</dt><dd>Definition</dd></dl>
<p id="note"><sup id="fn1">1</sup> Footnote body.</p>
<p><a href="javascript:show('#note')" data-target="#note">go</a> and <a href="javascript:void(0)">dead</a>.</p>
<p><a href="#fn1">back to note</a></p>
<p>{long_word}</p>
</body>
</html>"##
    )
}

/// A one-chapter EPUB3 exercising the CSS/style pipeline: a `<style>` block in
/// the head (mixing `@font-face`, `@media print`/`screen`, unsupported selectors
/// and properties with supported ones), inline `style=""` attributes, an
/// external stylesheet full of junk, a `<link>` pointing at an embedded font,
/// and an embedded font file to be stripped.
pub fn epub3_css_kitchen() -> Vec<u8> {
    const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>CSS Kitchen</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:0f0f0f0f-1234-1234-1234-abcdefabcdef</dc:identifier>
    <meta name="cover" content="cover-img"/>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="ext" href="styles/ext.css" media-type="text/css"/>
    <item id="font" href="fonts/DejaVu.ttf" media-type="application/vnd.ms-opentype"/>
    <item id="cover-img" href="images/cover.jpg" media-type="image/jpeg" properties="cover-image"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

    const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body>
<nav epub:type="toc">
<ol>
<li><a href="text/chapter.xhtml">CSS Kitchen</a></li>
</ol>
</nav>
</body>
</html>"#;

    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head>
<title>CSS Kitchen</title>
<link rel="stylesheet" type="text/css" href="../styles/ext.css"/>
<link rel="stylesheet" type="text/css" href="../fonts/DejaVu.ttf"/>
<style>
@font-face { font-family: "DejaVu"; src: url(../fonts/DejaVu.ttf); }
body { color: green; text-align: justify; }
.note { font-family: serif; margin-left: 2em; }
.note:hover { color: red; }
@media print { .p { display: none; } }
@media screen { .s { text-indent: 1em; } }
</style>
</head>
<body>
<h1>CSS Kitchen</h1>
<p style="color:red;text-align:center;font-size:12px">Filtered inline.</p>
<p style="color:blue">Dropped inline.</p>
<p class="note">A note.</p>
</body>
</html>"#;

    const EXT_CSS: &[u8] = br#"p { color: red; margin: 1em; }
.x > .y { color: blue; }
@import url(missing.css);
.z:hover { text-align: center; }
.keep { text-align: center; margin-left: 2em; }
"#;

    // Not a real font; it is stripped before anything reads it.
    const FONT: &[u8] = b"\x00\x01\x00\x00 not a real font, stripped on convert ";

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
        ("OEBPS/styles/ext.css", EXT_CSS),
        ("OEBPS/fonts/DejaVu.ttf", FONT),
        ("OEBPS/images/cover.jpg", COVER_JPG),
    ])
}

/// A three-chapter EPUB3 book that provokes the cross-chapter `<style>` bleed.
/// Chapter 1's head `<style>` defines `.note{margin-left:2em}` and chapter 2's
/// defines `.note{margin-left:4em}` - the SAME author class, different values.
/// Chapter 3 has NO `<style>` at all but does carry a `<p class="note">`. Every
/// chapter has a `.note` paragraph. Once the head styles are relocated into the
/// single book-wide `et-relocated.css` (which the device applies to every
/// chapter), an unscoped `.note` rule from chapter 1 would restyle chapters 2
/// and 3, and chapter 2's would fight chapter 1's. Per-chapter scoping must keep
/// each rule matching only its own chapter, and leave chapter 3 (which
/// contributed nothing) entirely untouched.
pub fn epub3_style_bleed() -> Vec<u8> {
    const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Style Bleed</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:beadfeed-1111-2222-3333-444455556677</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="text/chapter2.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch3" href="text/chapter3.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
    <itemref idref="ch3"/>
  </spine>
</package>"#;

    const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body>
<nav epub:type="toc">
<ol>
<li><a href="text/chapter1.xhtml">Chapter 1</a></li>
<li><a href="text/chapter2.xhtml">Chapter 2</a></li>
<li><a href="text/chapter3.xhtml">Chapter 3</a></li>
</ol>
</nav>
</body>
</html>"#;

    const CHAPTER1: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter 1</title>
<style>.note { margin-left: 2em; }</style>
</head>
<body><h1>Chapter 1</h1><p class="note">Note one.</p></body></html>"#;

    const CHAPTER2: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter 2</title>
<style>.note { margin-left: 4em; }</style>
</head>
<body><h1>Chapter 2</h1><p class="note">Note two.</p></body></html>"#;

    const CHAPTER3: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter 3</title></head>
<body><h1>Chapter 3</h1><p class="note">Note three.</p></body></html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
        ("OEBPS/text/chapter2.xhtml", CHAPTER2),
        ("OEBPS/text/chapter3.xhtml", CHAPTER3),
    ])
}

/// A two-chapter EPUB3 book whose first chapter is padded past 600KB across
/// 60 headed sections (each with its own anchor id), for M8's chapter-split
/// pass. Section 1 carries a same-document forward link to the last section,
/// and the last section links back to section 1 - since the two cannot land
/// in the same part once split, this exercises same-document href
/// retargeting across parts. Chapter 2 links into chapter 1's middle section
/// (cross-chapter href retargeting) and to chapter 1 with no fragment
/// (must always land on part 1). The nav doc's TOC carries fragment entries
/// for the first, middle and last sections.
pub fn epub3_oversize_chapter() -> Vec<u8> {
    const N_SECTIONS: usize = 60;
    const PARAS_PER_SECTION: usize = 5;
    const PARA_LEN: usize = 2000;
    let mid = N_SECTIONS / 2;

    let filler = "x".repeat(PARA_LEN);
    let mut chapter1_body = String::new();
    for i in 1..=N_SECTIONS {
        chapter1_body.push_str(&format!("<h1 id=\"sec{i}\">Section {i}</h1>\n"));
        if i == 1 {
            chapter1_body.push_str(&format!(
                "<p><a href=\"#sec{N_SECTIONS}\">jump ahead</a></p>\n"
            ));
        }
        if i == N_SECTIONS {
            chapter1_body.push_str("<p><a href=\"#sec1\">back to start</a></p>\n");
        }
        for _ in 0..PARAS_PER_SECTION {
            chapter1_body.push_str(&format!("<p>{filler}</p>\n"));
        }
    }

    const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Oversize Chapter</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:aaaaaaaa-1111-2222-3333-444455556666</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="text/chapter2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>"#;

    let nav_xhtml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body>
<nav epub:type="toc">
<ol>
<li><a href="text/chapter1.xhtml">Chapter 1</a>
  <ol>
  <li><a href="text/chapter1.xhtml#sec1">Section 1</a></li>
  <li><a href="text/chapter1.xhtml#sec{mid}">Section {mid}</a></li>
  <li><a href="text/chapter1.xhtml#sec{N_SECTIONS}">Section {N_SECTIONS}</a></li>
  </ol>
</li>
<li><a href="text/chapter2.xhtml">Chapter 2</a></li>
</ol>
</nav>
</body>
</html>"#
    );

    let chapter1 = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body>{chapter1_body}</body></html>"#
    );

    let chapter2 = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 2</title></head>
<body><p>Intro.</p>
<p><a href="chapter1.xhtml#sec{mid}">jump to chapter 1's middle</a></p>
<p><a href="chapter1.xhtml">go to chapter 1's start</a></p>
</body></html>"#
    );

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", nav_xhtml.as_bytes()),
        ("OEBPS/text/chapter1.xhtml", chapter1.as_bytes()),
        ("OEBPS/text/chapter2.xhtml", chapter2.as_bytes()),
    ])
}

/// A one-chapter EPUB3 whose chapter reproduces Project Gutenberg's
/// self-closing `<a id="Pagexv"/>` page-break anchors verbatim. Parsed
/// through the HTML5 tree builder (which does not honor `<a .../>` as
/// self-closing - see `epub_tailor_core::html`'s `dedupe` module docs), the
/// anchor element stays open across the enclosing `<p>`'s close tag and gets
/// cloned, `id` and all, onto every following block by the adoption agency
/// algorithm until something actually closes it - producing duplicate ids in
/// a naive serialization (epubcheck RSC-005). A same-document
/// `<a href="#Pagexv">` back-reference exercises that the surviving
/// (first-occurrence) id still resolves after dedupe.
pub fn epub3_gutenberg_style_ids() -> Vec<u8> {
    const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Gutenberg Style</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:11112222-3333-4444-5555-666677778888</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

    const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body>
<nav epub:type="toc">
<ol>
<li><a href="text/chapter1.xhtml">Chapter 1</a></li>
</ol>
</nav>
</body>
</html>"#;

    const CHAPTER1: &[u8] = br##"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body>
<h1>Chapter 1</h1>
<p><a id="Pagexv"/>Some text on page xv.</p>
<p>More text on the next page.</p>
<p><a id="Pagexvi"/>Even more text.</p>
<p>Final paragraph of the chapter.</p>
<p>See <a href="#Pagexv">back to page xv</a>.</p>
</body></html>"##;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
    ])
}

/// A minimal, well-formed EPUB2 book: same two chapters, but a NCX instead of
/// a nav doc (spine `toc="ncx"`, no `properties=nav` item anywhere).
pub fn epub2_minimal() -> Vec<u8> {
    const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Sample Book 2</dc:title>
    <dc:creator>John Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:87654321-4321-4321-4321-210987654321</dc:identifier>
  </metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="text/chapter2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>"#;

    const TOC_NCX: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="urn:uuid:87654321-4321-4321-4321-210987654321"/></head>
  <docTitle><text>Sample Book 2</text></docTitle>
  <navMap>
    <navPoint id="np1" playOrder="1">
      <navLabel><text>Chapter 1</text></navLabel>
      <content src="text/chapter1.xhtml"/>
    </navPoint>
    <navPoint id="np2" playOrder="2">
      <navLabel><text>Chapter 2</text></navLabel>
      <content src="text/chapter2.xhtml"/>
    </navPoint>
  </navMap>
</ncx>"#;

    const CHAPTER1: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body><h1>Chapter 1</h1><p>Text.</p></body></html>"#;

    const CHAPTER2: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 2</title></head>
<body><h1>Chapter 2</h1><p>Text.</p></body></html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/toc.ncx", TOC_NCX),
        ("OEBPS/text/chapter1.xhtml", CHAPTER1),
        ("OEBPS/text/chapter2.xhtml", CHAPTER2),
    ])
}

/// A one-chapter EPUB3 exercising the `--tables image` heuristic, with three
/// top-level tables in a single chapter:
///
/// - **Table A** - three columns: rasterized under `Image` (column count >= 3).
///   Its caption carries `&`, `<` and `"` so the generated `<img alt>` must
///   escape them to stay well-formed XHTML.
/// - **Table B** - two plain text columns: linearized under `Image` (a simple
///   table), but rasterized under `ImageAll`.
/// - **Table C** - holds a `<span id>` anchor target that a link elsewhere in
///   the chapter references: always linearized (safety fallback), under both
///   `Image` and `ImageAll`, so the anchor survives. The id sits on an inner
///   `<span>` so cell flattening keeps it (and the referencing link resolves).
pub fn epub3_tables() -> Vec<u8> {
    const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Tables</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:7ab1e000-1111-2222-3333-444455556666</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

    const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body>
<nav epub:type="toc">
<ol>
<li><a href="text/chapter.xhtml">Tables</a></li>
</ol>
</nav>
</body>
</html>"#;

    const CHAPTER: &[u8] = br##"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Tables</title></head>
<body>
<h1>Tables</h1>
<p><a href="#reftarget">jump to the referenced cell</a></p>
<h2>Table A</h2>
<table>
<caption>Fruit &amp; "Veg" &lt;prices&gt;</caption>
<thead><tr><th>Name</th><th>Qty</th><th>Price</th></tr></thead>
<tbody>
<tr><td>Apple</td><td>3</td><td>1.20</td></tr>
<tr><td>Pear</td><td>5</td><td>0.90</td></tr>
</tbody>
</table>
<h2>Table B</h2>
<table>
<tr><td>alpha</td><td>beta</td></tr>
<tr><td>gamma</td><td>delta</td></tr>
</table>
<h2>Table C</h2>
<table>
<tr><td>label</td><td><span id="reftarget">referenced value</span></td></tr>
<tr><td>second</td><td>row</td></tr>
</table>
</body>
</html>"##;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
    ])
}

/// A book with one 3-column table whose middle body cell holds its own text
/// plus a nested 2x2 table - the T7 nested-grid rendering path. It carries no
/// anchors, links or images, so under `--tables image` it rasterizes into a
/// single `chapter-table-1.png` with the inner grid drawn inside the parent
/// cell (and the transformation detail marked `(nested)`).
pub fn epub3_nested_tables() -> Vec<u8> {
    const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const CONTENT_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Nested Tables</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:7ab1e111-2222-3333-4444-555566667777</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

    const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body>
<nav epub:type="toc">
<ol>
<li><a href="text/chapter.xhtml">Nested Tables</a></li>
</ol>
</nav>
</body>
</html>"#;

    const CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Nested Tables</title></head>
<body>
<h1>Nested Tables</h1>
<table>
<caption>Fruit with details</caption>
<thead><tr><th>Name</th><th>Detail</th><th>Qty</th></tr></thead>
<tbody>
<tr>
<td>Apple</td>
<td>see grid<table><tr><td>a1</td><td>b1</td></tr><tr><td>a2</td><td>b2</td></tr></table></td>
<td>3</td>
</tr>
<tr><td>Pear</td><td>plain</td><td>5</td></tr>
</tbody>
</table>
</body>
</html>"#;

    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", CONTENT_OPF),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/text/chapter.xhtml", CHAPTER),
    ])
}
