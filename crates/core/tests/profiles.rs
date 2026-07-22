//! Behavior of the JSON profile model: builtin parsing, name/path resolution
//! and left-to-right composition semantics.

use epub_tailor_core::TableMode;
use epub_tailor_core::filter::{FilterAction, FilterRule};
use epub_tailor_core::profile::{DeviceCaps, Features, Panel, Profile, builtins, resolve};

fn temp_profile(name: &str, json: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("epub-tailor-profiles-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join(name);
    std::fs::write(&path, json).expect("write profile fixture");
    path
}

fn resolve_specs(specs: &[&str]) -> Result<Profile, String> {
    resolve(&specs.iter().map(|s| s.to_string()).collect::<Vec<_>>()).map_err(|e| e.to_string())
}

#[test]
fn x4_builtin_matches_legacy_constants() {
    let p = resolve_specs(&["x4"]).expect("x4 resolves");
    assert_eq!(p.name, "x4");
    assert_eq!(p.caps, DeviceCaps::x4());
    assert_eq!(p.features, Features::all_on());
    assert_eq!(p.jpeg_quality, 82);
    assert_eq!(p.tables, TableMode::Text);
    assert!(!p.split_tall_images);
    assert_eq!(p.max_chapter_bytes, 200 * 1024);
    assert_eq!(p.appendix.as_deref(), Some("x4"));
}

#[test]
fn x4_caps_pin_the_documented_firmware_limits() {
    // The JSON must stay in lock-step with the documented device constraints;
    // a drift in either direction fails here field by field.
    let caps = resolve_specs(&["x4"]).expect("x4 resolves").caps;
    assert_eq!(caps.screen_w, 480);
    assert_eq!(caps.screen_h, 800);
    assert_eq!(caps.ppi, 220);
    assert_eq!(caps.panel, Panel::Gray4);
    assert_eq!(caps.max_src_px, (2048, 1536));
    assert_eq!(caps.inline_max, (480, 730));
    assert_eq!(caps.cover_max, (480, 800));
    assert_eq!(caps.inline_budget_bytes, 100 * 1024);
    assert_eq!(caps.cover_budget_bytes, 127 * 1024);
    assert_eq!(caps.css_max_bytes, 128 * 1024);
    assert_eq!(caps.css_max_rules, 1500);
}

#[test]
fn x3_builtin_matches_legacy_constants() {
    let p = resolve_specs(&["x3"]).expect("x3 resolves");
    assert_eq!(p.name, "x3");
    assert_eq!(p.caps, DeviceCaps::x3());
    assert_eq!(p.features, Features::all_on());
    assert_eq!(p.appendix.as_deref(), Some("x3"));
    assert_eq!(p.caps.screen_w, 528);
    assert_eq!(p.caps.screen_h, 792);
    assert_eq!(p.caps.inline_max, (528, 722));
    assert_eq!(p.caps.cover_max, (528, 792));
}

#[test]
fn epub_builtin_is_repair_only() {
    let p = resolve_specs(&["epub"]).expect("epub resolves");
    assert_eq!(p.name, "epub");
    let f = p.features;
    // Genuine repairs stay on.
    assert!(f.dedupe_ids);
    assert!(f.unicode_hygiene);
    // Device downgrades are all off.
    assert!(!f.strip_fonts);
    assert!(!f.filter_css);
    assert!(!f.relocate_styles);
    assert!(!f.transcode_images);
    assert!(!f.rasterize_svg);
    assert!(!f.linearize_tables);
    assert!(!f.degrade_boxes);
    assert!(!f.bake_ordered_lists);
    assert!(!f.preserve_code_blocks);
    assert!(!f.normalize_footnotes);
    assert!(!f.relocate_anchors);
    assert!(!f.chapter_split);
    // No appendix of its own: the generic default applies.
    assert_eq!(p.appendix, None);
    assert_eq!(p.appendix_or_default(), "tailored");
}

#[test]
fn default_is_an_alias_for_epub_and_empty_specs_resolve_to_it() {
    let via_alias = resolve_specs(&["default"]).expect("default resolves");
    let via_name = resolve_specs(&["epub"]).expect("epub resolves");
    let via_empty = resolve_specs(&[]).expect("empty resolves");
    assert_eq!(via_alias.features, via_name.features);
    assert_eq!(via_alias.name, "epub");
    assert_eq!(via_empty.features, via_name.features);
    assert_eq!(via_empty.name, "epub");
}

#[test]
fn unknown_builtin_name_errors_listing_builtins() {
    let err = resolve_specs(&["x5"]).expect_err("x5 must not resolve");
    assert!(err.contains("unknown profile"), "got: {err}");
    assert!(err.contains("x4"), "error should list built-ins: {err}");
}

#[test]
fn builtins_lists_every_shipped_profile() {
    let names: Vec<String> = builtins().into_iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        vec![
            "epub",
            "x4",
            "x3",
            "nomad",
            "kindle",
            "kindle-paperwhite",
            "kindle-colorsoft",
            "kindle-scribe",
            "kindle-scribe-colorsoft",
            "kobo-clara-bw",
            "kobo-clara-colour",
            "kobo-libra-colour",
            "kobo-elipsa-2e",
            "pocketbook-verse",
            "pocketbook-verse-pro",
            "pocketbook-era",
            "pocketbook-era-color",
            "pocketbook-inkpad-4",
            "pocketbook-inkpad-color-3",
            "boox-page",
            "boox-go-7",
            "boox-go-color-7",
            "boox-palma-2-pro",
            "tolino-shine",
            "tolino-shine-color",
            "tolino-vision-color",
            "tolino-epos-3",
        ]
    );
}

#[test]
fn every_builtin_names_itself_and_its_own_appendix() {
    // A copy-paste slip between the ten device profiles would otherwise write
    // the wrong filename or resolve under the wrong name, silently.
    for profile in builtins() {
        let resolved = resolve_specs(&[&profile.name]).expect("built-in resolves by its own name");
        assert_eq!(resolved.name, profile.name);
        if profile.name != "epub" {
            assert_eq!(
                resolved.appendix.as_deref(),
                Some(profile.name.as_str()),
                "{} should write .{}.epub",
                profile.name,
                profile.name
            );
        }
    }
}

#[test]
fn the_color_devices_keep_their_color_and_the_rest_do_not() {
    // The one cap that silently destroys content if it is wrong: a color panel
    // marked grayscale gets its images grayscaled, with no warning.
    for name in [
        "kindle-colorsoft",
        "kindle-scribe-colorsoft",
        "kobo-clara-colour",
        "kobo-libra-colour",
        "pocketbook-era-color",
        "pocketbook-inkpad-color-3",
        "boox-go-color-7",
        "boox-palma-2-pro",
        "tolino-shine-color",
        "tolino-vision-color",
    ] {
        let p = resolve_specs(&[name]).expect("resolves");
        assert_eq!(p.caps.panel, Panel::Color, "{name} is a Kaleido 3 device");
        assert!(p.features.transcode_images, "{name} should tailor images");
    }
    for name in [
        "x4",
        "x3",
        "nomad",
        "kindle",
        "kindle-paperwhite",
        "kindle-scribe",
        "kobo-clara-bw",
        "kobo-elipsa-2e",
        "pocketbook-verse",
        "pocketbook-verse-pro",
        "pocketbook-era",
        "pocketbook-inkpad-4",
        "boox-page",
        "boox-go-7",
        "tolino-shine",
        "tolino-epos-3",
    ] {
        let p = resolve_specs(&[name]).expect("resolves");
        assert!(!p.caps.panel.is_color(), "{name} has a grayscale panel");
    }
}

#[test]
fn a_capable_reader_never_gets_the_crosspoint_downgrades() {
    // These transforms exist to work around CrossPoint firmware defects. On a
    // Kindle (real tables, real monospace, a large CSS subset) or a Kobo-based
    // tolino they would damage the book, so every non-Xteink profile must have
    // them off.
    for name in [
        "nomad",
        "kindle",
        "kindle-paperwhite",
        "kindle-colorsoft",
        "kindle-scribe",
        "kindle-scribe-colorsoft",
        "kobo-clara-bw",
        "kobo-clara-colour",
        "kobo-libra-colour",
        "kobo-elipsa-2e",
        "pocketbook-verse",
        "pocketbook-verse-pro",
        "pocketbook-era",
        "pocketbook-era-color",
        "pocketbook-inkpad-4",
        "pocketbook-inkpad-color-3",
        "boox-page",
        "boox-go-7",
        "boox-go-color-7",
        "boox-palma-2-pro",
        "tolino-shine",
        "tolino-shine-color",
        "tolino-vision-color",
        "tolino-epos-3",
    ] {
        let p = resolve_specs(&[name]).expect("resolves");
        let f = p.features;
        assert!(!f.filter_css, "{name}: CrossPoint's CSS grammar is not its");
        assert!(!f.linearize_tables, "{name} renders real tables");
        assert!(!f.bake_ordered_lists, "{name} numbers its own lists");
        assert!(!f.preserve_code_blocks, "{name} has real <pre> handling");
        assert!(!f.degrade_boxes, "{name} handles figures and asides");
        assert!(
            !f.relocate_anchors,
            "{name} resolves ids on inline elements"
        );
        // What every device profile does want.
        assert!(f.transcode_images, "{name} should fit images to its panel");
        assert!(f.rasterize_svg, "{name} should not gamble on SVG support");
        assert!(
            f.dedupe_ids && f.unicode_hygiene,
            "{name} still gets repair"
        );
        assert_eq!(
            f.remap_colors,
            !p.caps.panel.is_color(),
            "{name}: gray panels remap colors, color panels keep them"
        );
    }
}

#[test]
fn remap_colors_follows_the_panel_on_every_builtin() {
    // Every gray-panel builtin remaps colors to spaced gray tones; every
    // color-panel builtin (and the repair-only `epub` profile, whose
    // permissive caps are a color panel) keeps them.
    for p in builtins() {
        assert_eq!(
            p.features.remap_colors,
            !p.caps.panel.is_color(),
            "{}: remap_colors should follow the panel",
            p.name
        );
    }
}

#[test]
fn a_colour_spelling_resolves_to_the_same_profile_as_color() {
    // Kobo brands them "Colour"; we take either, so nobody has to guess.
    for (official, alias) in [
        ("kobo-clara-colour", "kobo-clara-color"),
        ("kobo-libra-colour", "kobo-libra-color"),
        ("pocketbook-era-color", "pocketbook-era-colour"),
        ("boox-go-color-7", "boox-go-colour-7"),
    ] {
        let by_name = resolve_specs(&[official]).expect("official name resolves");
        let by_alias = resolve_specs(&[alias]).expect("alias resolves");
        assert_eq!(by_alias, by_name, "{alias} should resolve to {official}");
        // The alias is accepted but never listed.
        assert!(
            !builtins().iter().any(|p| p.name == alias),
            "{alias} must not be listed twice"
        );
    }
}

#[test]
fn the_adobe_rmsdk_devices_sanitize_their_css() {
    // Kobo (plain .epub), PocketBook (EPUB2 path) and tolino (RMSDK mode) all
    // run through Adobe RMSDK, which discards a whole stylesheet over one
    // modern value function. They must all have the escape hatch on.
    for name in [
        "kobo-clara-bw",
        "kobo-clara-colour",
        "kobo-libra-colour",
        "kobo-elipsa-2e",
        "pocketbook-verse",
        "pocketbook-era",
        "pocketbook-inkpad-4",
        "tolino-shine",
        "tolino-epos-3",
    ] {
        let f = resolve_specs(&[name]).expect("resolves").features;
        assert!(f.sanitize_css, "{name} renders through Adobe RMSDK");
        assert!(
            !f.filter_css,
            "{name} must not get CrossPoint's CSS grammar"
        );
    }
    // The CrossPoint profiles do not: filter_css has already stripped anything
    // modern, so sanitizing on top would be redundant.
    for name in ["x4", "x3"] {
        let f = resolve_specs(&[name]).expect("resolves").features;
        assert!(!f.sanitize_css, "{name} filters instead");
        assert!(f.filter_css);
    }
}

#[test]
fn a_device_layer_inherits_the_shared_modern_reader_features() {
    // The device JSONs no longer carry a features block; they inherit it. If the
    // layer stack ever stopped composing, every one of them would silently fall
    // back to repair-only and quietly stop tailoring images.
    let p = resolve_specs(&["kobo-clara-bw"]).expect("resolves");
    assert!(p.features.transcode_images, "inherited from the base layer");
    assert!(p.features.rasterize_svg, "inherited from the base layer");
    assert_eq!(p.jpeg_quality, 85, "inherited from the base layer");
    // ...while its own layer still wins where it speaks.
    assert_eq!(p.caps.screen_w, 1072);
    assert_eq!(p.appendix.as_deref(), Some("kobo-clara-bw"));
}

#[test]
fn a_path_layer_overrides_scalars_but_keeps_the_rest() {
    let path = temp_profile("quality.json", r#"{ "options": { "jpeg_quality": 55 } }"#);
    let p = resolve_specs(&["x4", path.to_str().unwrap()]).expect("composition resolves");
    assert_eq!(p.jpeg_quality, 55);
    // Everything the layer is silent about survives from the x4 layer.
    assert_eq!(p.caps, DeviceCaps::x4());
    assert_eq!(p.features, Features::all_on());
    assert_eq!(p.appendix.as_deref(), Some("x4"));
    assert_eq!(p.name, "x4");
}

#[test]
fn features_merge_per_key_not_per_section() {
    let path = temp_profile(
        "nofonts.json",
        r#"{ "features": { "strip_fonts": false } }"#,
    );
    let p = resolve_specs(&["x4", path.to_str().unwrap()]).expect("composition resolves");
    assert!(!p.features.strip_fonts, "layered key wins");
    assert!(p.features.filter_css, "untouched keys survive");
    assert!(p.features.linearize_tables, "untouched keys survive");
}

#[test]
fn filters_concatenate_in_composition_order() {
    let first = temp_profile(
        "first.json",
        r#"{ "filters": [ { "action": "remove", "match": "InventedWatermark.example" } ] }"#,
    );
    let second = temp_profile(
        "second.json",
        r#"{ "filters": [
            { "action": "replace", "match": "colour", "with": "color" },
            { "action": "remove", "match": "SampleAd.example", "in": ["text", "href"] }
        ] }"#,
    );
    let p = resolve_specs(&["x4", first.to_str().unwrap(), second.to_str().unwrap()])
        .expect("composition resolves");
    assert_eq!(p.filters.len(), 3);
    assert_eq!(p.filters[0].action, FilterAction::Remove);
    assert_eq!(p.filters[0].pattern, "InventedWatermark.example");
    assert_eq!(p.filters[1].action, FilterAction::Replace);
    assert_eq!(p.filters[1].with.as_deref(), Some("color"));
    assert_eq!(p.filters[2].pattern, "SampleAd.example");
}

#[test]
fn a_filter_only_profile_starts_from_the_epub_baseline() {
    let path = temp_profile(
        "filter-only.json",
        r#"{ "filters": [ { "action": "remove", "match": "InventedWatermark.example" } ] }"#,
    );
    let p = resolve_specs(&[path.to_str().unwrap()]).expect("filter-only resolves");
    let epub = resolve_specs(&["epub"]).expect("epub resolves");
    assert_eq!(p.features, epub.features, "repair-only baseline");
    assert_eq!(p.caps, epub.caps);
    assert_eq!(p.filters.len(), 1);
}

#[test]
fn appendix_composes_later_wins() {
    let path = temp_profile("appendix.json", r#"{ "output": { "appendix": "mine" } }"#);
    let p = resolve_specs(&["x4", path.to_str().unwrap()]).expect("composition resolves");
    assert_eq!(p.appendix.as_deref(), Some("mine"));
}

#[test]
fn an_unknown_json_key_is_rejected() {
    let path = temp_profile("typo.json", r#"{ "featuers": { "strip_fonts": false } }"#);
    let err = resolve_specs(&[path.to_str().unwrap()]).expect_err("typo key must not parse");
    assert!(
        err.contains("featuers") || err.contains("unknown field"),
        "got: {err}"
    );
}

#[test]
fn a_missing_profile_file_errors_with_the_path() {
    let err = resolve_specs(&["./no-such-profile.json"]).expect_err("missing file");
    assert!(err.contains("no-such-profile.json"), "got: {err}");
}

#[test]
fn to_options_carries_profile_settings_into_convert_options() {
    let p = resolve_specs(&["x4"]).expect("x4 resolves");
    let opts = p.to_options();
    assert_eq!(opts.device, DeviceCaps::x4());
    assert_eq!(opts.features, Features::all_on());
    assert_eq!(opts.jpeg_quality, 82);
    assert_eq!(opts.tables, TableMode::Text);
    assert_eq!(opts.max_chapter_bytes, 200 * 1024);
    assert!(opts.filters.is_empty());
    assert!(!opts.dry_run);
}

#[test]
fn filter_rules_parse_targets_with_text_default() {
    let rule: FilterRule =
        serde_json::from_str(r#"{ "action": "remove", "match": "SampleAd.example" }"#)
            .expect("minimal rule parses");
    assert_eq!(rule.action, FilterAction::Remove);
    assert!(rule.targets_text());
    assert!(!rule.targets_href());
}
