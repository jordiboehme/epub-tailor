//! Write the hanging indent of a generated paragraph inline.
//!
//! CrossPoint 2.0 indents every paragraph unless its `text-indent` is
//! negative (`ParsedText.cpp:759-766` at firmware 2.0.0), so each paragraph we
//! generate hangs: a numbered list item hangs its label, and continuation
//! blocks, linearized table cells and code lines hang by `-1px` to line up.
//! The `et-*` class rules in `et-styles.css` carry the pair for other readers.
//! On the device a more specific author rule (a scoped `p{margin:0}`,
//! `.chapter p{margin:0}`, `.note{margin-left:0}`) would replace the margin
//! and leave the negative indent drawing the first line left of the text
//! column, so the same pair is also written inline: the device applies inline
//! styles over the resolved cascade (`ChapterHtmlSlimParser.cpp:1430-1433`).
//!
//! A right-to-left paragraph starts its first line at the right edge plus the
//! hang (`ParsedText.cpp:1541, 1703-1706`), so there the margin goes on the
//! right and the left margin is zeroed.

use kuchikiki::NodeRef;

use crate::html::dom::{get_attr, merge_inline_style, set_attr, text_content};
use crate::html::text::device_detects_rtl;

/// A generated paragraph's margin on the starting side and its first-line
/// indent, equal to its class rule in `et-styles.css`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hang {
    /// The class whose `et-styles.css` rule carries the same pair.
    pub(crate) class: &'static str,
    /// Margin on the side the line starts on.
    pub(crate) margin: &'static str,
    /// The (negative) first-line indent.
    pub(crate) indent: &'static str,
}

/// A numbered list item.
pub(crate) const OL_ITEM: Hang = Hang {
    class: "et-ol-item",
    margin: "1.5em",
    indent: "-1.5em",
};
/// A numbered item of a nested list.
pub(crate) const OL_NESTED: Hang = Hang {
    class: "et-ol-nested",
    margin: "2em",
    indent: "-1em",
};
/// A further block of a list item.
pub(crate) const OL_CONT: Hang = Hang {
    class: "et-ol-cont",
    margin: "2em",
    indent: "-1px",
};
/// A linearized table cell.
pub(crate) const TABLE_CELL: Hang = Hang {
    class: "et-table-cell",
    margin: "1em",
    indent: "-1px",
};
/// A code block.
pub(crate) const CODE: Hang = Hang {
    class: "et-code",
    margin: "1em",
    indent: "-1px",
};

/// Every inline hang, for the test that keeps them equal to `et-styles.css`.
#[cfg(test)]
pub(crate) const ALL: &[Hang] = &[OL_ITEM, OL_NESTED, OL_CONT, TABLE_CELL, CODE];

/// Write `hang` inline on `paragraph`, mirrored when the device lays it out
/// right to left. `context` is the source element the paragraph replaces or
/// came from (an `<li>`, `<td>` or `<pre>`), still in the document, whose
/// ancestors decide the direction.
pub(crate) fn hang_inline(paragraph: &NodeRef, context: &NodeRef, hang: Hang) {
    if paragraph_is_rtl(paragraph, context) {
        merge_inline_style(
            paragraph,
            &[
                ("margin-left", "0"),
                ("margin-right", hang.margin),
                ("text-indent", hang.indent),
            ],
        );
    } else {
        merge_inline_style(
            paragraph,
            &[("margin-left", hang.margin), ("text-indent", hang.indent)],
        );
    }
}

/// Whether the device lays `paragraph` out right to left. The nearest `dir`
/// of `rtl` or `ltr` decides: on the paragraph, its inline `direction`, then
/// `context` and its ancestors. A `dir` found there is copied onto the
/// paragraph, since the element carrying it may not survive (the `<li>`,
/// `<ol>`, `<td>`, `<table>` or `<pre>` is replaced). With no `dir` at all the
/// device looks at the first words (see [`device_detects_rtl`]). A
/// `direction` set in a stylesheet is not seen here.
fn paragraph_is_rtl(paragraph: &NodeRef, context: &NodeRef) -> bool {
    if let Some(rtl) = get_attr(paragraph, "dir").and_then(|d| explicit_rtl(&d)) {
        return rtl;
    }
    if let Some(rtl) = get_attr(paragraph, "style").and_then(|s| inline_direction(&s)) {
        return rtl;
    }
    for node in context.inclusive_ancestors() {
        if let Some(dir) = get_attr(&node, "dir")
            && let Some(rtl) = explicit_rtl(&dir)
        {
            set_attr(paragraph, "dir", dir.trim());
            return rtl;
        }
    }
    device_detects_rtl(&text_content(paragraph))
}

/// `Some(true)` for `rtl`, `Some(false)` for `ltr`, `None` for anything else
/// (`auto`, empty), compared as the device does, ignoring ASCII case.
fn explicit_rtl(dir: &str) -> Option<bool> {
    let dir = dir.trim();
    if dir.eq_ignore_ascii_case("rtl") {
        Some(true)
    } else if dir.eq_ignore_ascii_case("ltr") {
        Some(false)
    } else {
        None
    }
}

/// The direction an inline `direction:` declaration sets, if any.
fn inline_direction(style: &str) -> Option<bool> {
    style.split(';').rev().find_map(|decl| {
        let (name, value) = decl.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("direction") {
            explicit_rtl(value)
        } else {
            None
        }
    })
}
