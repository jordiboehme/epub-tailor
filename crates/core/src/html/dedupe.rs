//! Duplicate-id removal, the first transform run per chapter.
//!
//! Gutenberg-style source XHTML is full of self-closing page-break anchors
//! like `<a id="Pagexv"/>`. Served through html5ever's HTML5 tree builder
//! (which [`crate::html::parse_xhtml`] uses), `<a .../>` is not honored as
//! self-closing: the `<a>` element stays open across the enclosing block's
//! close tag, and the adoption agency algorithm CLONES it (attributes and
//! all, including `id`) when reconstructing active formatting elements at the
//! start of each following block. One source anchor can therefore turn into
//! several `<a id="...">` elements in the parsed DOM, producing duplicate ids
//! in the serialized output (epubcheck RSC-005).
//!
//! This runs before every other transform, in particular before
//! [`crate::html::anchors::relocate_ids`], so downstream id bookkeeping
//! (relocation, aliasing, the anchor cap) only ever sees a document with
//! unique ids.

use std::collections::HashSet;

use kuchikiki::NodeRef;

use crate::html::dom::{get_attr, remove_attr};
use crate::report::Transformation;

/// Walk `doc` in document order; the first element bearing a given `id` keeps
/// it, every later element with the same `id` has just that attribute
/// removed - nothing else about that element (including an `href`) is
/// touched, so an `href` target still resolves to the surviving id.
///
/// Covers both the parser-clone artifact above and a genuine same-id
/// collision already present in the source markup; either way, only one
/// element may legally carry a given id.
pub(crate) fn dedupe_ids(doc: &NodeRef, report: &mut Vec<Transformation>, chapter_path: &str) {
    let mut seen: HashSet<String> = HashSet::new();
    let mut removed = 0usize;

    for node in doc.inclusive_descendants() {
        let Some(id) = get_attr(&node, "id") else {
            continue;
        };
        if !seen.insert(id) {
            remove_attr(&node, "id");
            removed += 1;
        }
    }

    if removed > 0 {
        report.push(Transformation {
            kind: "duplicate-ids-removed".to_string(),
            detail: format!("removed {removed} duplicate id attribute(s)"),
            file: Some(chapter_path.to_string()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::testutil::{doc_from_body, serialize};

    fn run(body: &str) -> (String, Vec<Transformation>) {
        let doc = doc_from_body(body);
        let mut report = Vec::new();
        dedupe_ids(&doc, &mut report, "ch.xhtml");
        (serialize(&doc), report)
    }

    /// The exact Gutenberg pattern: a self-closing `<a id="x"/>` that
    /// html5ever's adoption agency clones across a following block boundary.
    #[test]
    fn parser_cloned_anchor_id_collapses_to_one() {
        let doc = crate::html::parse_xhtml(
            br#"<html><body><p><a id="x"/>one</p><p>two</p></body></html>"#,
        )
        .expect("fixture parses");
        // Confirm the parser really did clone it before asserting the fix.
        assert_eq!(
            doc.inclusive_descendants()
                .filter(|n| get_attr(n, "id").as_deref() == Some("x"))
                .count(),
            2,
            "expected html5ever to have cloned the anchor id"
        );
        let mut report = Vec::new();
        dedupe_ids(&doc, &mut report, "ch.xhtml");
        let out = serialize(&doc);
        assert_eq!(out.matches(r#"id="x""#).count(), 1, "got: {out}");
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].kind, "duplicate-ids-removed");
    }

    /// A legitimate duplicate already present in the source (two distinct
    /// elements hand-given the same id) collapses the same way.
    #[test]
    fn genuine_source_duplicate_collapses_to_one() {
        let (out, report) = run(r#"<p id="dup">one</p><p id="dup">two</p>"#);
        assert_eq!(out.matches(r#"id="dup""#).count(), 1, "got: {out}");
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].detail, "removed 1 duplicate id attribute(s)");
    }

    /// The later element's `id` is removed - nothing else about it, in
    /// particular its `href` (the clone case: a formatting `<a>` reconstructed
    /// with both `id` and `href`).
    #[test]
    fn only_the_id_attribute_is_stripped_from_the_later_element() {
        let doc = crate::html::parse_xhtml(
            br##"<html><body><p><a id="x" href="#x"/>one</p><p>two</p></body></html>"##,
        )
        .expect("fixture parses");
        let mut report = Vec::new();
        dedupe_ids(&doc, &mut report, "ch.xhtml");
        let out = serialize(&doc);
        assert_eq!(out.matches(r#"id="x""#).count(), 1, "got: {out}");
        assert_eq!(out.matches(r##"href="#x""##).count(), 2, "got: {out}");
    }

    #[test]
    fn no_duplicates_is_a_no_op() {
        let (out, report) = run(r#"<p id="a">one</p><p id="b">two</p>"#);
        assert!(out.contains(r#"id="a""#));
        assert!(out.contains(r#"id="b""#));
        assert!(report.is_empty());
    }
}
