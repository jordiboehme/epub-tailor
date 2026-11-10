//! Bake ordered-list numbering into text.
//!
//! The firmware numbers `<ol>` natively since 1.6.5, but decimal only. We keep
//! baking because it honors `start`, `type` and per-item `value`, and compounds
//! labels ("2.1.") for nested ordered lists. No `<ol>` survives, so nothing is
//! numbered twice. Each `<ol>` becomes numbered `<p class="et-ol-item">`
//! paragraphs. `<ul>` is left untouched - its native bullets are already
//! correct.
//!
//! Each numbered paragraph hangs its label, and each continuation block lines
//! up under the text; both carry their margin and indent inline as well as
//! through their class (see [`super::hang`]).

use kuchikiki::{NodeData, NodeRef};

use crate::html::dom::{add_class, child_elements, element, get_attr, is_named, set_attr, text};
use crate::html::hang::{self, hang_inline};
use crate::report::Transformation;

const NBSP: char = '\u{A0}';

/// Block-level children of an `<li>` that carry over as continuation blocks.
const LI_BLOCK: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "pre",
    "table",
    "figure",
    "section",
    "dl",
];

/// Replace every `<ol>` in `doc` with baked-in numbered paragraphs.
pub(crate) fn bake_ordered_lists(
    doc: &NodeRef,
    report: &mut Vec<Transformation>,
    chapter_path: &str,
) {
    // Process outermost lists first (they own the compound-label context), then
    // repeat: any `<ol>` preserved inside a moved `<ul>` loses its `<ol>`
    // ancestor once its parent is replaced, and is picked up as a fresh list.
    loop {
        let tops: Vec<NodeRef> = doc
            .inclusive_descendants()
            .filter(|n| is_named(n, "ol") && !has_ol_ancestor(n))
            .collect();
        if tops.is_empty() {
            break;
        }
        for ol in tops {
            let nodes = flatten_ol(&ol, None, report, chapter_path);
            for node in nodes {
                ol.insert_before(node);
            }
            ol.detach();
        }
    }
}

fn has_ol_ancestor(node: &NodeRef) -> bool {
    node.ancestors().any(|a| is_named(&a, "ol"))
}

/// Produce the replacement nodes for one `<ol>`. `parent_core` is the parent
/// item's label without its trailing dot (`None` at the top level).
fn flatten_ol(
    ol: &NodeRef,
    parent_core: Option<&str>,
    report: &mut Vec<Transformation>,
    chapter_path: &str,
) -> Vec<NodeRef> {
    let start = get_attr(ol, "start")
        .and_then(|s| s.trim().parse::<i32>().ok())
        .unwrap_or(1);
    let list_type = get_attr(ol, "type").unwrap_or_default();
    let nested = parent_core.is_some();

    let mut current = start;
    let mut out = Vec::new();
    let mut items = 0;
    for li in child_elements(ol) {
        if !is_named(&li, "li") {
            continue;
        }
        if let Some(v) = get_attr(&li, "value").and_then(|s| s.trim().parse::<i32>().ok()) {
            current = v;
        }
        let token = format_token(current, &list_type);
        let core = match parent_core {
            Some(parent) => format!("{parent}.{token}"),
            None => token,
        };
        out.extend(flatten_li(&li, &core, nested, report, chapter_path));
        current += 1;
        items += 1;
    }

    report.push(Transformation {
        kind: "ol-numbered".to_string(),
        detail: format!("numbered an ordered list of {items} item(s)"),
        file: Some(chapter_path.to_string()),
    });
    out
}

/// Produce the replacement nodes for one `<li>`: the labeled paragraph(s),
/// followed by any nested list content.
fn flatten_li(
    li: &NodeRef,
    core: &str,
    nested: bool,
    report: &mut Vec<Transformation>,
    chapter_path: &str,
) -> Vec<NodeRef> {
    let label = format!("{core}.");
    let (item_class, item_hang) = if nested {
        ("et-ol-item et-ol-nested", hang::OL_NESTED)
    } else {
        ("et-ol-item", hang::OL_ITEM)
    };
    // An `<li id="...">` is a same-document link target (e.g. GFM's rendered
    // footnote definitions, `<ol><li id="fn-1">`) - the `<li>` itself never
    // survives, so its id must move to whichever replacement element carries
    // the label, or every `href="#fn-1"` pointing at it would go dangling.
    let li_id = get_attr(li, "id");

    let mut inline = Vec::new();
    let mut blocks = Vec::new();
    let mut trailers = Vec::new();
    for child in li.children() {
        if is_named(&child, "ol") {
            trailers.extend(flatten_ol(&child, Some(core), report, chapter_path));
        } else if is_named(&child, "ul") {
            trailers.push(child);
        } else if local_is_block(&child) {
            blocks.push(child);
        } else {
            inline.push(child);
        }
    }
    trim_leading_whitespace(&mut inline);

    let mut result = Vec::new();
    let has_inline = inline.iter().any(|n| !is_whitespace_text(n));
    if blocks.is_empty() || has_inline {
        let paragraph = element("p", &[("class", item_class)]);
        paragraph.append(text(&format!("{label}{NBSP}")));
        for node in inline {
            paragraph.append(node);
        }
        transfer_id(&paragraph, li_id.as_deref());
        hang_inline(&paragraph, li, item_hang);
        result.push(paragraph);
        for block in blocks {
            add_class(&block, "et-ol-cont");
            hang_inline(&block, li, hang::OL_CONT);
            result.push(block);
        }
    } else {
        // Blocks only: the first block carries the label, the rest continue.
        let mut iter = blocks.into_iter();
        if let Some(first) = iter.next() {
            add_class(&first, item_class);
            first.prepend(text(&format!("{label}{NBSP}")));
            transfer_id(&first, li_id.as_deref());
            hang_inline(&first, li, item_hang);
            result.push(first);
        }
        for block in iter {
            add_class(&block, "et-ol-cont");
            hang_inline(&block, li, hang::OL_CONT);
            result.push(block);
        }
    }
    result.extend(trailers);
    result
}

/// Set `id` on `node` from the original `<li>`'s id, unless `node` already
/// carries one of its own (kept as-is rather than overwritten).
fn transfer_id(node: &NodeRef, li_id: Option<&str>) {
    if let Some(id) = li_id
        && get_attr(node, "id").is_none()
    {
        set_attr(node, "id", id);
    }
}

fn local_is_block(node: &NodeRef) -> bool {
    matches!(node.data(), NodeData::Element(e) if LI_BLOCK.contains(&e.name.local.as_ref()))
}

fn is_whitespace_text(node: &NodeRef) -> bool {
    matches!(node.data(), NodeData::Text(t) if t.borrow().trim().is_empty())
}

fn trim_leading_whitespace(inline: &mut Vec<NodeRef>) {
    while inline.first().is_some_and(is_whitespace_text) {
        inline.remove(0);
    }
    if let Some(first) = inline.first()
        && let Some(text) = first.as_text()
    {
        let trimmed = text.borrow().trim_start().to_string();
        *text.borrow_mut() = trimmed;
    }
}

/// Format a counter into a list token per the `<ol type>` value. Non-positive
/// counters (or out-of-range roman numerals) fall back to decimal.
fn format_token(n: i32, list_type: &str) -> String {
    match list_type {
        "a" => alpha(n, false),
        "A" => alpha(n, true),
        "i" => roman(n, false),
        "I" => roman(n, true),
        _ => n.to_string(),
    }
}

fn alpha(n: i32, upper: bool) -> String {
    if n < 1 {
        return n.to_string();
    }
    let mut n = n;
    let mut chars = Vec::new();
    while n > 0 {
        n -= 1;
        chars.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    let out: String = chars.into_iter().rev().collect();
    if upper { out.to_uppercase() } else { out }
}

fn roman(n: i32, upper: bool) -> String {
    if !(1..=3999).contains(&n) {
        return n.to_string();
    }
    const TABLE: [(i32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut n = n;
    let mut out = String::new();
    for (value, numeral) in TABLE {
        while n >= value {
            out.push_str(numeral);
            n -= value;
        }
    }
    if upper { out.to_uppercase() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::testutil::{doc_from_body, serialize};

    fn run(body: &str) -> (String, Vec<Transformation>) {
        let doc = doc_from_body(body);
        let mut report = Vec::new();
        bake_ordered_lists(&doc, &mut report, "ch.xhtml");
        (serialize(&doc), report)
    }

    #[test]
    fn decimal_list_snapshot() {
        let (out, report) = run("<ol><li>one</li><li>two</li></ol>");
        insta::assert_snapshot!(out);
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].kind, "ol-numbered");
    }

    #[test]
    fn start_and_type_and_value_snapshot() {
        let (out, _) =
            run("<ol start=\"3\" type=\"a\"><li>c</li><li value=\"10\">j</li><li>k</li></ol>");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn roman_type_snapshot() {
        let (out, _) = run("<ol type=\"I\"><li>one</li><li>two</li><li>three</li></ol>");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn nested_ol_compound_labels_snapshot() {
        let (out, report) = run("<ol><li>a<ol><li>x</li><li>y</li></ol></li><li>b</li></ol>");
        insta::assert_snapshot!(out);
        assert_eq!(report.len(), 2, "one transformation per ol");
    }

    #[test]
    fn ul_nested_in_ol_stays_a_list_snapshot() {
        let (out, _) = run("<ol><li>item<ul><li>bullet</li></ul></li></ol>");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn block_li_continuations_snapshot() {
        let (out, _) = run("<ol><li><p>first</p><p>second</p></li></ol>");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn li_id_is_preserved_on_the_replacement_paragraph() {
        // GFM footnote definitions render as `<ol><li id="fn-1">`; the id must
        // survive baking or every `href="#fn-1"` referencing it goes dangling.
        let (out, _) = run(r#"<ol><li id="fn-1">body text</li></ol>"#);
        assert_eq!(out.matches("<p").count(), 1, "got: {out}");
        assert!(out.contains(r#"id="fn-1""#), "got: {out}");
        assert!(out.contains(r#"class="et-ol-item""#), "got: {out}");
    }

    #[test]
    fn li_id_is_preserved_on_the_reused_block_child() {
        let (out, _) = run(r#"<ol><li id="fn-1"><p>body text</p></li></ol>"#);
        assert_eq!(out.matches("<p").count(), 1, "got: {out}");
        assert!(out.contains(r#"id="fn-1""#), "got: {out}");
        assert!(out.contains(r#"class="et-ol-item""#), "got: {out}");
    }

    const HANG: &str = "margin-left:1.5em;text-indent:-1.5em";
    const NESTED_HANG: &str = "margin-left:2em;text-indent:-1em";
    const RTL_HANG: &str = "margin-left:0;margin-right:1.5em;text-indent:-1.5em";

    #[test]
    fn items_carry_their_hang_inline() {
        let (out, _) = run("<ol><li>a<ol><li>x</li></ol></li><li>b</li></ol>");
        assert_eq!(
            out.matches(&format!(r#"style="{HANG}""#)).count(),
            2,
            "{out}"
        );
        assert_eq!(
            out.matches(&format!(r#"style="{NESTED_HANG}""#)).count(),
            1,
            "{out}"
        );
    }

    #[test]
    fn hang_is_merged_after_an_existing_inline_style() {
        let (out, _) =
            run(r#"<ol><li><p style="margin:0;text-indent:2em;font-style:italic">x</p></li></ol>"#);
        assert!(
            out.contains(&format!(r#"style="margin:0;font-style:italic;{HANG}""#)),
            "{out}"
        );
    }

    #[test]
    fn continuation_blocks_hang_inline_too() {
        let (out, _) = run(
            r#"<ol><li><p>first</p><p class="note" style="margin-left:0">second</p></li></ol>"#,
        );
        assert!(
            out.contains(
                r#"<p class="note et-ol-cont" style="margin-left:2em;text-indent:-1px">second</p>"#
            ),
            "{out}"
        );
    }

    #[test]
    fn rtl_dir_on_an_ancestor_hangs_on_the_right() {
        let (out, _) = run(r#"<div dir="rtl"><ol><li>x</li></ol></div>"#);
        assert!(out.contains(&format!(r#"style="{RTL_HANG}""#)), "{out}");
    }

    #[test]
    fn rtl_text_without_dir_hangs_on_the_right() {
        // The device detects the direction from the first words when no
        // `dir` is set (`ParsedText.cpp:780-790`).
        let (out, _) = run("<ol><li>\u{5e9}\u{5dc}\u{5d5}\u{5dd} world</li></ol>");
        assert!(out.contains(&format!(r#"style="{RTL_HANG}""#)), "{out}");
    }

    #[test]
    fn explicit_ltr_wins_over_rtl_text() {
        let (out, _) = run(r#"<div dir="ltr"><ol dir="auto"><li>\u{5e9}</li></ol></div>"#);
        assert!(out.contains(&format!(r#"style="{HANG}""#)), "{out}");
    }

    #[test]
    fn rtl_nested_item_hangs_on_the_right() {
        let (out, _) = run(r#"<ol dir="rtl"><li>a<ol><li>b</li></ol></li></ol>"#);
        assert!(
            out.contains(r#"style="margin-left:0;margin-right:2em;text-indent:-1em""#),
            "{out}"
        );
        // The `<ol dir>` does not survive, so the paragraphs carry it.
        assert_eq!(out.matches(r#"dir="rtl""#).count(), 2, "{out}");
    }

    #[test]
    fn inline_hangs_are_kept_by_the_inline_filter() {
        for style in [
            HANG,
            NESTED_HANG,
            RTL_HANG,
            "margin-left:0;margin-right:2em;text-indent:-1em",
        ] {
            assert_eq!(
                crate::css::filter_inline_style(style, false).as_deref(),
                Some(style)
            );
        }
    }

    #[test]
    fn top_level_ul_is_untouched() {
        let (out, report) = run("<ul><li>a</li><li>b</li></ul>");
        assert!(out.contains("<ul><li>a</li><li>b</li></ul>"), "got: {out}");
        assert!(report.is_empty());
    }
}
