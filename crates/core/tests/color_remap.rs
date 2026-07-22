//! End-to-end tests for the gray-tone remap ("palette solver"): convert the
//! color-kitchen book and assert the CSS and SVG surfaces come out as
//! perceptually spaced panel grays - or stay byte-identical where the feature
//! must not apply.

mod common;

use std::io::{Cursor, Read};

use common::epub3_color_kitchen;
use epub_tailor_core::{ConvertOptions, Input, convert, profile};
use zip::ZipArchive;

fn read_entry(epub: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    let mut file = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("output should contain {name}"));
    let mut out = String::new();
    file.read_to_string(&mut out).expect("entry is UTF-8");
    out
}

fn read_entry_bytes(epub: &[u8], name: &str) -> Vec<u8> {
    let mut archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    let mut file = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("output should contain {name}"));
    let mut out = Vec::new();
    file.read_to_end(&mut out).expect("entry reads");
    out
}

fn entry_names(epub: &[u8]) -> Vec<String> {
    let archive = ZipArchive::new(Cursor::new(epub)).expect("output is a valid zip");
    archive.file_names().map(str::to_string).collect()
}

fn options_for(profile_name: &str) -> ConvertOptions {
    profile::resolve(&[profile_name.to_string()])
        .expect("built-in profile resolves")
        .to_options()
}

/// Every `#rgb`/`#rrggbb` literal in `text` must be a gray (r = g = b).
fn assert_hex_literals_are_gray(text: &str, context: &str) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(offset) = text[i..].find('#') {
        let start = i + offset + 1;
        let hex: String = text[start..]
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect();
        match hex.len() {
            3 => {
                assert!(
                    hex.as_bytes()[0] == hex.as_bytes()[1]
                        && hex.as_bytes()[1] == hex.as_bytes()[2],
                    "{context}: non-gray #{hex}"
                );
            }
            6 => {
                assert!(
                    hex[0..2] == hex[2..4] && hex[2..4] == hex[4..6],
                    "{context}: non-gray #{hex}"
                );
            }
            _ => {}
        }
        i = start.min(bytes.len());
    }
}

/// The gray channel value of a `color:#...` declaration for `selector`.
fn gray_of(css: &str, selector: &str) -> u8 {
    let rule_start = css
        .find(selector)
        .unwrap_or_else(|| panic!("{selector} in: {css}"));
    let rule = &css[rule_start..];
    let value_start = rule.find("color:#").expect("a remapped color") + "color:#".len();
    let hex: String = rule[value_start..]
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect();
    match hex.len() {
        3 => u8::from_str_radix(&hex[0..1].repeat(2), 16).expect("hex"),
        6 => u8::from_str_radix(&hex[0..2], 16).expect("hex"),
        other => panic!("unexpected hex length {other} in {hex}"),
    }
}

#[test]
fn x4_defaults_remap_every_surviving_color() {
    let converted = convert(
        Input::Epub(epub3_color_kitchen()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");

    let kinds: Vec<&str> = converted
        .report
        .transformations
        .iter()
        .map(|t| t.kind.as_str())
        .collect();
    assert!(kinds.contains(&"colors-remapped"), "got: {kinds:?}");
    assert!(kinds.contains(&"svg-colors-remapped"), "got: {kinds:?}");

    // Six text-ish colors cannot stay apart on 4 levels: the collapse warns.
    assert!(
        converted
            .report
            .warnings
            .iter()
            .any(|w| w.message.contains("share a gray tone")),
        "expected a collapse warning, got: {:?}",
        converted.report.warnings
    );

    // The external sheet: colors survive filtering, remapped to gray4 levels.
    let ext = read_entry(&converted.epub, "OEBPS/styles/ext.css");
    for source in ["#e67e22", "#009688", "#663399", "#ffee88", "#b22222"] {
        assert!(!ext.contains(source), "{source} survived: {ext}");
    }
    assert_hex_literals_are_gray(&ext, "ext.css");
    for selector in [".alert", ".note"] {
        let gray = gray_of(&ext, selector);
        assert!(
            [0u8, 85, 170, 255].contains(&gray),
            "{selector} tone {gray} is not a gray4 level"
        );
        assert!(
            gray <= 136,
            "{selector} text tone {gray} is too light to read"
        );
    }

    // The relocated head <style> got its color remapped too.
    let relocated = read_entry(&converted.epub, "OEBPS/et-relocated.css");
    assert!(!relocated.contains("#b22222"), "got: {relocated}");
    assert!(relocated.contains("text-align:center"), "got: {relocated}");
    assert_hex_literals_are_gray(&relocated, "et-relocated.css");

    // Inline styles: remapped, and identically in both chapters.
    let ch1 = read_entry(&converted.epub, "OEBPS/text/chapter1.xhtml");
    let ch2 = read_entry(&converted.epub, "OEBPS/text/chapter2.xhtml");
    assert!(!ch1.contains("color:teal"), "source color survived:\n{ch1}");
    let tone_in = |chapter: &str| {
        let start = chapter
            .find(r#"style="color:#"#)
            .expect("a remapped inline style")
            + r#"style="color:"#.len();
        chapter[start..]
            .chars()
            .take_while(|c| *c != '"')
            .collect::<String>()
    };
    assert_eq!(
        tone_in(&ch1),
        tone_in(&ch2),
        "the same source color must get the same tone in every chapter"
    );

    // SVG is rasterized away entirely on x4...
    assert!(!ch1.contains("<svg"), "no inline svg survives:\n{ch1}");
    let names = entry_names(&converted.epub);
    assert!(
        !names.iter().any(|n| n.ends_with(".svg")),
        "no svg resource survives: {names:?}"
    );

    // ...and the inline teal/orange diagram rendered as two clearly distinct
    // mid grays (the whole point: equal-luminance hues must not collapse).
    let inline_raster = names
        .iter()
        .find(|n| n.contains("-svg-") && n.ends_with(".png"))
        .expect("the inline svg became a raster");
    let png = read_entry_bytes(&converted.epub, inline_raster);
    let luma = image::load_from_memory(&png)
        .expect("raster decodes")
        .to_luma8();
    let mut counts = [0u32; 256];
    for pixel in luma.pixels() {
        counts[pixel.0[0] as usize] += 1;
    }
    let region_tones: Vec<usize> = (10..=244).filter(|&v| counts[v] > 200).collect();
    assert!(
        region_tones.len() >= 2
            && region_tones.last().unwrap() - region_tones.first().unwrap() >= 30,
        "expected two separated mid tones, histogram peaks: {region_tones:?}"
    );
}

#[test]
fn gray16_sanitize_profile_remaps_but_keeps_the_sheet_whole() {
    let opts = options_for("kobo-clara-bw");
    let converted =
        convert(Input::Epub(epub3_color_kitchen()), &opts).expect("conversion should succeed");

    // The sheet keeps every rule (no subset filtering) but the colors turned
    // gray. `.muted` (#444444) is already a 16-level gray and stays itself.
    let ext = read_entry(&converted.epub, "OEBPS/styles/ext.css");
    for selector in ["body", ".alert", ".note", ".muted", ".box"] {
        assert!(ext.contains(selector), "{selector} must survive: {ext}");
    }
    for source in ["#e67e22", "#009688", "#663399", "#ffee88"] {
        assert!(!ext.contains(source), "{source} survived: {ext}");
    }
    assert_hex_literals_are_gray(&ext, "ext.css");
    assert!(
        ext.contains(".muted{color:#444"),
        "an on-level gray stays: {ext}"
    );

    // The head <style> stays in-chapter (no relocation) with its color gray.
    let ch1 = read_entry(&converted.epub, "OEBPS/text/chapter1.xhtml");
    assert!(ch1.contains("<style"), "kobo keeps the head style:\n{ch1}");
    assert!(!ch1.contains("#b22222"), "the h1 color is remapped:\n{ch1}");
    assert!(
        !ch1.contains("color:teal"),
        "the inline style is remapped:\n{ch1}"
    );
    assert!(
        ch1.contains(r#"style="color:#"#),
        "the inline style survives remapped:\n{ch1}"
    );
}

#[test]
fn gray16_without_rasterization_ships_the_remapped_svg() {
    let mut opts = options_for("kobo-clara-bw");
    opts.features.rasterize_svg = false;
    let converted =
        convert(Input::Epub(epub3_color_kitchen()), &opts).expect("conversion should succeed");

    let svg = read_entry(&converted.epub, "OEBPS/images/diagram.svg");
    for source in ["#009688", "#e67e22", "navy", "gold", "#663399", "#b22222"] {
        assert!(!svg.contains(source), "{source} survived: {svg}");
    }
    assert_hex_literals_are_gray(&svg, "diagram.svg");
    assert!(
        svg.contains(r#"viewBox="0 0 200 100""#),
        "untouched bytes stay: {svg}"
    );
    assert!(
        svg.contains(r##"fill="url(#sky)""##),
        "the gradient reference is untouched: {svg}"
    );
    assert!(
        svg.contains(".wire{stroke:#"),
        "the style block is remapped: {svg}"
    );

    // The chapter's inline SVG also survives, remapped in place.
    let ch1 = read_entry(&converted.epub, "OEBPS/text/chapter1.xhtml");
    assert!(
        ch1.contains("<svg"),
        "svg survives without rasterization:\n{ch1}"
    );
    assert!(
        !ch1.contains("#009688"),
        "inline fills are remapped:\n{ch1}"
    );
}

#[test]
fn color_panels_and_disabled_remap_keep_colors_verbatim() {
    // A color panel never remaps, whatever the profile says.
    let converted = convert(
        Input::Epub(epub3_color_kitchen()),
        &options_for("kindle-colorsoft"),
    )
    .expect("conversion should succeed");
    let ext = read_entry(&converted.epub, "OEBPS/styles/ext.css");
    for source in ["#e67e22", "#009688", "#663399", "#ffee88"] {
        assert!(
            ext.contains(source),
            "{source} must survive untouched: {ext}"
        );
    }
    assert!(
        !converted
            .report
            .transformations
            .iter()
            .any(|t| t.kind.contains("colors-remapped")),
        "no remap on a color panel"
    );

    // Switching the feature off on x4 restores the old behavior: the subset
    // filter drops colors entirely.
    let mut opts = ConvertOptions::default();
    opts.features.remap_colors = false;
    let converted =
        convert(Input::Epub(epub3_color_kitchen()), &opts).expect("conversion should succeed");
    let ext = read_entry(&converted.epub, "OEBPS/styles/ext.css");
    assert!(
        !ext.contains("color:"),
        "colors are dropped when off: {ext}"
    );
    assert!(
        !converted
            .report
            .transformations
            .iter()
            .any(|t| t.kind.contains("colors-remapped")),
        "no remap when the feature is off"
    );
}

#[test]
fn dry_run_reports_the_same_transformations() {
    let wet = convert(
        Input::Epub(epub3_color_kitchen()),
        &ConvertOptions::default(),
    )
    .expect("conversion should succeed");
    let opts = ConvertOptions {
        dry_run: true,
        ..ConvertOptions::default()
    };
    let dry = convert(Input::Epub(epub3_color_kitchen()), &opts).expect("dry run should succeed");

    let kinds = |report: &epub_tailor_core::ConvertReport| {
        let mut kinds: Vec<String> = report
            .transformations
            .iter()
            .map(|t| t.kind.clone())
            .collect();
        kinds.sort();
        kinds
    };
    assert_eq!(kinds(&wet.report), kinds(&dry.report));
}

#[test]
fn remapping_is_a_fixed_point_across_runs() {
    let once = convert(
        Input::Epub(epub3_color_kitchen()),
        &ConvertOptions::default(),
    )
    .expect("first conversion succeeds");
    let twice = convert(Input::Epub(once.epub.clone()), &ConvertOptions::default())
        .expect("second conversion succeeds");
    assert!(
        !twice
            .report
            .transformations
            .iter()
            .any(|t| t.kind.contains("colors-remapped")),
        "already-solved grays must map to themselves, got: {:?}",
        twice
            .report
            .transformations
            .iter()
            .filter(|t| t.kind.contains("colors-remapped"))
            .collect::<Vec<_>>()
    );
}
