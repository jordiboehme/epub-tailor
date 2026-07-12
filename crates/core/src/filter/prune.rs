//! Cascade pruning: after a removal empties a text node or detaches an
//! anchor, walk up through the emptied ancestors and detach them too, so a
//! watermark block like `<div><p><a><i>...</i></a></p></div>` disappears
//! whole instead of leaving a husk of empty elements.

use kuchikiki::NodeRef;

use crate::html::dom::local_name;
use crate::report::Transformation;

/// Elements that carry meaning without text: an ancestor holding one of these
/// is never "empty" and stops the walk.
const KEEP_SET: &[&str] = &[
    "img", "br", "hr", "svg", "image", "video", "audio", "object", "iframe", "embed", "input",
    "canvas",
];

/// Elements never pruned regardless of content: document structure and table
/// scaffolding (an emptied `<td>` must survive so rows keep their shape), and
/// `<title>`, which the EPUB spec requires.
const PROTECTED: &[&str] = &[
    "html", "head", "title", "body", "table", "thead", "tbody", "tfoot", "tr", "td", "th",
    "colgroup", "col",
];

/// Starting at `start` (the parent of a removed node), detach every ancestor
/// the removal left empty: whitespace-only text, no keep-set element, not
/// protected. Stops at the first ancestor that still holds content.
pub(crate) fn prune_upward(
    start: &NodeRef,
    transformations: &mut Vec<Transformation>,
    chapter_path: &str,
) {
    let mut current = Some(start.clone());
    while let Some(node) = current {
        let Some(name) = local_name(&node) else {
            break;
        };
        if PROTECTED.contains(&name.as_str()) || !is_empty(&node) {
            break;
        }
        let parent = node.parent();
        node.detach();
        transformations.push(Transformation {
            kind: "filter-pruned".to_string(),
            detail: format!("removed a <{name}> emptied by a filter"),
            file: Some(chapter_path.to_string()),
        });
        current = parent;
    }
}

/// Whether a subtree holds nothing worth keeping: only whitespace text and no
/// keep-set element anywhere below (or at) `node`.
fn is_empty(node: &NodeRef) -> bool {
    for descendant in node.inclusive_descendants() {
        if let Some(element) = descendant.as_element()
            && KEEP_SET.contains(&element.name.local.as_ref())
        {
            return false;
        }
        if let Some(text) = descendant.as_text()
            && !text.borrow().trim().is_empty()
        {
            return false;
        }
    }
    true
}
