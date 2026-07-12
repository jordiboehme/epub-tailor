//! The all-capable `epub` profile is repair-only: fonts, CSS, images, tables
//! and `<style>` blocks all pass through untouched, while genuine repairs
//! (META-INF junk removal, duplicate ids, unicode hygiene) still run.

mod common;

use std::io::{Cursor, Read};

use epub_tailor_core::profile::{DeviceCaps, Features};
use epub_tailor_core::{ConvertOptions, Input, convert};
use zip::ZipArchive;

/// The resolved built-in `epub` profile as ConvertOptions.
fn repair_only_opts() -> ConvertOptions {
    ConvertOptions {
        device: DeviceCaps::permissive(),
        features: Features::repair_only(),
        ..ConvertOptions::default()
    }
}

fn entry(epub: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    let mut file = match archive.by_name(name) {
        Ok(file) => file,
        Err(_) => return None,
    };
    let mut data = Vec::new();
    file.read_to_end(&mut data).expect("read entry");
    Some(data)
}

fn entry_names(epub: &[u8]) -> Vec<String> {
    let archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    archive.file_names().map(String::from).collect()
}

#[test]
fn repair_profile_keeps_embedded_fonts_byte_identical() {
    let source = common::epub3_css_kitchen();
    let converted = convert(Input::Epub(source), &repair_only_opts()).expect("conversion succeeds");
    let font = entry(&converted.epub, "OEBPS/fonts/DejaVu.ttf")
        .expect("font resource must survive the repair profile");
    assert!(
        font.starts_with(b"\x00\x01\x00\x00"),
        "font bytes must be untouched"
    );
}

#[test]
fn repair_profile_keeps_stylesheets_byte_identical() {
    let source = common::epub3_css_kitchen();
    let mut source_archive = ZipArchive::new(Cursor::new(&source[..])).expect("fixture zip");
    let mut original_css = Vec::new();
    source_archive
        .by_name("OEBPS/styles/ext.css")
        .expect("fixture has ext.css")
        .read_to_end(&mut original_css)
        .expect("read fixture css");

    let converted =
        convert(Input::Epub(source.clone()), &repair_only_opts()).expect("conversion succeeds");
    let css = entry(&converted.epub, "OEBPS/styles/ext.css").expect("ext.css must survive");
    assert_eq!(
        css, original_css,
        "the repair profile must not filter or rewrite CSS"
    );
}

#[test]
fn repair_profile_keeps_tables_and_boxes_in_chapters() {
    let source = common::epub3_kitchen_sink();
    let converted = convert(Input::Epub(source), &repair_only_opts()).expect("conversion succeeds");
    let chapter = entry(&converted.epub, "OEBPS/text/kitchen.xhtml").expect("chapter survives");
    let chapter = String::from_utf8(chapter).expect("chapter is UTF-8");
    assert!(chapter.contains("<table"), "tables must pass through");
    assert!(chapter.contains("<aside"), "asides must pass through");
    assert!(chapter.contains("<figure"), "figures must pass through");
    assert!(
        chapter.contains("<dl"),
        "definition lists must pass through"
    );
    assert!(
        chapter.contains("<pre"),
        "code blocks must not be rebuilt by the repair profile"
    );
}

#[test]
fn repair_profile_keeps_head_style_blocks_in_place() {
    let source = common::epub3_css_kitchen();
    let converted = convert(Input::Epub(source), &repair_only_opts()).expect("conversion succeeds");
    let chapter =
        entry(&converted.epub, "OEBPS/text/chapter.xhtml").expect("styled chapter survives");
    let chapter = String::from_utf8(chapter).expect("chapter is UTF-8");
    assert!(
        chapter.contains("<style"),
        "the repair profile must not relocate <style> blocks, got:\n{chapter}"
    );
    assert!(
        !entry_names(&converted.epub)
            .iter()
            .any(|n| n.ends_with("et-relocated.css")),
        "no relocated stylesheet may be generated"
    );
}

#[test]
fn repair_profile_keeps_images_byte_identical() {
    let source = common::epub3_kitchen_sink();
    let mut source_archive = ZipArchive::new(Cursor::new(&source[..])).expect("fixture zip");
    let mut original_cover = Vec::new();
    source_archive
        .by_name("OEBPS/images/cover.jpg")
        .expect("fixture has a cover")
        .read_to_end(&mut original_cover)
        .expect("read fixture cover");

    let converted =
        convert(Input::Epub(source.clone()), &repair_only_opts()).expect("conversion succeeds");
    let cover = entry(&converted.epub, "OEBPS/images/cover.jpg").expect("cover must survive");
    assert_eq!(
        cover, original_cover,
        "the repair profile must not transcode images"
    );
}

#[test]
fn repair_profile_still_runs_unicode_hygiene() {
    let source = common::epub3_kitchen_sink();
    let converted = convert(Input::Epub(source), &repair_only_opts()).expect("conversion succeeds");
    let chapter = entry(&converted.epub, "OEBPS/text/kitchen.xhtml").expect("chapter survives");
    let chapter = String::from_utf8(chapter).expect("chapter is UTF-8");
    assert!(
        chapter.contains("caf\u{e9}"),
        "decomposed text must be NFC-normalized even by the repair profile"
    );
}

#[test]
fn repair_profile_drops_meta_inf_junk() {
    // epub3_minimal plus two junk META-INF entries vendors like to leave behind.
    let source = common::epub3_minimal();
    let mut archive = ZipArchive::new(Cursor::new(&source[..])).expect("fixture zip");
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).expect("entry");
        let mut data = Vec::new();
        file.read_to_end(&mut data).expect("read entry");
        entries.push((file.name().to_string(), data));
    }
    entries.push(("META-INF/cdp.info".to_string(), b"junk".to_vec()));
    entries.push((
        "META-INF/com.apple.ibooks.display-options.xml".to_string(),
        b"<display_options/>".to_vec(),
    ));
    let with_junk = common::build_epub(
        &entries
            .iter()
            .map(|(name, data)| (name.as_str(), data.as_slice()))
            .collect::<Vec<_>>(),
    );

    let converted =
        convert(Input::Epub(with_junk), &repair_only_opts()).expect("conversion succeeds");
    let names = entry_names(&converted.epub);
    assert!(
        !names.iter().any(|n| n == "META-INF/cdp.info"),
        "junk META-INF entries must be dropped"
    );
    assert!(
        !names
            .iter()
            .any(|n| n == "META-INF/com.apple.ibooks.display-options.xml"),
        "junk META-INF entries must be dropped"
    );
    assert!(
        converted
            .report
            .warnings
            .iter()
            .any(|w| w.message.contains("META-INF")),
        "the report must mention the dropped entries"
    );
}

#[test]
fn x4_defaults_still_linearize_tables() {
    // Guard that gating did not invert: the default (x4-equivalent) options
    // must still remove every <table>.
    let source = common::epub3_kitchen_sink();
    let converted =
        convert(Input::Epub(source), &ConvertOptions::default()).expect("conversion succeeds");
    let chapter = entry(&converted.epub, "OEBPS/text/kitchen.xhtml").expect("chapter survives");
    let chapter = String::from_utf8(chapter).expect("chapter is UTF-8");
    assert!(
        !chapter.contains("<table"),
        "x4 defaults must still linearize tables"
    );
}
