//! End-to-end checks for the CrossPoint 2.0 rendering rules that only show up
//! once a whole book goes through `convert`: author CSS that resets the
//! generated list hang, font sizes that would override the reader's size
//! setting, the order the device loads stylesheets in, and full-page
//! background tints.

mod common;

use common::{CONTAINER_XML, build_epub, entry};
use epub_tailor_core::{ConvertOptions, Input, convert};

/// A one-chapter EPUB3 book. `head_style` goes into a `<style>` element in
/// the chapter head (skipped when empty), `sheet` into a linked
/// `styles/main.css` (always present), and `body` is the chapter body.
fn book(head_style: &str, sheet: &str, body: &str) -> Vec<u8> {
    let opf = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>The Spice Guide</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:5a1e5a1e-1111-2222-3333-444455556677</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="css" href="styles/main.css" media-type="text/css"/>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    let nav = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol><li><a href="text/chapter1.xhtml">Arrakis</a></li></ol></nav></body>
</html>"#;
    let style = if head_style.is_empty() {
        String::new()
    } else {
        format!("<style>{head_style}</style>")
    };
    let chapter = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Arrakis</title><link rel="stylesheet" type="text/css" href="../styles/main.css"/>{style}</head>
<body>{body}</body></html>"#
    );
    build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", opf),
        ("OEBPS/nav.xhtml", nav),
        ("OEBPS/styles/main.css", sheet.as_bytes()),
        ("OEBPS/text/chapter1.xhtml", chapter.as_bytes()),
    ])
}

/// Convert `epub` with the x4 defaults and return the output archive.
fn fit(epub: Vec<u8>) -> epub_tailor_core::Converted {
    convert(Input::Epub(epub), &ConvertOptions::default()).expect("conversion should succeed")
}

fn text_entry(epub: &[u8], name: &str) -> String {
    String::from_utf8(entry(epub, name).unwrap_or_else(|| panic!("{name} is in the output")))
        .expect("entry is UTF-8")
}

const LIST: &str = "<ol><li>Find water<ol><li>Stillsuit</li></ol></li><li>Ride a worm</li></ol>";

#[test]
fn list_hang_survives_a_head_style_margin_reset() {
    // `p{margin:0}` becomes `p.cpr1-e-p{margin:0}`, which outranks the
    // `.et-ol-item` class rule on the device; the inline pair beats both.
    let out = fit(book("p{margin:0}", "", LIST));
    let chapter = text_entry(&out.epub, "OEBPS/text/chapter1.xhtml");
    assert!(
        chapter.contains(r#"style="margin-left:1.5em;text-indent:-1.5em""#),
        "{chapter}"
    );
    assert!(
        chapter.contains(r#"style="margin-left:2em;text-indent:-1em""#),
        "{chapter}"
    );
}

#[test]
fn list_hang_survives_a_linked_descendant_margin_reset() {
    let body = format!(r#"<div class="chapter">{LIST}</div>"#);
    let out = fit(book("", ".chapter p{margin:0}", &body));
    let chapter = text_entry(&out.epub, "OEBPS/text/chapter1.xhtml");
    let items = chapter.matches("et-ol-item").count();
    let hung = chapter
        .matches(r#"style="margin-left:1.5em;text-indent:-1.5em""#)
        .count()
        + chapter
            .matches(r#"style="margin-left:2em;text-indent:-1em""#)
            .count();
    assert_eq!(items, 3, "{chapter}");
    assert_eq!(hung, 3, "every item hangs inline:\n{chapter}");
}

#[test]
fn rtl_list_hangs_on_the_right() {
    let body = "<div dir=\"rtl\"><ol><li>\u{5de}\u{5d9}\u{5dd}</li></ol></div>";
    let out = fit(book("", "", body));
    let chapter = text_entry(&out.epub, "OEBPS/text/chapter1.xhtml");
    assert!(
        chapter.contains(r#"style="margin-left:0;margin-right:1.5em;text-indent:-1.5em""#),
        "{chapter}"
    );
}
