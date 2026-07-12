//! Splitting a parsed Markdown document into chapters on heading boundaries.
//!
//! Splitting happens on the comrak AST itself, before rendering: each chapter
//! chunk is rendered to HTML independently (via [`crate::markdown::mod@render`]),
//! so GFM footnote references and their definitions resolve correctly as long
//! as both live in the same chunk (comrak numbers and collects footnotes per
//! render call). comrak itself moves every footnote definition to the end of
//! the document regardless of where it was written, so [`split_chapters`]
//! re-homes each one with the chunk that actually references it (see
//! [`attach_footnote_definitions`]) before that "same chunk" guarantee holds.
//! Slugs are computed here, from the AST, in the same document order the
//! rendered `<h1>`-`<h6>` elements will appear in, so [`assign_heading_ids`]
//! can zip them onto the parsed-back DOM positionally.

use std::collections::HashMap;

use comrak::Node;
use comrak::nodes::NodeValue;
use kuchikiki::NodeRef;

use crate::html::dom::{local_name, set_attr};
use crate::report::Warning;

/// One chapter's worth of top-level AST nodes, plus the metadata needed to
/// title it and build its table-of-contents entries.
pub(crate) struct Chunk<'a> {
    /// The chunk's top-level nodes, in document order (excluding any
    /// `FrontMatter` node, which is never part of a chunk).
    pub nodes: Vec<Node<'a>>,
    /// The chunk's own heading text, or empty for the front chunk (the
    /// content before the first split heading, which has none of its own -
    /// the caller titles it with the book title instead).
    pub title: String,
    /// The slug id for the chunk's own defining heading (`nodes[0]`), or
    /// `None` for the front chunk.
    pub heading_slug: Option<String>,
    /// Headings one level below the split level, found anywhere in the
    /// chunk, in document order.
    pub sub_headings: Vec<SubHeading>,
}

impl Chunk<'_> {
    /// Whether this is the front chunk (content before the first split
    /// heading), which has no heading of its own.
    pub(crate) fn is_front(&self) -> bool {
        self.heading_slug.is_none()
    }

    /// The slugs of every heading in this chunk that must get an `id`
    /// (the chunk's own heading, if any, followed by its sub-headings), in
    /// the same document order their rendered `<hN>` elements will appear in.
    pub(crate) fn heading_ids_in_order(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.heading_slug.iter().cloned().collect();
        ids.extend(self.sub_headings.iter().map(|h| h.slug.clone()));
        ids
    }
}

/// A heading one level below the split level: gets its own slug id and a
/// level-2 table-of-contents entry (a fragment within its chapter).
pub(crate) struct SubHeading {
    pub slug: String,
    pub title: String,
}

/// A single empty front chunk, for the degenerate case of a Markdown source
/// with no content blocks at all: the book still needs exactly one chapter.
pub(crate) fn empty_front_chunk<'a>() -> Chunk<'a> {
    Chunk {
        nodes: Vec::new(),
        title: String::new(),
        heading_slug: None,
        sub_headings: Vec::new(),
    }
}

/// Split `root`'s top-level content into chapter [`Chunk`]s at every heading
/// whose level is `<= split_level`. Content before the first such heading (if
/// any) becomes a front chunk; a document with no split-level heading at all
/// becomes a single front chunk holding everything. The `FrontMatter` node (if
/// comrak captured one) is always excluded.
///
/// comrak moves every `FootnoteDefinition` out of the normal flow and appends
/// it as a trailing child of the document root, regardless of where in the
/// source it was written - grouping by top-level position alone would dump
/// every footnote definition into whichever chunk happens to be last, even
/// when its reference lives in an earlier chapter (an internal `#fn-name` link
/// pointing at a different chapter file, which epubcheck rejects as a
/// dangling fragment). [`attach_footnote_definitions`] re-homes each
/// definition with the chunk that actually references it instead.
///
/// A footnote referenced from more than one chapter cannot be fixed this way,
/// since the definition can only live in one chapter file: every other
/// referencing chapter is left with a dangling `#fn-name` fragment. That case
/// gets a [`Warning`] instead of a silent dangling link (no cross-file href
/// rewriting is attempted).
pub(crate) fn split_chapters<'a>(
    root: Node<'a>,
    split_level: u8,
    warnings: &mut Vec<Warning>,
) -> Vec<Chunk<'a>> {
    let mut chunks = Vec::new();
    let mut current: Vec<Node<'a>> = Vec::new();
    let mut footnote_defs: Vec<Node<'a>> = Vec::new();

    for child in root.children() {
        match &child.data.borrow().value {
            NodeValue::FrontMatter(_) => continue,
            NodeValue::FootnoteDefinition(_) => {
                footnote_defs.push(child);
                continue;
            }
            _ => {}
        }
        if is_split_heading(child, split_level) && !current.is_empty() {
            chunks.push(build_chunk(std::mem::take(&mut current), split_level));
        }
        current.push(child);
    }
    if !current.is_empty() {
        chunks.push(build_chunk(current, split_level));
    }

    attach_footnote_definitions(&mut chunks, footnote_defs, warnings);
    chunks
}

/// Re-home each footnote definition with the first chunk that references it
/// (matched by footnote name), appending it to that chunk's nodes so it
/// renders in the same `format_html` call as its reference and their ids
/// agree. A definition nothing references anywhere falls back to the last
/// chunk, matching comrak's own default end-of-document placement.
///
/// A definition referenced from more than one chunk records a [`Warning`]
/// naming the footnote and every referencing chapter, since only the first
/// keeps a resolvable reference; the rest link to a fragment that will not
/// exist in their own chapter file.
fn attach_footnote_definitions<'a>(
    chunks: &mut [Chunk<'a>],
    footnote_defs: Vec<Node<'a>>,
    warnings: &mut Vec<Warning>,
) {
    for def in footnote_defs {
        let name = match &def.data.borrow().value {
            NodeValue::FootnoteDefinition(nfd) => nfd.name.clone(),
            _ => continue,
        };
        let referencing: Vec<usize> = chunks
            .iter()
            .enumerate()
            .filter(|(_, chunk)| {
                chunk
                    .nodes
                    .iter()
                    .any(|node| references_footnote(node, &name))
            })
            .map(|(index, _)| index)
            .collect();
        if referencing.len() > 1 {
            let chapters: Vec<String> = referencing
                .iter()
                .map(|&index| chunk_label(chunks, index))
                .collect();
            warnings.push(Warning {
                message: format!(
                    "footnote '{name}' is referenced from more than one chapter ({}); \
                     the definition stays with the first, so the reference in the rest \
                     will point at a fragment that does not exist in their chapter file",
                    chapters.join(", ")
                ),
                file: None,
            });
        }
        let target = referencing
            .first()
            .copied()
            .unwrap_or_else(|| chunks.len().saturating_sub(1));
        if let Some(chunk) = chunks.get_mut(target) {
            chunk.nodes.push(def);
        }
    }
}

/// A human-readable label for the chunk at `index`, for a footnote warning:
/// its own heading title, or `"chapter N"` (1-based) for the front chunk,
/// which has none.
fn chunk_label(chunks: &[Chunk<'_>], index: usize) -> String {
    let chunk = &chunks[index];
    if chunk.title.is_empty() {
        format!("chapter {}", index + 1)
    } else {
        chunk.title.clone()
    }
}

/// Whether `node` (or any of its descendants) is a `FootnoteReference` for
/// `name`.
fn references_footnote(node: Node<'_>, name: &str) -> bool {
    node.descendants().any(
        |d| matches!(&d.data.borrow().value, NodeValue::FootnoteReference(nfr) if nfr.name == name),
    )
}

/// The text of the first level-1 heading anywhere in the document (searched
/// depth-first, so a heading nested in e.g. a block quote is still found),
/// for the book title fallback. `None` if the document has no H1.
pub(crate) fn first_h1_text(root: Node<'_>) -> Option<String> {
    for node in root.descendants() {
        if let NodeValue::Heading(h) = &node.data.borrow().value
            && h.level == 1
        {
            let text = collapse_whitespace(&node_text(node));
            return (!text.is_empty()).then_some(text);
        }
    }
    None
}

fn is_split_heading(node: Node<'_>, split_level: u8) -> bool {
    matches!(&node.data.borrow().value, NodeValue::Heading(h) if h.level <= split_level)
}

fn build_chunk<'a>(nodes: Vec<Node<'a>>, split_level: u8) -> Chunk<'a> {
    let mut seen: HashMap<String, u32> = HashMap::new();

    let first_is_heading = is_split_heading(nodes[0], split_level);
    let (title, heading_slug) = if first_is_heading {
        let text = collapse_whitespace(&node_text(nodes[0]));
        let slug = dedupe_slug(&slugify(&text), &mut seen);
        (text, Some(slug))
    } else {
        (String::new(), None)
    };

    let sub_level = split_level + 1;
    let mut sub_headings = Vec::new();
    for top in &nodes {
        for node in top.descendants() {
            if let NodeValue::Heading(h) = &node.data.borrow().value
                && h.level == sub_level
            {
                let text = collapse_whitespace(&node_text(node));
                let slug = dedupe_slug(&slugify(&text), &mut seen);
                sub_headings.push(SubHeading { slug, title: text });
            }
        }
    }

    Chunk {
        nodes,
        title,
        heading_slug,
        sub_headings,
    }
}

/// Concatenate the literal text of a node's inline content (text, inline
/// code, and a space for each soft/hard line break), for use as a heading's
/// display title.
fn node_text(node: Node<'_>) -> String {
    let mut out = String::new();
    collect_text(node, &mut out);
    out
}

fn collect_text(node: Node<'_>, out: &mut String) {
    match &node.data.borrow().value {
        NodeValue::Text(t) => out.push_str(t),
        NodeValue::Code(c) => out.push_str(&c.literal),
        NodeValue::SoftBreak | NodeValue::LineBreak => out.push(' '),
        _ => {}
    }
    for child in node.children() {
        collect_text(child, out);
    }
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lowercase, alphanumeric-and-hyphens slug: every run of non-alphanumeric
/// characters becomes a single hyphen, leading/trailing hyphens are trimmed.
/// Falls back to `"section"` if nothing alphanumeric survives.
pub(crate) fn slugify(text: &str) -> String {
    let mut slug = String::with_capacity(text.len());
    let mut prev_hyphen = true; // avoid a leading hyphen
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                slug.push(lower);
            }
            prev_hyphen = false;
        } else if !prev_hyphen {
            slug.push('-');
            prev_hyphen = true;
        }
    }
    let trimmed = slug.trim_end_matches('-');
    if trimmed.is_empty() {
        "section".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Dedupe `base` against `seen`: the first occurrence keeps `base` as-is,
/// later ones get `-2`, `-3`, ...
fn dedupe_slug(base: &str, seen: &mut HashMap<String, u32>) -> String {
    let count = seen.entry(base.to_string()).or_insert(0);
    *count += 1;
    if *count == 1 {
        base.to_string()
    } else {
        format!("{base}-{count}")
    }
}

/// Set the precomputed slug ids (see [`Chunk::heading_ids_in_order`]) onto the
/// chunk's rendered-then-parsed `<hN>` elements, matched positionally: DOM
/// elements at levels `1..=split_level+1` appear in the same document order as
/// the AST headings [`build_chunk`] collected them from.
pub(crate) fn assign_heading_ids(doc: &NodeRef, ids: &[String], split_level: u8) {
    if ids.is_empty() {
        return;
    }
    let max_level = split_level + 1;
    let headings: Vec<NodeRef> = doc
        .inclusive_descendants()
        .filter(|n| is_heading_up_to(n, max_level))
        .collect();
    for (node, id) in headings.iter().zip(ids) {
        set_attr(node, "id", id);
    }
}

fn is_heading_up_to(node: &NodeRef, max_level: u8) -> bool {
    let Some(name) = local_name(node) else {
        return false;
    };
    (1..=max_level).any(|level| name == format!("h{level}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::{parse_xhtml, serialize_xhtml};
    use crate::markdown::render::comrak_options;
    use comrak::Arena;

    // Helper macro-ish: since `Node<'a>` borrows the arena, tests build the
    // arena/options/root together and run the assertions in the same scope.
    macro_rules! with_root {
        ($md:expr, $split_level:expr, |$root:ident| $body:block) => {{
            let arena = Arena::new();
            let options = comrak_options();
            let $root = comrak::parse_document(&arena, $md, &options);
            let _ = $split_level;
            $body
        }};
    }

    #[test]
    fn slugify_lowercases_and_hyphenates() {
        assert_eq!(slugify("Chapter One"), "chapter-one");
        assert_eq!(slugify("Café  &  Bar!!"), "café-bar");
        assert_eq!(slugify("---"), "section");
        assert_eq!(slugify(""), "section");
    }

    #[test]
    fn dedupe_slug_appends_dash_two_then_three() {
        let mut seen = HashMap::new();
        assert_eq!(dedupe_slug("intro", &mut seen), "intro");
        assert_eq!(dedupe_slug("intro", &mut seen), "intro-2");
        assert_eq!(dedupe_slug("intro", &mut seen), "intro-3");
    }

    #[test]
    fn no_split_heading_becomes_one_front_chunk() {
        with_root!("Just a paragraph.\n\nAnother one.\n", 1u8, |root| {
            let mut warnings = Vec::new();
            let chunks = split_chapters(root, 1, &mut warnings);
            assert_eq!(chunks.len(), 1);
            assert!(chunks[0].is_front());
            assert!(chunks[0].sub_headings.is_empty());
        });
    }

    #[test]
    fn splits_on_h1_with_pre_heading_front_chunk() {
        with_root!(
            "Intro text.\n\n# Chapter One\n\nBody one.\n\n# Chapter Two\n\nBody two.\n",
            1u8,
            |root| {
                let mut warnings = Vec::new();
                let chunks = split_chapters(root, 1, &mut warnings);
                assert_eq!(chunks.len(), 3, "front + 2 chapters");
                assert!(chunks[0].is_front());
                assert!(!chunks[1].is_front());
                assert_eq!(chunks[1].title, "Chapter One");
                assert_eq!(chunks[1].heading_slug.as_deref(), Some("chapter-one"));
                assert_eq!(chunks[2].title, "Chapter Two");
            }
        );
    }

    #[test]
    fn no_pre_heading_content_means_no_front_chunk() {
        with_root!("# Chapter One\n\nBody.\n", 1u8, |root| {
            let mut warnings = Vec::new();
            let chunks = split_chapters(root, 1, &mut warnings);
            assert_eq!(chunks.len(), 1);
            assert!(!chunks[0].is_front());
            assert_eq!(chunks[0].title, "Chapter One");
        });
    }

    #[test]
    fn split_level_two_splits_on_h1_and_h2() {
        with_root!(
            "# Part One\n\n## Section A\n\nText.\n\n## Section B\n\nText.\n\n# Part Two\n\nText.\n",
            2u8,
            |root| {
                let mut warnings = Vec::new();
                let chunks = split_chapters(root, 2, &mut warnings);
                assert_eq!(
                    chunks.len(),
                    4,
                    "Part One / Section A / Section B / Part Two"
                );
                assert_eq!(chunks[0].title, "Part One");
                assert_eq!(chunks[1].title, "Section A");
                assert_eq!(chunks[2].title, "Section B");
                assert_eq!(chunks[3].title, "Part Two");
                // At split_level 2, "one level below" is H3: none exist here.
                for chunk in &chunks {
                    assert!(chunk.sub_headings.is_empty());
                }
            }
        );
    }

    #[test]
    fn sub_headings_one_level_below_split_level_are_collected() {
        with_root!(
            "# Chapter One\n\n## Section A\n\nText.\n\n## Section B\n\nText.\n",
            1u8,
            |root| {
                let mut warnings = Vec::new();
                let chunks = split_chapters(root, 1, &mut warnings);
                assert_eq!(chunks.len(), 1);
                let sub = &chunks[0].sub_headings;
                assert_eq!(sub.len(), 2);
                assert_eq!(sub[0].title, "Section A");
                assert_eq!(sub[1].title, "Section B");
            }
        );
    }

    #[test]
    fn duplicate_heading_text_within_a_chapter_dedupes_slugs() {
        with_root!(
            "# Chapter One\n\n## Notes\n\nA.\n\n## Notes\n\nB.\n",
            1u8,
            |root| {
                let mut warnings = Vec::new();
                let chunks = split_chapters(root, 1, &mut warnings);
                let sub = &chunks[0].sub_headings;
                assert_eq!(sub[0].slug, "notes");
                assert_eq!(sub[1].slug, "notes-2");
            }
        );
    }

    #[test]
    fn footnote_referenced_from_two_chapters_warns_and_keeps_the_first() {
        with_root!(
            "# Chapter One\n\nNote.[^1]\n\n# Chapter Two\n\nAgain.[^1]\n\n[^1]: Body.\n",
            1u8,
            |root| {
                let mut warnings = Vec::new();
                let chunks = split_chapters(root, 1, &mut warnings);
                assert_eq!(chunks.len(), 2);
                assert!(
                    warnings
                        .iter()
                        .any(|w| w.message.contains("Chapter One")
                            && w.message.contains("Chapter Two")),
                    "expected a warning naming both referencing chapters: {warnings:?}"
                );
                // The definition is only ever homed in one chunk (the first
                // referencing one) - no cross-file href rewriting attempted.
                let def_count: usize = chunks
                    .iter()
                    .map(|c| {
                        c.nodes
                            .iter()
                            .filter(|n| {
                                matches!(n.data.borrow().value, NodeValue::FootnoteDefinition(_))
                            })
                            .count()
                    })
                    .sum();
                assert_eq!(
                    def_count, 1,
                    "the definition must land in exactly one chapter"
                );
            }
        );
    }

    #[test]
    fn footnote_referenced_from_one_chapter_does_not_warn() {
        with_root!("# Chapter One\n\nNote.[^1]\n\n[^1]: Body.\n", 1u8, |root| {
            let mut warnings = Vec::new();
            split_chapters(root, 1, &mut warnings);
            assert!(
                warnings.is_empty(),
                "a single-chapter footnote must not warn: {warnings:?}"
            );
        });
    }

    #[test]
    fn first_h1_text_finds_the_first_level_one_heading() {
        with_root!(
            "Some intro.\n\n## Not this one\n\n# The Real Title\n\nBody.\n",
            1u8,
            |root| {
                assert_eq!(first_h1_text(root), Some("The Real Title".to_string()));
            }
        );
    }

    #[test]
    fn first_h1_text_is_none_without_any_h1() {
        with_root!("## Only an H2\n\nBody.\n", 1u8, |root| {
            assert_eq!(first_h1_text(root), None);
        });
    }

    #[test]
    fn assign_heading_ids_sets_ids_in_document_order() {
        let doc = parse_xhtml(
            b"<html><body><h1>Chapter One</h1><p>t</p><h2>Section A</h2><p>t</p><h2>Section B</h2></body></html>",
        )
        .expect("parses");
        assign_heading_ids(
            &doc,
            &[
                "chapter-one".to_string(),
                "section-a".to_string(),
                "section-b".to_string(),
            ],
            1,
        );
        let out = String::from_utf8(serialize_xhtml(&doc)).unwrap();
        assert!(out.contains(r#"<h1 id="chapter-one">"#), "got: {out}");
        assert!(out.contains(r#"<h2 id="section-a">"#), "got: {out}");
        assert!(out.contains(r#"<h2 id="section-b">"#), "got: {out}");
    }

    #[test]
    fn assign_heading_ids_ignores_headings_deeper_than_split_level_plus_one() {
        let doc = parse_xhtml(b"<html><body><h1>Chapter</h1><h3>Deep</h3></body></html>")
            .expect("parses");
        // split_level 1 -> only h1/h2 are addressed; the lone id is the chapter's.
        assign_heading_ids(&doc, &["chapter".to_string()], 1);
        let out = String::from_utf8(serialize_xhtml(&doc)).unwrap();
        assert!(out.contains(r#"<h1 id="chapter">"#), "got: {out}");
        assert!(!out.contains("<h3 id"), "h3 must not get an id: {out}");
    }
}
