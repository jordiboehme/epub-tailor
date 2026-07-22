use serde::{Deserialize, Serialize};

use crate::filter::FilterRule;
use crate::metadata::{ClearField, MergeMode, MetadataDoc};
use crate::profile::{DeviceCaps, Features};

/// User-facing knobs controlling how a conversion is performed.
///
/// Usually built from a resolved [`crate::profile::Profile`] via
/// [`crate::profile::Profile::to_options`]; the `Default` is the x4-equivalent
/// full device conversion, which keeps every test meaning "x4 conversion".
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub device: DeviceCaps,
    pub features: Features,
    pub filters: Vec<FilterRule>,
    pub jpeg_quality: u8,
    pub tables: TableMode,
    pub split_tall_images: bool,
    pub max_chapter_bytes: usize,
    pub split_level: u8,
    pub dry_run: bool,
    /// Metadata the user supplied to fill what the book is missing (from
    /// `--metadata`, the per-field flags, or a looked-up record). Empty by
    /// default: the book's own metadata is authoritative.
    pub metadata: MetadataDoc,
    /// Whether [`Self::metadata`] fills only the gaps (the default) or
    /// overwrites.
    pub metadata_merge: MergeMode,
    /// Fields to remove from the book, applied after [`Self::metadata`] so an
    /// explicit clear always wins. Empty by default.
    pub metadata_clears: Vec<ClearField>,
    /// A cover image to embed, already read from disk by the caller. This crate
    /// never opens a file (nor a socket), so a cover arrives as bytes.
    pub cover_image: Option<CoverImage>,
    /// Provenance stamp written into the output OPF as
    /// `<meta property="tailor:fitted">`, so a later folder scan can tell a
    /// product from a source. `None` writes nothing. Set by `fit`, never by
    /// `md` - a Markdown conversion produces a source, not a fitted book.
    pub output_stamp: Option<String>,
    /// The name of the profile that fitted the book, written next to the
    /// stamp as `<meta property="tailor:profile">`. Only meaningful with
    /// [`Self::output_stamp`]; ignored when the stamp is `None`.
    pub output_profile: Option<String>,
}

/// A cover image handed to [`crate::convert`] as bytes.
#[derive(Debug, Clone)]
pub struct CoverImage {
    /// The encoded image.
    pub data: Vec<u8>,
    /// Its media type, e.g. `image/jpeg`.
    pub media_type: String,
    /// The filename to store it under, e.g. `cover.jpg`.
    pub file_name: String,
}

impl ConvertOptions {
    /// Whether the gray-tone remap runs for these options: the profile enables
    /// it AND the panel is grayscale. The panel guard is here, not in the
    /// profiles, so a hand-written profile claiming `remap_colors` on a color
    /// panel still converts colors to nothing but colors.
    pub(crate) fn remap_active(&self) -> bool {
        self.features.remap_colors && !self.device.panel.is_color()
    }
}

impl Default for ConvertOptions {
    fn default() -> Self {
        ConvertOptions {
            device: DeviceCaps::x4(),
            features: Features::all_on(),
            filters: Vec::new(),
            jpeg_quality: 82,
            tables: TableMode::Text,
            split_tall_images: false,
            max_chapter_bytes: 200 * 1024,
            split_level: 1,
            dry_run: false,
            metadata: MetadataDoc::default(),
            metadata_merge: MergeMode::Fill,
            metadata_clears: Vec::new(),
            cover_image: None,
            output_stamp: None,
            output_profile: None,
        }
    }
}

/// How HTML/Markdown tables are represented in the output (when the profile
/// enables table linearization).
///
/// The CrossPoint firmware has no table rendering support at all, so with the
/// x4/x3 profiles a `<table>` never survives conversion: it is either
/// flattened to paragraphs or rendered to a rasterized image (which the
/// firmware does display).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TableMode {
    /// Flatten every table to "Header: value" style paragraphs.
    Text,
    /// Rasterize complex tables (per the layout heuristic) and linearize the
    /// simple ones. Tables that carry anchor targets, links or images, or that
    /// are too tall, are always linearized so nothing is lost.
    Image,
    /// Rasterize every table, subject to the same safety fallbacks that keep an
    /// anchor-bearing, link-bearing, image-bearing or over-tall table as text.
    ImageAll,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_the_x4_equivalent_conversion() {
        let opts = ConvertOptions::default();
        assert_eq!(opts.device, DeviceCaps::x4());
        assert_eq!(opts.features, Features::all_on());
        assert!(opts.filters.is_empty());
        assert_eq!(opts.jpeg_quality, 82);
        assert_eq!(opts.tables, TableMode::Text);
        assert!(!opts.split_tall_images);
        assert_eq!(opts.max_chapter_bytes, 200 * 1024);
        assert_eq!(opts.split_level, 1);
        assert!(!opts.dry_run);
        assert!(opts.output_stamp.is_none());
    }

    #[test]
    fn remap_is_active_on_gray_panels_only() {
        let opts = ConvertOptions::default();
        assert!(opts.remap_active(), "the x4 default is a gray panel");

        let mut color = ConvertOptions::default();
        color.device.panel = crate::profile::caps::Panel::Color;
        assert!(
            !color.remap_active(),
            "a color panel never remaps, whatever the profile claims"
        );

        let mut off = ConvertOptions::default();
        off.features.remap_colors = false;
        assert!(!off.remap_active());
    }

    #[test]
    fn table_mode_parses_kebab_case_json() {
        let mode: TableMode = serde_json::from_str("\"image-all\"").expect("parses");
        assert_eq!(mode, TableMode::ImageAll);
        let mode: TableMode = serde_json::from_str("\"text\"").expect("parses");
        assert_eq!(mode, TableMode::Text);
    }
}
