//! Hand-rolled strict-XHTML serializer.
//!
//! kuchikiki's own serializer emits HTML5 (unquoted attributes, void elements
//! without the trailing slash, unescaped `<`/`>` in some contexts), which
//! epubcheck rejects for `application/xhtml+xml` content documents. This
//! serializer instead emits strict, XML-well-formed XHTML.

use std::borrow::Cow;

use kuchikiki::{NodeData, NodeRef};

use crate::html::escape::escape_into;

/// The XHTML namespace, forced onto the root `<html>` element on output.
const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";
/// The EPUB Structural Semantics namespace, declared on the root iff any
/// element carries an `epub:`-prefixed attribute.
const EPUB_NS: &str = "http://www.idpf.org/2007/ops";
/// The XLink namespace, declared on the root iff any element carries an
/// `xlink:`-prefixed attribute (inline SVG image/anchor references).
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
/// The SVG and MathML namespaces, pinned onto inline foreign roots so the
/// subtree keeps its meaning when the output is parsed as strict XML.
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const MATHML_NS: &str = "http://www.w3.org/1998/Math/MathML";

/// Attribute prefixes with a guaranteed namespace declaration in the output:
/// `xml` and `xmlns` are built into XML itself, `epub` and `xlink` are
/// declared on the root when used. An attribute with any other prefix
/// (Calibre leftovers, Vue-style `:class`) cannot be represented in
/// namespace-valid XML and is dropped.
const DECLARED_PREFIXES: [&str; 4] = ["xml", "xmlns", "xlink", "epub"];

/// HTML void elements: serialized self-closing (`<br/>`), never with a
/// separate end tag.
const VOID_ELEMENTS: [&str; 13] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Serialize a parsed document as strict XHTML: an XML declaration and a
/// `<!DOCTYPE html>`, then the `<html>` element tree.
///
/// See the module docs for why this is hand-rolled rather than delegated to
/// kuchikiki. The output is guaranteed to be well-formed XML: text and
/// attribute values are escaped, void elements self-close, non-void empty
/// elements get an explicit end tag, comments are sanitized, and processing
/// instructions plus the parser's quirks doctype are dropped.
pub fn serialize_xhtml(doc: &NodeRef) -> Vec<u8> {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n");
    if let Some(html) = find_root_html(doc) {
        let needs = subtree_ns_needs(&html);
        write_element(&html, Some(&needs), &mut out);
    }
    out.into_bytes()
}

/// Serialize a single node's subtree with no XML declaration, doctype or root
/// namespace attributes: just the node's own markup, as it would appear
/// nested inside a full document. Used to measure how many bytes one
/// top-level `<body>` block would contribute, when planning an oversize
/// chapter split - not a substitute for [`serialize_xhtml`], whose namespace
/// declarations only make sense on the document root.
pub(crate) fn serialize_fragment(node: &NodeRef) -> Vec<u8> {
    let mut out = String::new();
    write_node(node, &mut out);
    out.into_bytes()
}

/// Find the root `<html>` element: the node itself if it is one, otherwise the
/// first `<html>` descendant of the parsed document.
fn find_root_html(doc: &NodeRef) -> Option<NodeRef> {
    if is_element_named(doc, "html") {
        return Some(doc.clone());
    }
    doc.descendants().find(|n| is_element_named(n, "html"))
}

fn is_element_named(node: &NodeRef, name: &str) -> bool {
    matches!(node.data(), NodeData::Element(e) if e.name.local.as_ref() == name)
}

/// Which optional namespaces the root must declare, found by scanning the
/// subtree for `epub:`- and `xlink:`-prefixed attribute names (as they will
/// be emitted).
struct RootNs {
    epub: bool,
    xlink: bool,
}

fn subtree_ns_needs(root: &NodeRef) -> RootNs {
    let mut needs = RootNs {
        epub: false,
        xlink: false,
    };
    for node in std::iter::once(root.clone()).chain(root.descendants()) {
        let NodeData::Element(e) = node.data() else {
            continue;
        };
        let elem_name = e.name.local.as_ref();
        if is_emittable_name(elem_name) {
            needs.epub |= elem_name.starts_with("epub:");
            needs.xlink |= elem_name.starts_with("xlink:");
        }
        for (key, attr) in &e.attributes.borrow().map {
            let Some(name) = emittable_attr_name(key.local.as_ref(), attr) else {
                continue;
            };
            needs.epub |= name.starts_with("epub:");
            needs.xlink |= name.starts_with("xlink:");
        }
    }
    needs
}

/// The element's own `xmlns="..."` value, whichever representation the parse
/// produced (a foreign-content parse namespaces the attribute, a plain HTML
/// parse leaves it a literal local name).
fn source_xmlns(attributes: &kuchikiki::Attributes) -> Option<&str> {
    attributes.map.iter().find_map(|(key, attr)| {
        (emittable_attr_name(key.local.as_ref(), attr).as_deref() == Some("xmlns"))
            .then_some(attr.value.as_str())
    })
}

/// Serialize an element and its subtree. `root` is `Some` on the document's
/// `<html>` element, which gets the XHTML namespace (plus `xmlns:epub` and
/// `xmlns:xlink` as needed) forced onto its opening tag.
fn write_element(node: &NodeRef, root: Option<&RootNs>, out: &mut String) {
    let NodeData::Element(elem) = node.data() else {
        return;
    };
    let name = elem.name.local.as_ref();
    if !is_emittable_name(name) {
        // A tag name no XML reader accepts (a tokenizer artifact of broken
        // markup, e.g. `<a\u{FFFD}-->`, or an undeclarable prefix): emit the
        // children without the unrepresentable wrapper.
        for child in node.children() {
            write_node(&child, out);
        }
        return;
    }
    out.push('<');
    out.push_str(name);

    let attributes = elem.attributes.borrow();
    // The namespace declaration this opening tag must carry: XHTML on the
    // document root, SVG/MathML on inline foreign roots (falling back to the
    // canonical namespace when the source declared none). Pinning it right
    // after the tag name keeps parse∘serialize a fixed point no matter where
    // the parsed declaration sat in the attribute map.
    let pinned_ns: Option<String> = if root.is_some() {
        Some(XHTML_NS.to_string())
    } else {
        match name {
            "svg" => Some(SVG_NS),
            "math" => Some(MATHML_NS),
            _ => None,
        }
        .map(|canonical| source_xmlns(&attributes).unwrap_or(canonical).to_string())
    };
    if let Some(ns) = &pinned_ns {
        out.push_str(" xmlns=\"");
        escape_into(ns, true, out);
        out.push('"');
    }
    if let Some(needs) = root {
        if needs.epub {
            out.push_str(" xmlns:epub=\"");
            out.push_str(EPUB_NS);
            out.push('"');
        }
        if needs.xlink {
            out.push_str(" xmlns:xlink=\"");
            out.push_str(XLINK_NS);
            out.push('"');
        }
    }
    for (key, attr) in &attributes.map {
        let Some(attr_name) = emittable_attr_name(key.local.as_ref(), attr) else {
            continue;
        };
        // Skip declarations already pinned or forced above so none is ever
        // duplicated (a duplicate attribute is malformed XML).
        if pinned_ns.is_some() && attr_name == "xmlns" {
            continue;
        }
        if root.is_some() && matches!(attr_name.as_ref(), "xmlns:epub" | "xmlns:xlink") {
            continue;
        }
        out.push(' ');
        out.push_str(&attr_name);
        out.push_str("=\"");
        escape_into(&attr.value, true, out);
        out.push('"');
    }
    drop(attributes);

    if VOID_ELEMENTS.contains(&name) {
        out.push_str("/>");
        return;
    }

    out.push('>');
    for child in node.children() {
        write_node(&child, out);
    }
    out.push_str("</");
    out.push_str(name);
    out.push('>');
}

/// The attribute's name as written to the output, or `None` when it cannot be
/// represented in well-formed, namespace-valid XML and must be dropped.
///
/// html5ever's foreign-content adjustment hands a bare `xmlns` on `<svg>` or
/// `<math>` to the serializer with an *empty* prefix; emitting the colon for
/// it anyway is what 0.4.0/0.4.1 did, corrupting books with the malformed
/// attribute name `:xmlns`.
fn emittable_attr_name<'a>(local: &'a str, attr: &kuchikiki::Attribute) -> Option<Cow<'a, str>> {
    let name: Cow<'a, str> = match &attr.prefix {
        Some(prefix) if !prefix.is_empty() => Cow::Owned(format!("{}:{local}", prefix.as_ref())),
        _ => Cow::Borrowed(local),
    };
    is_emittable_name(&name).then_some(name)
}

/// Whether an element or attribute name can be written into namespace-valid
/// XML: an `NCName`, or `prefix:NCName` with a prefix this serializer
/// guarantees is declared.
fn is_emittable_name(name: &str) -> bool {
    match name.split_once(':') {
        None => is_ncname(name),
        Some((prefix, rest)) => DECLARED_PREFIXES.contains(&prefix) && is_ncname(rest),
    }
}

/// Pragmatic `NCName` check (an XML name, no colon): enough to guarantee the
/// name parses as one QName part in every strict XML reader. Also used by
/// [`crate::validate`]'s QName scan.
pub(crate) fn is_ncname(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '-' | '.' | '_'))
}

fn write_node(node: &NodeRef, out: &mut String) {
    match node.data() {
        NodeData::Element(_) => write_element(node, None, out),
        NodeData::Text(text) => escape_into(&text.borrow(), false, out),
        NodeData::Comment(comment) => write_comment(&comment.borrow(), out),
        // Our own XML declaration and doctype are emitted at the top; any
        // parser-inserted quirks doctype or processing instruction is dropped.
        NodeData::Doctype(_) | NodeData::ProcessingInstruction(_) => {}
        NodeData::Document(_) | NodeData::DocumentFragment => {
            for child in node.children() {
                write_node(&child, out);
            }
        }
    }
}

/// Write an XML comment, sanitizing the content so it stays well-formed and
/// round-trips through this crate's own (HTML5) parser:
///
/// - control characters invalid in XML are dropped;
/// - every hyphen that immediately follows another hyphen gets a space
///   inserted before it, so no run of 2+ hyphens survives as a literal `--`
///   (an XML comment may not contain `--`);
/// - a trailing `-` gains a space, since it would otherwise merge with the
///   closing `-->` into an illegal `--->`;
/// - a leading `>` or `->` gains a space before the `>`, since HTML5's
///   tokenizer (unlike the XML grammar) treats a `>` as the very first or
///   second character right after `<!--` as an "abrupt-closing-of-empty-comment"
///   — it would close the comment immediately, empty, on re-parse.
///
/// The hyphen-run pass is a single left-to-right scan tracking only "was the
/// previous character emitted a hyphen": every hyphen run of length N
/// therefore comes out as N hyphens separated by single spaces, which by
/// construction cannot contain "--" anywhere, no matter how long the run.
/// `str::replace("--", "- -")` is *not* equivalent to this: it scans for
/// non-overlapping "--" matches, so odd-length runs leave a residual "--"
/// (`"---"` -> `"- --"`, `"-----"` -> `"- - --"`), which is exactly the bug
/// this function fixes.
fn write_comment(content: &str, out: &mut String) {
    let mut cleaned = String::with_capacity(content.len());
    let mut prev_was_hyphen = false;
    for c in content
        .chars()
        .filter(|c| (*c as u32) >= 0x20 || matches!(c, '\t' | '\n' | '\r'))
    {
        if c == '-' && prev_was_hyphen {
            cleaned.push(' ');
        }
        cleaned.push(c);
        prev_was_hyphen = c == '-';
    }
    if cleaned.ends_with('-') {
        cleaned.push(' ');
    }
    if cleaned.starts_with('>') {
        cleaned.insert(0, ' ');
    } else if cleaned.starts_with("->") {
        cleaned.insert(1, ' ');
    }
    debug_assert!(
        !cleaned.contains("--"),
        "sanitized comment still has --: {cleaned:?}"
    );
    debug_assert!(
        !cleaned.ends_with('-'),
        "sanitized comment still ends with -: {cleaned:?}"
    );
    debug_assert!(
        !cleaned.starts_with('>') && !cleaned.starts_with("->"),
        "sanitized comment still starts with a re-parse-unsafe > : {cleaned:?}"
    );
    out.push_str("<!--");
    out.push_str(&cleaned);
    out.push_str("-->");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::parse::parse_xhtml;

    fn round(src: &[u8]) -> String {
        String::from_utf8(serialize_xhtml(&parse_xhtml(src).expect("parse"))).expect("utf8")
    }

    #[test]
    fn emits_xml_declaration_and_doctype() {
        let out = round(b"<html><head><title>T</title></head><body></body></html>");
        assert!(
            out.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html"),
            "got: {out}"
        );
    }

    #[test]
    fn void_elements_self_close() {
        let out = round(b"<html><body><br><hr><img src=\"a.jpg\"></body></html>");
        assert!(out.contains("<br/>"), "got: {out}");
        assert!(out.contains("<hr/>"), "got: {out}");
        assert!(out.contains("<img src=\"a.jpg\"/>"), "got: {out}");
    }

    #[test]
    fn attribute_values_are_escaped() {
        let out =
            round(br#"<html><body><img src="a.jpg" alt="Tom & Jerry's <cat>"></body></html>"#);
        assert!(
            out.contains(r#"alt="Tom &amp; Jerry's &lt;cat&gt;""#),
            "got: {out}"
        );
    }

    #[test]
    fn epub_type_preserved_and_xmlns_epub_declared() {
        let out = round(
            br#"<html><body><nav epub:type="toc"><ol><li><a href="c.xhtml">C</a></li></ol></nav></body></html>"#,
        );
        assert!(out.contains(r#"epub:type="toc""#), "got: {out}");
        assert!(
            out.contains(r#"xmlns:epub="http://www.idpf.org/2007/ops""#),
            "got: {out}"
        );
        assert!(
            out.contains(r#"xmlns="http://www.w3.org/1999/xhtml""#),
            "got: {out}"
        );
    }

    #[test]
    fn no_xmlns_epub_when_no_epub_attrs() {
        let out = round(b"<html><body><p>hi</p></body></html>");
        assert!(
            out.contains(r#"xmlns="http://www.w3.org/1999/xhtml""#),
            "got: {out}"
        );
        assert!(!out.contains("xmlns:epub"), "got: {out}");
    }

    /// Strict-XML gate for the foreign-content tests: the exact bug this
    /// guards against (`:xmlns`, undeclared prefixes, duplicate attributes)
    /// parses fine with the lenient HTML parser, so only a strict XML parse
    /// proves well-formedness.
    fn assert_strict_xml(out: &str) {
        let opts = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        };
        if let Err(e) = roxmltree::Document::parse_with_options(out, opts) {
            panic!("output is not well-formed XML: {e}\n{out}");
        }
    }

    fn assert_fixed_point(out: &str) {
        let again = round(out.as_bytes());
        assert_eq!(out, again, "parse∘serialize must be a fixed point");
    }

    #[test]
    fn inline_svg_xmlns_is_preserved() {
        let out = round(
            br#"<html><body><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><circle r="4"></circle></svg></body></html>"#,
        );
        assert!(
            out.contains(r#"<svg xmlns="http://www.w3.org/2000/svg""#),
            "got: {out}"
        );
        assert!(!out.contains(":xmlns"), "got: {out}");
        assert_strict_xml(&out);
        assert_fixed_point(&out);
    }

    #[test]
    fn inline_math_xmlns_is_preserved() {
        let out = round(
            br#"<html><body><math xmlns="http://www.w3.org/1998/Math/MathML"><mi>x</mi></math></body></html>"#,
        );
        assert!(
            out.contains(r#"<math xmlns="http://www.w3.org/1998/Math/MathML""#),
            "got: {out}"
        );
        assert!(!out.contains(":xmlns"), "got: {out}");
        assert_strict_xml(&out);
        assert_fixed_point(&out);
    }

    #[test]
    fn inline_svg_without_xmlns_gains_canonical_namespace() {
        let out = round(
            br#"<html><body><svg viewBox="0 0 10 10"><circle r="4"></circle></svg></body></html>"#,
        );
        assert!(
            out.contains(r#"<svg xmlns="http://www.w3.org/2000/svg""#),
            "got: {out}"
        );
        assert_strict_xml(&out);
        assert_fixed_point(&out);
    }

    #[test]
    fn xlink_href_forces_root_declaration() {
        let out = round(
            br#"<html><body><svg viewBox="0 0 4 4"><a xlink:href="c.xhtml"><circle r="1"></circle></a></svg></body></html>"#,
        );
        assert!(out.contains(r#"xlink:href="c.xhtml""#), "got: {out}");
        assert!(
            out.contains(r#"xmlns:xlink="http://www.w3.org/1999/xlink""#),
            "got: {out}"
        );
        assert_strict_xml(&out);
        assert_fixed_point(&out);
    }

    #[test]
    fn existing_xlink_declaration_is_not_duplicated() {
        let out = round(
            br#"<html><body><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><a xlink:href="c.xhtml"><circle r="1"></circle></a></svg></body></html>"#,
        );
        // A duplicated attribute name on one element is malformed XML, so the
        // strict parse is the real assertion here.
        assert_strict_xml(&out);
        assert_fixed_point(&out);
    }

    #[test]
    fn corrupted_colon_xmlns_heals_to_a_declaration() {
        // The exact malformation epub-tailor 0.4.0/0.4.1 wrote into books.
        let out = round(
            br#"<html><body><svg :xmlns="http://www.w3.org/2000/svg" height="100%" viewBox="0 0 671 1024" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="cover.jpg" width="671"></image></svg></body></html>"#,
        );
        assert!(
            out.contains(r#"<svg xmlns="http://www.w3.org/2000/svg""#),
            "got: {out}"
        );
        assert!(!out.contains(":xmlns"), "got: {out}");
        assert_strict_xml(&out);
        assert_fixed_point(&out);
    }

    #[test]
    fn colon_xmlns_beside_real_xmlns_drops_the_corrupt_twin() {
        let out = round(
            br#"<html><body><svg :xmlns="http://bogus.example/ns" xmlns="http://www.w3.org/2000/svg"><circle r="1"></circle></svg></body></html>"#,
        );
        assert!(
            out.contains(r#"<svg xmlns="http://www.w3.org/2000/svg""#),
            "got: {out}"
        );
        assert!(!out.contains("bogus.example"), "got: {out}");
        assert!(!out.contains(":xmlns"), "got: {out}");
        assert_strict_xml(&out);
        assert_fixed_point(&out);
    }

    #[test]
    fn unknown_prefix_attributes_are_dropped() {
        let out = round(
            br#"<html><body><p calibre:timestamp="2024" :class="x" @click="go()" class="ok">t</p></body></html>"#,
        );
        assert!(out.contains(r#"<p class="ok">"#), "got: {out}");
        assert!(!out.contains("calibre:"), "got: {out}");
        assert!(!out.contains(":class"), "got: {out}");
        assert!(!out.contains("@click"), "got: {out}");
        assert_strict_xml(&out);
        assert_fixed_point(&out);
    }

    #[test]
    fn garbage_element_name_is_unwrapped() {
        // Found by the serialize_roundtrip proptest: an abruptly closed
        // comment followed by a control character makes html5ever produce an
        // element literally named `a\u{FFFD}--`, which no XML reader accepts
        // as a tag name. The children survive; the unrepresentable wrapper
        // does not.
        let out = round(b"<html><body><!--><a\x00-->text</body></html>");
        assert!(!out.contains('\u{FFFD}'), "got: {out}");
        assert!(out.contains("text"), "got: {out}");
        assert_strict_xml(&out);
        assert_fixed_point(&out);
    }

    #[test]
    fn epub_prefixed_element_declares_the_namespace() {
        let out = round(
            b"<html><body><epub:switch><epub:default><p>x</p></epub:default></epub:switch></body></html>",
        );
        assert!(out.contains("<epub:switch>"), "got: {out}");
        assert!(
            out.contains(r#"xmlns:epub="http://www.idpf.org/2007/ops""#),
            "got: {out}"
        );
        assert_strict_xml(&out);
        assert_fixed_point(&out);
    }

    #[test]
    fn xml_and_epub_prefixes_survive_hardening() {
        let out =
            round(br#"<html><body><p xml:lang="de" epub:type="pagebreak">x</p></body></html>"#);
        assert!(out.contains(r#"xml:lang="de""#), "got: {out}");
        assert!(out.contains(r#"epub:type="pagebreak""#), "got: {out}");
        assert_strict_xml(&out);
        assert_fixed_point(&out);
    }

    #[test]
    fn text_nodes_are_escaped() {
        let out = round(b"<html><body><p>fish &amp; chips &lt; food</p></body></html>");
        assert!(out.contains(">fish &amp; chips &lt; food<"), "got: {out}");
    }

    #[test]
    fn empty_non_void_element_gets_explicit_close() {
        let out = round(b"<html><body><div></div></body></html>");
        assert!(out.contains("<div></div>"), "got: {out}");
    }

    #[test]
    fn nested_structure_round_trips_identically() {
        let src = br#"<html><head><title>T</title></head><body><div class="a"><p>Hello <em>world</em></p></div></body></html>"#;
        let s1 = serialize_xhtml(&parse_xhtml(src).expect("parse"));
        let s2 = serialize_xhtml(&parse_xhtml(&s1).expect("reparse"));
        assert_eq!(s1, s2, "parse∘serialize must be a fixed point");
        let text = String::from_utf8(s1).unwrap();
        assert!(text.contains("<div class=\"a\">"), "got: {text}");
        assert!(text.contains("<em>world</em>"), "got: {text}");
    }

    #[test]
    fn comments_are_preserved_and_double_hyphens_sanitized() {
        let out = round(b"<html><body><!--a--b--></body></html>");
        assert!(out.contains("<!--a- -b-->"), "got: {out}");
    }

    /// Directly exercise `write_comment` for exact-byte expectations: the
    /// full `<!--...-->` it produces must never contain `--` in its content
    /// and must never end the content with `-` (which would merge with the
    /// closing delimiter into `--->`).
    fn sanitize(content: &str) -> String {
        let mut out = String::new();
        write_comment(content, &mut out);
        out
    }

    /// Every case here is a regression test for the old
    /// `content.replace("--", "- -")` sanitizer, which is non-overlapping:
    /// odd-length hyphen runs left a residual `--` (`"---"` -> `"- --"`,
    /// still malformed XML).
    #[test]
    fn hyphen_run_of_two_is_split() {
        assert_eq!(sanitize("--"), "<!--- - -->");
    }

    #[test]
    fn hyphen_run_of_three_is_split() {
        assert_eq!(sanitize("---"), "<!--- - - -->");
    }

    #[test]
    fn hyphen_run_of_four_is_split() {
        assert_eq!(sanitize("----"), "<!--- - - - -->");
    }

    #[test]
    fn hyphen_run_of_five_is_split() {
        assert_eq!(sanitize("-----"), "<!--- - - - - -->");
    }

    #[test]
    fn hyphen_run_inside_text_is_split() {
        assert_eq!(sanitize("a---b"), "<!--a- - -b-->");
    }

    #[test]
    fn trailing_hyphen_gains_a_space() {
        assert_eq!(sanitize("ab-"), "<!--ab- -->");
    }

    #[test]
    fn leading_hyphen_is_left_alone() {
        // A single leading `-` does not combine with the opening `<!--` to
        // form an illegal `--`: the XML comment grammar only forbids `--`
        // that is not immediately followed by `>`, so content starting with
        // one hyphen (followed by a non-hyphen) is well-formed as-is.
        assert_eq!(sanitize("-ab"), "<!---ab-->");
    }

    #[test]
    fn mixed_leading_runs_and_trailing_hyphen() {
        assert_eq!(sanitize("-a--b---c-"), "<!---a- -b- - -c- -->");
    }

    /// Separate from the hyphen-run finding: a sanitized comment must not
    /// *begin* with `>` or `->` either, since HTML5's tokenizer (though not
    /// the XML grammar) special-cases a `>` as the first or second character
    /// right after `<!--` as an "abrupt-closing-of-empty-comment" -- it ends
    /// the comment immediately, empty, discarding everything else. Content
    /// like this is reachable from real HTML: html5ever happily parses
    /// `<!--\x01>-->` (a control char, then `>`, inside a comment) into a
    /// comment node whose data is `"\x01>"`; once the control char is
    /// stripped, the cleaned content starts with `>`, and re-parsing our own
    /// output would silently truncate it. Found via the extended
    /// `serialize_roundtrip` proptest (see `comment_strategy`).
    #[test]
    fn leading_gt_after_delimiter_gains_a_space() {
        assert_eq!(sanitize(">foo"), "<!-- >foo-->");
    }

    #[test]
    fn leading_hyphen_gt_after_delimiter_gains_a_space() {
        assert_eq!(sanitize("->foo"), "<!--- >foo-->");
    }

    #[test]
    fn leading_gt_round_trips_to_a_fixed_point() {
        for content in [">foo", "->foo", "\u{1}>--"] {
            let src = format!("<html><body><!--{content}--></body></html>");
            let s1 = serialize_xhtml(&parse_xhtml(src.as_bytes()).expect("parse"));
            let s2 = serialize_xhtml(&parse_xhtml(&s1).expect("reparse"));
            assert_eq!(
                s1,
                s2,
                "parse∘serialize must be a fixed point for comment {content:?}\nfirst: {}",
                String::from_utf8_lossy(&s1)
            );
        }
    }

    #[test]
    fn sanitized_output_never_contains_double_hyphen_or_trailing_hyphen() {
        for content in [
            "--",
            "---",
            "----",
            "-----",
            "a---b",
            "ab-",
            "-ab",
            "-a--b---c-",
            "",
            "-",
            "no-hyphens-here",
            ">foo",
            "->foo",
            "\u{1}>--",
        ] {
            let out = sanitize(content);
            let inner = &out[4..out.len() - 3]; // strip "<!--" and "-->"
            assert!(
                !inner.contains("--"),
                "content {content:?} -> {out} still has --"
            );
            assert!(
                !inner.ends_with('-'),
                "content {content:?} -> {out} still ends with -"
            );
            assert!(
                !inner.starts_with('>') && !inner.starts_with("->"),
                "content {content:?} -> {out} still starts with a re-parse-unsafe >"
            );
        }
    }

    /// For each regression case, verify parse-then-serialize of the
    /// sanitized output is a fixed point: re-parsing our own sanitized
    /// comment through the real HTML tokenizer and serializing again must
    /// reproduce byte-identical output (`write_comment` applied to already
    /// sanitized content is a no-op).
    #[test]
    fn sanitized_comments_round_trip_to_a_fixed_point() {
        for content in [
            "--",
            "---",
            "----",
            "-----",
            "a---b",
            "ab-",
            "-ab",
            "-a--b---c-",
        ] {
            let src = format!("<html><body><!--{content}--></body></html>");
            let s1 = serialize_xhtml(&parse_xhtml(src.as_bytes()).expect("parse"));
            let s2 = serialize_xhtml(&parse_xhtml(&s1).expect("reparse"));
            assert_eq!(
                s1,
                s2,
                "parse∘serialize must be a fixed point for comment {content:?}\nfirst: {}",
                String::from_utf8_lossy(&s1)
            );
        }
    }
}
