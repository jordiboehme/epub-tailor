//! Relocate `<style>` blocks and filter inline `style=""` attributes.
//!
//! The firmware never reads `<style>` in `<head>` (the whole head is skipped),
//! so any styling authored there is silently lost. This transform runs FIRST in
//! the chapter pipeline: it lifts the text of every `<style>` element out of the
//! chapter and returns it to the caller (which concatenates every chapter's CSS,
//! filters it, and writes it to an external sheet the device does read), and it
//! filters each inline `style=""` attribute down to the supported declaration
//! subset, dropping the attribute when nothing survives.

use kuchikiki::NodeRef;

use crate::css::filter_inline_style;
use crate::html::dom::{collect_by_name, get_attr, remove_attr, set_attr, text_content};
use crate::report::Transformation;

/// Extract and remove every `<style>` element, and filter every inline style
/// attribute, in `doc`. `keep_colors` keeps concrete inline color declarations
/// for the gray-tone remap pass to rewrite. Returns the concatenated raw text
/// of the removed `<style>` elements (empty when there were none) for the
/// caller to relocate.
pub(crate) fn relocate_styles(
    doc: &NodeRef,
    keep_colors: bool,
    report: &mut Vec<Transformation>,
    chapter_path: &str,
) -> String {
    let mut extracted = String::new();
    for style in collect_by_name(doc, "style") {
        let text = text_content(&style);
        if !text.trim().is_empty() {
            if !extracted.is_empty() {
                extracted.push('\n');
            }
            extracted.push_str(&text);
        }
        style.detach();
    }

    let mut rewritten = 0usize;
    let mut dropped = 0usize;
    for node in doc.inclusive_descendants() {
        let Some(style) = get_attr(&node, "style") else {
            continue;
        };
        match filter_inline_style(&style, keep_colors) {
            Some(kept) => {
                if kept != style {
                    set_attr(&node, "style", &kept);
                    rewritten += 1;
                }
            }
            None => {
                remove_attr(&node, "style");
                dropped += 1;
            }
        }
    }

    if rewritten + dropped > 0 {
        report.push(Transformation {
            kind: "inline-style-filtered".to_string(),
            detail: format!(
                "filtered {rewritten} inline style attribute(s) and dropped {dropped} empty one(s)"
            ),
            file: Some(chapter_path.to_string()),
        });
    }

    extracted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::testutil::{doc_from_body, serialize};

    fn run(body: &str) -> (String, String, Vec<Transformation>) {
        let doc = doc_from_body(body);
        let mut report = Vec::new();
        let extracted = relocate_styles(&doc, false, &mut report, "ch.xhtml");
        (serialize(&doc), extracted, report)
    }

    #[test]
    fn style_element_is_extracted_and_removed() {
        let (out, extracted, _) = run("<style>.a{color:red;text-align:center}</style><p>hi</p>");
        assert!(!out.contains("<style"), "style must be gone: {out}");
        assert!(
            extracted.contains(".a{color:red;text-align:center}"),
            "got: {extracted}"
        );
    }

    #[test]
    fn inline_style_is_filtered_to_the_subset() {
        let (out, _, report) = run("<p style=\"color:red;text-align:center;font-size:9px\">hi</p>");
        assert!(out.contains(r#"style="text-align:center""#), "got: {out}");
        assert!(!out.contains("color:red"), "got: {out}");
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].kind, "inline-style-filtered");
    }

    #[test]
    fn empty_inline_style_attribute_is_dropped() {
        let (out, _, _) = run("<p style=\"color:red;font-size:9px\">hi</p>");
        assert!(!out.contains("style="), "attribute must be dropped: {out}");
    }

    #[test]
    fn keep_colors_preserves_inline_color_declarations() {
        let doc = doc_from_body(r#"<p style="color:teal;font-size:9px">hi</p>"#);
        let mut report = Vec::new();
        relocate_styles(&doc, true, &mut report, "ch.xhtml");
        let out = serialize(&doc);
        assert!(out.contains(r#"style="color:teal""#), "got: {out}");
    }

    #[test]
    fn no_styles_yields_empty_and_no_report() {
        let (out, extracted, report) = run("<p>plain</p>");
        assert!(out.contains("<p>plain</p>"));
        assert!(extracted.is_empty());
        assert!(report.is_empty());
    }
}
