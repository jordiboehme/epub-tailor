//! Whitespace-preserving code blocks.
//!
//! The firmware collapses runs of whitespace and has no monospace font, so a
//! `<pre>` loses all its indentation and line structure. We bake the layout
//! into the text: newlines become `<br/>`, and significant spaces become
//! non-breaking spaces (U+00A0), inside a single `<p class="et-code">`.

use kuchikiki::NodeRef;

use crate::html::dom::{element, text, text_content};
use crate::report::Transformation;

/// A tab expands to this many non-breaking spaces.
const TAB_WIDTH: usize = 4;
const NBSP: char = '\u{A0}';

/// Rewrite every `<pre>` block in `doc` into a whitespace-preserving
/// `<p class="et-code">`. Inline `<code>` outside a `<pre>` is left untouched.
pub(crate) fn preserve_code_blocks(
    doc: &NodeRef,
    report: &mut Vec<Transformation>,
    chapter_path: &str,
) {
    let pres: Vec<NodeRef> = doc.inclusive_descendants().filter(is_pre).collect();
    for pre in pres {
        // Skip a <pre> already detached by an earlier replacement (e.g. an
        // invalidly nested one whose ancestor we rewrote first).
        if pre.parent().is_none() {
            continue;
        }
        let raw = text_content(&pre);
        let paragraph = element("p", &[("class", "et-code")]);
        let mut lines: Vec<&str> = raw.split('\n').collect();
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        for (idx, line) in lines.iter().enumerate() {
            let rendered = render_line(line);
            if !rendered.is_empty() {
                paragraph.append(text(&rendered));
            }
            if idx + 1 < lines.len() {
                paragraph.append(element("br", &[]));
            }
        }
        pre.insert_before(paragraph);
        pre.detach();
        report.push(Transformation {
            kind: "code-block-preserved".to_string(),
            detail: format!("preserved a code block of {} line(s)", lines.len()),
            file: Some(chapter_path.to_string()),
        });
    }
}

fn is_pre(node: &NodeRef) -> bool {
    matches!(node.data(), kuchikiki::NodeData::Element(e) if e.name.local.as_ref() == "pre")
}

/// Render one source line, converting significant whitespace to non-breaking
/// spaces: leading spaces 1:1, tabs to [`TAB_WIDTH`] each, and interior runs of
/// two or more spaces to that many non-breaking spaces (single interior spaces
/// stay ordinary so the line can still wrap between words).
fn render_line(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();

    let mut leading = 0;
    while leading < chars.len() && (chars[leading] == ' ' || chars[leading] == '\t') {
        leading += 1;
    }
    for &c in &chars[..leading] {
        match c {
            '\t' => out.extend(std::iter::repeat_n(NBSP, TAB_WIDTH)),
            _ => out.push(NBSP),
        }
    }

    let mut i = leading;
    while i < chars.len() {
        match chars[i] {
            '\t' => {
                out.extend(std::iter::repeat_n(NBSP, TAB_WIDTH));
                i += 1;
            }
            ' ' => {
                let start = i;
                while i < chars.len() && chars[i] == ' ' {
                    i += 1;
                }
                let run = i - start;
                if run >= 2 {
                    out.extend(std::iter::repeat_n(NBSP, run));
                } else {
                    out.push(' ');
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::testutil::{doc_from_body, serialize};

    fn run(body: &str) -> (String, Vec<Transformation>) {
        let doc = doc_from_body(body);
        let mut report = Vec::new();
        preserve_code_blocks(&doc, &mut report, "ch.xhtml");
        (serialize(&doc), report)
    }

    #[test]
    fn pre_with_tabs_and_indentation_snapshot() {
        let (out, report) = run("<pre><code>def f():\n\tif x:\n\t\treturn  1\n</code></pre>");
        insta::assert_snapshot!(out);
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].kind, "code-block-preserved");
    }

    #[test]
    fn drops_inner_markup_keeping_text() {
        let (out, _) = run("<pre><span style=\"color:red\">tok</span>en</pre>");
        assert!(
            out.contains(r#"<p class="et-code">token</p>"#),
            "got: {out}"
        );
        assert!(
            !out.contains("<span"),
            "syntax spans must be dropped: {out}"
        );
    }

    #[test]
    fn leading_spaces_map_one_to_one() {
        let (out, _) = run("<pre>  two</pre>");
        assert!(
            out.contains("<p class=\"et-code\">\u{A0}\u{A0}two</p>"),
            "got: {out}"
        );
    }

    #[test]
    fn single_interior_space_stays_breakable() {
        let (out, _) = run("<pre>a b</pre>");
        assert!(out.contains("<p class=\"et-code\">a b</p>"), "got: {out}");
    }

    #[test]
    fn inline_code_outside_pre_is_untouched() {
        let (out, report) = run("<p>use <code>x</code> here</p>");
        assert!(out.contains("<code>x</code>"), "got: {out}");
        assert!(report.is_empty());
    }
}
