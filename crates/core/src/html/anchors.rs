//! Anchor id relocation, aliasing and the per-document cap.
//!
//! The firmware only honors link targets that are `id`s on *block* elements
//! (ids on inline elements are dropped), and caps a document at 1024 anchors.
//! We move every inline id up to its nearest block ancestor. When that block
//! already carries an id we cannot move a second one, so we drop the inline id
//! and record an alias so references to it can be pointed at the block's id -
//! same-document references now, cross-document references in a second pass
//! ([`apply_anchor_aliases`]) once every chapter's aliases are known.

use std::collections::{HashMap, HashSet};

use kuchikiki::NodeRef;

use crate::epub::model::{TocEntry, normalize_href};
use crate::html::dom::{collect_by_name, get_attr, local_name, remove_attr, set_attr};
use crate::report::{Transformation, Warning};

/// The book's anchor aliases: (chapter zip path, dropped inline id) -> the
/// surviving block id in that chapter that references should be redirected to.
/// Keys are path-qualified because different chapters routinely reuse the same
/// id string (per-chapter footnote ids like `fn1`); a bare-id key would let one
/// chapter's alias hijack a reference to another chapter's still-valid id.
pub type AliasMap = HashMap<(String, String), String>;

/// The device drops anchors past this many per document.
pub(crate) const ID_CAP: usize = 1024;

/// Inline elements whose `id` must be relocated to a block ancestor.
const INLINE_ID: &[&str] = &[
    "span", "a", "sup", "sub", "em", "i", "b", "strong", "u", "small", "code",
];

/// Block elements an inline id may be relocated onto.
const BLOCK_ANCESTORS: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "blockquote",
    "section",
    "figure",
    "figcaption",
    "article",
    "aside",
    "header",
    "footer",
    "main",
    "nav",
    "td",
    "th",
    "dt",
    "dd",
    "caption",
];

/// Move inline ids onto block ancestors in `doc`, rewrite this chapter's own
/// references to any aliased ids, and return the alias map for the cross-chapter
/// pass.
pub(crate) fn relocate_ids(
    doc: &NodeRef,
    report: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
    chapter_path: &str,
) -> AliasMap {
    let mut aliases = AliasMap::new();
    let mut relocated = 0usize;

    let targets: Vec<(NodeRef, String)> = doc
        .inclusive_descendants()
        .filter_map(|node| {
            let name = local_name(&node)?;
            if !INLINE_ID.contains(&name.as_str()) {
                return None;
            }
            let id = get_attr(&node, "id")?;
            Some((node, id))
        })
        .collect();

    for (inline, id) in targets {
        let Some(block) = inline.ancestors().find(|ancestor| {
            local_name(ancestor).is_some_and(|n| BLOCK_ANCESTORS.contains(&n.as_str()))
        }) else {
            warnings.push(Warning {
                message: format!("inline id '{id}' has no block ancestor; left in place"),
                file: Some(chapter_path.to_string()),
            });
            continue;
        };
        match get_attr(&block, "id") {
            None => {
                set_attr(&block, "id", &id);
                remove_attr(&inline, "id");
                relocated += 1;
            }
            Some(existing) => {
                remove_attr(&inline, "id");
                if existing != id {
                    aliases.insert((chapter_path.to_string(), id), existing);
                }
                relocated += 1;
            }
        }
    }

    if !aliases.is_empty() {
        for anchor in collect_by_name(doc, "a") {
            if let Some(href) = get_attr(&anchor, "href")
                && let Some(fragment) = href.strip_prefix('#')
                && let Some(new) = aliases.get(&(chapter_path.to_string(), fragment.to_string()))
            {
                set_attr(&anchor, "href", &format!("#{new}"));
            }
        }
    }

    if relocated > 0 {
        report.push(Transformation {
            kind: "anchor-relocated".to_string(),
            detail: format!("relocated {relocated} inline anchor id(s) to block elements"),
            file: Some(chapter_path.to_string()),
        });
    }
    aliases
}

/// Rewrite the fragment of any `<a href>` in `doc` (the chapter at
/// `chapter_path`) whose target document and fragment match an aliased id.
/// Applied book-wide after every chapter's aliases are merged, so
/// cross-document references (`chapter.xhtml#old`) are fixed up too: the href's
/// path part is resolved against this chapter's directory to a zip-absolute
/// path before the (path, id) lookup, so an alias only ever rewrites references
/// to its own chapter. Hrefs with a URL scheme (external links) are skipped.
pub fn apply_anchor_aliases(doc: &NodeRef, aliases: &AliasMap, chapter_path: &str) {
    if aliases.is_empty() {
        return;
    }
    let chapter_dir = parent_dir(chapter_path);
    for anchor in collect_by_name(doc, "a") {
        let Some(href) = get_attr(&anchor, "href") else {
            continue;
        };
        let Some((path, fragment)) = href.split_once('#') else {
            continue;
        };
        if has_scheme(path) {
            continue;
        }
        let target_doc = if path.is_empty() {
            chapter_path.to_string()
        } else {
            normalize_href(&chapter_dir, path)
        };
        if let Some(new) = aliases.get(&(target_doc, fragment.to_string())) {
            set_attr(&anchor, "href", &format!("{path}#{new}"));
        }
    }
}

/// Rewrite the fragment of every `book.toc` entry whose target document and
/// fragment match an aliased id. `TocEntry.href` is already a zip-absolute
/// path with an optional `#fragment` (see [`TocEntry`]), so unlike
/// [`apply_anchor_aliases`] this is a pure `(path, fragment)` lookup - no
/// `normalize_href` resolution needed. Must run after every chapter's
/// aliases are merged (book-wide, like `apply_anchor_aliases`) so
/// `nav.xhtml`/`toc.ncx`, which are regenerated verbatim from `book.toc`,
/// never point at an id that alias relocation just dropped.
pub fn apply_toc_aliases(toc: &mut [TocEntry], aliases: &AliasMap) {
    if aliases.is_empty() {
        return;
    }
    for entry in toc.iter_mut() {
        let Some((path, fragment)) = entry.href.split_once('#') else {
            continue;
        };
        if let Some(new) = aliases.get(&(path.to_string(), fragment.to_string())) {
            entry.href = format!("{path}#{new}");
        }
    }
}

/// Whether an href's path part starts with a URL scheme (RFC 3986:
/// `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"`), i.e. is an external
/// reference rather than a relative path within the book.
fn has_scheme(path: &str) -> bool {
    let Some(colon) = path.find(':') else {
        return false;
    };
    let scheme = &path[..colon];
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Parent directory of a zip-absolute path (`""` if it has no `/`).
fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(idx) => path[..idx].to_string(),
        None => String::new(),
    }
}

/// Enforce the [`ID_CAP`]. Only drops ids not referenced anywhere in the book
/// (`referenced`), so live links keep working; warns about what was dropped and,
/// if referenced ids alone still exceed the cap, that the device will drop some.
pub(crate) fn cap_ids(
    doc: &NodeRef,
    referenced: &HashSet<String>,
    warnings: &mut Vec<Warning>,
    chapter_path: &str,
) {
    let with_id: Vec<NodeRef> = doc
        .inclusive_descendants()
        .filter(|n| get_attr(n, "id").is_some())
        .collect();
    if with_id.len() <= ID_CAP {
        return;
    }

    let mut dropped = 0usize;
    for node in &with_id {
        if let Some(id) = get_attr(node, "id")
            && !referenced.contains(&id)
        {
            remove_attr(node, "id");
            dropped += 1;
        }
    }
    if dropped > 0 {
        warnings.push(Warning {
            message: format!(
                "dropped {dropped} unreferenced anchor id(s) over the {ID_CAP} per-document cap"
            ),
            file: Some(chapter_path.to_string()),
        });
    }
    let remaining = with_id.len() - dropped;
    if remaining > ID_CAP {
        warnings.push(Warning {
            message: format!(
                "{remaining} referenced anchor ids remain over the {ID_CAP} cap; \
                 the device will drop some"
            ),
            file: Some(chapter_path.to_string()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::testutil::{doc_from_body, serialize};

    fn relocate(body: &str) -> (String, AliasMap, Vec<Transformation>, Vec<Warning>) {
        let doc = doc_from_body(body);
        let mut report = Vec::new();
        let mut warnings = Vec::new();
        let aliases = relocate_ids(&doc, &mut report, &mut warnings, "ch.xhtml");
        (serialize(&doc), aliases, report, warnings)
    }

    #[test]
    fn span_id_moves_to_block_snapshot() {
        let (out, aliases, report, warnings) = relocate("<p><span id=\"x\">hi</span></p>");
        insta::assert_snapshot!(out);
        assert!(aliases.is_empty(), "no conflict, so no alias");
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].kind, "anchor-relocated");
        assert!(warnings.is_empty());
    }

    /// Convenience: the alias registry key for chapter `path` and old id `id`.
    fn key(path: &str, id: &str) -> (String, String) {
        (path.to_string(), id.to_string())
    }

    #[test]
    fn id_conflict_aliases_and_rewrites_same_doc_snapshot() {
        let (out, aliases, _, _) =
            relocate("<p id=\"para\"><sup id=\"fn1\">1</sup></p><p><a href=\"#fn1\">back</a></p>");
        insta::assert_snapshot!(out);
        assert_eq!(
            aliases.get(&key("ch.xhtml", "fn1")),
            Some(&"para".to_string())
        );
    }

    #[test]
    fn cross_file_reference_is_rewritten_in_second_pass() {
        // Chapter 1 aliases fn1 -> para.
        let ch1 = doc_from_body("<p id=\"para\"><sup id=\"fn1\">1</sup></p>");
        let mut report = Vec::new();
        let mut warnings = Vec::new();
        let aliases = relocate_ids(&ch1, &mut report, &mut warnings, "text/ch1.xhtml");
        assert_eq!(
            aliases.get(&key("text/ch1.xhtml", "fn1")),
            Some(&"para".to_string())
        );

        // Chapter 2, in the same directory, links to it cross-file; the
        // book-wide pass resolves the relative href and rewrites it.
        let ch2 = doc_from_body("<p><a href=\"ch1.xhtml#fn1\">see</a></p>");
        apply_anchor_aliases(&ch2, &aliases, "text/ch2.xhtml");
        let out = serialize(&ch2);
        assert!(out.contains(r##"href="ch1.xhtml#para""##), "got: {out}");
    }

    #[test]
    fn alias_from_one_chapter_does_not_hijack_same_id_in_another() {
        // Chapter A: fn1 conflicts with an existing block id -> aliased.
        let a = doc_from_body("<p id=\"para\"><sup id=\"fn1\">1</sup></p>");
        let mut report = Vec::new();
        let mut warnings = Vec::new();
        let mut aliases = relocate_ids(&a, &mut report, &mut warnings, "a.xhtml");
        // Chapter B: fn1 relocates cleanly onto its block -> no alias.
        let b = doc_from_body("<p><sup id=\"fn1\">1</sup></p>");
        aliases.extend(relocate_ids(&b, &mut report, &mut warnings, "b.xhtml"));

        // Chapter C references both chapters' fn1: only A's is aliased, so
        // only the a.xhtml ref may be rewritten.
        let c = doc_from_body(
            "<p><a href=\"a.xhtml#fn1\">to a</a> <a href=\"b.xhtml#fn1\">to b</a></p>",
        );
        apply_anchor_aliases(&c, &aliases, "c.xhtml");
        let out = serialize(&c);
        assert!(
            out.contains(r##"href="a.xhtml#para""##),
            "aliased ref must be rewritten: {out}"
        );
        assert!(
            out.contains(r##"href="b.xhtml#fn1""##),
            "ref to the unaliased chapter's id must stay unchanged: {out}"
        );
    }

    #[test]
    fn external_href_with_matching_fragment_is_left_alone() {
        let a = doc_from_body("<p id=\"para\"><sup id=\"fn1\">1</sup></p>");
        let mut report = Vec::new();
        let mut warnings = Vec::new();
        let aliases = relocate_ids(&a, &mut report, &mut warnings, "a.xhtml");

        let c = doc_from_body("<p><a href=\"https://example.com/a.xhtml#fn1\">x</a></p>");
        apply_anchor_aliases(&c, &aliases, "c.xhtml");
        let out = serialize(&c);
        assert!(
            out.contains(r##"href="https://example.com/a.xhtml#fn1""##),
            "external links must never be rewritten: {out}"
        );
    }

    #[test]
    fn inline_id_with_no_block_ancestor_warns_and_is_left_in_place() {
        // A <span id> directly under <body>, with no block ancestor to
        // relocate onto, is silently kept by relocate_ids - but the
        // firmware only honors ids on block elements, so the id is
        // effectively dead. That must be surfaced as a warning.
        let (out, aliases, _, warnings) = relocate("<span id=\"x\">t</span>");
        assert!(
            out.contains("id=\"x\""),
            "the id must be left untouched: {out}"
        );
        assert!(aliases.is_empty());
        assert_eq!(warnings.len(), 1, "got: {warnings:?}");
        assert!(warnings[0].message.contains('x'));
        assert!(warnings[0].message.contains("no block ancestor"));
        assert_eq!(warnings[0].file.as_deref(), Some("ch.xhtml"));
    }

    #[test]
    fn duplicate_value_id_is_deduplicated() {
        // Inline id equal to the block's existing id must not produce a
        // duplicate id (which would be invalid XML).
        let (out, _, _, _) = relocate("<p id=\"x\"><span id=\"x\">t</span></p>");
        assert_eq!(out.matches("id=\"x\"").count(), 1, "got: {out}");
    }

    fn toc_entry(href: &str) -> TocEntry {
        TocEntry {
            title: "T".to_string(),
            href: href.to_string(),
            level: 1,
        }
    }

    #[test]
    fn toc_href_fragment_follows_the_alias() {
        let mut aliases = AliasMap::new();
        aliases.insert(key("text/ch1.xhtml", "fn1"), "para".to_string());
        let mut toc = vec![toc_entry("text/ch1.xhtml#fn1")];
        apply_toc_aliases(&mut toc, &aliases);
        assert_eq!(toc[0].href, "text/ch1.xhtml#para");
    }

    #[test]
    fn toc_entry_with_no_fragment_is_untouched() {
        let mut aliases = AliasMap::new();
        aliases.insert(key("text/ch1.xhtml", "fn1"), "para".to_string());
        let mut toc = vec![toc_entry("text/ch1.xhtml")];
        apply_toc_aliases(&mut toc, &aliases);
        assert_eq!(toc[0].href, "text/ch1.xhtml");
    }

    #[test]
    fn toc_entry_with_unmatched_fragment_is_untouched() {
        let mut aliases = AliasMap::new();
        aliases.insert(key("text/ch1.xhtml", "fn1"), "para".to_string());
        let mut toc = vec![toc_entry("text/ch1.xhtml#fn2")];
        apply_toc_aliases(&mut toc, &aliases);
        assert_eq!(toc[0].href, "text/ch1.xhtml#fn2");
    }

    #[test]
    fn toc_entry_in_another_chapter_is_not_hijacked() {
        // Only ch1's fn1 is aliased; a ch2 entry with the same fragment string
        // must not be rewritten by it.
        let mut aliases = AliasMap::new();
        aliases.insert(key("text/ch1.xhtml", "fn1"), "para".to_string());
        let mut toc = vec![toc_entry("text/ch2.xhtml#fn1")];
        apply_toc_aliases(&mut toc, &aliases);
        assert_eq!(toc[0].href, "text/ch2.xhtml#fn1");
    }

    #[test]
    fn cap_drops_unreferenced_ids_over_the_limit() {
        let mut body = String::new();
        for i in 0..(ID_CAP + 5) {
            body.push_str(&format!("<p id=\"p{i}\">t</p>"));
        }
        let doc = doc_from_body(&body);
        let mut referenced = HashSet::new();
        referenced.insert("p0".to_string());
        let mut warnings = Vec::new();
        cap_ids(&doc, &referenced, &mut warnings, "ch.xhtml");
        let out = serialize(&doc);
        assert!(out.contains("id=\"p0\""), "referenced id must survive");
        assert!(
            !out.contains("id=\"p1\""),
            "unreferenced id must be dropped"
        );
        assert_eq!(warnings.len(), 1);
    }
}
