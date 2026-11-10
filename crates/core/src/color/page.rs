//! Keep whole pages white on a CrossPoint (gray4) panel.
//!
//! The device draws a light `background-color` as a dithered shade
//! (`color::palette` maps it to 170). That suits a callout box, but a tint on
//! the page itself (`body{background:wheat}`) or on a wrapper around the whole
//! chapter would shade every page. So each chapter's `<html>`, `<body>` and
//! the chain of sole element children below `<body>` get an inline
//! `background-color:#fff`, which the device applies over any stylesheet rule
//! (`ChapterHtmlSlimParser.cpp:1430-1433` at firmware 2.0.0). The chain stops
//! at the first element with more than one element child or with text of
//! its own beside its element child (that element is
//! still a wrapper of the whole chapter and gets white too) or at an element
//! that is not a plain container, so boxes deeper in the chapter keep their
//! shade.

use kuchikiki::NodeRef;

use crate::html::dom::{child_elements, collect_by_name, find_body, is_named, merge_inline_style};

/// Elements that only group other blocks and may wrap a whole chapter.
const CONTAINERS: &[&str] = &[
    "div",
    "section",
    "article",
    "main",
    "header",
    "footer",
    "aside",
    "nav",
    "blockquote",
    "figure",
    "center",
];

const WHITE: (&str, &str) = ("background-color", "#fff");

/// Set an inline white background on `doc`'s page-level elements (see the
/// module docs).
pub(crate) fn keep_page_white(doc: &NodeRef) {
    for html in collect_by_name(doc, "html") {
        merge_inline_style(&html, &[WHITE]);
    }
    let Some(body) = find_body(doc) else {
        return;
    };
    merge_inline_style(&body, &[WHITE]);
    let mut current = body;
    loop {
        // Text next to the element makes it one block among several.
        if current
            .children()
            .any(|c| c.as_text().is_some_and(|t| !t.borrow().trim().is_empty()))
        {
            return;
        }
        let children = child_elements(&current);
        let [only] = children.as_slice() else {
            return;
        };
        if !CONTAINERS.iter().any(|name| is_named(only, name)) {
            return;
        }
        merge_inline_style(only, &[WHITE]);
        current = only.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::testutil::{doc_from_body, serialize};

    fn run(body: &str) -> String {
        let doc = doc_from_body(body);
        keep_page_white(&doc);
        serialize(&doc)
    }

    #[test]
    fn html_and_body_get_white() {
        let out = run("<p>a</p><p>b</p>");
        assert!(
            out.contains(r#"<body style="background-color:#fff">"#),
            "{out}"
        );
        assert!(
            out.contains(r#"xhtml" style="background-color:#fff">"#),
            "{out}"
        );
        assert!(out.contains("<p>a</p>"), "{out}");
    }

    #[test]
    fn the_sole_wrapper_chain_gets_white_and_stops_at_the_split() {
        let out = run(
            r#"<section><div class="page"><p>a</p><div class="tip"><p>b</p></div></div></section>"#,
        );
        assert!(
            out.contains(r#"<section style="background-color:#fff">"#),
            "{out}"
        );
        assert!(
            out.contains(r#"<div class="page" style="background-color:#fff">"#),
            "{out}"
        );
        assert!(out.contains(r#"<div class="tip">"#), "{out}");
    }

    #[test]
    fn text_beside_the_only_element_ends_the_chain() {
        // The text is page content, so the element next to it is a box.
        let out = run(r#"Text<div class="tip"><p>b</p></div>"#);
        assert!(out.contains(r#"<div class="tip">"#), "{out}");
        let out = run(r#"<div class="page">Intro <div class="tip"><p>b</p></div></div>"#);
        assert!(
            out.contains(r#"<div class="page" style="background-color:#fff">"#),
            "{out}"
        );
        assert!(out.contains(r#"<div class="tip">"#), "{out}");
        // Whitespace between tags is not text.
        let out = run("\n  <div class=\"page\">\n <p>a</p></div>\n");
        assert!(
            out.contains(r#"<div class="page" style="background-color:#fff">"#),
            "{out}"
        );
    }

    #[test]
    fn a_sole_paragraph_is_not_a_wrapper() {
        let out = run(r#"<p style="margin:0">a</p>"#);
        assert!(out.contains(r#"<p style="margin:0">"#), "{out}");
    }

    #[test]
    fn an_existing_background_is_replaced() {
        let out = run(r#"<div style="background-color:#aaa;margin:0"><p>a</p><p>b</p></div>"#);
        assert!(
            out.contains(r#"<div style="margin:0;background-color:#fff">"#),
            "{out}"
        );
    }
}
