//! Content filter behavior: replace and remove rules on chapter text and
//! link hrefs, cascade pruning of emptied elements, metadata filtering and
//! report counts.

mod common;

use std::io::{Cursor, Read};

use epub_tailor_core::filter::FilterRule;
use epub_tailor_core::profile::{DeviceCaps, Features};
use epub_tailor_core::{ConvertOptions, Input, convert};
use zip::ZipArchive;

const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body>
<nav epub:type="toc">
<ol>
<li><a href="chapter.xhtml">InventedWatermark.example Chapter</a></li>
</ol>
</nav>
</body>
</html>"#;

/// A one-chapter book whose chapter body is `body`, with a watermarked title
/// and TOC entry.
fn book_with_body(body: &str) -> Vec<u8> {
    let opf = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>My Book [InventedWatermark.example]</dc:title>
    <dc:creator>Jane Author</dc:creator>
    <dc:creator>InventedWatermark.example</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:12345678-1234-1234-1234-123456789012</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="chapter.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;
    let chapter = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title></head>
<body>{body}</body>
</html>"#
    );
    common::build_epub(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", CONTAINER_XML),
        ("OEBPS/content.opf", opf),
        ("OEBPS/nav.xhtml", NAV_XHTML),
        ("OEBPS/chapter.xhtml", chapter.as_bytes()),
    ])
}

/// The chapter-trailing watermark block the motivating real-world case leaves
/// in every chapter.
const WATERMARK_BLOCK: &str = concat!(
    r#"<div style="float: none; margin: 10px 0px 10px 0px; text-align: center;">"#,
    r#"<p><a href="https://inventedwatermark.example"><i>InventedWatermark.example</i></a></p>"#,
    r#"</div>"#
);

fn rules(json: &str) -> Vec<FilterRule> {
    serde_json::from_str(json).expect("test rules parse")
}

/// Repair-only options plus the given filter rules.
fn opts_with_filters(filters: Vec<FilterRule>) -> ConvertOptions {
    ConvertOptions {
        device: DeviceCaps::permissive(),
        features: Features::repair_only(),
        filters,
        ..ConvertOptions::default()
    }
}

fn chapter_of(epub: &[u8]) -> String {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    let mut file = archive.by_name("OEBPS/chapter.xhtml").expect("chapter");
    let mut data = String::new();
    file.read_to_string(&mut data).expect("read chapter");
    data
}

fn opf_of(epub: &[u8]) -> String {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    let mut file = archive.by_name("OEBPS/content.opf").expect("opf");
    let mut data = String::new();
    file.read_to_string(&mut data).expect("read opf");
    data
}

#[test]
fn replace_rewrites_every_occurrence_in_text() {
    let source = book_with_body("<p>The colour of colourful things.</p>");
    let opts = opts_with_filters(rules(
        r#"[{ "action": "replace", "match": "colour", "with": "color" }]"#,
    ));
    let converted = convert(Input::Epub(source), &opts).expect("conversion succeeds");
    let chapter = chapter_of(&converted.epub);
    assert!(!chapter.contains("colour"), "got:\n{chapter}");
    assert!(chapter.contains("The color of colorful things."));
    assert!(
        converted
            .report
            .transformations
            .iter()
            .any(|t| t.kind == "filter-replaced"),
        "the report must count replacements"
    );
}

#[test]
fn remove_deletes_the_match_but_keeps_a_non_empty_element() {
    let source = book_with_body("<p>Downloaded from InventedWatermark.example for free.</p>");
    let opts = opts_with_filters(rules(
        r#"[{ "action": "remove", "match": "InventedWatermark.example" }]"#,
    ));
    let converted = convert(Input::Epub(source), &opts).expect("conversion succeeds");
    let chapter = chapter_of(&converted.epub);
    assert!(!chapter.contains("InventedWatermark"), "got:\n{chapter}");
    assert!(
        chapter.contains("Downloaded from"),
        "surrounding text must survive, got:\n{chapter}"
    );
}

#[test]
fn remove_by_text_prunes_the_emptied_watermark_block() {
    let source = book_with_body(&format!("<p>Real content.</p>{WATERMARK_BLOCK}"));
    let opts = opts_with_filters(rules(
        r#"[{ "action": "remove", "match": "InventedWatermark.example" }]"#,
    ));
    let converted = convert(Input::Epub(source), &opts).expect("conversion succeeds");
    let chapter = chapter_of(&converted.epub);
    assert!(!chapter.contains("InventedWatermark"), "got:\n{chapter}");
    assert!(!chapter.contains("<div"), "the emptied div must be pruned");
    assert!(
        !chapter.contains("inventedwatermark.example"),
        "the emptied anchor (and its href) must be pruned, got:\n{chapter}"
    );
    assert!(chapter.contains("Real content."));
    assert!(
        converted
            .report
            .transformations
            .iter()
            .any(|t| t.kind == "filter-pruned"),
        "the report must count pruned elements"
    );
}

#[test]
fn remove_by_href_detaches_the_anchor_even_with_generic_text() {
    let body = concat!(
        "<p>Real content.</p>",
        r#"<div><p><a href="https://inventedwatermark.example/dl">Read more</a></p></div>"#
    );
    let source = book_with_body(body);
    let opts = opts_with_filters(rules(
        r#"[{ "action": "remove", "match": "inventedwatermark.example", "in": ["href"] }]"#,
    ));
    let converted = convert(Input::Epub(source), &opts).expect("conversion succeeds");
    let chapter = chapter_of(&converted.epub);
    assert!(!chapter.contains("inventedwatermark"), "got:\n{chapter}");
    assert!(
        !chapter.contains("Read more"),
        "the anchor text goes with it"
    );
    assert!(!chapter.contains("<div"), "the emptied div must be pruned");
    assert!(chapter.contains("Real content."));
}

#[test]
fn pruning_stops_at_elements_that_still_hold_content() {
    let body = concat!(
        r#"<p><img src="nothing.png" alt=""/>"#,
        r#"<a href="https://inventedwatermark.example">InventedWatermark.example</a></p>"#
    );
    let source = book_with_body(body);
    let opts = opts_with_filters(rules(
        r#"[{ "action": "remove", "match": "InventedWatermark.example", "in": ["text", "href"] }]"#,
    ));
    let converted = convert(Input::Epub(source), &opts).expect("conversion succeeds");
    let chapter = chapter_of(&converted.epub);
    assert!(!chapter.contains("InventedWatermark"), "got:\n{chapter}");
    assert!(
        chapter.contains("<img"),
        "the img and its parent p must survive, got:\n{chapter}"
    );
}

#[test]
fn protected_table_cells_are_never_pruned() {
    let body = concat!(
        "<table><tbody><tr>",
        "<td>InventedWatermark.example</td><td>kept</td>",
        "</tr></tbody></table>"
    );
    let source = book_with_body(body);
    let opts = opts_with_filters(rules(
        r#"[{ "action": "remove", "match": "InventedWatermark.example" }]"#,
    ));
    let converted = convert(Input::Epub(source), &opts).expect("conversion succeeds");
    let chapter = chapter_of(&converted.epub);
    assert!(!chapter.contains("InventedWatermark"));
    assert!(
        chapter.contains("<table") && chapter.contains("<td"),
        "table structure must survive even when a cell empties, got:\n{chapter}"
    );
    assert!(chapter.contains("kept"));
}

#[test]
fn rules_apply_in_order() {
    let source = book_with_body("<p>alpha</p>");
    let opts = opts_with_filters(rules(
        r#"[
            { "action": "replace", "match": "alpha", "with": "beta" },
            { "action": "replace", "match": "beta", "with": "gamma" }
        ]"#,
    ));
    let converted = convert(Input::Epub(source), &opts).expect("conversion succeeds");
    let chapter = chapter_of(&converted.epub);
    assert!(chapter.contains("gamma"), "got:\n{chapter}");
}

#[test]
fn metadata_and_toc_are_filtered_too() {
    let source = book_with_body("<p>Real content.</p>");
    let opts = opts_with_filters(rules(
        r#"[{ "action": "remove", "match": "InventedWatermark.example" }]"#,
    ));
    let converted = convert(Input::Epub(source), &opts).expect("conversion succeeds");
    let opf = opf_of(&converted.epub);
    assert!(
        !opf.contains("InventedWatermark"),
        "title and creators must be filtered, got:\n{opf}"
    );
    assert!(opf.contains("My Book"), "the title itself survives");
    assert!(opf.contains("Jane Author"), "clean authors survive");
    // The all-watermark creator entry is dropped entirely.
    assert!(!opf.contains("<dc:creator></dc:creator>"));

    let mut archive =
        ZipArchive::new(Cursor::new(&converted.epub[..])).expect("output is a valid zip");
    let mut nav = String::new();
    archive
        .by_name("OEBPS/nav.xhtml")
        .expect("nav")
        .read_to_string(&mut nav)
        .expect("read nav");
    assert!(
        !nav.contains("InventedWatermark"),
        "TOC titles must be filtered, got:\n{nav}"
    );
}

#[test]
fn a_title_that_would_empty_is_left_with_a_warning() {
    let source = book_with_body("<p>Real content.</p>");
    let opts = opts_with_filters(rules(
        r#"[{ "action": "remove", "match": "My Book [InventedWatermark.example]" }]"#,
    ));
    let converted = convert(Input::Epub(source), &opts).expect("conversion succeeds");
    let opf = opf_of(&converted.epub);
    assert!(
        opf.contains("My Book [InventedWatermark.example]"),
        "an emptying title match must be left untouched, got:\n{opf}"
    );
    assert!(
        converted
            .report
            .warnings
            .iter()
            .any(|w| w.message.contains("title")),
        "a warning must explain why the title kept the match"
    );
}

#[test]
fn filters_run_before_device_transforms() {
    // With the full x4 feature set the watermark block must be gone before
    // boxes degrade / anchors relocate, leaving no trace of it.
    let source = book_with_body(&format!("<p>Real content.</p>{WATERMARK_BLOCK}"));
    let opts = ConvertOptions {
        filters: rules(
            r#"[{ "action": "remove", "match": "InventedWatermark.example", "in": ["text", "href"] }]"#,
        ),
        ..ConvertOptions::default()
    };
    let converted = convert(Input::Epub(source), &opts).expect("conversion succeeds");
    let chapter = chapter_of(&converted.epub);
    assert!(
        !chapter.to_lowercase().contains("inventedwatermark"),
        "got:\n{chapter}"
    );
    assert!(chapter.contains("Real content."));
}

#[test]
fn remove_targeting_files_drops_matching_stray_resources() {
    // Some vendors drop a marker file named after their site into the
    // archive; a `file`-targeted remove rule must delete the resource so the
    // regenerated manifest never lists it.
    let source = book_with_body("<p>Real content.</p>");
    let mut archive = ZipArchive::new(Cursor::new(&source[..])).expect("fixture zip");
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).expect("entry");
        let mut data = Vec::new();
        file.read_to_end(&mut data).expect("read entry");
        entries.push((file.name().to_string(), data));
    }
    entries.push(("inventedwatermark.example".to_string(), b"marker".to_vec()));
    let with_marker = common::build_epub(
        &entries
            .iter()
            .map(|(name, data)| (name.as_str(), data.as_slice()))
            .collect::<Vec<_>>(),
    );

    let opts = opts_with_filters(rules(
        r#"[{ "action": "remove", "match": "inventedwatermark.example", "in": ["file"] }]"#,
    ));
    let converted = convert(Input::Epub(with_marker), &opts).expect("conversion succeeds");
    let archive = ZipArchive::new(Cursor::new(&converted.epub[..])).expect("output is a valid zip");
    let names: Vec<String> = archive.file_names().map(String::from).collect();
    assert!(
        !names.iter().any(|n| n.contains("inventedwatermark")),
        "the marker file must be gone, got: {names:?}"
    );
    let opf = opf_of(&converted.epub);
    assert!(
        !opf.contains("inventedwatermark"),
        "the manifest must not list the dropped file, got:\n{opf}"
    );
    assert!(
        converted
            .report
            .transformations
            .iter()
            .any(|t| t.kind == "filter-removed" && t.detail.contains("resource")),
        "the report must mention the dropped resource"
    );
}

#[test]
fn file_target_never_drops_spine_or_structural_resources() {
    // A reckless pattern that matches the chapter itself must not break the
    // book: spine documents and the OPF/nav are protected.
    let source = book_with_body("<p>Real content.</p>");
    let opts = opts_with_filters(rules(
        r#"[{ "action": "remove", "match": "chapter.xhtml", "in": ["file"] }]"#,
    ));
    let converted = convert(Input::Epub(source), &opts).expect("conversion succeeds");
    let chapter = chapter_of(&converted.epub);
    assert!(chapter.contains("Real content."), "spine content survives");
}
