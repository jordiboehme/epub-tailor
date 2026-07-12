//! Property test for the strict-XHTML serializer: for arbitrary trees built
//! from a small tag/attribute/text alphabet, `serialize_xhtml` is a fixed
//! point under `parse_xhtml` — that is, re-parsing serialized output and
//! serializing it again yields byte-identical output. This is the core
//! guarantee the EPUB round trip relies on.

use epub_tailor_core::{parse_xhtml, serialize_xhtml};
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
}

const CONTAINER_TAGS: &[&str] = &["div", "p", "span", "em", "strong", "blockquote", "li"];
const VOID_TAGS: &[&str] = &["br", "hr", "img"];
const ATTR_NAMES: &[&str] = &["class", "id", "title", "epub:type", "data-x"];

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

fn node_strategy() -> impl Strategy<Value = Node> {
    let leaf = prop_oneof![
        text_strategy().prop_map(Node::Text),
        comment_strategy().prop_map(Node::Comment),
        (proptest::sample::select(VOID_TAGS), attrs_strategy())
            .prop_map(|(tag, attrs)| Node::Void(tag, attrs)),
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
}
