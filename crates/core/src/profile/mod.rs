//! Composable JSON profiles: what a conversion targets and which transforms
//! it runs.
//!
//! A profile bundles device capability numbers ([`DeviceCaps`]), per-transform
//! switches ([`Features`]), tunables (JPEG quality, table mode, chapter split
//! size), an output filename appendix and content filter rules
//! ([`crate::filter::FilterRule`]).
//!
//! The built-ins ship embedded: `epub` (alias `default`, pure repair),
//! `x4`/`x3` (full device conversions for the Xteink readers running CrossPoint
//! firmware), and one per researched device - `nomad`, the `kindle-*` family
//! and the `tolino-*` family. They live as real JSON files under
//! `crates/core/profiles/` so the file format and the code can never drift
//! apart - tests resolve them through the same parser user profiles go through.
//!
//! The CrossPoint transforms are a poor fit for a capable reader: `filter_css`
//! filters down to CrossPoint's ~12-property grammar, and `linearize_tables`,
//! `bake_ordered_lists` and `preserve_code_blocks` work around defects a Kindle
//! or a Kobo-based reader does not have. The newer device profiles therefore
//! switch those off and keep what genuinely helps: repair, image fitting to the
//! panel and SVG rasterization. See `docs/device-constraints.md` for the
//! per-device evidence.
//!
//! [`resolve`] composes any number of profile layers left to right: scalar
//! settings later-wins per leaf, `features` merge per key and `filters`
//! concatenate in order.

pub mod caps;
pub mod features;

pub use caps::{DeviceCaps, Panel};
pub use features::Features;

use serde::{Deserialize, Serialize};

use crate::filter::FilterRule;
use crate::options::{ConvertOptions, TableMode};
use features::RawFeatures;

/// The output filename appendix used when no composed profile defines one:
/// `book.epub` becomes `book.tailored.epub`.
pub const DEFAULT_APPENDIX: &str = "tailored";

const EPUB_JSON: &str = include_str!("../../profiles/epub.json");
const X4_JSON: &str = include_str!("../../profiles/x4.json");
const X3_JSON: &str = include_str!("../../profiles/x3.json");

/// The shared feature block every capable modern reader wants. Not a built-in
/// name of its own: on its own it carries no screen geometry, so
/// `transcode_images` would re-encode every image without resizing it - lossy
/// for no gain. It only ever appears underneath a device layer.
const MODERN_READER_JSON: &str = include_str!("../../profiles/_modern-reader.json");

const NOMAD_JSON: &str = include_str!("../../profiles/nomad.json");
const KINDLE_JSON: &str = include_str!("../../profiles/kindle.json");
const KINDLE_PAPERWHITE_JSON: &str = include_str!("../../profiles/kindle-paperwhite.json");
const KINDLE_COLORSOFT_JSON: &str = include_str!("../../profiles/kindle-colorsoft.json");
const KINDLE_SCRIBE_JSON: &str = include_str!("../../profiles/kindle-scribe.json");
const KINDLE_SCRIBE_COLORSOFT_JSON: &str =
    include_str!("../../profiles/kindle-scribe-colorsoft.json");
const TOLINO_SHINE_JSON: &str = include_str!("../../profiles/tolino-shine.json");
const TOLINO_SHINE_COLOR_JSON: &str = include_str!("../../profiles/tolino-shine-color.json");
const TOLINO_VISION_COLOR_JSON: &str = include_str!("../../profiles/tolino-vision-color.json");
const TOLINO_EPOS_3_JSON: &str = include_str!("../../profiles/tolino-epos-3.json");
const KOBO_CLARA_BW_JSON: &str = include_str!("../../profiles/kobo-clara-bw.json");
const KOBO_CLARA_COLOUR_JSON: &str = include_str!("../../profiles/kobo-clara-colour.json");
const KOBO_LIBRA_COLOUR_JSON: &str = include_str!("../../profiles/kobo-libra-colour.json");
const KOBO_ELIPSA_2E_JSON: &str = include_str!("../../profiles/kobo-elipsa-2e.json");
const POCKETBOOK_VERSE_JSON: &str = include_str!("../../profiles/pocketbook-verse.json");
const POCKETBOOK_VERSE_PRO_JSON: &str = include_str!("../../profiles/pocketbook-verse-pro.json");
const POCKETBOOK_ERA_JSON: &str = include_str!("../../profiles/pocketbook-era.json");
const POCKETBOOK_ERA_COLOR_JSON: &str = include_str!("../../profiles/pocketbook-era-color.json");
const POCKETBOOK_INKPAD_4_JSON: &str = include_str!("../../profiles/pocketbook-inkpad-4.json");
const POCKETBOOK_INKPAD_COLOR_3_JSON: &str =
    include_str!("../../profiles/pocketbook-inkpad-color-3.json");
const BOOX_PAGE_JSON: &str = include_str!("../../profiles/boox-page.json");
const BOOX_GO_7_JSON: &str = include_str!("../../profiles/boox-go-7.json");
const BOOX_GO_COLOR_7_JSON: &str = include_str!("../../profiles/boox-go-color-7.json");
const BOOX_PALMA_2_PRO_JSON: &str = include_str!("../../profiles/boox-palma-2-pro.json");

/// One built-in profile: the name it lists under, any alternative spellings
/// that resolve to it, and the layer stack it composes from.
struct Builtin {
    name: &'static str,
    /// Alternative spellings, accepted but not listed. Kobo brands its color
    /// devices "Colour"; we take either.
    aliases: &'static [&'static str],
    /// Composed left to right, exactly like user-supplied `--profile` layers.
    layers: &'static [&'static str],
}

/// Every built-in profile, in listing order: the device-neutral baseline first,
/// then the CrossPoint readers, then one entry per device whose firmware
/// behavior we have researched (see `research/`).
const BUILTINS: &[Builtin] = &[
    Builtin {
        name: "epub",
        aliases: &["default"],
        layers: &[EPUB_JSON],
    },
    // The Xteink readers are their own world: a microcontroller-class renderer
    // that needs every downgrade the pipeline has. They do not share the
    // modern-reader base.
    Builtin {
        name: "x4",
        aliases: &[],
        layers: &[X4_JSON],
    },
    Builtin {
        name: "x3",
        aliases: &[],
        layers: &[X3_JSON],
    },
    Builtin {
        name: "nomad",
        aliases: &["supernote-nomad"],
        layers: &[MODERN_READER_JSON, NOMAD_JSON],
    },
    Builtin {
        name: "kindle",
        aliases: &[],
        layers: &[MODERN_READER_JSON, KINDLE_JSON],
    },
    Builtin {
        name: "kindle-paperwhite",
        aliases: &[],
        layers: &[MODERN_READER_JSON, KINDLE_PAPERWHITE_JSON],
    },
    Builtin {
        name: "kindle-colorsoft",
        aliases: &[],
        layers: &[MODERN_READER_JSON, KINDLE_COLORSOFT_JSON],
    },
    Builtin {
        name: "kindle-scribe",
        aliases: &[],
        layers: &[MODERN_READER_JSON, KINDLE_SCRIBE_JSON],
    },
    Builtin {
        name: "kindle-scribe-colorsoft",
        aliases: &[],
        layers: &[MODERN_READER_JSON, KINDLE_SCRIBE_COLORSOFT_JSON],
    },
    Builtin {
        name: "kobo-clara-bw",
        aliases: &[],
        layers: &[MODERN_READER_JSON, KOBO_CLARA_BW_JSON],
    },
    Builtin {
        name: "kobo-clara-colour",
        aliases: &["kobo-clara-color"],
        layers: &[MODERN_READER_JSON, KOBO_CLARA_COLOUR_JSON],
    },
    Builtin {
        name: "kobo-libra-colour",
        aliases: &["kobo-libra-color"],
        layers: &[MODERN_READER_JSON, KOBO_LIBRA_COLOUR_JSON],
    },
    Builtin {
        name: "kobo-elipsa-2e",
        aliases: &[],
        layers: &[MODERN_READER_JSON, KOBO_ELIPSA_2E_JSON],
    },
    Builtin {
        name: "pocketbook-verse",
        aliases: &[],
        layers: &[MODERN_READER_JSON, POCKETBOOK_VERSE_JSON],
    },
    Builtin {
        name: "pocketbook-verse-pro",
        aliases: &[],
        layers: &[MODERN_READER_JSON, POCKETBOOK_VERSE_PRO_JSON],
    },
    Builtin {
        name: "pocketbook-era",
        aliases: &[],
        layers: &[MODERN_READER_JSON, POCKETBOOK_ERA_JSON],
    },
    Builtin {
        name: "pocketbook-era-color",
        aliases: &["pocketbook-era-colour"],
        layers: &[MODERN_READER_JSON, POCKETBOOK_ERA_COLOR_JSON],
    },
    Builtin {
        name: "pocketbook-inkpad-4",
        aliases: &[],
        layers: &[MODERN_READER_JSON, POCKETBOOK_INKPAD_4_JSON],
    },
    Builtin {
        name: "pocketbook-inkpad-color-3",
        aliases: &["pocketbook-inkpad-colour-3"],
        layers: &[MODERN_READER_JSON, POCKETBOOK_INKPAD_COLOR_3_JSON],
    },
    Builtin {
        name: "boox-page",
        aliases: &[],
        layers: &[MODERN_READER_JSON, BOOX_PAGE_JSON],
    },
    Builtin {
        name: "boox-go-7",
        aliases: &[],
        layers: &[MODERN_READER_JSON, BOOX_GO_7_JSON],
    },
    Builtin {
        name: "boox-go-color-7",
        aliases: &["boox-go-colour-7"],
        layers: &[MODERN_READER_JSON, BOOX_GO_COLOR_7_JSON],
    },
    Builtin {
        name: "boox-palma-2-pro",
        aliases: &[],
        layers: &[MODERN_READER_JSON, BOOX_PALMA_2_PRO_JSON],
    },
    Builtin {
        name: "tolino-shine",
        aliases: &[],
        layers: &[MODERN_READER_JSON, TOLINO_SHINE_JSON],
    },
    Builtin {
        name: "tolino-shine-color",
        aliases: &["tolino-shine-colour"],
        layers: &[MODERN_READER_JSON, TOLINO_SHINE_COLOR_JSON],
    },
    Builtin {
        name: "tolino-vision-color",
        aliases: &["tolino-vision-colour"],
        layers: &[MODERN_READER_JSON, TOLINO_VISION_COLOR_JSON],
    },
    Builtin {
        name: "tolino-epos-3",
        aliases: &[],
        layers: &[MODERN_READER_JSON, TOLINO_EPOS_3_JSON],
    },
];

impl Builtin {
    /// Whether `name` (already lowercased) selects this profile.
    fn matches(&self, name: &str) -> bool {
        self.name == name || self.aliases.contains(&name)
    }
}

/// A fully resolved profile: the composition of one or more layers over the
/// repair-only baseline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Profile {
    pub name: String,
    pub description: String,
    pub caps: DeviceCaps,
    pub features: Features,
    pub jpeg_quality: u8,
    pub tables: TableMode,
    pub split_tall_images: bool,
    pub max_chapter_bytes: usize,
    /// Output filename appendix (`book.<appendix>.epub`); `None` falls back
    /// to [`DEFAULT_APPENDIX`].
    pub appendix: Option<String>,
    pub filters: Vec<FilterRule>,
}

impl Profile {
    /// The output filename appendix, falling back to [`DEFAULT_APPENDIX`].
    pub fn appendix_or_default(&self) -> &str {
        self.appendix.as_deref().unwrap_or(DEFAULT_APPENDIX)
    }

    /// Translate this profile into the options `convert` consumes.
    ///
    /// Metadata is deliberately absent: a profile describes a *device* and is
    /// meant to be reused across every book you own, while metadata belongs to
    /// exactly one book. The caller layers that on afterwards.
    pub fn to_options(&self) -> ConvertOptions {
        ConvertOptions {
            device: self.caps,
            features: self.features,
            filters: self.filters.clone(),
            jpeg_quality: self.jpeg_quality,
            tables: self.tables,
            split_tall_images: self.split_tall_images,
            max_chapter_bytes: self.max_chapter_bytes,
            split_level: 1,
            dry_run: false,
            ..ConvertOptions::default()
        }
    }
}

/// Errors from loading or composing profiles.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// The spec is neither a built-in name nor a readable path.
    #[error("unknown profile '{spec}' (built-ins: {}; or pass a path to a .json file)", builtin_names().join(", "))]
    Unknown { spec: String },
    /// A profile file could not be read from disk.
    #[error("cannot read profile {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// A profile file is not valid profile JSON.
    #[error("invalid profile {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

/// One profile layer as it appears on disk: every leaf optional, unknown keys
/// rejected so a typo never silently does nothing.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfile {
    name: Option<String>,
    description: Option<String>,
    device: Option<RawDevice>,
    features: Option<RawFeatures>,
    options: Option<RawOptions>,
    output: Option<RawOutput>,
    filters: Option<Vec<FilterRule>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDevice {
    screen: Option<RawScreen>,
    panel: Option<Panel>,
    images: Option<RawImages>,
    css: Option<RawCss>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScreen {
    width: Option<u32>,
    height: Option<u32>,
    ppi: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawImages {
    max_source_px: Option<(u32, u32)>,
    inline_max: Option<(u32, u32)>,
    cover_max: Option<(u32, u32)>,
    inline_budget_kb: Option<usize>,
    cover_budget_kb: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCss {
    max_file_kb: Option<usize>,
    max_rules: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOptions {
    jpeg_quality: Option<u8>,
    tables: Option<TableMode>,
    split_tall_images: Option<bool>,
    max_chapter_kb: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOutput {
    appendix: Option<String>,
}

/// Resolve a list of profile specs (built-in names or file paths) into one
/// composed [`Profile`], layered left to right over the repair-only baseline.
/// An empty list yields the built-in `epub` profile.
///
/// # Errors
/// Returns a [`ProfileError`] for an unknown name, an unreadable file or
/// invalid profile JSON.
pub fn resolve(specs: &[String]) -> Result<Profile, ProfileError> {
    let mut profile = base_profile();
    for spec in specs {
        for raw in load_layers(spec)? {
            apply_layer(&mut profile, raw);
        }
    }
    Ok(profile)
}

/// Every built-in profile, fully resolved, in listing order.
pub fn builtins() -> Vec<Profile> {
    BUILTINS
        .iter()
        .map(|builtin| {
            resolve(&[builtin.name.to_string()]).expect("built-in profiles must resolve")
        })
        .collect()
}

/// The built-in profile names, in listing order. Aliases are accepted by
/// [`resolve`] but not listed.
pub fn builtin_names() -> Vec<&'static str> {
    BUILTINS.iter().map(|builtin| builtin.name).collect()
}

/// The repair-only baseline every resolution starts from: the built-in `epub`
/// profile applied over neutral defaults.
fn base_profile() -> Profile {
    let mut profile = Profile {
        name: "epub".to_string(),
        description: String::new(),
        caps: DeviceCaps::permissive(),
        features: Features::repair_only(),
        jpeg_quality: 82,
        tables: TableMode::Text,
        split_tall_images: false,
        max_chapter_bytes: 200 * 1024,
        appendix: None,
        filters: Vec::new(),
    };
    let raw = parse_raw(EPUB_JSON, "built-in epub").expect("built-in epub profile must parse");
    apply_layer(&mut profile, raw);
    profile
}

/// Load one layer: a built-in name (case-insensitive) or a path to a JSON
/// file. Anything containing a path separator or ending in `.json` is treated
/// as a path.
fn load_layers(spec: &str) -> Result<Vec<RawProfile>, ProfileError> {
    let name = spec.to_ascii_lowercase();
    if let Some(builtin) = BUILTINS.iter().find(|builtin| builtin.matches(&name)) {
        return builtin
            .layers
            .iter()
            .map(|json| parse_raw(json, spec))
            .collect();
    }
    let looks_like_path =
        spec.contains('/') || spec.contains('\\') || spec.to_ascii_lowercase().ends_with(".json");
    if !looks_like_path {
        return Err(ProfileError::Unknown {
            spec: spec.to_string(),
        });
    }
    let text = std::fs::read_to_string(spec).map_err(|source| ProfileError::Io {
        path: spec.to_string(),
        source,
    })?;
    Ok(vec![parse_raw(&text, spec)?])
}

fn parse_raw(text: &str, origin: &str) -> Result<RawProfile, ProfileError> {
    serde_json::from_str(text).map_err(|source| ProfileError::Parse {
        path: origin.to_string(),
        source,
    })
}

/// Merge one raw layer into the resolved profile: scalars later-wins per
/// leaf, features per key, filters concatenated.
fn apply_layer(profile: &mut Profile, raw: RawProfile) {
    if let Some(name) = raw.name {
        profile.name = name;
    }
    if let Some(description) = raw.description {
        profile.description = description;
    }
    if let Some(device) = raw.device {
        if let Some(screen) = device.screen {
            if let Some(width) = screen.width {
                profile.caps.screen_w = width;
            }
            if let Some(height) = screen.height {
                profile.caps.screen_h = height;
            }
            if let Some(ppi) = screen.ppi {
                profile.caps.ppi = ppi;
            }
        }
        if let Some(panel) = device.panel {
            profile.caps.panel = panel;
        }
        if let Some(images) = device.images {
            if let Some(max_source_px) = images.max_source_px {
                profile.caps.max_src_px = max_source_px;
            }
            if let Some(inline_max) = images.inline_max {
                profile.caps.inline_max = inline_max;
            }
            if let Some(cover_max) = images.cover_max {
                profile.caps.cover_max = cover_max;
            }
            if let Some(kb) = images.inline_budget_kb {
                profile.caps.inline_budget_bytes = kb * 1024;
            }
            if let Some(kb) = images.cover_budget_kb {
                profile.caps.cover_budget_bytes = kb * 1024;
            }
        }
        if let Some(css) = device.css {
            if let Some(kb) = css.max_file_kb {
                profile.caps.css_max_bytes = kb * 1024;
            }
            if let Some(max_rules) = css.max_rules {
                profile.caps.css_max_rules = max_rules;
            }
        }
    }
    if let Some(features) = raw.features {
        features.apply(&mut profile.features);
    }
    if let Some(options) = raw.options {
        if let Some(jpeg_quality) = options.jpeg_quality {
            profile.jpeg_quality = jpeg_quality;
        }
        if let Some(tables) = options.tables {
            profile.tables = tables;
        }
        if let Some(split_tall_images) = options.split_tall_images {
            profile.split_tall_images = split_tall_images;
        }
        if let Some(kb) = options.max_chapter_kb {
            profile.max_chapter_bytes = kb * 1024;
        }
    }
    if let Some(output) = raw.output
        && let Some(appendix) = output.appendix
    {
        profile.appendix = Some(appendix);
    }
    if let Some(filters) = raw.filters {
        profile.filters.extend(filters);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The baseline (what an empty spec list resolves to) and an explicit
    /// `epub` layer must be the same profile, or layering `epub` on top of
    /// something else would behave differently from starting fresh.
    #[test]
    fn epub_layer_is_idempotent_over_the_baseline() {
        let base = resolve(&[]).expect("baseline resolves");
        let once = resolve(&["epub".to_string()]).expect("epub resolves");
        assert_eq!(base, once);
    }

    #[test]
    fn builtin_names_are_case_insensitive() {
        let p = resolve(&["X4".to_string()]).expect("X4 resolves");
        assert_eq!(p.name, "x4");
        assert_eq!(p.caps, DeviceCaps::x4());
    }
}
