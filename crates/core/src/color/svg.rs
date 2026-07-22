//! Collect and rewrite SVG paint colors, one independent solve per SVG.
//!
//! usvg resolves paint from three sources - presentation attributes (`fill`,
//! `stroke`, `stop-color`, `color`), inline `style=""` attributes, and
//! `<style>` elements (CSS beating presentation attributes) - so all three are
//! remapped with the same palette, making the priority irrelevant. An SVG
//! whose `<style>` cannot be parsed is skipped whole (half-remapped colors
//! would break the stay-distinct guarantee), with a warning.
//!
//! Two tree APIs serve the two places SVGs live:
//!
//! - **Standalone resources** are parsed with roxmltree, which is read-only,
//!   so the rewrite is byte-range splicing: replace exactly the attribute
//!   values and `<style>` text that change, verified against the raw source
//!   slice first (an entity-escaped value fails the check and is skipped with
//!   a warning), leaving every other byte untouched.
//! - **Inline `<svg>` elements** are live kuchikiki nodes and are mutated in
//!   place; [`crate::image::svg::serialize_svg_subtree`] then picks the new
//!   values up for rasterization, or they ship as-is when SVG survives.

use std::ops::Range;

use kuchikiki::NodeRef;
use lightningcss::traits::Parse;
use lightningcss::values::color::CssColor;

use crate::html::dom::{get_attr, is_named, set_attr, text_content};
use crate::profile::caps::Panel;
use crate::report::Warning;

use super::css::{
    collect_inline_style_colors, collect_stylesheet_colors, key_color, remap_inline_style_colors,
    remap_stylesheet_colors,
};
use super::palette::{Collected, Palette, PaletteStats, Role, build_palette};
use super::space::Rgb8;

/// The presentation attributes carrying a single paint color, with the role
/// each maps to. `color` maps to [`Role::Text`] so an attribute value and the
/// identical value in CSS (`Property::Color`) hit the same palette key.
const PAINT_ATTRIBUTES: &[(&str, Role)] = &[
    ("fill", Role::SvgFill),
    ("stroke", Role::SvgStroke),
    ("stop-color", Role::SvgStop),
    ("color", Role::Text),
];

/// The role for a paint attribute name, if it is one.
fn paint_role(name: &str) -> Option<Role> {
    PAINT_ATTRIBUTES
        .iter()
        .find(|(attr, _)| *attr == name)
        .map(|&(_, role)| role)
}

/// Parse one paint attribute value to its palette key. `None` for the values
/// the remap leaves alone: `none`, `url(#...)` (the gradient's stops are
/// remapped instead), `currentColor` (resolves to the remapped `color`),
/// `inherit`, `context-*`, transparent paint, and anything unparseable.
fn paint_key(value: &str) -> Option<Rgb8> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.starts_with("url(") {
        return None;
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "none" | "inherit" | "currentcolor" | "context-fill" | "context-stroke" => return None,
        _ => {}
    }
    key_color(&CssColor::parse_string(trimmed).ok()?)
}

/// The remapped value for one paint attribute, or `None` when it should keep
/// its original spelling (not a concrete color, no palette entry, or already
/// exactly its tone).
fn remap_paint(value: &str, role: Role, palette: &Palette) -> Option<String> {
    let key = paint_key(value)?;
    let gray = palette.gray_for(key, role)?;
    if gray == key {
        return None;
    }
    Some(gray.to_hex())
}

/// What one SVG contributes to its own solve.
struct SvgColors {
    collected: Collected,
    /// A `<style>` element failed to parse: skip the whole SVG.
    style_unreadable: bool,
}

// ---------------------------------------------------------------------------
// Standalone SVG resources (roxmltree + byte splicing)
// ---------------------------------------------------------------------------

/// Collect the paint colors of a standalone SVG string.
fn collect_svg_str(doc: &roxmltree::Document<'_>, path: &str) -> SvgColors {
    let mut out = SvgColors {
        collected: Collected::default(),
        style_unreadable: false,
    };
    let mut scratch = Vec::new();
    for node in doc.descendants().filter(roxmltree::Node::is_element) {
        if node.tag_name().name() == "style" {
            let text = node.text().unwrap_or_default();
            if !collect_stylesheet_colors(text, path, &mut out.collected, &mut scratch) {
                out.style_unreadable = true;
                return out;
            }
            continue;
        }
        for attr in node.attributes() {
            if attr.name() == "style" {
                collect_inline_style_colors(attr.value(), &mut out.collected);
            } else if let Some(role) = paint_role(attr.name())
                && let Some(rgb) = paint_key(attr.value())
            {
                out.collected.add(rgb, role);
            }
        }
    }
    out
}

/// Rewrite a standalone SVG resource with its own per-SVG solve. Returns the
/// spliced source, the solve stats and the number of values rewritten - or
/// `None` when there is nothing to do (no concrete colors, an unparseable
/// document, or an unreadable `<style>`).
pub(crate) fn remap_svg_str(
    svg: &str,
    panel: Panel,
    path: &str,
    warnings: &mut Vec<Warning>,
) -> Option<(String, PaletteStats, usize)> {
    // A parse failure is not warned about here: the rasterizer parses the
    // identical bytes right after and reports it once.
    let doc = roxmltree::Document::parse(svg).ok()?;
    let colors = collect_svg_str(&doc, path);
    if colors.style_unreadable {
        warnings.push(Warning {
            message: format!(
                "could not parse the <style> block in {path}; left the SVG's colors unchanged"
            ),
            file: Some(path.to_string()),
        });
        return None;
    }
    if colors.collected.is_empty() {
        return None;
    }
    let (palette, stats) = build_palette(&colors.collected, panel);

    // Gather splices as (range, replacement), verifying each range still holds
    // the exact parsed text (an entity-escaped value does not).
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut rewritten = 0usize;
    for node in doc.descendants().filter(roxmltree::Node::is_element) {
        if node.tag_name().name() == "style" {
            let Some(text_node) = node.first_child().filter(|c| c.is_text()) else {
                continue;
            };
            let text = text_node.text().unwrap_or_default();
            let range = text_node.range();
            if svg.get(range.clone()) != Some(text) {
                warnings.push(Warning {
                    message: format!("skipped remapping an entity-escaped <style> block in {path}"),
                    file: Some(path.to_string()),
                });
                continue;
            }
            if let Some((remapped, count)) = remap_stylesheet_colors(text, path, &palette, warnings)
            {
                edits.push((range, escape_text(&remapped)));
                rewritten += count;
            }
            continue;
        }
        for attr in node.attributes() {
            let replacement = if attr.name() == "style" {
                remap_inline_style_colors(attr.value(), &palette).map(|(style, count)| {
                    rewritten += count;
                    style
                })
            } else {
                paint_role(attr.name())
                    .and_then(|role| remap_paint(attr.value(), role, &palette))
                    .inspect(|_| rewritten += 1)
            };
            let Some(replacement) = replacement else {
                continue;
            };
            let range = attr.range_value();
            if svg.get(range.clone()) != Some(attr.value()) {
                warnings.push(Warning {
                    message: format!("skipped remapping an entity-escaped attribute in {path}"),
                    file: Some(path.to_string()),
                });
                continue;
            }
            edits.push((range, escape_attr(&replacement)));
        }
    }
    if edits.is_empty() {
        return None;
    }

    // Apply in descending offset order so earlier ranges stay valid.
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.0.start));
    let mut out = svg.to_string();
    for (range, replacement) in edits {
        out.replace_range(range, &replacement);
    }
    Some((out, stats, rewritten))
}

/// Escape a spliced attribute value for either quote style.
fn escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Escape spliced element text.
fn escape_text(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;")
}

// ---------------------------------------------------------------------------
// Inline <svg> elements (kuchikiki, mutated in place)
// ---------------------------------------------------------------------------

/// Rewrite one inline `<svg>` subtree in place with its own per-SVG solve.
/// Returns the solve stats and the number of values rewritten, or `None` when
/// there was nothing to do.
pub(crate) fn remap_inline_svg(
    svg: &NodeRef,
    panel: Panel,
    chapter_path: &str,
    warnings: &mut Vec<Warning>,
) -> Option<(PaletteStats, usize)> {
    let mut collected = Collected::default();
    let mut scratch = Vec::new();
    for node in svg.inclusive_descendants() {
        if is_named(&node, "style") {
            if !collect_stylesheet_colors(
                &text_content(&node),
                chapter_path,
                &mut collected,
                &mut scratch,
            ) {
                warnings.push(Warning {
                    message: format!(
                        "could not parse a <style> block in an inline SVG in {chapter_path}; \
                         left the SVG's colors unchanged"
                    ),
                    file: Some(chapter_path.to_string()),
                });
                return None;
            }
            continue;
        }
        for (name, role) in PAINT_ATTRIBUTES {
            if let Some(value) = get_attr(&node, name)
                && let Some(rgb) = paint_key(&value)
            {
                collected.add(rgb, *role);
            }
        }
        if let Some(style) = get_attr(&node, "style") {
            collect_inline_style_colors(&style, &mut collected);
        }
    }
    if collected.is_empty() {
        return None;
    }

    let (palette, stats) = build_palette(&collected, panel);
    let mut rewritten = 0usize;
    // Snapshot the walk: swapping a <style> element's text mutates the tree,
    // which must not happen under a live descendant iterator.
    for node in svg.inclusive_descendants().collect::<Vec<_>>() {
        if is_named(&node, "style") {
            let text = text_content(&node);
            if let Some((remapped, count)) =
                remap_stylesheet_colors(&text, chapter_path, &palette, warnings)
            {
                for child in node.children().collect::<Vec<_>>() {
                    child.detach();
                }
                node.append(NodeRef::new_text(remapped));
                rewritten += count;
            }
            continue;
        }
        for (name, role) in PAINT_ATTRIBUTES {
            if let Some(value) = get_attr(&node, name)
                && let Some(replacement) = remap_paint(&value, *role, &palette)
            {
                set_attr(&node, name, &replacement);
                rewritten += 1;
            }
        }
        if let Some(style) = get_attr(&node, "style")
            && let Some((remapped, count)) = remap_inline_style_colors(&style, &palette)
        {
            set_attr(&node, "style", &remapped);
            rewritten += count;
        }
    }
    (rewritten > 0).then_some((stats, rewritten))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::solve::JND_L;
    use crate::color::space::lightness_of_gray;
    use crate::html::dom::collect_by_name;
    use crate::html::testutil::doc_from_body;
    use crate::image::svg::{rasterize_svg, serialize_svg_subtree};
    use crate::profile::DeviceCaps;

    fn remap(svg: &str) -> Option<(String, PaletteStats, usize)> {
        let mut warnings = Vec::new();
        remap_svg_str(svg, Panel::Gray16, "images/diagram.svg", &mut warnings)
    }

    /// Extract the value of every `name="..."` attribute in `svg`.
    fn attr_values(svg: &str, name: &str) -> Vec<String> {
        let doc = roxmltree::Document::parse(svg).expect("output parses");
        doc.descendants()
            .filter_map(|n| n.attribute(name))
            .map(str::to_string)
            .collect()
    }

    fn as_gray(value: &str) -> u8 {
        let hex = value.strip_prefix('#').expect("gray hex");
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap();
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap();
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap();
        assert_eq!(r, g, "not a gray: {value}");
        assert_eq!(g, b, "not a gray: {value}");
        r
    }

    #[test]
    fn fills_and_strokes_become_distinct_grays() {
        // Teal and orange have near-equal luminance - the collapse this whole
        // feature exists to prevent.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
            <rect width="40" height="40" fill="#009688" stroke="black"/>
            <circle cx="70" cy="70" r="20" fill="#e67e22"/>
        </svg>"##;
        let (out, stats, rewritten) = remap(svg).expect("colors change");
        assert_eq!(stats.colors_in, 3);
        assert!(rewritten >= 2, "both hues rewritten, got {rewritten}");
        let fills = attr_values(&out, "fill");
        let a = lightness_of_gray(as_gray(&fills[0]));
        let b = lightness_of_gray(as_gray(&fills[1]));
        assert!(
            (a - b).abs() >= JND_L - 1.0,
            "teal at L*{a} and orange at L*{b} must stay apart"
        );
    }

    #[test]
    fn untouched_bytes_stay_byte_identical() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 10\">\n  \
                   <!-- a comment -->\n  <rect width=\"8\" height=\"8\" fill=\"teal\"/>\n</svg>";
        let (out, _, _) = remap(svg).expect("teal changes");
        let fills = attr_values(&out, "fill");
        let expected = svg.replace("teal", &fills[0]);
        assert_eq!(out, expected, "only the paint value may change");
    }

    #[test]
    fn gradient_stops_are_remapped() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><defs>
            <linearGradient id="g">
              <stop offset="0" stop-color="navy"/>
              <stop offset="1" stop-color="gold"/>
            </linearGradient></defs>
            <rect width="10" height="10" fill="url(#g)"/>
        </svg>"#;
        let (out, _, rewritten) = remap(svg).expect("stops change");
        assert_eq!(rewritten, 2);
        for stop in attr_values(&out, "stop-color") {
            as_gray(&stop);
        }
        let fills = attr_values(&out, "fill");
        assert_eq!(fills, vec!["url(#g)"], "the url reference is untouched");
    }

    #[test]
    fn style_attributes_and_style_blocks_are_remapped() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
            <style>.a{fill:red}</style>
            <rect class="a" width="10" height="10"/>
            <circle cx="5" cy="5" r="2" style="fill:red"/>
        </svg>"#;
        let (out, _, rewritten) = remap(svg).expect("colors change");
        assert!(
            rewritten >= 2,
            "style block and style attr, got {rewritten}"
        );
        assert!(!out.contains("red"), "no source color survives: {out}");
        assert!(out.contains(".a{fill:#"), "style block remapped: {out}");
        assert!(out.contains("style=\"fill:#"), "style attr remapped: {out}");
    }

    #[test]
    fn skip_values_are_left_alone() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
            <rect width="10" height="10" fill="none" stroke="currentColor"/>
            <circle cx="5" cy="5" r="2" fill="inherit" stroke="context-stroke"/>
        </svg>"#;
        assert_eq!(remap(svg), None, "nothing concrete to remap");
    }

    #[test]
    fn a_wrapper_svg_is_left_untouched() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="cover.jpg"/></svg>"#;
        assert_eq!(remap(svg), None);
    }

    #[test]
    fn an_unreadable_style_block_skips_the_whole_svg() {
        // lightningcss error-recovers almost anything; unbalanced braces at
        // the top level are one of the few total parse failures.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
            <style>}{ not css at all {{{</style>
            <rect width="10" height="10" fill="red"/>
        </svg>"#;
        let mut warnings = Vec::new();
        let out = remap_svg_str(svg, Panel::Gray16, "images/bad.svg", &mut warnings);
        if let Some((remapped, _, _)) = out {
            // If lightningcss recovered after all, the fill must have been
            // remapped consistently; the invariant is no half-remapping.
            assert!(
                !remapped.contains("\"red\""),
                "consistent remap: {remapped}"
            );
        } else {
            assert!(
                warnings.iter().any(|w| w.message.contains("<style>")),
                "the skip must be explained: {warnings:?}"
            );
        }
    }

    #[test]
    fn the_remapped_svg_renders_with_separated_tones() {
        // End to end through resvg: after remapping, the two near-equal-luma
        // shapes must render as clearly different grays.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" viewBox="0 0 100 50">
            <rect x="0" y="0" width="50" height="50" fill="#009688"/>
            <rect x="50" y="0" width="50" height="50" fill="#e67e22"/>
        </svg>"##;
        let (out, _, _) = remap(svg).expect("colors change");
        let mut warnings = Vec::new();
        let img = rasterize_svg(&out, &DeviceCaps::x4(), &mut warnings, "sep.svg")
            .expect("remapped SVG still renders");
        let gray = img.to_luma8();
        let (w, h) = gray.dimensions();
        let left = gray.get_pixel(w / 4, h / 2).0[0] as i32;
        let right = gray.get_pixel(3 * w / 4, h / 2).0[0] as i32;
        assert!(
            (left - right).abs() >= 20,
            "rendered tones must separate: left {left}, right {right}"
        );
    }

    #[test]
    fn inline_svg_attributes_and_styles_are_remapped_in_place() {
        let doc = doc_from_body(
            r##"<p><svg viewBox="0 0 100 100">
                <style>.s{stroke:teal}</style>
                <rect class="s" width="40" height="40" fill="#009688"/>
                <circle cx="70" cy="70" r="20" style="fill:#e67e22"/>
            </svg></p>"##,
        );
        let svg = collect_by_name(&doc, "svg")
            .into_iter()
            .next()
            .expect("svg");
        let mut warnings = Vec::new();
        let (stats, rewritten) = remap_inline_svg(&svg, Panel::Gray16, "ch.xhtml", &mut warnings)
            .expect("colors change");
        assert!(stats.colors_in >= 3, "teal fill, teal stroke, orange fill");
        assert!(rewritten >= 3, "got {rewritten}");
        let serialized = serialize_svg_subtree(&svg);
        assert!(!serialized.contains("teal"), "{serialized}");
        assert!(!serialized.contains("#009688"), "{serialized}");
        assert!(!serialized.contains("#e67e22"), "{serialized}");
        assert!(
            roxmltree::Document::parse(&serialized).is_ok(),
            "still valid SVG: {serialized}"
        );
    }

    #[test]
    fn an_inline_svg_without_colors_reports_nothing() {
        let doc =
            doc_from_body(r#"<p><svg viewBox="0 0 10 10"><rect width="8" height="8"/></svg></p>"#);
        let svg = collect_by_name(&doc, "svg")
            .into_iter()
            .next()
            .expect("svg");
        let mut warnings = Vec::new();
        assert_eq!(
            remap_inline_svg(&svg, Panel::Gray16, "ch.xhtml", &mut warnings),
            None
        );
    }

    #[test]
    fn gray4_collapses_and_reports_via_stats() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
            <rect width="10" height="10" fill="#282828"/>
            <rect width="10" height="10" fill="#505050"/>
            <rect width="10" height="10" fill="#787878"/>
            <rect width="10" height="10" fill="#a0a0a0"/>
            <rect width="10" height="10" fill="#c8c8c8"/>
            <rect width="10" height="10" fill="#f0f0f0"/>
        </svg>"##;
        let mut warnings = Vec::new();
        let (out, stats, _) = remap_svg_str(svg, Panel::Gray4, "images/crowd.svg", &mut warnings)
            .expect("levels shift");
        assert!(stats.tones_out <= 4);
        assert!(stats.collapsed > 0, "the collapse is reported in stats");
        for fill in attr_values(&out, "fill") {
            let value = as_gray(&fill);
            assert!(
                crate::color::solve::panel_levels(4).contains(&value),
                "{fill} is not a gray4 level"
            );
        }
    }
}
