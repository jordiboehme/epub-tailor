//! The generic profile: per-copy marker removal and the convergence property.

mod common;

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
