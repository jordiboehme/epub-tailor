//! XHTML handling: parsing bytes into a DOM ([`parse_xhtml`]), the device
//! transforms that rewrite the DOM in place ([`transform_chapter`]), and
//! serializing a DOM back to strict, epubcheck-clean XHTML
//! ([`serialize_xhtml`]).
//!
//! Each transform lives in its own submodule and exists because the CrossPoint
//! firmware mishandles the construct it rewrites (see
//! `docs/device-constraints.md`). [`transform_chapter`] runs them in a fixed
//! order between parse and serialize; the anchor-alias step is completed by a
//! second, book-wide pass ([`apply_anchor_aliases`]).

pub(crate) mod dom;
pub(crate) mod escape;
pub mod parse;
pub mod serialize;
pub(crate) mod table_render;

mod anchors;
mod boxes;
mod code;
mod dedupe;
mod footnotes;
mod lists;
mod styles;
pub(crate) mod tables;
mod text;

pub use parse::parse_xhtml;
pub use serialize::serialize_xhtml;

pub(crate) use dom::find_body;
pub(crate) use serialize::serialize_fragment;
pub(crate) use text::clean_string;

pub(crate) use anchors::cap_ids;
pub use anchors::{AliasMap, apply_anchor_aliases, apply_toc_aliases};

use kuchikiki::NodeRef;

use crate::options::ConvertOptions;
use crate::report::{Transformation, Warning};

/// The per-chapter output of [`transform_chapter`] the caller needs for its
/// later book-wide passes.
pub struct ChapterTransform {
    /// Anchor ids aliased onto an existing block id in this chapter, for the
    /// cross-document [`apply_anchor_aliases`] pass.
    pub aliases: AliasMap,
    /// The raw text lifted out of this chapter's `<style>` elements, for the
    /// caller to concatenate, filter and relocate into an external sheet
    /// (empty when the chapter had no `<style>`).
    pub extracted_css: String,
}

/// Apply the profile-enabled transforms to a single chapter document, in
/// place.
///
/// The transforms run in a fixed order - dedupe, styles, boxes, tables, lists,
/// code, footnotes, anchors, text - because later stages assume the block
/// structure earlier stages produce; each step runs only when the profile's
/// [`crate::profile::Features`] enables it. Duplicate-id removal runs first,
/// before anything else looks at an `id`: parsing Gutenberg-style
/// `<a id="..."/>` source through the HTML5 tree builder can clone a
/// formatting element (attributes and all) across a block boundary, and every
/// downstream id consumer - alias relocation, the anchor cap, this pass's own
/// callers - must see unique ids. The style step runs next so head/inline CSS
/// is lifted out before the structural transforms rewrite the tree.
/// Transformations and warnings are appended to the caller's collections; see
/// [`ChapterTransform`] for what is returned.
pub fn transform_chapter(
    doc: &NodeRef,
    opts: &ConvertOptions,
    report: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
    chapter_path: &str,
) -> ChapterTransform {
    let features = &opts.features;
    if features.dedupe_ids {
        dedupe::dedupe_ids(doc, report, chapter_path);
    }
    let extracted_css = if features.relocate_styles {
        styles::relocate_styles(doc, opts.remap_active(), report, chapter_path)
    } else {
        String::new()
    };
    if features.degrade_boxes {
        boxes::degrade_boxes(doc, report, chapter_path);
    }
    if features.linearize_tables {
        tables::linearize_tables(doc, opts, report, chapter_path);
    }
    if features.bake_ordered_lists {
        lists::bake_ordered_lists(doc, report, chapter_path);
    }
    if features.preserve_code_blocks {
        code::preserve_code_blocks(doc, report, chapter_path);
    }
    if features.normalize_footnotes {
        footnotes::normalize_links(doc, report, warnings, chapter_path);
    }
    let aliases = if features.relocate_anchors {
        anchors::relocate_ids(doc, report, warnings, chapter_path)
    } else {
        AliasMap::new()
    };
    if features.unicode_hygiene {
        text::unicode_hygiene(doc, report, warnings, chapter_path);
    }
    ChapterTransform {
        aliases,
        extracted_css,
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    //! Shared helpers for the transforms' unit and snapshot tests: wrap a body
    //! fragment in a minimal document, and serialize a document to a string.

    use kuchikiki::NodeRef;

    use super::{parse_xhtml, serialize_xhtml};

    pub(crate) fn doc_from_body(body: &str) -> NodeRef {
        let html = format!("<html><head><title>T</title></head><body>{body}</body></html>");
        parse_xhtml(html.as_bytes()).expect("fixture parses")
    }

    pub(crate) fn serialize(doc: &NodeRef) -> String {
        String::from_utf8(serialize_xhtml(doc)).expect("serializer emits UTF-8")
    }
}
