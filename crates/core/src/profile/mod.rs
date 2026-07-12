//! Composable JSON profiles: what a conversion targets and which transforms
//! it runs.
//!
//! A profile bundles device capability numbers ([`DeviceCaps`]), per-transform
//! switches ([`Features`]), tunables (JPEG quality, table mode, chapter split
//! size), an output filename appendix and content filter rules
//! ([`crate::filter::FilterRule`]). Three built-ins ship embedded: `epub`
//! (alias `default`, pure repair), `x4` and `x3` (full device conversions for
//! the Xteink readers running CrossPoint firmware). They live as real JSON
//! files under `crates/core/profiles/` so the file format and the code can
//! never drift apart - tests resolve them through the same parser user
//! profiles go through.
//!
//! [`resolve`] composes any number of profile layers left to right: scalar
//! settings later-wins per leaf, `features` merge per key and `filters`
//! concatenate in order.

pub mod caps;
pub mod features;

pub use caps::DeviceCaps;
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
        }
    }
}

/// Errors from loading or composing profiles.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// The spec is neither a built-in name nor a readable path.
    #[error("unknown profile '{0}' (built-ins: epub, x4, x3; or pass a path to a .json file)")]
    Unknown(String),
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
    gray_levels: Option<u8>,
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
        let raw = load_raw(spec)?;
        apply_layer(&mut profile, raw);
    }
    Ok(profile)
}

/// The three built-in profiles, fully resolved: `epub`, `x4`, `x3`.
pub fn builtins() -> Vec<Profile> {
    ["epub", "x4", "x3"]
        .iter()
        .map(|name| resolve(&[name.to_string()]).expect("built-in profiles must resolve"))
        .collect()
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
fn load_raw(spec: &str) -> Result<RawProfile, ProfileError> {
    let builtin = match spec.to_ascii_lowercase().as_str() {
        "epub" | "default" => Some(EPUB_JSON),
        "x4" => Some(X4_JSON),
        "x3" => Some(X3_JSON),
        _ => None,
    };
    if let Some(json) = builtin {
        return parse_raw(json, spec);
    }
    let looks_like_path =
        spec.contains('/') || spec.contains('\\') || spec.to_ascii_lowercase().ends_with(".json");
    if !looks_like_path {
        return Err(ProfileError::Unknown(spec.to_string()));
    }
    let text = std::fs::read_to_string(spec).map_err(|source| ProfileError::Io {
        path: spec.to_string(),
        source,
    })?;
    parse_raw(&text, spec)
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
        if let Some(gray_levels) = device.gray_levels {
            profile.caps.gray_levels = gray_levels;
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
