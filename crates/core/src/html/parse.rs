//! Parsing XHTML/HTML bytes into a kuchikiki DOM.

use kuchikiki::traits::*;
use kuchikiki::{Attribute, ExpandedName, NodeData, NodeRef};

use crate::error::ConvertError;

/// Parse XHTML/HTML `bytes` into a kuchikiki DOM tree.
///
/// The input is expected to already be UTF-8 (M1's reader normalizes every
/// text resource). html5ever's parser recovers from malformed markup rather
/// than failing, so the only error case is genuinely undecodable input.
///
/// Documents damaged by epub-tailor 0.4.0/0.4.1's `:xmlns` serializer bug are
/// healed right after parsing (see [`heal_corrupt_xmlns`]), so re-fitting a
/// corrupted book repairs it.
///
/// # Errors
/// Returns [`ConvertError::InvalidEpub`] if `bytes` is not valid UTF-8.
pub fn parse_xhtml(bytes: &[u8]) -> Result<NodeRef, ConvertError> {
    let text = std::str::from_utf8(bytes).map_err(|e| {
        ConvertError::InvalidEpub(format!("XHTML document is not valid UTF-8: {e}"))
    })?;
    let doc = kuchikiki::parse_html().one(text);
    heal_corrupt_xmlns(&doc);
    Ok(doc)
}

/// Heal the corrupted namespace declarations epub-tailor 0.4.0/0.4.1 wrote:
/// serializing html5ever's empty-prefix `xmlns` produced the malformed
/// attribute name `:xmlns`. Parsed leniently that comes back as a plain
/// attribute whose local name is `:xmlns` - re-emitting it verbatim would
/// make the corruption a stable fixed point, so it is repaired here at the
/// parse boundary, for every consumer of the DOM.
fn heal_corrupt_xmlns(doc: &NodeRef) {
    let corrupt_key = ExpandedName::new("", ":xmlns");
    for node in doc.inclusive_descendants() {
        let NodeData::Element(elem) = node.data() else {
            continue;
        };
        let mut attrs = elem.attributes.borrow_mut();
        // shift_remove, not remove: kuchikiki's map is insertion-ordered and
        // its remove is indexmap's order-perturbing swap_remove.
        let Some(corrupt) = attrs.map.shift_remove(&corrupt_key) else {
            continue;
        };
        let declared = attrs.map.contains_key(&ExpandedName::new("", "xmlns"))
            || attrs
                .map
                .contains_key(&ExpandedName::new("http://www.w3.org/2000/xmlns/", "xmlns"));
        if !declared {
            // Reinsert in the exact shape html5ever's foreign-content
            // adjustment produces, so downstream code sees one canonical form.
            attrs.map.insert(
                ExpandedName::new("http://www.w3.org/2000/xmlns/", "xmlns"),
                Attribute {
                    prefix: Some("".into()),
                    value: corrupt.value,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wellformed_document() {
        let doc = parse_xhtml(b"<html><head><title>T</title></head><body><p>Hi</p></body></html>")
            .expect("should parse");
        assert!(
            doc.select("p").expect("valid selector").count() == 1,
            "expected exactly one <p>"
        );
    }

    #[test]
    fn recovers_from_malformed_markup() {
        // Unclosed tags, stray text: html5ever recovers instead of erroring.
        let doc = parse_xhtml(b"<html><body><p>one<p>two<b>bold").expect("should recover");
        assert_eq!(doc.select("p").expect("valid selector").count(), 2);
    }

    #[test]
    fn rejects_invalid_utf8() {
        let err = parse_xhtml(&[0xFF, 0xFE, 0x00]).expect_err("invalid UTF-8 should error");
        assert!(matches!(err, ConvertError::InvalidEpub(_)));
    }

    #[test]
    fn heals_corrupt_colon_xmlns_into_a_namespace_declaration() {
        let doc = parse_xhtml(
            br#"<html><body><svg :xmlns="http://www.w3.org/2000/svg"></svg></body></html>"#,
        )
        .expect("parse");
        let svg = doc.select_first("svg").expect("svg element");
        let attrs = svg.attributes.borrow();
        assert!(
            !attrs
                .map
                .contains_key(&kuchikiki::ExpandedName::new("", ":xmlns")),
            "corrupt attribute must be gone"
        );
        let healed = attrs
            .map
            .get(&kuchikiki::ExpandedName::new(
                "http://www.w3.org/2000/xmlns/",
                "xmlns",
            ))
            .expect("healed declaration");
        assert_eq!(healed.value, "http://www.w3.org/2000/svg");
        assert_eq!(healed.prefix.as_deref(), Some(""));
    }

    #[test]
    fn corrupt_colon_xmlns_never_shadows_a_real_declaration() {
        let doc = parse_xhtml(
            br#"<html><body><svg :xmlns="http://bogus.example/ns" xmlns="http://www.w3.org/2000/svg"></svg></body></html>"#,
        )
        .expect("parse");
        let svg = doc.select_first("svg").expect("svg element");
        let attrs = svg.attributes.borrow();
        assert!(
            !attrs
                .map
                .contains_key(&kuchikiki::ExpandedName::new("", ":xmlns")),
            "corrupt attribute must be gone"
        );
        let real = attrs
            .map
            .get(&kuchikiki::ExpandedName::new(
                "http://www.w3.org/2000/xmlns/",
                "xmlns",
            ))
            .expect("real declaration survives");
        assert_eq!(real.value, "http://www.w3.org/2000/svg");
    }
}
