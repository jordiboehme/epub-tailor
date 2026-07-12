//! Make a stylesheet survive Adobe RMSDK.
//!
//! RMSDK - the Adobe Reader Mobile SDK - is the engine behind a plain `.epub`
//! on a Kobo, the EPUB2 path on a PocketBook (their manual carries the
//! "Contains Reader(R) Mobile technology by Adobe Systems Incorporated"
//! attribution) and tolino's opt-in RMSDK mode. Its CSS parser is frozen around
//! 2013 and, crucially, has *no fault tolerance*: one construct it cannot parse
//! and it discards the **entire stylesheet** - on some firmware refusing to open
//! the book at all, which the reader sees as a corrupt file with no explanation.
//!
//! So this is not a subset filter. Unlike [`super::subset`], which reduces a
//! stylesheet to the dozen properties the CrossPoint firmware understands, this
//! pass keeps everything and removes only the few constructs that detonate:
//!
//! - **Declarations using a modern value function** - `calc()`, `var()`,
//!   `clamp()`, `min()`, `max()`, `env()`. None of them existed when RMSDK's
//!   parser was written, and none of them do anything on an e-ink reader
//!   anyway, so dropping the declaration costs nothing and saves the sheet.
//! - **`@supports`** - a feature-query block RMSDK predates entirely.
//! - **Media queries using range syntax** (`(400px <= width)`), which is far
//!   newer than the parser.
//!
//! Everything else - `@font-face`, `@media`, `@import`, keyframes, every
//! ordinary declaration - passes through untouched. The transform is a fixed
//! point: `sanitize_css(sanitize_css(x)) == sanitize_css(x)`.
//!
//! Sourcing note: this failure mode is community-established (the `kobofix`
//! project; the write-up "Your EPUB Is Fine. Kobo Disagrees. Blame Adobe.")
//! rather than documented by Adobe or Kobo, whose publisher spec is silent on
//! it. It is cheap and non-destructive, and the failure it prevents - a book
//! that will not open - is the worst outcome this tool has.

use std::sync::{Arc, RwLock};

use lightningcss::declaration::DeclarationBlock;
use lightningcss::rules::CssRule;
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::traits::ToCss;

use crate::report::Warning;

/// Value functions RMSDK's parser cannot read. Matched as `name(` against the
/// serialized value, so the `max-width` *property* is never confused with the
/// `max()` *function*.
const MODERN_FUNCTIONS: &[&str] = &["calc(", "var(", "clamp(", "min(", "max(", "env("];

/// The result of sanitizing one stylesheet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SanitizedCss {
    /// The stylesheet with the RMSDK-hostile constructs removed.
    pub css: String,
    /// Declarations dropped for using a modern value function.
    pub decls_dropped: usize,
    /// At-rules dropped whole (`@supports`, range-syntax `@media`).
    pub rules_dropped: usize,
}

/// Whether a serialized declaration value uses a function no e-ink CSS parser
/// of this generation can read. A vendor prefix does not save it:
/// `-webkit-calc(` contains `calc(`.
///
/// Shared with [`super::subset`]: CrossPoint's parser is even more primitive
/// than RMSDK's, so a `calc()` is no more use to it than to Adobe. Neither pass
/// may emit one.
pub(crate) fn uses_modern_function(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    MODERN_FUNCTIONS.iter().any(|f| lower.contains(f))
}

/// Strip the declarations RMSDK would choke on, returning the surviving block
/// and how many were dropped.
fn sanitize_declarations(block: &DeclarationBlock<'_>) -> (String, usize) {
    let mut kept: Vec<String> = Vec::new();
    let mut dropped = 0usize;
    for (property, important) in block
        .declarations
        .iter()
        .map(|p| (p, false))
        .chain(block.important_declarations.iter().map(|p| (p, true)))
    {
        let name = property.property_id().name().to_string();
        // A custom property is dead weight the moment `var()` is gone - nothing
        // left in the sheet can read it - and it is syntax RMSDK's parser has
        // never seen. Drop the definition along with its uses.
        if name.starts_with("--") {
            dropped += 1;
            continue;
        }
        let Ok(value) = property.value_to_css_string(PrinterOptions::default()) else {
            dropped += 1;
            continue;
        };
        if uses_modern_function(&value) {
            dropped += 1;
            continue;
        }
        // lightningcss serializes `0.5em` as `.5em`; an old parser may reject a
        // number with no integer part, so put the zero back (same reasoning as
        // the CrossPoint filter, and the same helper).
        let value = super::subset::restore_leading_zeros(&value);
        let suffix = if important { "!important" } else { "" };
        kept.push(format!("{name}:{value}{suffix}"));
    }
    (kept.join(";"), dropped)
}

/// Walk `rules`, appending the sanitized form of each to `out`.
fn sanitize_rules(
    rules: &[CssRule<'_>],
    out: &mut String,
    decls_dropped: &mut usize,
    rules_dropped: &mut usize,
) {
    for rule in rules {
        match rule {
            CssRule::Style(style) => {
                let Ok(selectors) = style.selectors.to_css_string(PrinterOptions::default()) else {
                    *rules_dropped += 1;
                    continue;
                };
                let (declarations, dropped) = sanitize_declarations(&style.declarations);
                *decls_dropped += dropped;
                // A rule whose every declaration was modern is now empty. Drop
                // the husk rather than emit `selector{}`.
                if declarations.is_empty() {
                    continue;
                }
                out.push_str(&selectors);
                out.push('{');
                out.push_str(&declarations);
                out.push('}');
                // Nested rules are CSS Nesting, which RMSDK predates; they are
                // dropped with the rest of what it cannot parse.
            }
            // A feature query is newer than the parser: drop the whole block.
            CssRule::Supports(_) => *rules_dropped += 1,
            CssRule::Media(media) => {
                let Ok(query) = media.query.to_css_string(PrinterOptions::default()) else {
                    *rules_dropped += 1;
                    continue;
                };
                // Range syntax - `(400px <= width <= 700px)` - is far newer than
                // the parser. The legacy `(min-width: 400px)` form is fine.
                if query.contains('<') || query.contains('>') {
                    *rules_dropped += 1;
                    continue;
                }
                let mut inner = String::new();
                sanitize_rules(&media.rules.0, &mut inner, decls_dropped, rules_dropped);
                if inner.is_empty() {
                    continue;
                }
                out.push_str("@media ");
                out.push_str(&query);
                out.push('{');
                out.push_str(&inner);
                out.push('}');
            }
            // Everything else - @font-face, @import, keyframes, @page - is
            // either understood by RMSDK or harmlessly ignored by it. Passing it
            // through verbatim is the whole point: this is not a subset filter.
            other => match other.to_css_string(PrinterOptions::default()) {
                Ok(text) => out.push_str(&text),
                Err(_) => *rules_dropped += 1,
            },
        }
    }
}

/// Remove the constructs that make Adobe RMSDK discard a whole stylesheet,
/// naming the source `path` in any warnings. Never fails: an unparseable
/// stylesheet is left to the caller as an empty result plus a warning.
pub fn sanitize_css(css: &str, path: &str, warnings: &mut Vec<Warning>) -> SanitizedCss {
    let parse_warnings = Arc::new(RwLock::new(Vec::new()));
    let options = ParserOptions {
        filename: path.to_string(),
        error_recovery: true,
        warnings: Some(Arc::clone(&parse_warnings)),
        ..Default::default()
    };

    let stylesheet = match StyleSheet::parse(css, options) {
        Ok(stylesheet) => stylesheet,
        Err(_) => {
            warnings.push(Warning {
                message: format!("could not parse the CSS in {path}; dropped it"),
                file: Some(path.to_string()),
            });
            return SanitizedCss::default();
        }
    };

    let mut out = String::new();
    let (mut decls_dropped, mut rules_dropped) = (0usize, 0usize);
    sanitize_rules(
        &stylesheet.rules.0,
        &mut out,
        &mut decls_dropped,
        &mut rules_dropped,
    );

    if decls_dropped > 0 || rules_dropped > 0 {
        warnings.push(Warning {
            message: format!(
                "removed {decls_dropped} declaration(s) and {rules_dropped} rule(s) from {path} \
                 that Adobe RMSDK cannot parse (it would have discarded the whole stylesheet)"
            ),
            file: Some(path.to_string()),
        });
    }

    SanitizedCss {
        css: out,
        decls_dropped,
        rules_dropped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(css: &str) -> SanitizedCss {
        let mut warnings = Vec::new();
        sanitize_css(css, "test.css", &mut warnings)
    }

    #[test]
    fn a_modern_function_loses_its_declaration_and_nothing_else() {
        let out = run("p{color:#333;width:calc(100% - 2em);margin:1em}");
        assert!(!out.css.contains("calc"), "calc must go: {}", out.css);
        assert!(out.css.contains("color"), "siblings survive: {}", out.css);
        assert!(out.css.contains("margin"), "siblings survive: {}", out.css);
        assert_eq!(out.decls_dropped, 1);
    }

    #[test]
    fn no_modern_function_survives_in_the_output() {
        // The contract is about the *output*, not about dropping: where the
        // value can be folded to a plain one (`min(1em, 2em)` is just `1em`),
        // lightningcss does it for us and the declaration survives intact,
        // which is strictly better. Where it cannot, we drop the declaration.
        // Either way RMSDK never meets a function it cannot parse.
        for value in [
            "calc(1px + 1em)",
            "var(--x)",
            "clamp(1em, 2vw, 3em)",
            "min(1em, 2em)",
            "max(1em, 2em)",
            "env(safe-area-inset-top)",
            "calc(100% - 2em)",
        ] {
            let out = run(&format!("p{{color:red;width:{value}}}"));
            for func in MODERN_FUNCTIONS {
                assert!(
                    !out.css.to_ascii_lowercase().contains(func),
                    "{value} left a {func} in the output: {}",
                    out.css
                );
            }
            assert!(out.css.contains("color"), "{value}: sibling must survive");
        }
    }

    #[test]
    fn a_foldable_function_keeps_its_declaration() {
        // min(1em, 2em) is just 1em: no reason to lose the property.
        let out = run("p{width:min(1em, 2em)}");
        assert_eq!(out.decls_dropped, 0, "a foldable value need not be dropped");
        assert!(out.css.contains("width"), "got: {}", out.css);
        assert!(!out.css.contains("min("), "got: {}", out.css);
    }

    #[test]
    fn the_max_width_property_is_not_the_max_function() {
        // "max-width" contains "max" but not "max(" - it must survive.
        let out = run("p{max-width:30em;min-height:2em}");
        assert_eq!(out.decls_dropped, 0);
        assert!(out.css.contains("max-width"));
        assert!(out.css.contains("min-height"));
    }

    #[test]
    fn a_supports_block_is_dropped_whole() {
        let out = run("p{color:red}@supports (display:grid){p{display:grid}}");
        assert!(!out.css.contains("supports"));
        assert!(out.css.contains("color"));
        assert_eq!(out.rules_dropped, 1);
    }

    #[test]
    fn font_face_and_media_survive() {
        let out = run("@font-face{font-family:X;src:url(x.otf)}\
             @media screen{p{color:red}}");
        assert!(out.css.contains("@font-face"), "got: {}", out.css);
        assert!(out.css.contains("@media"), "got: {}", out.css);
        assert!(out.css.contains("color"), "got: {}", out.css);
        assert_eq!(out.decls_dropped, 0);
        assert_eq!(out.rules_dropped, 0);
    }

    #[test]
    fn a_modern_function_inside_a_media_block_is_stripped_there_too() {
        let out = run("@media screen{p{width:calc(100% - 1em);color:blue}}");
        assert!(!out.css.contains("calc"));
        assert!(out.css.contains("color"));
        assert_eq!(out.decls_dropped, 1);
    }

    #[test]
    fn a_rule_emptied_by_sanitizing_leaves_no_husk() {
        let out = run("p{width:calc(100% - 1em)}");
        assert!(!out.css.contains("p{}"), "no empty rule: {}", out.css);
        assert_eq!(out.decls_dropped, 1);
    }

    #[test]
    fn sanitizing_is_a_fixed_point() {
        let css = "p{color:#333;width:calc(100% - 2em)}@supports (x:y){p{color:blue}}\
                   @media screen{h1{margin:0}}";
        let once = run(css).css;
        let twice = run(&once).css;
        assert_eq!(once, twice, "sanitize must be idempotent");
    }

    #[test]
    fn a_fractional_length_keeps_its_leading_zero() {
        // `.5em` is valid CSS but an old parser may refuse a number with no
        // integer part, and refusing is how RMSDK loses a whole stylesheet.
        let out = run("p{margin:0.5em;text-indent:0.25em}");
        assert!(out.css.contains("0.5em"), "got: {}", out.css);
        assert!(out.css.contains("0.25em"), "got: {}", out.css);
        assert!(!out.css.contains(":.5em"), "got: {}", out.css);
    }

    #[test]
    fn a_custom_property_definition_goes_with_its_uses() {
        // Once var() is stripped nothing can read it, and the `--x:` syntax is
        // itself newer than the parser.
        let out = run(":root{--accent:#369}h1{color:var(--accent);text-align:center}");
        assert!(!out.css.contains("--accent"), "got: {}", out.css);
        assert!(out.css.contains("text-align"), "siblings live: {}", out.css);
    }

    #[test]
    fn a_clean_stylesheet_keeps_all_of_its_declarations() {
        let out = run("body{margin:0;font-size:16px}h1{text-align:center;font-weight:700}");
        assert_eq!(out.decls_dropped, 0);
        assert_eq!(out.rules_dropped, 0);
        for expected in ["margin", "font-size", "text-align", "font-weight"] {
            assert!(out.css.contains(expected), "{expected} must survive");
        }
    }
}
