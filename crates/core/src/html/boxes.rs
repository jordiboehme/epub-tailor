//! Degrade callout boxes the firmware renders poorly: `<aside>`, `<figure>`
//! (with `<figcaption>`) and `<dl>`. `<section>` and `<div>` are left alone -
//! the device renders them fine. A `<dl>` whose terms are only bullet glyphs
//! is a list in disguise and becomes a real `<ul>`.

use kuchikiki::{NodeData, NodeRef};

use crate::html::dom::{
    add_class, collect_by_name, element, get_attr, has_descendant_named, is_named, move_children,
    replace_with, set_attr, text_content, unwrap_element,
};
use crate::report::Transformation;

/// Block elements. Inside an `<aside>` their presence means we simply unwrap
/// it (splicing children in place) rather than rebuilding it as paragraphs;
/// inside a `<dd>` they are the blocks, everything else is inline content.
const BLOCK_ELEMENTS: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "dl",
    "table",
    "figure",
    "section",
    "blockquote",
    "pre",
];

/// Rewrite every `<aside>`, `<figure>` and `<dl>` in `doc`.
pub(crate) fn degrade_boxes(doc: &NodeRef, report: &mut Vec<Transformation>, chapter_path: &str) {
    for aside in collect_by_name(doc, "aside") {
        degrade_aside(&aside, report, chapter_path);
    }
    for figure in collect_by_name(doc, "figure") {
        degrade_figure(&figure, report, chapter_path);
    }
    for dl in collect_by_name(doc, "dl") {
        degrade_dl(&dl, report, chapter_path);
    }
}

fn record(report: &mut Vec<Transformation>, chapter_path: &str, detail: &str) {
    report.push(Transformation {
        kind: "box-degraded".to_string(),
        detail: detail.to_string(),
        file: Some(chapter_path.to_string()),
    });
}

fn degrade_aside(aside: &NodeRef, report: &mut Vec<Transformation>, chapter_path: &str) {
    if aside.parent().is_none() {
        return;
    }
    if !has_meaningful_content(aside) {
        aside.detach();
        record(report, chapter_path, "removed an empty aside");
        return;
    }
    if has_descendant_named(aside, BLOCK_ELEMENTS) {
        unwrap_element(aside);
        record(
            report,
            chapter_path,
            "unwrapped an aside with block content",
        );
        return;
    }

    // No block content: a leading bold run becomes a titled paragraph, the
    // rest becomes a plain paragraph.
    let children: Vec<NodeRef> = aside.children().collect();
    let mut lead = Vec::new();
    let mut body = Vec::new();
    let mut in_lead = true;
    for child in children {
        if in_lead {
            if is_bold(&child) {
                lead.push(child);
                continue;
            }
            if is_whitespace_text(&child) {
                continue;
            }
            in_lead = false;
        }
        body.push(child);
    }

    let mut replacements = Vec::new();
    if !lead.is_empty() {
        let strong = element("strong", &[]);
        for bold in &lead {
            move_children(bold, &strong);
        }
        let title = element("p", &[("class", "et-box-title")]);
        title.append(strong);
        replacements.push(title);
    }
    if !body.is_empty() {
        let paragraph = element("p", &[]);
        for node in body {
            paragraph.append(node);
        }
        trim_leading_whitespace(&paragraph);
        if paragraph.first_child().is_some() {
            replacements.push(paragraph);
        }
    }

    replace_with(aside, replacements);
    record(report, chapter_path, "degraded an aside to paragraphs");
}

fn degrade_figure(figure: &NodeRef, report: &mut Vec<Transformation>, chapter_path: &str) {
    if figure.parent().is_none() {
        return;
    }
    let mut body = Vec::new();
    let mut captions = Vec::new();
    for child in figure.children() {
        if is_named(&child, "figcaption") {
            let caption = element("p", &[("class", "et-caption")]);
            move_children(&child, &caption);
            captions.push(caption);
        } else {
            body.push(child);
        }
    }
    // Body (including any <img>) first, then the caption(s) after it.
    for node in body {
        figure.insert_before(node);
    }
    for caption in captions {
        figure.insert_before(caption);
    }
    figure.detach();
    record(report, chapter_path, "unwrapped a figure");
}

fn degrade_dl(dl: &NodeRef, report: &mut Vec<Transformation>, chapter_path: &str) {
    if dl.parent().is_none() {
        return;
    }
    let terms: Vec<NodeRef> = dl.children().filter(|c| is_named(c, "dt")).collect();
    if !terms.is_empty() && terms.iter().all(is_bullet_marker) {
        let replacements = marker_dl_to_list(dl);
        replace_with(dl, replacements);
        record(
            report,
            chapter_path,
            "turned a bullet-marker dl into a list",
        );
        return;
    }
    let mut replacements = Vec::new();
    for child in dl.children() {
        if is_named(&child, "dt") {
            let strong = element("strong", &[]);
            move_children(&child, &strong);
            let paragraph = element("p", &[("class", "et-dt")]);
            paragraph.append(strong);
            replacements.push(paragraph);
        } else if is_named(&child, "dd") {
            replacements.extend(dd_to_blocks(&child));
        }
    }
    replace_with(dl, replacements);
    record(report, chapter_path, "flattened a dl to paragraphs");
}

/// A `<dt>` holding nothing but one or two non-alphanumeric glyphs ("+", "•",
/// "–", "✓"): a bullet a publisher floated beside its `<dd>`, not a term.
fn is_bullet_marker(dt: &NodeRef) -> bool {
    let content = text_content(dt);
    let marker = content.trim();
    (1..=2).contains(&marker.chars().count()) && !marker.chars().any(char::is_alphanumeric)
}

/// Rebuild a bullet-marker `<dl>` as a `<ul>`: the firmware draws its own "•"
/// with a proper hanging indent, where a floated `<dt>` lands on a line of its
/// own. The item takes the `<dd>`'s inline content, or its first paragraph's;
/// further blocks follow the list (indented) and a new list resumes after them.
fn marker_dl_to_list(dl: &NodeRef) -> Vec<NodeRef> {
    let mut result = Vec::new();
    let mut list: Option<NodeRef> = None;
    let mut term_id = None;
    for child in dl.children() {
        if is_named(&child, "dt") {
            term_id = get_attr(&child, "id");
            continue;
        }
        if !is_named(&child, "dd") {
            continue;
        }
        let item = element("li", &[]);
        if let Some(id) = term_id.take().or_else(|| get_attr(&child, "id")) {
            set_attr(&item, "id", &id);
        }
        let (inline, mut blocks): (Vec<NodeRef>, Vec<NodeRef>) =
            child.children().partition(|n| !is_dd_block(n));
        if inline.iter().any(|n| !is_whitespace_text(n)) {
            for node in inline {
                item.append(node);
            }
        } else if !blocks.is_empty() {
            let first = blocks.remove(0);
            if get_attr(&item, "id").is_none()
                && let Some(id) = get_attr(&first, "id")
            {
                set_attr(&item, "id", &id);
            }
            move_children(&first, &item);
        }
        trim_leading_whitespace(&item);
        trim_trailing_whitespace(&item);
        list.get_or_insert_with(|| element("ul", &[])).append(item);
        if !blocks.is_empty() {
            result.extend(list.take());
            for block in blocks {
                add_class(&block, "et-dd");
                result.push(block);
            }
        }
    }
    result.extend(list);
    result
}

/// A `<dd>` as indented blocks: its own block children take the indent class,
/// runs of inline content get a paragraph - never a paragraph around a
/// paragraph.
fn dd_to_blocks(dd: &NodeRef) -> Vec<NodeRef> {
    if !dd.children().any(|n| is_dd_block(&n)) {
        let paragraph = element("p", &[("class", "et-dd")]);
        move_children(dd, &paragraph);
        return vec![paragraph];
    }
    let mut result = Vec::new();
    let mut run: Option<NodeRef> = None;
    for node in dd.children() {
        if is_dd_block(&node) {
            result.extend(run.take());
            add_class(&node, "et-dd");
            result.push(node);
        } else if !is_whitespace_text(&node) || run.is_some() {
            run.get_or_insert_with(|| element("p", &[("class", "et-dd")]))
                .append(node);
        }
    }
    result.extend(run);
    result
}

fn is_dd_block(node: &NodeRef) -> bool {
    matches!(node.data(), NodeData::Element(e) if BLOCK_ELEMENTS.contains(&e.name.local.as_ref()))
}

fn has_meaningful_content(node: &NodeRef) -> bool {
    node.children().any(|c| match c.data() {
        NodeData::Element(_) => true,
        NodeData::Text(t) => !t.borrow().trim().is_empty(),
        _ => false,
    })
}

fn is_bold(node: &NodeRef) -> bool {
    is_named(node, "strong") || is_named(node, "b")
}

fn is_whitespace_text(node: &NodeRef) -> bool {
    matches!(node.data(), NodeData::Text(t) if t.borrow().trim().is_empty())
}

fn trim_leading_whitespace(paragraph: &NodeRef) {
    if let Some(first) = paragraph.first_child()
        && let Some(text) = first.as_text()
    {
        let trimmed = text.borrow().trim_start().to_string();
        if trimmed.is_empty() {
            first.detach();
        } else {
            *text.borrow_mut() = trimmed;
        }
    }
}

fn trim_trailing_whitespace(node: &NodeRef) {
    if let Some(last) = node.last_child()
        && let Some(text) = last.as_text()
    {
        let trimmed = text.borrow().trim_end().to_string();
        if trimmed.is_empty() {
            last.detach();
        } else {
            *text.borrow_mut() = trimmed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::testutil::{doc_from_body, serialize};

    fn run(body: &str) -> (String, Vec<Transformation>) {
        let doc = doc_from_body(body);
        let mut report = Vec::new();
        degrade_boxes(&doc, &mut report, "ch.xhtml");
        (serialize(&doc), report)
    }

    #[test]
    fn aside_with_bold_lead_snapshot() {
        let (out, report) = run("<aside><strong>Note:</strong> mind the gap.</aside>");
        insta::assert_snapshot!(out);
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].kind, "box-degraded");
    }

    #[test]
    fn aside_with_block_content_is_unwrapped() {
        let (out, _) = run("<aside><p>one</p><p>two</p></aside>");
        assert!(!out.contains("<aside"), "aside must be gone: {out}");
        assert!(out.contains("<p>one</p><p>two</p>"), "got: {out}");
    }

    #[test]
    fn empty_aside_is_removed() {
        let (out, report) = run("<aside>   </aside>");
        assert!(!out.contains("aside"), "got: {out}");
        assert_eq!(report[0].detail, "removed an empty aside");
    }

    #[test]
    fn figure_caption_moves_after_image_snapshot() {
        let (out, _) =
            run("<figure><img src=\"a.jpg\" alt=\"a\"/><figcaption>A cat</figcaption></figure>");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn dl_becomes_paragraphs_snapshot() {
        let (out, _) = run("<dl><dt>Term</dt><dd>Definition</dd><dt>T2</dt><dd>D2</dd></dl>");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn dd_with_paragraphs_does_not_nest_paragraphs() {
        let (out, _) = run("<dl><dt>Term</dt><dd><p class=\"x\">one</p><p>two</p></dd></dl>");
        assert!(!out.contains("<p class=\"et-dd\"><p"), "nested p: {out}");
        assert!(
            out.contains("<p class=\"x et-dd\">one</p><p class=\"et-dd\">two</p>"),
            "the dd's own paragraphs carry the indent: {out}"
        );
    }

    #[test]
    fn marker_dl_becomes_a_bullet_list() {
        // A publisher pattern: a list set as a dl whose terms are one bullet
        // glyph each, floated left by CSS the device ignores.
        let (out, report) = run(concat!(
            "<dl><dt class=\"term\">+</dt><dd><p class=\"Aufz\">Bleib bei deinem Kind.</p></dd>",
            "<dt>+</dt><dd>Fühle <em>mit</em>.</dd></dl>"
        ));
        assert!(
            out.contains("<ul><li>Bleib bei deinem Kind.</li><li>Fühle <em>mit</em>.</li></ul>"),
            "got: {out}"
        );
        assert!(
            !out.contains('+'),
            "the glyph gives way to the native bullet: {out}"
        );
        assert_eq!(report[0].detail, "turned a bullet-marker dl into a list");
    }

    #[test]
    fn marker_dl_keeps_further_blocks_after_the_item() {
        let (out, _) =
            run("<dl><dt>•</dt><dd><p>first</p><p>second</p></dd><dt>•</dt><dd>third</dd></dl>");
        assert!(
            out.contains(
                "<ul><li>first</li></ul><p class=\"et-dd\">second</p><ul><li>third</li></ul>"
            ),
            "got: {out}"
        );
    }

    #[test]
    fn marker_dl_moves_ids_to_the_item() {
        let (out, _) = run("<dl><dt id=\"a\">–</dt><dd id=\"b\">x</dd></dl>");
        assert!(out.contains("<li id=\"a\">x</li>"), "got: {out}");
    }

    #[test]
    fn a_dl_with_word_terms_stays_a_definition_list() {
        let (out, _) = run("<dl><dt>+</dt><dd>plus</dd><dt>Minus</dt><dd>minus</dd></dl>");
        assert!(!out.contains("<ul>"), "real terms are not bullets: {out}");
        assert!(out.contains("<strong>Minus</strong>"), "got: {out}");
    }

    #[test]
    fn section_and_div_are_left_alone() {
        let (out, report) = run("<section><div>keep</div></section>");
        assert!(
            out.contains("<section><div>keep</div></section>"),
            "got: {out}"
        );
        assert!(report.is_empty());
    }
}
