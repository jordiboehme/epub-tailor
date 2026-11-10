//! Scope one chapter's relocated head/body `<style>` CSS to that chapter, so it
//! cannot bleed onto the rest of the book.
//!
//! The device applies every `.css` file in the book to every chapter (it scans
//! the zip and ignores `<link>`). So once each chapter's head `<style>` is
//! lifted into the single book-wide `et-relocated.css`, an unscoped author rule
//! like `p { ... }` or `.note { ... }` restyles every chapter, not just the one
//! it came from.
//!
//! The fix tags the contributing chapter's own elements with a chapter-unique
//! class and rewrites each selector to require it. The three simple shapes
//! keep a per-element scope class:
//!
//! | Original | Scoped         | Tagging                                        |
//! |----------|----------------|------------------------------------------------|
//! | `.X`     | `.cpr{k}-c-X`  | every element whose class list contains `X`    |
//! | `T.X`    | `T.cpr{k}-c-X` | only `<T>` elements carrying `X`               |
//! | `T`      | `T.cpr{k}-e-T` | every `<T>` element (works for `body`/`html`)  |
//!
//! Every other selector CrossPoint 2.0 parses (descendant and child chains,
//! ids, compound classes, `::first-letter`) is scoped through one root class,
//! `cpr{k}-r`, put first in the `class` attribute of `<html>` and `<body>`.
//! The device records every open element as an ancestor up to depth 16
//! (`ChapterHtmlSlimParser.cpp:1425-1429`, `ChapterHtmlSlimParser.h:94`) and
//! matches only an ancestor's first four classes (`CssParser.h:32`):
//!
//! | Original      | Scoped                 |
//! |---------------|------------------------|
//! | `div > p`     | `.cpr{k}-r div > p`    |
//! | `#intro`      | `.cpr{k}-r #intro`     |
//! | `body.x p`    | `body.cpr{k}-r.x p`    |
//!
//! A first compound on `html` or `body` takes the class inline right after its
//! tag; one that already has four classes is dropped, since the device could
//! not match it with the root class added.
//!
//! `k` is the 1-based index over chapters that yielded CSS. A selector that
//! matches no element in its chapter is a dead rule and dropped; a comma group
//! keeps only its surviving members, and a rule with no survivors is dropped
//! entirely. The prefix is `cpr` (not `et-`) so it does not trip the `et-*`
//! helper-class detection that pulls in `et-styles.css`. The `-c-`/`-e-`/`-r`
//! suffixes keep a class-derived name from colliding with an element-derived
//! one (an author class literally named `p` cannot collide with the scope class
//! for `<p>`). Every scoped selector must itself pass the subset filter, so
//! re-filtering the combined relocated sheet is a fixed point.

use kuchikiki::NodeRef;

use crate::css::filter_css_rules;
use crate::css::subset::{
    MAX_SELECTOR_LEN, is_simple_selector, push_rule, selector_is_supported, strip_first_letter,
};
use crate::html::dom::{collect_by_name, get_attr, is_named, set_attr};
use crate::report::Warning;

/// Classes the device matches on an ancestor element (`CssParser.h:32`).
const MAX_ANCESTOR_CLASSES: usize = 4;

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
    let root_class = format!("cpr{scope_idx}-r");
    let mut root_scoped = false;
    let mut out = String::new();
    for rule in &rules {
        let mut scoped = Vec::new();
        for selector in &rule.selectors {
            // Only the simple shapes may take the per-element path: it splits
            // on the first `.`, which would misread a compound like `.a.b`.
            let result = if is_simple_selector(selector) {
                scope_selector(doc, selector, scope_idx)
            } else {
                let result = scope_through_root(doc, selector, &root_class);
                root_scoped |= result.is_some();
                result
            };
            scoped.extend(result);
        }
        // A comma group with no surviving members is a dead rule; drop it.
        if scoped.is_empty() {
            continue;
        }
        push_rule(&mut out, &scoped, &rule.declarations);
    }
    if root_scoped {
        for root in ["html", "body"] {
            for node in collect_by_name(doc, root) {
                prepend_scope_class(&node, &root_class);
            }
        }
    }
    out
}

/// Rewrite a device-legal selector that is not one of the simple shapes to
/// require `root_class` on `<html>`/`<body>`, without touching `doc`. A first
/// compound on `html` or `body` gets the class inline right after its tag;
/// anything else gets it as an ancestor prefix. Returns `None` for a dead
/// selector (no element of `doc` matches, or kuchikiki cannot parse it) and
/// for one the device or the subset filter would not take once scoped.
fn scope_through_root(doc: &NodeRef, selector: &str, root_class: &str) -> Option<String> {
    let first_compound = selector.split(' ').next().unwrap_or(selector);
    let tag_end = first_compound
        .find(['#', '.', ':'])
        .unwrap_or(first_compound.len());
    let tag = &first_compound[..tag_end];
    let on_root = tag.eq_ignore_ascii_case("html") || tag.eq_ignore_ascii_case("body");
    let scoped = if on_root {
        // The root class takes one of the four classes the device matches
        // on an ancestor, so a compound that already has four cannot match.
        if first_compound.matches('.').count() + 1 > MAX_ANCESTOR_CLASSES {
            return None;
        }
        format!("{tag}.{root_class}{}", &selector[tag_end..])
    } else {
        format!(".{root_class} {selector}")
    };
    // The device skips longer selectors (`CssParser.cpp:50, 1175`), and the
    // combined relocated sheet is filtered again, so the scoped form must
    // pass the filter too for that second pass to be a fixed point.
    if scoped.len() > MAX_SELECTOR_LEN || !selector_is_supported(&scoped) {
        return None;
    }
    // Dead-rule check. The device stores `::first-letter` as its own rule key
    // on the element, so match the element. A prefixed selector also needs a
    // root ancestor above its first compound.
    let chain = strip_first_letter(selector);
    let probe = if on_root {
        chain.to_string()
    } else {
        format!("html {chain}, body {chain}")
    };
    let matched = doc
        .select(&probe)
        .is_ok_and(|mut matches| matches.next().is_some());
    matched.then_some(scoped)
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

/// Put `class` first in `node`'s `class` attribute unless already present, so
/// it is among the four classes the device matches on an ancestor
/// (`CssParser.h:32`). Idempotent and token-aware like [`add_scope_class`].
fn prepend_scope_class(node: &NodeRef, class: &str) {
    match get_attr(node, "class") {
        Some(existing) if existing.split_whitespace().any(|token| token == class) => {}
        Some(existing) if !existing.trim().is_empty() => {
            set_attr(node, "class", &format!("{class} {existing}"));
        }
        _ => set_attr(node, "class", class),
    }
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

    /// Like [`run`] at index 1, with `class` set on the `<body>` first.
    fn run_with_body_class(class: &str, body: &str, css: &str) -> (String, String) {
        let doc = doc_from_body(body);
        set_attr(&crate::html::dom::find_body(&doc).unwrap(), "class", class);
        let mut warnings = Vec::new();
        let out = scope_relocated_css(&doc, css, 1, "ch.xhtml", false, &mut warnings);
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
        // `.note:hover`, `p + p` and `a[href]` are not device-legal even in
        // CrossPoint 2.0, so `filter_css_rules` drops them and only the plain
        // `.note` rule reaches the scoper.
        let (css, _) = run(
            r#"<p class="note">x</p><p>y</p><a href="z">z</a>"#,
            ".note:hover{text-align:center}p + p{text-indent:1em}a[href]{margin:0}\
             .note{margin-left:2em}",
            1,
        );
        assert_eq!(css, ".cpr1-c-note{margin-left:2em}");
    }

    #[test]
    fn descendant_selector_is_scoped_through_the_root_class() {
        let (css, dom) = run("<div><p>x</p></div>", "div p{font-style:italic}", 1);
        assert_eq!(css, ".cpr1-r div p{font-style:italic}");
        assert!(
            dom.contains(r#"<body class="cpr1-r">"#),
            "body carries the root class:\n{dom}"
        );
        assert!(
            dom.contains(r#"xhtml" class="cpr1-r">"#),
            "html carries the root class:\n{dom}"
        );
    }

    #[test]
    fn child_and_id_selectors_are_prefixed_with_the_root_class() {
        let (css, _) = run(
            r#"<div><p id="intro">x</p></div>"#,
            "div > p{text-indent:0}#intro{font-style:italic}",
            1,
        );
        assert_eq!(
            css,
            ".cpr1-r div > p{text-indent:0}.cpr1-r #intro{font-style:italic}"
        );
    }

    #[test]
    fn body_compound_gets_the_root_class_inline() {
        let (css, dom) = run_with_body_class("dark", "<p>x</p>", "body.dark p{text-indent:1em}");
        assert_eq!(css, "body.cpr1-r.dark p{text-indent:1em}");
        assert!(
            dom.contains(r#"<body class="cpr1-r dark">"#),
            "root class goes first:\n{dom}"
        );
    }

    #[test]
    fn root_class_goes_right_after_the_tag_before_an_id() {
        let doc = doc_from_body("<p>x</p>");
        set_attr(&crate::html::dom::find_body(&doc).unwrap(), "id", "top");
        let mut warnings = Vec::new();
        let css = scope_relocated_css(
            &doc,
            "BODY#top.x p{margin:0}body#top p{text-indent:0}",
            1,
            "ch.xhtml",
            false,
            &mut warnings,
        );
        // `.x` is not on the body, so that selector is dead.
        assert_eq!(css, "body.cpr1-r#top p{text-indent:0}");
    }

    #[test]
    fn body_compound_with_four_classes_is_dropped() {
        // The device matches only four classes of an ancestor, so the root
        // class leaves no room for a fourth author class.
        let (css, dom) = run_with_body_class(
            "a b c d",
            "<p>x</p>",
            "body.a.b.c.d p{margin:0}body.a.b.c p{text-indent:0}",
        );
        assert_eq!(css, "body.cpr1-r.a.b.c p{text-indent:0}");
        assert!(dom.contains(r#"<body class="cpr1-r a b c d">"#), "{dom}");
    }

    #[test]
    fn root_class_goes_first_on_a_crowded_body() {
        let (css, dom) = run_with_body_class("a b c d", "<div><p>x</p></div>", "div p{margin:0}");
        assert_eq!(css, ".cpr1-r div p{margin:0}");
        assert!(
            dom.contains(r#"<body class="cpr1-r a b c d">"#),
            "root class first:\n{dom}"
        );
    }

    #[test]
    fn root_class_goes_before_a_simple_scope_class_on_the_body() {
        // The simple path appends `cpr1-e-body` while the loop runs; the root
        // class is still put first, and only once.
        let (css, dom) = run(
            "<div><p>x</p></div>",
            "body{text-align:justify}div p{margin:0}html p{text-indent:0}",
            1,
        );
        assert_eq!(
            css,
            "body.cpr1-e-body{text-align:justify}.cpr1-r div p{margin:0}html.cpr1-r p{text-indent:0}"
        );
        assert!(
            dom.contains(r#"<body class="cpr1-r cpr1-e-body">"#),
            "{dom}"
        );
        assert_eq!(count(&dom, "cpr1-r"), 2, "html and body, once each:\n{dom}");
    }

    #[test]
    fn uppercase_tags_on_the_root_path_still_match() {
        // DOM tag names are lowercase; the device compares tags ignoring
        // ASCII case, so the authored case can stay in the output.
        let (css, _) = run(
            "<div><p>x</p></div>",
            "DIV P{margin:0}BODY P{text-indent:0}",
            1,
        );
        assert_eq!(css, ".cpr1-r DIV P{margin:0}BODY.cpr1-r P{text-indent:0}");
    }

    #[test]
    fn first_letter_selector_survives_scoping() {
        let (css, _) = run(
            "<p>x</p>",
            "p::first-letter{float:left;initial-letter:3}",
            1,
        );
        assert_eq!(css, ".cpr1-r p:first-letter{float:left;initial-letter:3}");
    }

    #[test]
    fn dead_descendant_selector_is_dropped() {
        let (css, dom) = run("<div><p>x</p></div>", "aside p{margin:0}", 1);
        assert_eq!(css, "");
        assert!(!dom.contains("cpr"), "nothing tagged:\n{dom}");
    }

    #[test]
    fn selector_matching_only_the_html_element_is_dropped() {
        // `.cpr1-r .top p` needs a root-classed ancestor above `.top`; when
        // `.top` is the `<html>` element itself, nothing is above it.
        let doc = doc_from_body("<p>x</p>");
        let html = doc.select_first("html").unwrap();
        set_attr(html.as_node(), "class", "top");
        let mut warnings = Vec::new();
        let css = scope_relocated_css(
            &doc,
            ".top p{margin:0}",
            1,
            "ch.xhtml",
            false,
            &mut warnings,
        );
        assert_eq!(css, "");
    }

    #[test]
    fn literal_dotted_class_token_is_not_a_compound() {
        // `.a.b` is a compound now and goes through the root path, so an
        // element with the single token `a.b` is not taken for it (the old
        // simple path split on the first `.` and mis-scoped it).
        let (css, dom) = run(
            r#"<div><p class="a.b">x</p></div>"#,
            ".a.b{text-indent:1em}p{margin-left:2em}",
            1,
        );
        assert_eq!(css, "p.cpr1-e-p{margin-left:2em}");
        assert!(!dom.contains("cpr1-c-"), "nothing class-scoped:\n{dom}");
        assert!(!dom.contains("cpr1-r"), "no root class:\n{dom}");
    }

    #[test]
    fn scoped_selectors_that_the_filter_would_reject_are_dropped() {
        // Scoping must keep the relocated sheet a fixed point of the filter:
        // a prefix that pushes a selector past the filter's length or
        // compound limit drops the selector instead.
        let long_class = "a".repeat(230);
        let deep =
            "<div><div><div><div><div><div><div><p>x</p></div></div></div></div></div></div></div>";
        let css_in =
            format!(".{long_class} p{{margin:0}}div div div div div div div p{{text-indent:0}}");
        let body = format!(r#"<div class="{long_class}">{deep}</div>"#);
        let (css, dom) = run(&body, &css_in, 1);
        assert_eq!(css, "");
        assert!(!dom.contains("cpr1-r"), "no root class:\n{dom}");
    }

    #[test]
    fn scoped_sheet_is_a_fixed_point_of_the_filter() {
        let (css, _) = run_with_body_class(
            "dark",
            r#"<div class="a b"><p id="i">x</p></div>"#,
            "body.dark p{margin:0}div > p{text-indent:0}.a.b #i{font-style:italic}\
             p::first-letter{float:left}p{text-align:justify}",
        );
        assert!(css.contains(".cpr1-r"), "{css}");
        let mut warnings = Vec::new();
        let refiltered = crate::css::filter_css(&css, "et-relocated.css", false, &mut warnings);
        assert_eq!(refiltered.css, css);
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
