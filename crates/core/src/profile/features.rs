//! The per-transform switches a profile carries.

use serde::{Deserialize, Serialize};

/// Which pipeline transforms a profile enables.
///
/// Every flag maps to exactly one step in `convert()` or
/// `transform_chapter()`; a disabled flag means the corresponding content
/// passes through untouched. Archive repair (META-INF cleanup, OPF/nav/NCX
/// regeneration, strict XHTML re-serialization) is not a feature: it is the
/// tool's unconditional core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Features {
    /// Remove embedded font resources and the `<link>`s that point at them.
    pub strip_fonts: bool,
    /// Filter stylesheets down to the device-supported CSS grammar and caps.
    pub filter_css: bool,
    /// Lift `<head>`/inline `<style>` CSS into an external stylesheet.
    pub relocate_styles: bool,
    /// Transcode, fit and re-encode raster images to the device caps.
    pub transcode_images: bool,
    /// Rasterize SVG resources and inline `<svg>` elements.
    pub rasterize_svg: bool,
    /// Linearize (or rasterize) `<table>` markup the device cannot render.
    pub linearize_tables: bool,
    /// Degrade `<aside>`/`<figure>`/`<dl>` boxes to plain flow content.
    pub degrade_boxes: bool,
    /// Bake `<ol>` numbering into the item text.
    pub bake_ordered_lists: bool,
    /// Rebuild `<pre>`/`<code>` blocks with explicit breaks and spacing.
    pub preserve_code_blocks: bool,
    /// Normalize footnote links and drop `javascript:` hrefs.
    pub normalize_footnotes: bool,
    /// Move anchor ids onto block elements and cap them per chapter.
    pub relocate_anchors: bool,
    /// Remove duplicate element ids (a genuine EPUB spec violation).
    pub dedupe_ids: bool,
    /// NFC-normalize text and strip XML-invalid characters.
    pub unicode_hygiene: bool,
    /// Split chapters over the per-file byte cap at block boundaries.
    pub chapter_split: bool,
}

impl Features {
    /// Every transform on: the full device conversion (x4/x3 profiles).
    pub fn all_on() -> Self {
        Features {
            strip_fonts: true,
            filter_css: true,
            relocate_styles: true,
            transcode_images: true,
            rasterize_svg: true,
            linearize_tables: true,
            degrade_boxes: true,
            bake_ordered_lists: true,
            preserve_code_blocks: true,
            normalize_footnotes: true,
            relocate_anchors: true,
            dedupe_ids: true,
            unicode_hygiene: true,
            chapter_split: true,
        }
    }

    /// Repair-only: everything device-specific off; only transforms that fix
    /// genuine EPUB spec violations stay on (the `epub` profile).
    pub fn repair_only() -> Self {
        Features {
            strip_fonts: false,
            filter_css: false,
            relocate_styles: false,
            transcode_images: false,
            rasterize_svg: false,
            linearize_tables: false,
            degrade_boxes: false,
            bake_ordered_lists: false,
            preserve_code_blocks: false,
            normalize_footnotes: false,
            relocate_anchors: false,
            dedupe_ids: true,
            unicode_hygiene: true,
            chapter_split: false,
        }
    }
}

/// The `features` section of a profile file: every key optional so layers
/// merge per key, unknown keys rejected so typos never pass silently.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawFeatures {
    pub strip_fonts: Option<bool>,
    pub filter_css: Option<bool>,
    pub relocate_styles: Option<bool>,
    pub transcode_images: Option<bool>,
    pub rasterize_svg: Option<bool>,
    pub linearize_tables: Option<bool>,
    pub degrade_boxes: Option<bool>,
    pub bake_ordered_lists: Option<bool>,
    pub preserve_code_blocks: Option<bool>,
    pub normalize_footnotes: Option<bool>,
    pub relocate_anchors: Option<bool>,
    pub dedupe_ids: Option<bool>,
    pub unicode_hygiene: Option<bool>,
    pub chapter_split: Option<bool>,
}

impl RawFeatures {
    /// Merge this layer into `features`, key by key.
    pub(crate) fn apply(&self, features: &mut Features) {
        macro_rules! merge {
            ($($key:ident),* $(,)?) => {
                $(if let Some(value) = self.$key { features.$key = value; })*
            };
        }
        merge!(
            strip_fonts,
            filter_css,
            relocate_styles,
            transcode_images,
            rasterize_svg,
            linearize_tables,
            degrade_boxes,
            bake_ordered_lists,
            preserve_code_blocks,
            normalize_footnotes,
            relocate_anchors,
            dedupe_ids,
            unicode_hygiene,
            chapter_split,
        );
    }
}
