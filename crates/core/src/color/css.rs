//! Collect colors from CSS and rewrite them as solved gray tones.
//!
//! Both passes hand-walk the lightningcss AST the way [`crate::css::subset`]
//! and [`crate::css::sanitize`] do (the pinned alpha has no visitor feature):
//! style rules are re-emitted declaration by declaration, `@media` bodies are
//! recursed into, and every other rule passes through verbatim, so
//! `@font-face` and friends survive on sanitize-only profiles. A sheet whose
//! colors all already match their palette grays is reported unchanged
//! (`None`), keeping untouched books byte-stable.

use std::sync::{Arc, RwLock};

use lightningcss::declaration::DeclarationBlock;
use lightningcss::properties::Property;
use lightningcss::properties::custom::{Token, TokenOrValue};
use lightningcss::properties::svg::SVGPaint;
use lightningcss::rules::CssRule;
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::traits::{Parse, ToCss};
use lightningcss::values::color::{CssColor, RGBA};

use crate::css::subset::restore_leading_zeros;
use crate::report::Warning;

use super::palette::{Collected, Palette, Role};
use super::space::{Rgb8, composite_over_white};

/// Visit every rewritable color in one property with its role. Shared by the
/// collect and remap passes so both see the identical color set.
fn for_each_color(property: &mut Property<'_>, f: &mut impl FnMut(&mut CssColor, Role)) {
    match property {
        Property::Color(color) => f(color, Role::Text),
        Property::BackgroundColor(color) => f(color, Role::Background),
        Property::Background(backgrounds) => {
            for background in backgrounds.iter_mut() {
                f(&mut background.color, Role::Background);
            }
        }
        Property::BorderTopColor(color)
        | Property::BorderRightColor(color)
        | Property::BorderBottomColor(color)
        | Property::BorderLeftColor(color) => f(color, Role::Border),
        Property::BorderColor(border_color) => {
            f(&mut border_color.top, Role::Border);
            f(&mut border_color.right, Role::Border);
            f(&mut border_color.bottom, Role::Border);
            f(&mut border_color.left, Role::Border);
        }
        Property::Border(border) => f(&mut border.color, Role::Border),
        Property::Fill(SVGPaint::Color(color)) => f(color, Role::SvgFill),
        Property::Stroke(SVGPaint::Color(color)) => f(color, Role::SvgStroke),
        // `stop-color` is not a property lightningcss knows; it arrives as a
        // custom property whose token list carries parsed color tokens - except
        // named colors, which stay plain ident tokens and are normalized here
        // so both passes see them.
        Property::Custom(custom) if custom.name.as_ref().eq_ignore_ascii_case("stop-color") => {
            for token in custom.value.0.iter_mut() {
                let named = match token {
                    TokenOrValue::Token(Token::Ident(name)) => {
                        CssColor::parse_string(name.as_ref()).ok()
                    }
                    _ => None,
                };
                if let Some(color) = named {
                    *token = TokenOrValue::Color(color);
                }
                if let TokenOrValue::Color(color) = token {
                    f(color, Role::SvgStop);
                }
            }
        }
        _ => {}
    }
}

/// The palette key for a concrete color: its RGB composited over white.
/// `None` for the values the remap leaves alone - `currentColor`,
/// `light-dark()`, system colors, and fully transparent paint (turning
/// transparency opaque would repaint whatever shows through it).
pub(super) fn key_color(color: &CssColor) -> Option<Rgb8> {
    let rgba = match color {
        CssColor::RGBA(rgba) => *rgba,
        CssColor::LAB(..) | CssColor::Predefined(..) | CssColor::Float(..) => {
            match color.to_rgb() {
                Ok(CssColor::RGBA(rgba)) => rgba,
                _ => return None,
            }
        }
        CssColor::CurrentColor | CssColor::LightDark(..) | CssColor::System(_) => return None,
    };
    let alpha = rgba.alpha_f32();
    if alpha == 0.0 {
        return None;
    }
    Some(composite_over_white(rgba.red, rgba.green, rgba.blue, alpha))
}

/// Parse a stylesheet, reporting `path` in the warning on failure.
fn parse_sheet<'i>(
    css: &'i str,
    path: &str,
    warnings: &mut Vec<Warning>,
) -> Option<StyleSheet<'i, 'i>> {
    let parse_warnings = Arc::new(RwLock::new(Vec::new()));
    let options = ParserOptions {
        filename: path.to_string(),
        error_recovery: true,
        warnings: Some(parse_warnings),
        ..Default::default()
    };
    match StyleSheet::parse(css, options) {
        Ok(stylesheet) => Some(stylesheet),
        Err(_) => {
            warnings.push(Warning {
                message: format!("could not parse the CSS in {path}; left its colors unchanged"),
                file: Some(path.to_string()),
            });
            None
        }
    }
}

/// Apply `f` to every property of every style rule, recursing into `@media`.
fn visit_rules(rules: &mut [CssRule<'_>], f: &mut impl FnMut(&mut Property<'_>)) {
    for rule in rules {
        match rule {
            CssRule::Style(style) => {
                for property in style
                    .declarations
                    .declarations
                    .iter_mut()
                    .chain(style.declarations.important_declarations.iter_mut())
                {
                    f(property);
                }
            }
            CssRule::Media(media) => visit_rules(&mut media.rules.0, f),
            _ => {}
        }
    }
}

/// Gather every concrete color (with role) in `css` into `out`. Returns
/// whether the sheet parsed at all (the SVG remap skips a whole SVG whose
/// `<style>` it cannot read, rather than half-remapping it).
pub(crate) fn collect_stylesheet_colors(
    css: &str,
    path: &str,
    out: &mut Collected,
    warnings: &mut Vec<Warning>,
) -> bool {
    let Some(mut stylesheet) = parse_sheet(css, path, warnings) else {
        return false;
    };
    visit_rules(&mut stylesheet.rules.0, &mut |property| {
        for_each_color(property, &mut |color, role| {
            if let Some(rgb) = key_color(color) {
                out.add(rgb, role);
            }
        });
    });
    true
}

/// Replace `color` with its palette gray. Returns whether anything changed.
fn remap_color(color: &mut CssColor, role: Role, palette: &Palette) -> bool {
    let Some(rgb) = key_color(color) else {
        return false;
    };
    let Some(gray) = palette.gray_for(rgb, role) else {
        return false;
    };
    let replacement = CssColor::RGBA(RGBA::new(gray.r, gray.g, gray.b, 1.0));
    if *color == replacement {
        return false;
    }
    *color = replacement;
    true
}

/// Rewrite every palette color in `css`, returning the new sheet text and the
/// number of colors rewritten - or `None` when nothing changed (or the sheet
/// does not parse), so the caller keeps the original bytes.
pub(crate) fn remap_stylesheet_colors(
    css: &str,
    path: &str,
    palette: &Palette,
    warnings: &mut Vec<Warning>,
) -> Option<(String, usize)> {
    let mut stylesheet = parse_sheet(css, path, warnings)?;
    let mut rewritten = 0usize;
    visit_rules(&mut stylesheet.rules.0, &mut |property| {
        for_each_color(property, &mut |color, role| {
            if remap_color(color, role, palette) {
                rewritten += 1;
            }
        });
    });
    if rewritten == 0 {
        return None;
    }

    let mut out = String::new();
    emit_rules(&stylesheet.rules.0, &mut out);
    Some((out, rewritten))
}

/// Serialize a rewritten rule list: the sanitize-pass shape - style rules
/// minified declaration by declaration, `@media` recursed, everything else
/// verbatim.
fn emit_rules(rules: &[CssRule<'_>], out: &mut String) {
    for rule in rules {
        match rule {
            CssRule::Style(style) => {
                let Ok(selectors) = style.selectors.to_css_string(PrinterOptions::default()) else {
                    continue;
                };
                let declarations = emit_declarations(&style.declarations);
                if declarations.is_empty() {
                    continue;
                }
                out.push_str(&selectors);
                out.push('{');
                out.push_str(&declarations);
                out.push('}');
            }
            CssRule::Media(media) => {
                let Ok(query) = media.query.to_css_string(PrinterOptions::default()) else {
                    continue;
                };
                let mut inner = String::new();
                emit_rules(&media.rules.0, &mut inner);
                if inner.is_empty() {
                    continue;
                }
                out.push_str("@media ");
                out.push_str(&query);
                out.push('{');
                out.push_str(&inner);
                out.push('}');
            }
            CssRule::Ignored => {}
            other => {
                if let Ok(text) = other.to_css_string(PrinterOptions::default()) {
                    out.push_str(&text);
                }
            }
        }
    }
}

/// Serialize a declaration block as minified `name:value` pairs, preserving
/// `!important` and restoring stripped leading zeros (the same conventions as
/// the subset and sanitize passes).
fn emit_declarations(block: &DeclarationBlock<'_>) -> String {
    let mut kept: Vec<String> = Vec::new();
    for (property, important) in block
        .declarations
        .iter()
        .map(|p| (p, false))
        .chain(block.important_declarations.iter().map(|p| (p, true)))
    {
        let name = property.property_id().name().to_string();
        let Ok(value) = property.value_to_css_string(PrinterOptions::default()) else {
            continue;
        };
        let value = restore_leading_zeros(&value);
        let suffix = if important { "!important" } else { "" };
        kept.push(format!("{name}:{value}{suffix}"));
    }
    kept.join(";")
}

/// Gather every concrete color in one inline `style=""` value into `out`.
pub(crate) fn collect_inline_style_colors(style: &str, out: &mut Collected) {
    let options = ParserOptions {
        error_recovery: true,
        ..Default::default()
    };
    let Ok(mut block) = DeclarationBlock::parse_string(style, options) else {
        return;
    };
    for property in block
        .declarations
        .iter_mut()
        .chain(block.important_declarations.iter_mut())
    {
        for_each_color(property, &mut |color, role| {
            if let Some(rgb) = key_color(color) {
                out.add(rgb, role);
            }
        });
    }
}

/// Rewrite every palette color in one inline `style=""` value, or `None` when
/// nothing changed.
pub(crate) fn remap_inline_style_colors(style: &str, palette: &Palette) -> Option<(String, usize)> {
    let options = ParserOptions {
        error_recovery: true,
        ..Default::default()
    };
    let mut block = DeclarationBlock::parse_string(style, options).ok()?;
    let mut rewritten = 0usize;
    for property in block
        .declarations
        .iter_mut()
        .chain(block.important_declarations.iter_mut())
    {
        for_each_color(property, &mut |color, role| {
            if remap_color(color, role, palette) {
                rewritten += 1;
            }
        });
    }
    if rewritten == 0 {
        return None;
    }
    Some((emit_declarations(&block), rewritten))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::palette::build_palette;
    use crate::profile::caps::Panel;

    /// Collect from `css`, solve for `panel`, remap, and return the pieces.
    fn remap(css: &str, panel: Panel) -> (Option<(String, usize)>, Palette) {
        let mut collected = Collected::default();
        let mut warnings = Vec::new();
        collect_stylesheet_colors(css, "test.css", &mut collected, &mut warnings);
        let (palette, _) = build_palette(&collected, panel);
        let out = remap_stylesheet_colors(css, "test.css", &palette, &mut warnings);
        (out, palette)
    }

    /// Every color literal in `css` must be a gray `#rrggbb` with r = g = b.
    fn assert_all_grays(css: &str) {
        let mut collected = Collected::default();
        collect_stylesheet_colors(css, "check.css", &mut collected, &mut Vec::new());
        for (rgb, _) in collected.entries() {
            assert!(
                rgb.r == rgb.g && rgb.g == rgb.b,
                "non-gray survived: {rgb:?} in: {css}"
            );
        }
    }

    #[test]
    fn a_colored_sheet_is_rewritten_to_grays() {
        let (out, _) = remap(
            ".a{color:red}.b{color:teal}.c{background-color:#eef;border-color:rgb(10,20,30)}",
            Panel::Gray16,
        );
        let (css, rewritten) = out.expect("colors change");
        // 3 longhand colors + the `border-color` shorthand's 4 sides.
        assert_eq!(rewritten, 7);
        assert_all_grays(&css);
    }

    #[test]
    fn gnarly_sheet_snapshot() {
        let input = r#"
@font-face { font-family: "X"; src: url(x.woff2); }
body { color: green; text-align: justify; }
.note { color: #b22222; margin-left: 2em; }
.tip { background-color: rgba(255, 255, 0, 0.25); }
.lab { color: lab(52% 40 59); }
.imp { color: navy !important; }
@media screen { .s { color: teal; margin: 0.5em; } }
@media print { .p { color: red; } }
.plain { margin: 0.5em; }
"#;
        let (out, _) = remap(input, Panel::Gray16);
        let (css, _) = out.expect("colors change");
        insta::assert_snapshot!(css);
        assert_all_grays(&css);
        assert!(css.contains("@font-face"), "font-face survives: {css}");
        assert!(css.contains("!important"), "importance survives: {css}");
        assert!(css.contains("0.5em"), "leading zeros restored: {css}");
    }

    #[test]
    fn remapping_a_remapped_sheet_changes_nothing() {
        let input = ".a{color:red}.b{color:teal;background-color:#eef}";
        let (out, _) = remap(input, Panel::Gray16);
        let (once, _) = out.expect("first pass rewrites");
        let (again, _) = remap(&once, Panel::Gray16);
        assert_eq!(again, None, "second pass must be a fixed point: {once}");
    }

    #[test]
    fn a_sheet_without_colors_is_left_untouched() {
        let (out, _) = remap(".a{margin:1em;text-align:center}", Panel::Gray16);
        assert_eq!(out, None);
    }

    #[test]
    fn current_color_and_transparent_are_left_alone() {
        let (out, _) = remap(
            ".a{color:currentColor}.b{background-color:transparent}",
            Panel::Gray16,
        );
        assert_eq!(out, None);
    }

    #[test]
    fn translucency_is_composited_over_white_and_emitted_opaque() {
        // 25% black over white is a light gray; the output must be opaque and
        // light, never a dark gray with alpha.
        let (out, _) = remap(".x{background-color:rgba(0,0,0,0.25)}", Panel::Gray16);
        let (css, _) = out.expect("the translucent color is rewritten");
        assert!(!css.contains("rgba"), "output is opaque: {css}");
        let mut collected = Collected::default();
        collect_stylesheet_colors(&css, "check.css", &mut collected, &mut Vec::new());
        let (rgb, _) = collected.entries().next().expect("one color");
        assert!(rgb.r > 150, "25% black over white is light, got {rgb:?}");
    }

    #[test]
    fn roles_are_told_apart_when_collecting() {
        let mut collected = Collected::default();
        collect_stylesheet_colors(
            ".a{color:red;background-color:red;border-color:red;fill:red;stroke:red;stop-color:red}",
            "roles.css",
            &mut collected,
            &mut Vec::new(),
        );
        let red = Rgb8::new(255, 0, 0);
        let roles: Vec<Role> = collected
            .entries()
            .filter(|&(rgb, _)| rgb == red)
            .map(|(_, role)| role)
            .collect();
        for role in [
            Role::Text,
            Role::Background,
            Role::Border,
            Role::SvgFill,
            Role::SvgStroke,
            Role::SvgStop,
        ] {
            assert!(roles.contains(&role), "missing {role:?}, got {roles:?}");
        }
    }

    #[test]
    fn shorthand_border_and_background_colors_are_remapped() {
        // Sanitize-only profiles keep the shorthands whole; their embedded
        // colors must still turn gray.
        let (out, _) = remap(
            ".a{border:1px solid red}.b{background:#336699}",
            Panel::Gray16,
        );
        let (css, rewritten) = out.expect("shorthand colors change");
        assert_eq!(rewritten, 2);
        assert_all_grays(&css);
        assert!(css.contains("1px solid"), "border shape survives: {css}");
    }

    #[test]
    fn inline_styles_are_collected_and_remapped() {
        let mut collected = Collected::default();
        collect_inline_style_colors("color:teal;text-align:center", &mut collected);
        let (palette, _) = build_palette(&collected, Panel::Gray16);
        let (rewritten, count) =
            remap_inline_style_colors("color:teal;text-align:center", &palette)
                .expect("the color changes");
        assert_eq!(count, 1);
        assert!(
            rewritten.contains("text-align:center"),
            "unrelated declarations survive: {rewritten}"
        );
        assert!(!rewritten.contains("teal"), "teal is gone: {rewritten}");
    }

    #[test]
    fn an_inline_style_without_palette_hits_stays_untouched() {
        let palette = Palette::default();
        assert_eq!(remap_inline_style_colors("color:red", &palette), None);
    }
}
