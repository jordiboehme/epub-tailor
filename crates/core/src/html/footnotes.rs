//! Normalize footnote/link references.
//!
//! The firmware only follows internal `href="#id"` links; `javascript:` hrefs
//! are inert. Many EPUB footnote widgets hide the real target in a `data-*` or
//! `onclick` attribute, so we recover a `#id` from any attribute and rewrite
//! the href to it. A `javascript:` link with no recoverable target is unwrapped
//! to plain text. External links (`http(s):`, `mailto:` ...) are left intact.

use kuchikiki::{NodeData, NodeRef};

use crate::html::dom::{collect_by_name, get_attr, set_attr, unwrap_element};
use crate::report::{Transformation, Warning};

/// Rewrite or unwrap every `javascript:` link in `doc`.
pub(crate) fn normalize_links(
    doc: &NodeRef,
    report: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
    chapter_path: &str,
) {
    for anchor in collect_by_name(doc, "a") {
        let Some(href) = get_attr(&anchor, "href") else {
            continue;
        };
        if !is_javascript(&href) {
            continue;
        }
        match anchor_ref_in_attrs(&anchor) {
            Some(fragment) => {
                set_attr(&anchor, "href", &fragment);
                report.push(Transformation {
                    kind: "link-rewritten".to_string(),
                    detail: format!("rewrote a javascript: link to {fragment}"),
                    file: Some(chapter_path.to_string()),
                });
            }
            None => {
                unwrap_element(&anchor);
                warnings.push(Warning {
                    message: "unwrapped a javascript: link with no recoverable anchor target"
                        .to_string(),
                    file: Some(chapter_path.to_string()),
                });
            }
        }
    }
}

fn is_javascript(href: &str) -> bool {
    href.trim_start()
        .to_ascii_lowercase()
        .starts_with("javascript:")
}

/// Find the first `#id` reference in any of the element's attribute values, in
/// attribute order.
fn anchor_ref_in_attrs(anchor: &NodeRef) -> Option<String> {
    let NodeData::Element(elem) = anchor.data() else {
        return None;
    };
    elem.attributes
        .borrow()
        .map
        .values()
        .find_map(|attr| find_anchor_ref(&attr.value))
}

/// Extract the first `#id` (with id characters `A-Za-z0-9_-.:`) from `value`.
fn find_anchor_ref(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'#' {
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < bytes.len() && is_id_char(bytes[end]) {
            end += 1;
        }
        if end > start {
            return Some(format!("#{}", &value[start..end]));
        }
    }
    None
}

fn is_id_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::testutil::{doc_from_body, serialize};

    fn run(body: &str) -> (String, Vec<Transformation>, Vec<Warning>) {
        let doc = doc_from_body(body);
        let mut report = Vec::new();
        let mut warnings = Vec::new();
        normalize_links(&doc, &mut report, &mut warnings, "ch.xhtml");
        (serialize(&doc), report, warnings)
    }

    #[test]
    fn javascript_with_data_target_is_rewritten_snapshot() {
        let (out, report, warnings) =
            run("<p><a href=\"javascript:void(0)\" data-target=\"#fn1\">1</a></p>");
        insta::assert_snapshot!(out);
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].kind, "link-rewritten");
        assert!(warnings.is_empty());
    }

    #[test]
    fn javascript_from_onclick_is_rewritten() {
        let (out, report, _) =
            run("<p><a href=\"javascript:void(0)\" onclick=\"show('#note7')\">x</a></p>");
        assert!(out.contains(r##"href="#note7""##), "got: {out}");
        assert_eq!(report.len(), 1);
    }

    #[test]
    fn javascript_without_target_is_unwrapped_snapshot() {
        let (out, report, warnings) =
            run("<p>see <a href=\"javascript:void(0)\">this</a> note</p>");
        insta::assert_snapshot!(out);
        assert!(report.is_empty());
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn external_and_internal_links_are_untouched() {
        let (out, report, warnings) =
            run("<p><a href=\"https://example.com\">ext</a> <a href=\"#sec\">int</a></p>");
        assert!(out.contains(r#"href="https://example.com""#), "got: {out}");
        assert!(out.contains(r##"href="#sec""##), "got: {out}");
        assert!(report.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn noteref_attribute_is_left_alone() {
        let (out, _, _) = run("<p><a epub:type=\"noteref\" href=\"notes.xhtml#n1\">1</a></p>");
        assert!(out.contains(r#"epub:type="noteref""#), "got: {out}");
        assert!(out.contains(r##"href="notes.xhtml#n1""##), "got: {out}");
    }
}
