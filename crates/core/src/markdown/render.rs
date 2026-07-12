//! comrak setup and the raw-HTML sanitize pass.
//!
//! Raw inline/block HTML is allowed through (`render.unsafe = true`, since
//! comrak's public field is the raw identifier `r#unsafe`) so authors can drop
//! down to HTML when Markdown cannot express something. That HTML is
//! untrusted, though, and the strict XHTML serializer plus epubcheck both
//! reject or choke on script-ish elements and inline event handlers, so
//! [`sanitize_chapter_dom`] strips them from the parsed chapter DOM before any
//! other processing sees it.

use comrak::Options;
use kuchikiki::{NodeData, NodeRef};

use crate::html::dom::collect_by_name;
use crate::report::Warning;

/// Elements removed entirely by [`sanitize_chapter_dom`]: script-ish or
/// interactive elements the device firmware cannot use and epubcheck rejects
/// in EPUB content documents.
const DANGEROUS_ELEMENTS: [&str; 5] = ["script", "iframe", "object", "embed", "form"];

/// Build the comrak [`Options`] used for every Markdown parse/render in this
/// crate: table, strikethrough, autolink, footnote and tasklist extensions,
/// frontmatter delimited by `---`, and raw HTML passed through unsafely (see
/// the module docs for why - [`sanitize_chapter_dom`] cleans it up after).
pub(crate) fn comrak_options() -> Options<'static> {
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.autolink = true;
    options.extension.footnotes = true;
    options.extension.tasklist = true;
    options.extension.front_matter_delimiter = Some("---".to_string());
    options.render.r#unsafe = true;
    options
}

/// Remove every [`DANGEROUS_ELEMENTS`] element and strip every `on*` attribute
/// from `doc`, in place. Meant to run immediately after a rendered Markdown
/// chapter is parsed into a DOM, before any other transform (including M3's)
/// sees it. Records one [`Warning`] per removed element or stripped attribute.
pub(crate) fn sanitize_chapter_dom(doc: &NodeRef, warnings: &mut Vec<Warning>, chapter_path: &str) {
    for name in DANGEROUS_ELEMENTS {
        for node in collect_by_name(doc, name) {
            node.detach();
            warnings.push(Warning {
                message: format!("removed a <{name}> element from raw HTML in the Markdown source"),
                file: Some(chapter_path.to_string()),
            });
        }
    }
    strip_event_handler_attrs(doc, warnings, chapter_path);
}

/// Strip every attribute whose name starts with `on` (`onclick`, `onerror`,
/// ...) from every element in `doc`.
fn strip_event_handler_attrs(doc: &NodeRef, warnings: &mut Vec<Warning>, chapter_path: &str) {
    for node in doc.inclusive_descendants() {
        let NodeData::Element(elem) = node.data() else {
            continue;
        };
        let to_remove: Vec<String> = elem
            .attributes
            .borrow()
            .map
            .keys()
            .map(|name| name.local.as_ref().to_string())
            .filter(|name| name.starts_with("on"))
            .collect();
        if to_remove.is_empty() {
            continue;
        }
        let mut attrs = elem.attributes.borrow_mut();
        for name in &to_remove {
            attrs.remove(name.as_str());
        }
        drop(attrs);
        for name in &to_remove {
            warnings.push(Warning {
                message: format!(
                    "stripped the `{name}` event handler attribute from raw HTML in the \
                     Markdown source"
                ),
                file: Some(chapter_path.to_string()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::{parse_xhtml, serialize_xhtml};

    fn doc(body: &str) -> NodeRef {
        let html = format!("<html><head><title>T</title></head><body>{body}</body></html>");
        parse_xhtml(html.as_bytes()).expect("fixture parses")
    }

    fn serialize(doc: &NodeRef) -> String {
        String::from_utf8(serialize_xhtml(doc)).expect("utf8")
    }

    #[test]
    fn comrak_options_enables_the_required_extensions() {
        let options = comrak_options();
        assert!(options.extension.table);
        assert!(options.extension.strikethrough);
        assert!(options.extension.autolink);
        assert!(options.extension.footnotes);
        assert!(options.extension.tasklist);
        assert_eq!(
            options.extension.front_matter_delimiter,
            Some("---".to_string())
        );
        assert!(options.render.r#unsafe);
    }

    #[test]
    fn script_element_is_removed_with_a_warning() {
        let d = doc(r#"<p>hi</p><script>alert(1)</script>"#);
        let mut warnings = Vec::new();
        sanitize_chapter_dom(&d, &mut warnings, "ch-001.xhtml");
        let out = serialize(&d);
        assert!(!out.contains("script"), "got: {out}");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("script"));
    }

    #[test]
    fn iframe_object_embed_and_form_are_all_removed() {
        let d = doc(
            r#"<iframe src="x"></iframe><object data="y"></object><embed src="z"/><form></form>"#,
        );
        let mut warnings = Vec::new();
        sanitize_chapter_dom(&d, &mut warnings, "ch-001.xhtml");
        let out = serialize(&d);
        for tag in ["iframe", "object", "embed", "form"] {
            assert!(!out.contains(tag), "expected no <{tag}>, got: {out}");
        }
        assert_eq!(warnings.len(), 4);
    }

    #[test]
    fn on_star_attributes_are_stripped() {
        let d = doc(r#"<p onclick="evil()" onmouseover="evil()">hi</p>"#);
        let mut warnings = Vec::new();
        sanitize_chapter_dom(&d, &mut warnings, "ch-001.xhtml");
        let out = serialize(&d);
        assert!(!out.contains("onclick"), "got: {out}");
        assert!(!out.contains("onmouseover"), "got: {out}");
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn ordinary_content_is_left_untouched() {
        let d = doc(r#"<p class="note">Hello <em>world</em></p>"#);
        let mut warnings = Vec::new();
        sanitize_chapter_dom(&d, &mut warnings, "ch-001.xhtml");
        let out = serialize(&d);
        assert!(out.contains(r#"<p class="note">Hello <em>world</em></p>"#));
        assert!(warnings.is_empty());
    }
}
