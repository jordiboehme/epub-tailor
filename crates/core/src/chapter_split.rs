//! Splitting an oversize spine chapter into parts at `<body>` block
//! boundaries.
//!
//! The device turns a 200KB+ spine document into a section with hundreds (or
//! thousands) of "pages" that stalls its indexer (see
//! `docs/device-constraints.md`), so any spine XHTML whose serialized size
//! exceeds [`ConvertOptions::max_chapter_bytes`] is cut into `<stem>-1.xhtml`,
//! `<stem>-2.xhtml`, ... parts. This runs after every other chapter transform
//! (anchors relocated and aliased, ids capped, images rewritten, CSS linked),
//! so it only has to reason about the final DOM, not any earlier pass.
//!
//! The hard part is bookkeeping: every internal reference to the original
//! file, anywhere in the book, has to keep working once its target has moved
//! into a numbered part. [`split_oversize_chapters`] builds an anchor map (an
//! id in the original document -> the part it landed in) while splitting, then
//! uses it to retarget the spine, the table of contents and every chapter's
//! `<a href>`s in one pass - `href="orig.xhtml"` (no fragment) always lands on
//! part 1; `href="orig.xhtml#id"` and same-document `href="#id"` land on
//! whichever part actually holds `id` now.
//!
//! This is a different map from [`crate::html::AliasMap`] (M3's inline-id ->
//! block-id registry): that one is fully applied and consumed earlier in
//! [`crate::convert`], before this pass ever runs, so there is nothing left of
//! it to reconcile with - every id here is already a block-level id sitting on
//! its final element.

use std::collections::{HashMap, HashSet};

use kuchikiki::NodeRef;

use crate::epub::model::{Book, normalize_href};
use crate::epub::relative_href;
use crate::html::dom::{
    child_elements, collect_by_name, find_head, get_attr, is_named, local_name, set_attr, text,
};
use crate::html::{find_body, parse_xhtml, serialize_fragment, serialize_xhtml};
use crate::options::ConvertOptions;
use crate::report::{Transformation, Warning};

/// Heading local names that make an attractive split point.
const HEADINGS: [&str; 6] = ["h1", "h2", "h3", "h4", "h5", "h6"];

/// Anchor id -> the part file it landed in, keyed by `(original chapter path,
/// id)` so ids that happen to repeat across chapters (per-chapter footnote ids
/// like `fn1`) never collide - the same path-qualification [`crate::html::AliasMap`]
/// uses, and for the same reason.
type AnchorMap = HashMap<(String, String), String>;

/// Split every spine chapter in `chapters` whose serialized size exceeds
/// `opts.max_chapter_bytes`, retargeting `book.spine`, `book.toc` and every
/// chapter's internal `<a href>`s so nothing dangles. Returns how many
/// chapters were split (not the number of parts produced).
///
/// Chapters below the limit are left in `chapters` untouched, so existing
/// small fixtures never trigger a split.
pub(crate) fn split_oversize_chapters(
    book: &mut Book,
    chapters: &mut Vec<(String, NodeRef)>,
    opts: &ConvertOptions,
    transformations: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
) -> u32 {
    let mut reserved: HashSet<String> = book.resources.keys().cloned().collect();
    let mut parts_of: HashMap<String, Vec<String>> = HashMap::new();
    let mut anchor_map: AnchorMap = HashMap::new();
    let mut source_of: HashMap<String, String> = HashMap::new();
    let mut chapters_split = 0u32;

    let old_chapters = std::mem::take(chapters);
    let mut new_chapters: Vec<(String, NodeRef)> = Vec::with_capacity(old_chapters.len());

    for (path, doc) in old_chapters {
        match plan_and_split(&path, &doc, opts, &mut reserved, warnings) {
            Some((parts, detail)) => {
                let part_paths: Vec<String> = parts.iter().map(|p| p.path.clone()).collect();
                for part in &parts {
                    for id in &part.ids {
                        anchor_map.insert((path.clone(), id.clone()), part.path.clone());
                    }
                }
                transformations.push(Transformation {
                    kind: "chapter-split".to_string(),
                    detail,
                    file: Some(path.clone()),
                });

                parts_of.insert(path.clone(), part_paths.clone());
                book.resources.shift_remove(&path);
                chapters_split += 1;
                for part in parts {
                    source_of.insert(part.path.clone(), path.clone());
                    new_chapters.push((part.path, part.doc));
                }
            }
            None => new_chapters.push((path, doc)),
        }
    }

    if !parts_of.is_empty() {
        retarget_spine(book, &parts_of);
        retarget_toc(book, &parts_of, &anchor_map);
        retarget_hrefs(&new_chapters, &source_of, &parts_of, &anchor_map);
    }

    *chapters = new_chapters;
    chapters_split
}

/// One part produced by splitting a chapter: its new zip-absolute path, its
/// document and every anchor id that ended up inside it (for the book-wide
/// anchor map).
struct SplitPart {
    path: String,
    doc: NodeRef,
    ids: Vec<String>,
}

/// Decide whether `doc` (the chapter at `path`) needs to be split, and if so,
/// build its parts. Returns `None` when the chapter is within the limit, has
/// no block boundary to split on or planning collapses back to a single
/// part - all of which leave the chapter whole.
fn plan_and_split(
    path: &str,
    doc: &NodeRef,
    opts: &ConvertOptions,
    reserved: &mut HashSet<String>,
    warnings: &mut Vec<Warning>,
) -> Option<(Vec<SplitPart>, String)> {
    let full_bytes = serialize_xhtml(doc);
    if full_bytes.len() <= opts.max_chapter_bytes {
        return None;
    }

    let body = find_body(doc)?;
    let blocks = child_elements(&body);
    if blocks.len() < 2 {
        warnings.push(Warning {
            message: format!(
                "{path} is {}KB, over the {}KB chapter limit, with no block boundary to split \
                 on; left whole",
                kb(full_bytes.len()),
                kb(opts.max_chapter_bytes)
            ),
            file: Some(path.to_string()),
        });
        return None;
    }

    let sizes: Vec<usize> = blocks.iter().map(|b| serialize_fragment(b).len()).collect();
    let is_heading: Vec<bool> = blocks
        .iter()
        .map(|b| local_name(b).is_some_and(|n| HEADINGS.contains(&n.as_str())))
        .collect();
    let shell_bytes = full_bytes.len().saturating_sub(sizes.iter().sum());
    let budget = opts.max_chapter_bytes.saturating_sub(shell_bytes).max(1);

    let ranges = plan_parts(&sizes, &is_heading, budget);
    if ranges.len() < 2 {
        return None;
    }

    for &(s, e) in &ranges {
        if e - s == 1 && sizes[s] > budget {
            warnings.push(Warning {
                message: format!(
                    "a block in {path} is {}KB, over the {}KB chapter limit; kept it whole \
                     rather than split inside it",
                    kb(sizes[s]),
                    kb(opts.max_chapter_bytes)
                ),
                file: Some(path.to_string()),
            });
        }
    }

    let parts = build_parts(path, &full_bytes, &ranges, reserved);
    let detail = format!(
        "{} {}KB -> {} parts",
        basename(path),
        kb(full_bytes.len()),
        parts.len()
    );
    Some((parts, detail))
}

/// Plan how to cut `sizes.len()` top-level blocks (each `sizes[i]` bytes, with
/// `is_heading[i]` marking a heading) into parts whose content fits `budget`
/// bytes, returning `(start, end)` (end-exclusive) block index ranges.
///
/// Splits only ever fall between blocks. Within the blocks that still fit a
/// part (a byte budget from the part's start), the cut prefers whichever
/// heading lands nearest the part's size midpoint (`budget / 2`); with no
/// heading in that window, it falls back to the greedy pack boundary - the
/// most content that still fits `budget`. A single block that alone exceeds
/// `budget` gets its own part (the caller warns about these).
fn plan_parts(sizes: &[usize], is_heading: &[bool], budget: usize) -> Vec<(usize, usize)> {
    let n = sizes.len();
    let mut cum = vec![0usize; n + 1];
    for i in 0..n {
        cum[i + 1] = cum[i] + sizes[i];
    }

    let mut ranges = Vec::new();
    let mut start = 0usize;
    while start < n {
        if cum[n] - cum[start] <= budget {
            ranges.push((start, n));
            break;
        }
        if sizes[start] > budget {
            ranges.push((start, start + 1));
            start += 1;
            continue;
        }

        let mut end = start + 1;
        while end < n && cum[end + 1] - cum[start] <= budget {
            end += 1;
        }

        let target = cum[start] + budget / 2;
        let cut = (start + 1..=end)
            .filter(|&i| is_heading[i])
            .min_by_key(|&i| cum[i].abs_diff(target))
            .unwrap_or(end);

        ranges.push((start, cut));
        start = cut;
    }
    ranges
}

/// Build the split-part documents for `ranges`, reparsing `full_bytes` fresh
/// for each part (kuchikiki has no subtree-clone primitive) and detaching
/// every top-level `<body>` block outside that part's range. Part paths are
/// reserved uniquely against `reserved` (updated in place) so a split never
/// collides with an existing resource or an earlier part.
fn build_parts(
    orig_path: &str,
    full_bytes: &[u8],
    ranges: &[(usize, usize)],
    reserved: &mut HashSet<String>,
) -> Vec<SplitPart> {
    let mut parts = Vec::with_capacity(ranges.len());
    for (index, &(s, e)) in ranges.iter().enumerate() {
        let Ok(doc) = parse_xhtml(full_bytes) else {
            continue;
        };
        let Some(body) = find_body(&doc) else {
            continue;
        };
        for (i, block) in child_elements(&body).into_iter().enumerate() {
            if i < s || i >= e {
                block.detach();
            }
        }
        if index > 0 {
            append_contd_to_title(&doc);
        }
        let ids = collect_ids(&doc);
        let path = reserve_part_path(orig_path, index + 1, reserved);
        parts.push(SplitPart { path, doc, ids });
    }
    parts
}

/// Every `id` attribute value anywhere in `doc`, in document order.
fn collect_ids(doc: &NodeRef) -> Vec<String> {
    doc.inclusive_descendants()
        .filter_map(|n| get_attr(&n, "id"))
        .collect()
}

/// Append `" (contd.)"` to `doc`'s `<title>` text, for part 2 and later.
fn append_contd_to_title(doc: &NodeRef) {
    let Some(head) = find_head(doc) else { return };
    for child in head.children() {
        if is_named(&child, "title") {
            child.append(text(" (contd.)"));
            break;
        }
    }
}

/// A unique zip path `<stem>-<n>.<ext>` for the `n`-th part of `orig_path`,
/// reserving the chosen path in `reserved` so a later part or an existing
/// resource never collides with it.
fn reserve_part_path(orig_path: &str, n: usize, reserved: &mut HashSet<String>) -> String {
    let dir = parent_dir(orig_path);
    let (stem, ext) = stem_and_ext(basename(orig_path));
    let name = |suffix: &str| {
        if ext.is_empty() {
            format!("{stem}-{n}{suffix}")
        } else {
            format!("{stem}-{n}{suffix}.{ext}")
        }
    };
    let mut candidate = join_dir(&dir, &name(""));
    let mut i = 2;
    while reserved.contains(&candidate) {
        candidate = join_dir(&dir, &name(&format!("-{i}")));
        i += 1;
    }
    reserved.insert(candidate.clone());
    candidate
}

/// Replace every spine entry for an original chapter with its ordered parts.
fn retarget_spine(book: &mut Book, parts_of: &HashMap<String, Vec<String>>) {
    let mut new_spine = Vec::with_capacity(book.spine.len());
    for path in &book.spine {
        match parts_of.get(path) {
            Some(parts) => new_spine.extend(parts.iter().cloned()),
            None => new_spine.push(path.clone()),
        }
    }
    book.spine = new_spine;
}

/// Retarget every table-of-contents entry that points at an original,
/// now-split chapter: a no-fragment entry always lands on part 1; a fragment
/// entry lands on whichever part holds that id (falling back to part 1 if the
/// id was somehow not found, so the entry still resolves to something real).
fn retarget_toc(book: &mut Book, parts_of: &HashMap<String, Vec<String>>, anchor_map: &AnchorMap) {
    for entry in &mut book.toc {
        let (path, fragment) = split_href(&entry.href);
        let Some(parts) = parts_of.get(&path) else {
            continue;
        };
        let target = target_part(parts, anchor_map, &path, fragment.as_deref());
        entry.href = match fragment {
            Some(frag) => format!("{target}#{frag}"),
            None => target,
        };
    }
}

/// Retarget every chapter's internal `<a href>`s that point at an original,
/// now-split chapter, in every chapter (not just the split ones): a
/// no-fragment reference lands on part 1; a fragment reference lands on
/// whichever part now holds that id. A same-document `href="#id"` resolves
/// against the referencing part's own original (pre-split) source chapter,
/// via `source_of`, and is left as a bare fragment when the id stayed in the
/// same part - only rewritten to a cross-file reference when the split moved
/// it elsewhere.
fn retarget_hrefs(
    chapters: &[(String, NodeRef)],
    source_of: &HashMap<String, String>,
    parts_of: &HashMap<String, Vec<String>>,
    anchor_map: &AnchorMap,
) {
    for (new_path, doc) in chapters {
        let chapter_dir = parent_dir(new_path);
        let source_path = source_of
            .get(new_path)
            .cloned()
            .unwrap_or_else(|| new_path.clone());
        for anchor in collect_by_name(doc, "a") {
            let Some(href) = get_attr(&anchor, "href") else {
                continue;
            };
            let (path_part, fragment) = split_href(&href);
            if has_scheme(&path_part) {
                continue;
            }
            let target_doc = if path_part.is_empty() {
                source_path.clone()
            } else {
                normalize_href(&chapter_dir, &path_part)
            };
            let Some(parts) = parts_of.get(&target_doc) else {
                continue;
            };
            let target = target_part(parts, anchor_map, &target_doc, fragment.as_deref());

            if path_part.is_empty() && &target == new_path {
                // Still the same file after the split: leave the bare fragment.
                continue;
            }
            let new_href = match &fragment {
                Some(frag) => format!("{}#{frag}", relative_href(&chapter_dir, &target)),
                None => relative_href(&chapter_dir, &target),
            };
            set_attr(&anchor, "href", &new_href);
        }
    }
}

/// The part path a reference to `(orig_path, fragment)` should resolve to: no
/// fragment always means part 1; a fragment resolves via the anchor map,
/// falling back to part 1 if the id was not found in it.
fn target_part(
    parts: &[String],
    anchor_map: &AnchorMap,
    orig_path: &str,
    fragment: Option<&str>,
) -> String {
    match fragment {
        None => parts[0].clone(),
        Some(frag) => anchor_map
            .get(&(orig_path.to_string(), frag.to_string()))
            .cloned()
            .unwrap_or_else(|| parts[0].clone()),
    }
}

/// Split `href` into its path part and optional fragment (raw, not
/// percent-decoded - matching [`crate::html::apply_anchor_aliases`]'s
/// treatment of fragments, since anchor ids are plain NCName-like tokens that
/// never need percent-encoding in practice).
fn split_href(href: &str) -> (String, Option<String>) {
    match href.split_once('#') {
        Some((p, f)) => (p.to_string(), Some(f.to_string())),
        None => (href.to_string(), None),
    }
}

/// Whether an href's path part starts with a URL scheme (RFC 3986), i.e. is an
/// external reference rather than a relative path within the book. Mirrors
/// [`crate::html::apply_anchor_aliases`]'s identical check.
fn has_scheme(path: &str) -> bool {
    let Some(colon) = path.find(':') else {
        return false;
    };
    let scheme = &path[..colon];
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Bytes rounded to the nearest kibibyte, for report/warning text.
fn kb(bytes: usize) -> usize {
    (bytes + 512) / 1024
}

/// The file name (last path segment) of a zip-absolute path.
fn basename(path: &str) -> &str {
    match path.rfind('/') {
        Some(idx) => &path[idx + 1..],
        None => path,
    }
}

/// Parent directory of a zip-absolute path (`""` if it has no `/`).
fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(idx) => path[..idx].to_string(),
        None => String::new(),
    }
}

/// Join a directory and a file name, tolerating an empty (root) directory.
fn join_dir(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// Split a file name into its stem and (dotless) extension; no extension (or
/// a leading-dot dotfile) keeps the whole name as the stem.
fn stem_and_ext(filename: &str) -> (String, String) {
    match filename.rfind('.') {
        Some(idx) if idx > 0 => (filename[..idx].to_string(), filename[idx + 1..].to_string()),
        _ => (filename.to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epub::model::{Metadata, Resource, TocEntry};
    use crate::html::serialize_xhtml;
    use indexmap::IndexMap;

    // -----------------------------------------------------------------
    // `plan_parts`: the pure split-planning function.
    // -----------------------------------------------------------------

    #[test]
    fn plan_parts_single_part_when_everything_fits() {
        let ranges = plan_parts(&[10, 10, 10], &[false, false, false], 100);
        assert_eq!(ranges, vec![(0, 3)]);
    }

    #[test]
    fn plan_parts_splits_at_heading_nearest_the_midpoint() {
        // Blocks: h1(5) p(40) h1(5) p(40) h1(5) p(40); budget 60.
        // First part's window is (0, end] where end is the greedy pack
        // boundary; the heading at index 2 (offset 45) is nearer the midpoint
        // (30) than falling through to the greedy boundary would suggest, so
        // it must be chosen as the cut.
        let sizes = [5, 40, 5, 40, 5, 40];
        let is_heading = [true, false, true, false, true, false];
        let ranges = plan_parts(&sizes, &is_heading, 60);
        // Expect three parts, each starting on a heading.
        assert_eq!(ranges, vec![(0, 2), (2, 4), (4, 6)]);
        for &(s, _) in &ranges {
            assert!(
                is_heading[s],
                "each part must start on a heading: {ranges:?}"
            );
        }
    }

    #[test]
    fn plan_parts_falls_back_to_nearest_block_boundary_without_a_heading() {
        // No headings at all: must still split, packing greedily.
        let sizes = [30, 30, 30, 30];
        let is_heading = [false, false, false, false];
        let ranges = plan_parts(&sizes, &is_heading, 65);
        // Greedy pack: two blocks (60) fit, three (90) don't -> (0,2), then (2,4).
        assert_eq!(ranges, vec![(0, 2), (2, 4)]);
    }

    #[test]
    fn plan_parts_keeps_an_oversized_single_block_whole() {
        let sizes = [10, 500, 10];
        let is_heading = [false, false, false];
        let ranges = plan_parts(&sizes, &is_heading, 100);
        assert!(
            ranges.contains(&(1, 2)),
            "the oversized block must stand in its own part: {ranges:?}"
        );
        // Never split inside it: no range starts or ends in the middle of it.
        for &(s, e) in &ranges {
            assert!(
                !(s == 1 && e != 2),
                "must not merge past the oversized block"
            );
        }
    }

    #[test]
    fn plan_parts_never_produces_empty_or_out_of_order_ranges() {
        let sizes = [12, 34, 56, 78, 90, 11, 22];
        let is_heading = [true, false, false, true, false, false, true];
        let ranges = plan_parts(&sizes, &is_heading, 100);
        let mut expected_start = 0;
        for &(s, e) in &ranges {
            assert_eq!(s, expected_start);
            assert!(e > s, "range must not be empty: {ranges:?}");
            expected_start = e;
        }
        assert_eq!(expected_start, sizes.len());
    }

    // -----------------------------------------------------------------
    // Whole-book bookkeeping: build a tiny two-chapter book by hand and drive
    // `split_oversize_chapters` end to end.
    // -----------------------------------------------------------------

    fn heading_body(n_sections: usize, filler_bytes: usize) -> String {
        let filler = "x".repeat(filler_bytes);
        let mut body = String::new();
        for i in 1..=n_sections {
            body.push_str(&format!(
                "<h1 id=\"sec{i}\">Section {i}</h1><p>{filler}</p>"
            ));
        }
        body
    }

    fn doc_with_body(title: &str, body: &str) -> NodeRef {
        let html = format!("<html><head><title>{title}</title></head><body>{body}</body></html>");
        parse_xhtml(html.as_bytes()).expect("fixture parses")
    }

    fn book_with_two_chapters(ch_a_body: &str, ch_b_body: &str) -> (Book, Vec<(String, NodeRef)>) {
        let mut resources = IndexMap::new();
        resources.insert(
            "text/a.xhtml".to_string(),
            Resource {
                data: Vec::new(),
                media_type: "application/xhtml+xml".to_string(),
            },
        );
        resources.insert(
            "text/b.xhtml".to_string(),
            Resource {
                data: Vec::new(),
                media_type: "application/xhtml+xml".to_string(),
            },
        );
        let book = Book {
            metadata: Metadata {
                title: "T".to_string(),
                authors: vec![],
                language: "en".to_string(),
                identifier: None,
            },
            resources,
            spine: vec!["text/a.xhtml".to_string(), "text/b.xhtml".to_string()],
            toc: vec![
                TocEntry {
                    title: "A".to_string(),
                    href: "text/a.xhtml".to_string(),
                    level: 1,
                },
                TocEntry {
                    title: "A - Section 2".to_string(),
                    href: "text/a.xhtml#sec2".to_string(),
                    level: 2,
                },
                TocEntry {
                    title: "B".to_string(),
                    href: "text/b.xhtml".to_string(),
                    level: 1,
                },
            ],
            cover: None,
            opf_path: "content.opf".to_string(),
            nav_path: None,
            ncx_path: None,
        };
        let chapters = vec![
            ("text/a.xhtml".to_string(), doc_with_body("A", ch_a_body)),
            ("text/b.xhtml".to_string(), doc_with_body("B", ch_b_body)),
        ];
        (book, chapters)
    }

    fn opts_with_limit(max_chapter_bytes: usize) -> ConvertOptions {
        ConvertOptions {
            max_chapter_bytes,
            ..ConvertOptions::default()
        }
    }

    #[test]
    fn small_chapters_are_left_untouched() {
        let (mut book, mut chapters) = book_with_two_chapters(&heading_body(2, 10), "<p>short</p>");
        let opts = ConvertOptions::default();
        let mut transformations = Vec::new();
        let mut warnings = Vec::new();
        let split = split_oversize_chapters(
            &mut book,
            &mut chapters,
            &opts,
            &mut transformations,
            &mut warnings,
        );
        assert_eq!(split, 0);
        assert_eq!(book.spine, vec!["text/a.xhtml", "text/b.xhtml"]);
        assert!(transformations.is_empty());
        assert_eq!(chapters.len(), 2);
    }

    #[test]
    fn exactly_at_limit_is_untouched() {
        let (mut book, mut chapters) = book_with_two_chapters(&heading_body(1, 5), "<p>b</p>");
        let full = serialize_xhtml(&chapters[0].1).len();
        let opts = opts_with_limit(full);
        let mut transformations = Vec::new();
        let mut warnings = Vec::new();
        let split = split_oversize_chapters(
            &mut book,
            &mut chapters,
            &opts,
            &mut transformations,
            &mut warnings,
        );
        assert_eq!(split, 0, "exactly-at-limit must not trigger a split");
        assert!(transformations.is_empty());
    }

    #[test]
    fn oversize_single_block_is_not_split_and_warns() {
        let big_body = format!("<p>{}</p>", "x".repeat(2000));
        let (mut book, mut chapters) = book_with_two_chapters(&big_body, "<p>b</p>");
        let opts = opts_with_limit(200);
        let mut transformations = Vec::new();
        let mut warnings = Vec::new();
        let split = split_oversize_chapters(
            &mut book,
            &mut chapters,
            &opts,
            &mut transformations,
            &mut warnings,
        );
        assert_eq!(split, 0);
        assert!(
            warnings
                .iter()
                .any(|w| w.message.contains("no block boundary")),
            "expected a warning about the unsplittable chapter: {warnings:?}"
        );
        assert_eq!(book.spine, vec!["text/a.xhtml", "text/b.xhtml"]);
    }

    /// The full bookkeeping test: chapter A is oversize and splits into
    /// several parts; the TOC's fragment entry, a cross-chapter href from B
    /// and A's own same-doc hrefs must all retarget correctly.
    #[test]
    fn split_retargets_spine_toc_and_every_chapter_href() {
        // Three headed sections, each padded so the whole chapter blows the
        // limit; a footnote-style same-doc link from section 1 forward to
        // section 3, and one back from section 3 to section 1. Whichever
        // parts sec1/sec3 actually land in, every reference to them must
        // resolve to wherever they really ended up.
        let body_a = format!(
            "<h1 id=\"sec1\">Section 1</h1><p>{pad}</p><p><a href=\"#sec3\">to 3</a></p>\
             <h1 id=\"sec2\">Section 2</h1><p>{pad}</p>\
             <h1 id=\"sec3\">Section 3</h1><p>{pad}</p><p><a href=\"#sec1\">back to 1</a></p>",
            pad = "y".repeat(30),
        );
        let body_b = "<p>b intro</p><p><a href=\"a.xhtml#sec3\">jump to A sec3</a></p>".to_string();
        let (mut book, mut chapters) = book_with_two_chapters(&body_a, &body_b);

        // Budget derived from the fixture's own measured sizes: section 1's
        // three blocks (heading, padded paragraph, forward link) plus a
        // small margin, so section 1 fits a part on its own but section 1
        // and 2 together do not - forcing a multi-part split without pinning
        // down the planner's exact grouping beyond that.
        let doc_a = &chapters[0].1;
        let body = find_body(doc_a).unwrap();
        let blocks = child_elements(&body);
        let sizes: Vec<usize> = blocks.iter().map(|b| serialize_fragment(b).len()).collect();
        let shell = serialize_xhtml(doc_a).len() - sizes.iter().sum::<usize>();
        let max_chapter_bytes = shell + sizes[0] + sizes[1] + sizes[2] + 5;
        let opts = opts_with_limit(max_chapter_bytes);
        let mut transformations = Vec::new();
        let mut warnings = Vec::new();

        let split = split_oversize_chapters(
            &mut book,
            &mut chapters,
            &opts,
            &mut transformations,
            &mut warnings,
        );

        assert_eq!(split, 1, "only chapter A should have split");
        assert_eq!(
            transformations.len(),
            1,
            "one chapter-split transformation, not one per part"
        );
        assert_eq!(transformations[0].kind, "chapter-split");
        assert!(transformations[0].detail.contains("-> "));

        // Spine: A's single entry becomes N ordered parts; B is untouched and
        // still comes after all of A's parts.
        assert!(
            book.spine.len() > 2,
            "spine should have grown: {:?}",
            book.spine
        );
        assert_eq!(book.spine.last().unwrap(), "text/b.xhtml");
        let a_parts: Vec<&String> = book.spine.iter().filter(|p| *p != "text/b.xhtml").collect();
        assert!(
            a_parts.len() >= 2,
            "chapter A must have split into >=2 parts"
        );
        for (i, part) in a_parts.iter().enumerate() {
            assert_eq!(**part, format!("text/a-{}.xhtml", i + 1));
        }

        // TOC: the level-1 "A" entry (no fragment) must point at part 1; the
        // "A - Section 2" fragment entry must point wherever sec2 landed.
        let toc_a = book.toc.iter().find(|e| e.title == "A").unwrap();
        assert_eq!(toc_a.href, "text/a-1.xhtml");
        let toc_sec2 = book
            .toc
            .iter()
            .find(|e| e.title == "A - Section 2")
            .unwrap();
        assert!(
            toc_sec2.href.starts_with("text/a-") && toc_sec2.href.ends_with("#sec2"),
            "got: {}",
            toc_sec2.href
        );

        // Ground truth: which of A's final parts actually holds each id.
        let owning_part = |id: &str| -> &str {
            chapters
                .iter()
                .find(|(p, doc)| {
                    p.starts_with("text/a-") && collect_ids(doc).iter().any(|i| i == id)
                })
                .map(|(p, _)| p.as_str())
                .unwrap_or_else(|| panic!("no A part holds id {id}"))
        };
        let sec1_part = owning_part("sec1");
        let sec3_part = owning_part("sec3");
        assert_ne!(
            sec1_part, sec3_part,
            "the fixture must actually force sec1 and sec3 apart for this test to mean anything"
        );

        let find = |path: &str| -> &NodeRef {
            &chapters
                .iter()
                .find(|(p, _)| p == path)
                .unwrap_or_else(|| panic!("no chapter at {path}"))
                .1
        };

        // Cross-chapter href: B's link to A's sec3 must point at sec3's real
        // owning part (not the vanished `a.xhtml`).
        let out_b = String::from_utf8(serialize_xhtml(find("text/b.xhtml"))).unwrap();
        assert!(
            !out_b.contains("href=\"a.xhtml#sec3\""),
            "must not still point at the removed file: {out_b}"
        );
        assert!(
            out_b.contains(&format!("href=\"{}#sec3\"", basename(sec3_part))),
            "must point at sec3's real part ({sec3_part}): {out_b}"
        );

        // Same-doc href within A: sec1's forward link to sec3 lives in
        // whichever part holds sec1; since sec1 and sec3 are (per the
        // assertion above) in different parts, it must become a cross-file
        // reference to sec3's real part rather than a bare `#sec3`.
        let sec1_part_out = String::from_utf8(serialize_xhtml(find(sec1_part))).unwrap();
        assert!(
            sec1_part_out.contains(&format!("href=\"{}#sec3\"", basename(sec3_part))),
            "sec1's part must reference sec3's real part ({sec3_part}): {sec1_part_out}"
        );

        // And the reverse: sec3's back-link to sec1 must resolve to sec1's
        // real part.
        let sec3_part_out = String::from_utf8(serialize_xhtml(find(sec3_part))).unwrap();
        assert!(
            sec3_part_out.contains(&format!("href=\"{}#sec1\"", basename(sec1_part))),
            "sec3's part must reference sec1's real part ({sec1_part}): {sec3_part_out}"
        );
    }

    #[test]
    fn part_two_title_gets_contd_suffix() {
        let sizes = [5, 100, 5, 100];
        let is_heading = [true, false, true, false];
        let ranges = plan_parts(&sizes, &is_heading, 110);
        assert_eq!(ranges, vec![(0, 2), (2, 4)]);

        let body = heading_body(2, 90);
        let doc = doc_with_body("My Book", &body);
        let full = serialize_xhtml(&doc);
        let mut reserved = HashSet::new();
        let parts = build_parts("text/a.xhtml", &full, &ranges, &mut reserved);
        assert_eq!(parts.len(), 2);
        let out0 = String::from_utf8(serialize_xhtml(&parts[0].doc)).unwrap();
        let out1 = String::from_utf8(serialize_xhtml(&parts[1].doc)).unwrap();
        assert!(out0.contains("<title>My Book</title>"), "got: {out0}");
        assert!(
            out1.contains("<title>My Book (contd.)</title>"),
            "got: {out1}"
        );
    }

    #[test]
    fn reserve_part_path_dedupes_against_existing_resources() {
        let mut reserved: HashSet<String> = HashSet::new();
        reserved.insert("text/a-1.xhtml".to_string());
        let path = reserve_part_path("text/a.xhtml", 1, &mut reserved);
        assert_eq!(path, "text/a-1-2.xhtml");
    }

    #[test]
    fn has_scheme_detects_absolute_urls_only() {
        assert!(has_scheme("https://example.com/x"));
        assert!(has_scheme("mailto:a@b.com"));
        assert!(!has_scheme("chapter1.xhtml"));
        assert!(!has_scheme("../images/x.png"));
        assert!(!has_scheme(""));
    }
}
