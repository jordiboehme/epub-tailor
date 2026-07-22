//! Scope one chapter's relocated head/body `<style>` CSS to that chapter, so it
//! cannot bleed onto the rest of the book.
//!
//! The device applies every `.css` file in the book to every chapter (it scans
//! the zip and ignores `<link>`), and its selector grammar is only `tag`,
//! `.class`, `tag.class` (see `docs/device-constraints.md`) - there is no
//! descendant or multi-class selector to scope with. So once each chapter's head
//! `<style>` is lifted into the single book-wide `et-relocated.css`, an unscoped
//! author rule like `p { ... }` or `.note { ... }` restyles every chapter, not
//! just the one it came from.
//!
//! The fix tags the contributing chapter's own elements with a chapter-unique
//! class and rewrites each selector to require it:
//!
//! | Original | Scoped         | Tagging                                        |
//! |----------|----------------|------------------------------------------------|
//! | `.X`     | `.cpr{k}-c-X`  | every element whose class list contains `X`    |
//! | `T.X`    | `T.cpr{k}-c-X` | only `<T>` elements carrying `X`               |
//! | `T`      | `T.cpr{k}-e-T` | every `<T>` element (works for `body`/`html`)  |
//!
//! `k` is the 1-based index over chapters that yielded CSS. A selector that
//! matches no element in its chapter is a dead rule and dropped; a comma group
//! keeps only its surviving members, and a rule with no survivors is dropped
//! entirely. The prefix is `cpr` (not `et-`) so it does not trip the `et-*`
//! helper-class detection that pulls in `et-styles.css`. The `-c-`/`-e-` infixes
//! keep a class-derived name from colliding with an element-derived one (an
//! author class literally named `p` cannot collide with the scope class for
//! `<p>`). The scoped selectors are themselves device-legal `tag.class`/`.class`
//! shapes, so re-filtering the combined relocated sheet is a fixed point.

use kuchikiki::NodeRef;

use crate::css::filter_css_rules;
use crate::css::subset::push_rule;
use crate::html::dom::{get_attr, is_named, set_attr};
use crate::report::Warning;

/// Filter `css` to the device subset, then scope every surviving rule to the
/// chapter `doc` it came from, tagging `doc`'s matching elements as a side
/// effect. `scope_idx` is the chapter's 1-based contributing index; `path` names
/// the source in any warnings. Returns the scoped, minified CSS, or `""` when
/// every rule died (nothing matched).
pub(crate) fn scope_relocated_css(
    doc: &NodeRef,
    css: &str,
    scope_idx: usize,
    path: &str,
    keep_colors: bool,
    warnings: &mut Vec<Warning>,
) -> String {
    let rules = filter_css_rules(css, path, keep_colors, warnings);
    let mut out = String::new();
    for rule in &rules {
        let scoped: Vec<String> = rule
            .selectors
            .iter()
            .filter_map(|selector| scope_selector(doc, selector, scope_idx))
            .collect();
        // A comma group with no surviving members is a dead rule; drop it.
        if scoped.is_empty() {
            continue;
        }
        push_rule(&mut out, &scoped, &rule.declarations);
    }
    out
}

/// Rewrite one device-legal selector (`tag`, `.class`, or `tag.class`) to
/// require the chapter's scope class, tagging every matching element in `doc` as
/// a side effect. Returns the scoped selector, or `None` when nothing matched
/// (a dead selector to drop).
fn scope_selector(doc: &NodeRef, selector: &str, scope_idx: usize) -> Option<String> {
    match selector.split_once('.') {
        // `T` - tag every `<T>` element, including `body`/`html` and nested ones.
        // DOM tag names are always lowercase (html5ever), so an authored
        // uppercase/mixed-case tag is lowercased for matching, the emitted
        // selector and the derived scope class; only the class part (below)
        // is case-sensitive.
        None => {
            let tag = selector.to_ascii_lowercase();
            let scope_class = format!("cpr{scope_idx}-e-{tag}");
            let matched = tag_matching(doc, &scope_class, |node| is_named(node, &tag));
            matched.then(|| format!("{tag}.{scope_class}"))
        }
        // `.X` - tag every element whose class list contains `X`.
        Some(("", class)) => {
            let scope_class = format!("cpr{scope_idx}-c-{class}");
            let matched = tag_matching(doc, &scope_class, |node| has_class(node, class));
            matched.then(|| format!(".{scope_class}"))
        }
        // `T.X` - tag only `<T>` elements carrying `X`. The tag is lowercased
        // like the bare-tag case above; `class` keeps its authored case.
        Some((tag, class)) => {
            let tag = tag.to_ascii_lowercase();
            let scope_class = format!("cpr{scope_idx}-c-{class}");
            let matched = tag_matching(doc, &scope_class, |node| {
                is_named(node, &tag) && has_class(node, class)
            });
            matched.then(|| format!("{tag}.{scope_class}"))
        }
    }
}

/// Add `scope_class` to every element of `doc` for which `matches` holds, and
/// report whether at least one did. Document order, so the mutation is
/// deterministic.
///
/// Called once per selector, so scoping a chapter's CSS is O(selectors x
/// elements) - a full DOM walk per selector rather than one combined pass.
/// That is deliberate: chapter-sized documents keep both factors small enough
/// that the simplicity is worth more than the constant-factor win.
fn tag_matching(doc: &NodeRef, scope_class: &str, matches: impl Fn(&NodeRef) -> bool) -> bool {
    let mut any = false;
    for node in doc.inclusive_descendants() {
        if matches(&node) {
            add_scope_class(&node, scope_class);
            any = true;
        }
    }
    any
}

/// Whether `node`'s `class` attribute contains the whitespace-separated token
/// `class`. `false` for non-element nodes (they have no attributes).
fn has_class(node: &NodeRef, class: &str) -> bool {
    get_attr(node, "class")
        .is_some_and(|value| value.split_whitespace().any(|token| token == class))
}

/// Add `class` to `node`'s `class` attribute unless already present. Idempotent
/// and token-aware, so an element matched by several rules gets each scope class
/// exactly once and never a duplicate token.
fn add_scope_class(node: &NodeRef, class: &str) {
    match get_attr(node, "class") {
        Some(existing) if existing.split_whitespace().any(|token| token == class) => {}
        Some(existing) if !existing.trim().is_empty() => {
            set_attr(node, "class", &format!("{existing} {class}"));
        }
        _ => set_attr(node, "class", class),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::testutil::{doc_from_body, serialize};

    /// Scope `css` for `body` at `idx` and return `(scoped_css, serialized_dom)`.
    fn run(body: &str, css: &str, idx: usize) -> (String, String) {
        let doc = doc_from_body(body);
        let mut warnings = Vec::new();
        let out = scope_relocated_css(&doc, css, idx, "ch.xhtml", false, &mut warnings);
        (out, serialize(&doc))
    }

    fn count(haystack: &str, needle: &str) -> usize {
        haystack.matches(needle).count()
    }

    #[test]
    fn bare_class_selector_is_scoped_and_tagged() {
        let (css, dom) = run(r#"<p class="note">hi</p>"#, ".note{margin-left:2em}", 1);
        assert_eq!(css, ".cpr1-c-note{margin-left:2em}");
        assert!(dom.contains("cpr1-c-note"), "note element tagged:\n{dom}");
    }

    #[test]
    fn tag_class_selector_is_scoped_and_only_that_tag_tagged() {
        let (css, dom) = run(
            r#"<p class="note">p</p><div class="note">div</div>"#,
            "p.note{text-align:center}",
            1,
        );
        assert_eq!(css, "p.cpr1-c-note{text-align:center}");
        // Only the <p> carries the scope class; the <div class="note"> does not.
        assert!(
            dom.contains(r#"<p class="note cpr1-c-note">"#),
            "the p should be tagged:\n{dom}"
        );
        assert!(
            dom.contains(r#"<div class="note">"#),
            "the div must NOT be tagged:\n{dom}"
        );
    }

    #[test]
    fn tag_selector_is_scoped_and_tags_every_such_element_including_nested() {
        let (css, dom) = run(
            "<div><p>a</p><section><p>b</p></section></div>",
            "p{text-align:justify}",
            1,
        );
        assert_eq!(css, "p.cpr1-e-p{text-align:justify}");
        assert_eq!(count(&dom, "cpr1-e-p"), 2, "both <p> tagged:\n{dom}");
    }

    #[test]
    fn body_selector_tags_the_body() {
        let (css, dom) = run("<p>x</p>", "body{text-align:justify}", 1);
        assert_eq!(css, "body.cpr1-e-body{text-align:justify}");
        assert!(
            dom.contains(r#"<body class="cpr1-e-body">"#),
            "body tagged:\n{dom}"
        );
    }

    #[test]
    fn dead_selector_is_dropped() {
        let (css, dom) = run("<p>x</p>", ".s{text-indent:1em}", 1);
        assert_eq!(css, "", "a selector matching nothing is a dead rule");
        assert!(!dom.contains("cpr"), "nothing tagged:\n{dom}");
    }

    #[test]
    fn comma_group_keeps_only_surviving_selectors() {
        let (css, _) = run(r#"<p class="a">x</p>"#, ".a,.gone{margin:1em}", 1);
        assert_eq!(css, ".cpr1-c-a{margin:1em}");
    }

    #[test]
    fn rule_with_no_surviving_selectors_is_dropped_entirely() {
        let (css, _) = run("<p>x</p>", ".gone,.also-gone{margin:1em}", 1);
        assert_eq!(css, "");
    }

    #[test]
    fn tagging_is_idempotent_across_rules() {
        // The element matches both `.note` and `p.note`; both scope to the same
        // `cpr1-c-note` class, which must be added exactly once.
        let (css, dom) = run(
            r#"<p class="note">x</p>"#,
            ".note{margin-left:2em}p.note{text-align:center}",
            1,
        );
        assert_eq!(
            css,
            ".cpr1-c-note{margin-left:2em}p.cpr1-c-note{text-align:center}"
        );
        assert_eq!(
            count(&dom, "cpr1-c-note"),
            1,
            "scope class added exactly once:\n{dom}"
        );
    }

    #[test]
    fn class_and_element_scope_names_do_not_collide() {
        // An author class literally named `p` and the `<p>` tag both target the
        // one element; the `-c-`/`-e-` infixes keep the two scope classes apart.
        let (css, dom) = run(
            r#"<p class="p">x</p>"#,
            ".p{margin:1em}p{text-align:justify}",
            1,
        );
        assert_eq!(css, ".cpr1-c-p{margin:1em}p.cpr1-e-p{text-align:justify}");
        assert!(dom.contains("cpr1-c-p"), "class scope present:\n{dom}");
        assert!(dom.contains("cpr1-e-p"), "element scope present:\n{dom}");
    }

    #[test]
    fn unsupported_selectors_are_filtered_before_scoping() {
        // `.note:hover` is not device-legal, so `filter_css_rules` drops it and
        // only the plain `.note` rule reaches the scoper.
        let (css, _) = run(
            r#"<p class="note">x</p>"#,
            ".note:hover{text-align:center}.note{margin-left:2em}",
            1,
        );
        assert_eq!(css, ".cpr1-c-note{margin-left:2em}");
    }

    #[test]
    fn uppercase_type_selector_is_scoped_and_tagged() {
        // DOM tag names are always lowercase (html5ever); an authored
        // uppercase type selector like `DIV` must still match the element
        // it clearly targets rather than being dropped as a dead rule.
        let (css, dom) = run("<div>x</div>", "DIV{margin:0}", 1);
        assert_eq!(css, "div.cpr1-e-div{margin:0}");
        assert!(dom.contains("cpr1-e-div"), "the div must be tagged:\n{dom}");
    }

    #[test]
    fn uppercase_class_token_in_a_tag_class_selector_keeps_its_case() {
        // The tag portion is lowercased for matching, but classes are
        // case-sensitive in HTML, so an authored class token's case must
        // survive into the emitted selector and the scope class name.
        let (css, dom) = run(r#"<p class="Note">x</p>"#, "P.Note{margin:0}", 1);
        assert_eq!(css, "p.cpr1-c-Note{margin:0}");
        assert!(dom.contains("cpr1-c-Note"), "the p must be tagged:\n{dom}");
    }

    #[test]
    fn scope_index_is_reflected_in_the_names() {
        let (css, dom) = run(r#"<p class="note">x</p>"#, ".note{margin-left:2em}", 3);
        assert_eq!(css, ".cpr3-c-note{margin-left:2em}");
        assert!(dom.contains("cpr3-c-note"), "{dom}");
    }

    #[test]
    fn keep_colors_survives_scoping() {
        // The relocated sheet is subset-filtered unconditionally; with the
        // remap active its colors must ride through the scoper for the later
        // rewrite to find them.
        let doc = doc_from_body(r#"<p class="note">hi</p>"#);
        let mut warnings = Vec::new();
        let out = scope_relocated_css(
            &doc,
            ".note{color:red;margin-left:2em}",
            1,
            "ch.xhtml",
            true,
            &mut warnings,
        );
        assert_eq!(out, ".cpr1-c-note{color:red;margin-left:2em}");
    }

    #[test]
    fn scoping_is_deterministic_in_output_and_mutation() {
        let body = r#"<p class="note">x</p><div class="note">y</div><p>z</p>"#;
        let css = ".note{margin-left:2em}p{text-align:justify}";
        let (css_a, dom_a) = run(body, css, 1);
        let (css_b, dom_b) = run(body, css, 1);
        assert_eq!(css_a, css_b, "same input -> identical output");
        assert_eq!(dom_a, dom_b, "same input -> identical DOM mutation");
    }
}
