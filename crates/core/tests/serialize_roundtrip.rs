//! Property tests for the strict-XHTML serializer: for arbitrary trees built
//! from a small tag/attribute/text alphabet, `serialize_xhtml` is a fixed
//! point under `parse_xhtml` — that is, re-parsing serialized output and
//! serializing it again yields byte-identical output — and its output is
//! well-formed, namespace-valid XML. The alphabet includes inline SVG/MathML
//! with namespace declarations, `xlink:` attributes and hostile attribute
//! names (`:xmlns`, `calibre:x`): the fixed-point property alone cannot catch
//! a serializer writing stable garbage (0.4.0/0.4.1's `:xmlns` corruption was
//! such a fixed point), which is what the strict-XML property is for.

use epub_tailor_core::{find_invalid_qname, parse_xhtml, serialize_xhtml};
use proptest::prelude::*;

/// A node in a randomly generated document fragment, drawn from a deliberately
/// small alphabet (no `<script>`/`<style>`, whose RAWTEXT content parsing would
/// make escaping non-idempotent, and which the brief covers with unit tests).
#[derive(Debug, Clone)]
enum Node {
    Text(String),
    Comment(String),
    Void(&'static str, Vec<(&'static str, String)>),
    Elem(&'static str, Vec<(&'static str, String)>, Vec<Node>),
    Foreign {
        kind: ForeignKind,
        declare_ns: bool,
        declare_xlink: bool,
        attrs: Vec<(&'static str, String)>,
        children: Vec<ForeignChild>,
    },
}

/// One level-deep foreign child element: (tag, attrs, text content).
type ForeignChild = (&'static str, Vec<(&'static str, String)>, String);

#[derive(Debug, Clone, Copy)]
enum ForeignKind {
    Svg,
    Math,
}

const CONTAINER_TAGS: &[&str] = &["div", "p", "span", "em", "strong", "blockquote", "li"];
const VOID_TAGS: &[&str] = &["br", "hr", "img"];
const ATTR_NAMES: &[&str] = &["class", "id", "title", "epub:type", "data-x"];
const SVG_CHILD_TAGS: &[&str] = &["circle", "rect", "g"];
const MATH_CHILD_TAGS: &[&str] = &["mi", "mrow", "mn"];
/// Foreign-content attribute names: html5ever's case adjustment (`viewBox`),
/// namespaced forms (`xlink:href`, `xml:lang`), plus the hostile names the
/// serializer must drop — `:xmlns` is the exact 0.4.0/0.4.1 corruption fed
/// back in as input.
const FOREIGN_ATTR_NAMES: &[&str] = &[
    "viewBox",
    "fill",
    "xlink:href",
    "xml:lang",
    "class",
    ":xmlns",
    ":class",
    "calibre:x",
];

/// Text drawn to include the characters the serializer must escape, plus a
/// sample of the C0 control characters that are invalid in XML 1.0 and must
/// be dropped (see `escape_into`/`write_comment`) rather than passed through.
fn text_strategy() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            Just('a'),
            Just('Z'),
            Just(' '),
            Just('&'),
            Just('<'),
            Just('>'),
            Just('"'),
            Just('\''),
            Just('é'),
            Just('\n'),
            Just('\u{0}'),
            Just('\u{1}'),
            Just('\u{7}'),
            Just('\u{1f}'),
        ],
        0..8,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

fn attr_value_strategy() -> impl Strategy<Value = String> {
    text_strategy()
}

/// Comment content drawn to include hyphen runs of varying length, framed by
/// arbitrary text on either side. This is the shape that broke the old
/// `content.replace("--", "- -")` sanitizer: `str::replace` is
/// non-overlapping, so odd-length runs (e.g. `"---"`) left a residual `--` in
/// the output, which is malformed inside an XML comment.
fn comment_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        text_strategy(),
        (text_strategy(), 2..6usize, text_strategy())
            .prop_map(|(pre, n, post)| format!("{pre}{}{post}", "-".repeat(n))),
    ]
}

fn attrs_strategy() -> impl Strategy<Value = Vec<(&'static str, String)>> {
    proptest::collection::vec(
        (proptest::sample::select(ATTR_NAMES), attr_value_strategy()),
        0..3,
    )
}

fn foreign_attrs_strategy() -> impl Strategy<Value = Vec<(&'static str, String)>> {
    proptest::collection::vec(
        (
            proptest::sample::select(FOREIGN_ATTR_NAMES),
            attr_value_strategy(),
        ),
        0..3,
    )
}

fn foreign_strategy() -> impl Strategy<Value = Node> {
    prop_oneof![Just(ForeignKind::Svg), Just(ForeignKind::Math)].prop_flat_map(|kind| {
        let child_tags = match kind {
            ForeignKind::Svg => SVG_CHILD_TAGS,
            ForeignKind::Math => MATH_CHILD_TAGS,
        };
        (
            any::<bool>(),
            any::<bool>(),
            foreign_attrs_strategy(),
            proptest::collection::vec(
                (
                    proptest::sample::select(child_tags),
                    foreign_attrs_strategy(),
                    text_strategy(),
                ),
                0..3,
            ),
        )
            .prop_map(
                move |(declare_ns, declare_xlink, attrs, children)| Node::Foreign {
                    kind,
                    declare_ns,
                    declare_xlink,
                    attrs,
                    children,
                },
            )
    })
}

fn node_strategy() -> impl Strategy<Value = Node> {
    let leaf = prop_oneof![
        text_strategy().prop_map(Node::Text),
        comment_strategy().prop_map(Node::Comment),
        (proptest::sample::select(VOID_TAGS), attrs_strategy())
            .prop_map(|(tag, attrs)| Node::Void(tag, attrs)),
        foreign_strategy(),
    ];
    leaf.prop_recursive(4, 32, 4, |inner| {
        (
            proptest::sample::select(CONTAINER_TAGS),
            attrs_strategy(),
            proptest::collection::vec(inner, 0..4),
        )
            .prop_map(|(tag, attrs, children)| Node::Elem(tag, attrs, children))
    })
}

fn render_attrs(attrs: &[(&str, String)], out: &mut String) {
    // Deduplicate attribute names within a start tag: the HTML parser keeps
    // only the first occurrence, so emitting duplicates would not round-trip.
    let mut seen = Vec::new();
    for (name, value) in attrs {
        if seen.contains(name) {
            continue;
        }
        seen.push(name);
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        for c in value.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                c => out.push(c),
            }
        }
        out.push('"');
    }
}

fn render(node: &Node, out: &mut String) {
    match node {
        Node::Text(t) => {
            for c in t.chars() {
                match c {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    c => out.push(c),
                }
            }
        }
        Node::Comment(content) => {
            // Raw, unescaped: HTML comments have no entity escaping, and
            // this is exactly how a real `--`-laden comment reaches the
            // parser (see the module docs on `write_comment`).
            out.push_str("<!--");
            out.push_str(content);
            out.push_str("-->");
        }
        Node::Void(tag, attrs) => {
            out.push('<');
            out.push_str(tag);
            render_attrs(attrs, out);
            out.push_str("/>");
        }
        Node::Elem(tag, attrs, children) => {
            out.push('<');
            out.push_str(tag);
            render_attrs(attrs, out);
            out.push('>');
            for child in children {
                render(child, out);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
        Node::Foreign {
            kind,
            declare_ns,
            declare_xlink,
            attrs,
            children,
        } => {
            let (root, ns) = match kind {
                ForeignKind::Svg => ("svg", "http://www.w3.org/2000/svg"),
                ForeignKind::Math => ("math", "http://www.w3.org/1998/Math/MathML"),
            };
            out.push('<');
            out.push_str(root);
            if *declare_ns {
                out.push_str(&format!(" xmlns=\"{ns}\""));
            }
            if *declare_xlink {
                out.push_str(" xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
            }
            render_attrs(attrs, out);
            out.push('>');
            for (tag, cattrs, text) in children {
                out.push('<');
                out.push_str(tag);
                render_attrs(cattrs, out);
                out.push('>');
                render(&Node::Text(text.clone()), out);
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
            }
            out.push_str("</");
            out.push_str(root);
            out.push('>');
        }
    }
}

fn document_from(nodes: &[Node]) -> String {
    let mut body = String::new();
    for node in nodes {
        render(node, &mut body);
    }
    format!("<html><head><title>T</title></head><body>{body}</body></html>")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn parse_serialize_is_a_fixed_point(nodes in proptest::collection::vec(node_strategy(), 0..6)) {
        let src = document_from(&nodes);
        let first = serialize_xhtml(&parse_xhtml(src.as_bytes()).expect("parse source"));
        let second = serialize_xhtml(&parse_xhtml(&first).expect("reparse"));
        prop_assert_eq!(
            &first,
            &second,
            "serialize output must be stable under parse∘serialize\nfirst: {}",
            String::from_utf8_lossy(&first)
        );
    }

    /// The property the fixed-point test cannot express: output must be
    /// well-formed, namespace-valid XML. 0.4.0/0.4.1's `:xmlns` corruption
    /// was a stable fixed point — only this assertion catches that bug class.
    #[test]
    fn serialized_output_is_strict_xml(nodes in proptest::collection::vec(node_strategy(), 0..6)) {
        let src = document_from(&nodes);
        let first = serialize_xhtml(&parse_xhtml(src.as_bytes()).expect("parse source"));
        let text = std::str::from_utf8(&first).expect("serializer output is UTF-8");
        let options = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
        if let Err(e) = roxmltree::Document::parse_with_options(text, options) {
            prop_assert!(false, "output is not well-formed XML: {e}\n{text}");
        }
        if let Some((name, line)) = find_invalid_qname(text) {
            prop_assert!(false, "output contains invalid QName '{name}' on line {line}\n{text}");
        }
    }
}
