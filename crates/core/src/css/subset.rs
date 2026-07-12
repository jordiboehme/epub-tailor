//! Filter arbitrary stylesheet text down to the tiny subset the CrossPoint
//! firmware actually parses.
//!
//! The device's CSS grammar (see `docs/device-constraints.md`) is minute:
//! selectors may only be `tag`, `.class`, `tag.class` or comma groups of those;
//! there are no @-rules; and roughly a dozen properties are honored. Everything
//! outside that is dead weight at best and layout noise at worst, so we drop it
//! and keep only what renders.
//!
//! Parsing goes through [`lightningcss`] in error-recovery mode, so a malformed
//! construct is skipped rather than failing the whole conversion. We do not use
//! lightningcss's minifier (it rewrites values like `0.5em` into `.5em`, which a
//! naive on-device parser may not accept); instead every kept selector and value
//! is serialized faithfully and we do the structural minification (no whitespace
//! between rules or declarations) ourselves. The result is deterministic and a
//! fixed point of the filter: `filter_css(filter_css(x)) == filter_css(x)`.

use std::sync::{Arc, RwLock};

use lightningcss::declaration::DeclarationBlock;
use lightningcss::media_query::{MediaList, MediaType, Qualifier};
use lightningcss::rules::CssRule;
use lightningcss::selector::SelectorList;
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::traits::ToCss;

use crate::report::Warning;

/// Properties the device honors, matched by canonical (unprefixed) name.
/// `display` and `vertical-align` are additionally value-restricted and handled
/// in [`property_is_supported`].
const ALLOWED_PROPERTIES: &[&str] = &[
    "text-align",
    "font-style",
    "font-weight",
    "text-decoration",
    "text-decoration-line",
    "text-indent",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "width",
    "height",
    "direction",
];

/// The outcome of filtering one stylesheet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilteredCss {
    /// The minified, device-conformant CSS.
    pub css: String,
    /// Number of style rules kept.
    pub rules_kept: usize,
    /// Number of rules (and at-rules) dropped.
    pub rules_dropped: usize,
    /// Number of individual declarations dropped from kept rules.
    pub decls_dropped: usize,
}

/// One surviving style rule: device-legal selectors paired with the already
/// minified, whitelisted declaration block behind them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilteredRule {
    /// The kept selectors, already validated as device-legal (`tag`,
    /// `.class`, or `tag.class`). Joining with `,` reproduces the selector
    /// list of the original rule (minus any rejected group members).
    pub selectors: Vec<String>,
    /// The minified `name:value;name:value` declaration block, with no
    /// surrounding braces.
    pub declarations: String,
}

/// Running totals threaded through the recursive rule walk.
#[derive(Default)]
struct Accumulator {
    rules: Vec<FilteredRule>,
    kept: usize,
    dropped: usize,
    decls_dropped: usize,
}

/// Parse `css` and walk it into an [`Accumulator`] of surviving rules and
/// counts, naming the source `path` in any warnings. Shared by [`filter_css`]
/// and [`filter_css_rules`] so both see the identical parse/process path.
/// Returns `None` when the stylesheet cannot be parsed at all (already
/// reported as a warning).
fn filter_css_to_accumulator(
    css: &str,
    path: &str,
    warnings: &mut Vec<Warning>,
) -> Option<Accumulator> {
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
            return None;
        }
    };

    let mut acc = Accumulator::default();
    process_rules(&stylesheet.rules.0, path, warnings, &mut acc);

    // One warning per construct lightningcss skipped during error recovery.
    let skipped = parse_warnings.read().map(|w| w.len()).unwrap_or(0);
    for _ in 0..skipped {
        warnings.push(Warning {
            message: format!("skipped an unparseable CSS construct in {path}"),
            file: Some(path.to_string()),
        });
    }

    Some(acc)
}

/// Filter `css` down to the device-supported subset, naming the source `path`
/// in any warnings. Never fails: parse errors are recovered from and reported as
/// warnings, and a stylesheet that cannot be parsed at all is dropped entirely.
pub fn filter_css(css: &str, path: &str, warnings: &mut Vec<Warning>) -> FilteredCss {
    let Some(acc) = filter_css_to_accumulator(css, path, warnings) else {
        return FilteredCss::default();
    };

    let mut out = String::new();
    for rule in &acc.rules {
        push_rule(&mut out, &rule.selectors, &rule.declarations);
    }

    FilteredCss {
        css: out,
        rules_kept: acc.kept,
        rules_dropped: acc.dropped,
        decls_dropped: acc.decls_dropped,
    }
}

/// Filter `css` down to the device-supported subset like [`filter_css`], but
/// return the surviving rules structured instead of pre-joined into a single
/// string. `filter_css(css, path, w).css` is exactly
/// `selectors.join(",") + "{" + declarations + "}"` for each returned rule,
/// concatenated with no separator.
pub fn filter_css_rules(css: &str, path: &str, warnings: &mut Vec<Warning>) -> Vec<FilteredRule> {
    filter_css_to_accumulator(css, path, warnings)
        .map(|acc| acc.rules)
        .unwrap_or_default()
}

/// Append one style rule's minified form - the selectors joined with `,`, then
/// `{`, the declarations, and `}` - to `out`. This is the single place a rule is
/// serialized, shared by [`filter_css`] and the per-chapter
/// [`scope`](super::scope) pass so both emit the identical rule shape.
pub(crate) fn push_rule(out: &mut String, selectors: &[String], declarations: &str) {
    out.push_str(&selectors.join(","));
    out.push('{');
    out.push_str(declarations);
    out.push('}');
}

/// Filter one inline `style=""` attribute value through the declaration
/// whitelist. Returns the minified survivors, or `None` when nothing survives
/// (so the caller can drop the attribute).
pub fn filter_inline_style(style: &str) -> Option<String> {
    let options = ParserOptions {
        error_recovery: true,
        ..Default::default()
    };
    let block = DeclarationBlock::parse_string(style, options).ok()?;
    let (declarations, _) = filter_declarations(&block);
    if declarations.is_empty() {
        None
    } else {
        Some(declarations)
    }
}

/// Walk a rule list, appending kept style rules to `acc.out`. `@media all` /
/// `@media screen` bodies are hoisted (their inner rules processed in place);
/// every other at-rule is dropped.
fn process_rules(
    rules: &[CssRule<'_>],
    path: &str,
    warnings: &mut Vec<Warning>,
    acc: &mut Accumulator,
) {
    for rule in rules {
        match rule {
            CssRule::Style(style) => emit_style_rule(&style.selectors, &style.declarations, acc),
            CssRule::Media(media) if media_is_screen_or_all(&media.query) => {
                process_rules(&media.rules.0, path, warnings, acc);
            }
            CssRule::Import(import) => {
                acc.dropped += 1;
                warnings.push(Warning {
                    message: format!(
                        "dropped an @import of {} in {path}; the device cannot follow it",
                        import.url
                    ),
                    file: Some(path.to_string()),
                });
            }
            // Error-recovery placeholder: the skipped construct is already
            // counted via the parser's warning list.
            CssRule::Ignored => {}
            _ => acc.dropped += 1,
        }
    }
}

/// Emit a single style rule if at least one selector and one declaration
/// survive; otherwise count it as dropped.
fn emit_style_rule(
    selectors: &SelectorList<'_>,
    declarations: &DeclarationBlock<'_>,
    acc: &mut Accumulator,
) {
    let kept_selectors: Vec<String> = selectors
        .0
        .iter()
        .filter_map(|selector| selector.to_css_string(PrinterOptions::default()).ok())
        .filter(|selector| selector_is_supported(selector))
        .collect();
    if kept_selectors.is_empty() {
        acc.dropped += 1;
        return;
    }

    let (declarations, dropped) = filter_declarations(declarations);
    acc.decls_dropped += dropped;
    if declarations.is_empty() {
        acc.dropped += 1;
        return;
    }

    acc.rules.push(FilteredRule {
        selectors: kept_selectors,
        declarations,
    });
    acc.kept += 1;
}

/// Filter a declaration block to the whitelist, stripping `!important`. Returns
/// the minified `name:value;name:value` string and the count of dropped
/// declarations.
fn filter_declarations(block: &DeclarationBlock<'_>) -> (String, usize) {
    let mut kept: Vec<String> = Vec::new();
    let mut dropped = 0usize;
    for property in block
        .declarations
        .iter()
        .chain(block.important_declarations.iter())
    {
        let id = property.property_id();
        let name = id.name();
        let Ok(value) = property.value_to_css_string(PrinterOptions::default()) else {
            dropped += 1;
            continue;
        };
        let value = restore_leading_zeros(&value);
        if property_is_supported(name, &value) {
            kept.push(format!("{name}:{value}"));
        } else {
            dropped += 1;
        }
    }
    (kept.join(";"), dropped)
}

/// Put back the integer `0` that lightningcss strips from a fractional length
/// (it serializes `0.5em` as `.5em`). The device's naive CSS parser may reject a
/// number with no integer part, so a leading-dot number gets its zero restored:
/// `.5em` -> `0.5em`, `-.5em` -> `-0.5em`, while `1.5em` is left untouched.
pub(crate) fn restore_leading_zeros(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 1);
    let mut prev: Option<char> = None;
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '.'
            && chars.peek().is_some_and(char::is_ascii_digit)
            && !prev.is_some_and(|p| p.is_ascii_digit())
        {
            out.push('0');
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

/// Whether a property (by canonical name and serialized value) is kept.
fn property_is_supported(name: &str, value: &str) -> bool {
    // A modern value function is unreadable to the device's parser whatever
    // property carries it: `width: calc(100% - 2em)` is in the allowed property
    // list but is still gibberish to firmware that cannot do arithmetic. See
    // [`super::sanitize::uses_modern_function`], which the RMSDK pass shares.
    if super::sanitize::uses_modern_function(value) {
        return false;
    }
    match name {
        // The device only models `display: none`; every other display is a no-op.
        "display" => value.eq_ignore_ascii_case("none"),
        // Only super/sub shift the baseline; other values do nothing on device.
        "vertical-align" => {
            value.eq_ignore_ascii_case("super") || value.eq_ignore_ascii_case("sub")
        }
        other => ALLOWED_PROPERTIES.contains(&other),
    }
}

/// Whether a serialized selector matches one of the three device-supported
/// shapes: `tag`, `.class`, or `tag.class`. Anything with a combinator,
/// descendant space, universal, id, attribute, pseudo, or a second class is
/// rejected.
fn selector_is_supported(selector: &str) -> bool {
    if selector.is_empty() {
        return false;
    }
    if selector.chars().any(is_structural_char) {
        return false;
    }
    match selector.split_once('.') {
        // `tag`
        None => is_ident(selector),
        // `.class` or `tag.class`; the class part must not carry a second `.`
        Some((tag, class)) => (tag.is_empty() || is_ident(tag)) && is_ident(class),
    }
}

/// Characters whose presence in a selector means it uses a construct the device
/// does not model (combinators, descendant, universal, id, attribute, pseudo,
/// escapes, namespaces, comma groups already split away, ...).
fn is_structural_char(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '>' | '+'
                | '~'
                | '*'
                | '#'
                | '['
                | ']'
                | ':'
                | '('
                | ')'
                | ','
                | '&'
                | '|'
                | '\\'
                | '@'
                | '%'
                | '"'
                | '\''
                | '='
        )
}

/// Whether `s` is a bare identifier (tag or class name) with no `.`.
fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && !s.contains('.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether a media query list is effectively `all` or `screen` (including
/// `only screen`), i.e. one the device would apply unconditionally.
fn media_is_screen_or_all(query: &MediaList<'_>) -> bool {
    query.media_queries.iter().any(|q| {
        !matches!(q.qualifier, Some(Qualifier::Not))
            && q.condition.is_none()
            && matches!(q.media_type, MediaType::All | MediaType::Screen)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn filter(css: &str) -> FilteredCss {
        let mut warnings = Vec::new();
        filter_css(css, "test.css", &mut warnings)
    }

    #[test]
    fn an_allowed_property_carrying_calc_is_still_dropped() {
        // `width` is on the allow-list, but the device cannot do arithmetic, so
        // `calc()` reaching it is gibberish. The property being supported is not
        // enough - the value has to be readable too.
        let out = filter("img{width:calc(100% - 2em);height:40px}");
        assert!(!out.css.contains("calc"), "got: {}", out.css);
        assert!(out.css.contains("height"), "siblings survive: {}", out.css);
        assert_eq!(out.decls_dropped, 1);
    }

    /// Re-parse `css` and assert every rule is a plain style rule whose
    /// selectors and properties are all device-supported (no at-rules survive).
    fn assert_conformant(css: &str) {
        let sheet = StyleSheet::parse(
            css,
            ParserOptions {
                error_recovery: true,
                ..Default::default()
            },
        )
        .expect("filtered CSS re-parses");
        for rule in &sheet.rules.0 {
            match rule {
                CssRule::Style(style) => {
                    for selector in style.selectors.0.iter() {
                        let text = selector
                            .to_css_string(PrinterOptions::default())
                            .expect("selector serializes");
                        assert!(
                            selector_is_supported(&text),
                            "unsupported selector survived: {text}"
                        );
                    }
                    for property in style
                        .declarations
                        .declarations
                        .iter()
                        .chain(style.declarations.important_declarations.iter())
                    {
                        let id = property.property_id();
                        let value = property
                            .value_to_css_string(PrinterOptions::default())
                            .expect("value serializes");
                        assert!(
                            property_is_supported(id.name(), &value),
                            "unsupported property survived: {}:{value}",
                            id.name()
                        );
                    }
                }
                CssRule::Ignored => {}
                other => panic!("an at-rule survived filtering: {other:?}"),
            }
        }
    }

    #[test]
    fn gnarly_sheet_snapshot() {
        let input = r#"
/* a comment */
@import url("reset.css");
@font-face { font-family: "X"; src: url(x.woff2); }
body { color: red; text-align: center; margin: 0 auto !important; }
div p { color: blue; }
#main { width: 10px; }
.card:hover { text-decoration: underline; }
a[href] { color: green; }
* { box-sizing: border-box; }
.a.b { text-indent: 1em; }
h1, .title { font-weight: 700; font-size: 20px; }
.hidden { display: none; }
.flex { display: flex; }
sub { vertical-align: sub; }
.up { vertical-align: super; }
.mid { vertical-align: middle; }
@media print { .p { text-align: left; } }
@media screen { .s { margin-left: 2em; color: red; } }
@media only screen { .o { padding: 1em; } }
@media (min-width: 500px) { .cond { text-align: right; } }
.trailing { text-align: center }
"#;
        let filtered = filter(input);
        insta::assert_snapshot!(filtered.css);
        assert_conformant(&filtered.css);
        assert!(filtered.rules_kept >= 8, "kept: {}", filtered.rules_kept);
    }

    #[test]
    fn filter_css_rules_reconstructs_to_filter_css_output() {
        // Same gnarly fixture as `gnarly_sheet_snapshot`, duplicated so this
        // test stands alone: `filter_css_rules` must go through the identical
        // parse/process path and, once joined back the way `filter_css` joins
        // its rules, reproduce `filter_css`'s output exactly.
        let input = r#"
/* a comment */
@import url("reset.css");
@font-face { font-family: "X"; src: url(x.woff2); }
body { color: red; text-align: center; margin: 0 auto !important; }
div p { color: blue; }
#main { width: 10px; }
.card:hover { text-decoration: underline; }
a[href] { color: green; }
* { box-sizing: border-box; }
.a.b { text-indent: 1em; }
h1, .title { font-weight: 700; font-size: 20px; }
.hidden { display: none; }
.flex { display: flex; }
sub { vertical-align: sub; }
.up { vertical-align: super; }
.mid { vertical-align: middle; }
@media print { .p { text-align: left; } }
@media screen { .s { margin-left: 2em; color: red; } }
@media only screen { .o { padding: 1em; } }
@media (min-width: 500px) { .cond { text-align: right; } }
.trailing { text-align: center }
"#;
        let filtered = filter(input);
        let mut warnings = Vec::new();
        let rules = filter_css_rules(input, "test.css", &mut warnings);
        let reconstructed: String = rules
            .iter()
            .map(|rule| format!("{}{{{}}}", rule.selectors.join(","), rule.declarations))
            .collect();
        assert_eq!(filtered.css, reconstructed);
    }

    #[test]
    fn media_print_is_dropped_and_screen_is_hoisted() {
        let filtered =
            filter("@media print{.p{text-align:left}}@media screen{.s{text-indent:1em}}");
        assert_eq!(filtered.css, ".s{text-indent:1em}");
    }

    #[test]
    fn only_screen_is_hoisted() {
        let filtered = filter("@media only screen{.o{text-align:center}}");
        assert_eq!(filtered.css, ".o{text-align:center}");
    }

    #[test]
    fn conditional_screen_query_is_dropped() {
        let filtered = filter("@media screen and (min-width:500px){.c{text-align:right}}");
        assert_eq!(filtered.css, "");
    }

    #[test]
    fn important_flag_is_stripped_but_declaration_kept() {
        let filtered = filter(".x{text-align:center !important}");
        assert_eq!(filtered.css, ".x{text-align:center}");
    }

    #[test]
    fn display_none_kept_but_other_display_dropped() {
        assert_eq!(filter(".a{display:none}").css, ".a{display:none}");
        assert_eq!(filter(".a{display:flex}").css, "");
    }

    #[test]
    fn vertical_align_restricted_to_super_and_sub() {
        assert_eq!(
            filter("sub{vertical-align:sub}").css,
            "sub{vertical-align:sub}"
        );
        assert_eq!(
            filter(".u{vertical-align:super}").css,
            ".u{vertical-align:super}"
        );
        assert_eq!(filter(".m{vertical-align:middle}").css, "");
    }

    #[test]
    fn unsupported_selectors_are_dropped() {
        assert_eq!(filter("div p{text-align:center}").css, "");
        assert_eq!(filter("#id{text-align:center}").css, "");
        assert_eq!(filter(".a.b{text-align:center}").css, "");
        assert_eq!(filter("a:hover{text-align:center}").css, "");
        assert_eq!(filter("*{text-align:center}").css, "");
        assert_eq!(filter("[href]{text-align:center}").css, "");
    }

    #[test]
    fn supported_selector_shapes_are_kept() {
        assert_eq!(filter("p{text-align:center}").css, "p{text-align:center}");
        assert_eq!(filter(".c{text-align:center}").css, ".c{text-align:center}");
        assert_eq!(
            filter("p.c{text-align:center}").css,
            "p.c{text-align:center}"
        );
    }

    #[test]
    fn partial_selector_group_keeps_survivors() {
        // `div p` is dropped, `.keep` survives, so the rule is kept with only it.
        let filtered = filter("div p,.keep{text-align:center}");
        assert_eq!(filtered.css, ".keep{text-align:center}");
    }

    #[test]
    fn at_import_is_dropped_with_a_warning() {
        let mut warnings = Vec::new();
        let filtered = filter_css(
            "@import url(evil.css);p{text-align:left}",
            "s.css",
            &mut warnings,
        );
        assert_eq!(filtered.css, "p{text-align:left}");
        assert!(
            warnings.iter().any(|w| w.message.contains("@import")),
            "expected an @import warning, got: {warnings:?}"
        );
    }

    #[test]
    fn malformed_declaration_is_recovered_and_rest_survives() {
        let filtered = filter(".broken{color: ;text-align:right}");
        assert_eq!(filtered.css, ".broken{text-align:right}");
    }

    #[test]
    fn fractional_lengths_are_preserved_verbatim() {
        // The device parser may reject `.5em`; we must keep the leading zero.
        assert_eq!(filter(".d{margin-top:0.5em}").css, ".d{margin-top:0.5em}");
    }

    #[test]
    fn inline_style_keeps_supported_drops_unsupported() {
        assert_eq!(
            filter_inline_style("color:red;text-align:center;font-size:12px"),
            Some("text-align:center".to_string())
        );
    }

    #[test]
    fn inline_style_all_unsupported_returns_none() {
        assert_eq!(filter_inline_style("color:blue;font-size:9px"), None);
    }

    #[test]
    fn inline_style_strips_important() {
        assert_eq!(
            filter_inline_style("text-indent:1em !important"),
            Some("text-indent:1em".to_string())
        );
    }

    #[test]
    fn idempotent_on_gnarly_sheet() {
        let once = filter(
            "@media screen{.s{margin-left:2em}}body{text-align:center;color:red}.a.b{width:1px}",
        );
        let twice = filter(&once.css);
        assert_eq!(once.css, twice.css);
    }

    // -- property test: output is always conformant and idempotent ----------

    fn selector_fragment() -> impl Strategy<Value = &'static str> {
        prop::sample::select(vec![
            ".a", "div", "div.a", "#id", ".a .b", "a:hover", "*", "[x]", ".a.b", "h1", "span",
        ])
    }

    fn declaration_fragment() -> impl Strategy<Value = &'static str> {
        prop::sample::select(vec![
            "text-align:center",
            "color:red",
            "margin:1em",
            "margin-left:2em",
            "display:none",
            "display:flex",
            "vertical-align:super",
            "vertical-align:top",
            "font-weight:700 !important",
            "text-indent:0.5em",
            "foo:bar",
            "width:10px",
        ])
    }

    fn at_rule_fragment() -> impl Strategy<Value = String> {
        prop::sample::select(vec![
            "@media screen{.s{text-align:left}}".to_string(),
            "@media print{.p{text-align:left}}".to_string(),
            "@media all{.q{margin:1em}}".to_string(),
            "@media (min-width:9px){.c{text-align:right}}".to_string(),
            "@font-face{font-family:x;src:url(x.ttf)}".to_string(),
            "@import url(z.css);".to_string(),
        ])
    }

    fn chunk() -> impl Strategy<Value = String> {
        prop_oneof![
            (
                selector_fragment(),
                prop::collection::vec(declaration_fragment(), 0..4)
            )
                .prop_map(|(sel, decls)| format!("{sel}{{{}}}", decls.join(";"))),
            at_rule_fragment(),
            Just("!! not css at all ##".to_string()),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        #[test]
        fn output_is_conformant_and_idempotent(chunks in prop::collection::vec(chunk(), 0..12)) {
            let input = chunks.join("\n");
            let mut warnings = Vec::new();
            let filtered = filter_css(&input, "gen.css", &mut warnings);
            assert_conformant(&filtered.css);
            let twice = filter_css(&filtered.css, "gen.css", &mut Vec::new());
            prop_assert_eq!(twice.css, filtered.css);
        }
    }
}
