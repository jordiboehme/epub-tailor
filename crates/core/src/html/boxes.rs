//! Degrade callout boxes the firmware renders poorly: `<aside>`, `<figure>`
//! (with `<figcaption>`) and `<dl>`. `<section>` and `<div>` are left alone -
//! the device renders them fine.

use kuchikiki::{NodeData, NodeRef};

use crate::html::dom::{
    collect_by_name, element, has_descendant_named, is_named, move_children, replace_with,
    unwrap_element,
};
use crate::report::Transformation;

/// Block elements whose presence inside an `<aside>` means we simply unwrap it
/// (splicing children in place) rather than rebuilding it as paragraphs.
const ASIDE_BLOCK_TRIGGER: &[&str] = &[
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
    if has_descendant_named(aside, ASIDE_BLOCK_TRIGGER) {
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
    let mut replacements = Vec::new();
    for child in dl.children() {
        if is_named(&child, "dt") {
            let strong = element("strong", &[]);
            move_children(&child, &strong);
            let paragraph = element("p", &[("class", "et-dt")]);
            paragraph.append(strong);
            replacements.push(paragraph);
        } else if is_named(&child, "dd") {
            let paragraph = element("p", &[("class", "et-dd")]);
            move_children(&child, &paragraph);
            replacements.push(paragraph);
        }
    }
    replace_with(dl, replacements);
    record(report, chapter_path, "flattened a dl to paragraphs");
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
    fn section_and_div_are_left_alone() {
        let (out, report) = run("<section><div>keep</div></section>");
        assert!(
            out.contains("<section><div>keep</div></section>"),
            "got: {out}"
        );
        assert!(report.is_empty());
    }
}
