//! Unreferenced-resource removal by reachability.
//!
//! Manifest membership is the wrong test. Files referenced only from a CSS
//! `@font-face` or an HTML `src` while missing from the manifest are a spec
//! violation *and* common in the wild, and the writer's keep-and-promote
//! behaviour is the correct repair for them. So the graph is walked from real
//! roots along real references, and only what nothing points at is dropped.
//!
//! Under-collection here is the dangerous direction: a reference type this
//! module fails to recognize looks identical, from the outside, to a resource
//! nothing points at, and gets deleted right along with genuine stray files.
//! So the element/attribute scan below is deliberately broad (every element,
//! not a hand-picked tag list).
//!
//! [`prune`]'s tail also runs an independent safety-net scan over what
//! survives. It is deliberately NOT built from [`refs_of`] (the walk's own
//! extraction): a walk-based check can only ever re-derive the same edges the
//! walk already found, so it is structurally incapable of catching the walk's
//! own blind spots - exactly the failure mode that shipped once already (a
//! dead `xlink:href` lookup silently deleted real cover art, and a check
//! built from the same broken extraction would have re-derived the same
//! empty result and stayed silent too). So the net here shares no code with
//! the walk at all: a plain byte-substring search for each dropped file's
//! basename inside every surviving textual document. Cruder, but independent
//! - which is the entire point of a safety net.

use std::collections::HashMap;
use std::collections::HashSet;

use super::normalize_media_type;
use crate::epub::Book;
use crate::epub::model::{Resource, normalize_href};
use crate::html::dom::{collect_by_name, get_attr_local, local_name};
use crate::html::parse::parse_xhtml;
use crate::report::{Transformation, Warning};

/// Attributes (by local name, namespace-agnostic - see [`get_attr_local`])
/// that can name another resource. `href` also catches SVG1.1
/// `xlink:href`, whose local name html5ever's foreign-content adjustment
/// rewrites to plain `href` in a non-null namespace.
const REF_ATTRS: &[&str] = &["src", "href", "poster", "data"];

/// Normalized media types [`prune`]'s independent substring safety net treats
/// as text worth searching - every format the walk itself understands, since
/// a non-textual (raster/font) resource can never itself *name* another
/// resource.
const TEXTUAL_MEDIA_TYPES: &[&str] = &[
    "application/xhtml+xml",
    "text/html",
    "text/css",
    "image/svg+xml",
    "application/smil+xml",
    "application/x-dtbncx+xml",
    "application/oebps-package+xml",
];

/// Parent directory of a zip-absolute path (`""` if it has no `/`).
fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(idx) => path[..idx].to_string(),
        None => String::new(),
    }
}

/// Whether `raw` opens with a URI scheme (`http:`, `data:`, `mailto:`,
/// `javascript:`, ...) rather than naming a book-relative path.
fn has_scheme(raw: &str) -> bool {
    match raw.find(':') {
        Some(idx) if idx > 0 => {
            let scheme = &raw[..idx];
            scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        _ => false,
    }
}

/// Resolve `raw` against the directory of the document it appeared in,
/// dropping a `#fragment` and skipping anything that is not a book-relative
/// path (an absolute URI, a `data:` URI, or a bare same-page fragment).
fn resolve(base_dir: &str, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('#') || has_scheme(raw) {
        return None;
    }
    let normalized = normalize_href(base_dir, raw);
    let path = normalized.split('#').next().unwrap_or(&normalized);
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

/// Every URL a `srcset` list names (`"a.png 2x, b.png 1x"`), ignoring each
/// entry's optional width/pixel-density descriptor.
fn srcset_refs(srcset: &str, base_dir: &str) -> Vec<String> {
    srcset
        .split(',')
        .filter_map(|entry| entry.split_whitespace().next())
        .filter_map(|url| resolve(base_dir, url))
        .collect()
}

/// Collects every `url()` a stylesheet contains, wherever it appears.
struct UrlCollector {
    found: Vec<String>,
}

impl<'i> lightningcss::visitor::Visitor<'i> for UrlCollector {
    type Error = std::convert::Infallible;

    fn visit_types(&self) -> lightningcss::visitor::VisitTypes {
        lightningcss::visit_types!(URLS)
    }

    fn visit_url(
        &mut self,
        url: &mut lightningcss::values::url::Url<'i>,
    ) -> Result<(), Self::Error> {
        self.found.push(url.url.as_ref().to_string());
        Ok(())
    }
}

/// Upper bound on how far past a `url(` or an `@import`'s opening quote
/// either raw safety-net scan below will search for its closing `)` or
/// matching quote, and so on the length of any value either one pushes as a
/// reference. No legitimate book-relative path is anywhere close to this
/// long.
///
/// Without a bound, a stylesheet crafted as many thousands of nested,
/// unclosed `url(url(url(...` tokens forces every single occurrence to
/// search all the way to the end of the remaining text, one after another -
/// O(occurrences * remaining length) time, allocating a same-sized substring
/// on top. Measured on the scan as first written (no bound): 32 KB of such
/// input produced 8000 references and ~122 MB of allocation; extrapolated, a
/// few hundred KB of it is multiple GB and tens of seconds. This crate's
/// input is an arbitrary downloaded EPUB, so that cost is attacker
/// -controlled, not just a theoretical worst case.
const MAX_RAW_SPAN: usize = 2048;

/// The prefix of `s`, up to `max` bytes, walked back to the nearest char
/// boundary so slicing it never panics - `max` bytes in is not guaranteed to
/// land between two UTF-8 code points.
fn bounded_prefix(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Byte offset of the first case-insensitive occurrence of ASCII `needle` in
/// `haystack`. CSS keywords and function names are ASCII case-insensitive
/// (`url(`, `URL(`, `Url(` all name the same function; `@import`, `@IMPORT`
/// the same at-rule), so a plain `str::find` alone would miss a
/// bad-url-remnants or misplaced-`@import` trigger spelled anything but
/// lowercase.
///
/// Safe to compare byte-wise even though `haystack` is UTF-8: every byte of
/// `needle` is ASCII (< 0x80), and no UTF-8 continuation byte or lead byte of
/// a multi-byte sequence ever equals an ASCII byte value, so a match can only
/// ever start on a genuine character boundary.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

/// A permissive, grammar-independent scan for every literal `url(...)`
/// occurrence in `css`, run alongside the AST walk in [`css_refs`] as a
/// safety net.
///
/// CSS Syntax Level 3 defines that a malformed or unterminated `url(` token
/// consumes every code point up to the *next* unescaped `)` in the whole
/// token stream ("consume the remnants of a bad url"), regardless of
/// intervening braces, rule boundaries, or newlines. That is mandated
/// tokenizer behaviour, not a hand-rolled-scanner bug: verified directly
/// against this crate's pinned lightningcss, a rule like
/// `a{background:url(broken.png}` really does swallow a later, otherwise
/// well-formed `p.b{background:url(good.png)}` into a single bad-url token,
/// so the AST visitor never sees `good.png` as a value at all - no amount of
/// `error_recovery` changes that, because it happens during tokenization,
/// before any rule-level recovery is even possible.
///
/// Under-collecting here deletes a file the book genuinely uses; collecting
/// too much only leaves harmless junk reachable (a garbage span that happens
/// not to name any real resource resolves to a path nothing in the book has,
/// so it is a silent no-op). So this scan re-finds every literal
/// `url(` occurrence independently - never skipping past one already claimed
/// by a still-open match - pairing each with its own nearest `)` within
/// [`MAX_RAW_SPAN`] bytes. A captured value containing `{`, `}` or a newline
/// is discarded rather than pushed: no legitimate path contains any of
/// those, so it is always the tokenizer's garbage, not a real reference -
/// filtering it out here removes the inert junk this scan would otherwise
/// hand back for a match like `broken.png}\np.b{background:url(good.png`.
fn raw_url_refs(css: &str, base_dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = find_ci(&css[search_from..], "url(") {
        let start = search_from + rel;
        let after = start + "url(".len();
        let window = bounded_prefix(&css[after..], MAX_RAW_SPAN);
        if let Some(rel_end) = window.find(')') {
            let raw = window[..rel_end].trim().trim_matches(['"', '\'']);
            if !raw.contains(['{', '}', '\n', '\r'])
                && let Some(p) = resolve(base_dir, raw)
            {
                out.push(p);
            }
        }
        // Advance only past the `url(` token just matched, not past whatever
        // it (mis)consumed, so a later `url(` inside the same garbage span is
        // still found on its own.
        search_from = after;
    }
    out
}

/// A permissive, grammar-independent scan for every `@import "…"` /
/// `@import '…'` string-literal form in `css` (the form with no `url(...)`
/// wrapper), run alongside the AST walk in [`css_refs`] as a safety net,
/// mirroring [`raw_url_refs`].
///
/// An `@import` is only valid as one of the first rules in a stylesheet
/// (after only `@charset` and other `@import`/`@layer` statements); one
/// anywhere else is invalid per CSS grammar, and lightningcss's
/// `error_recovery` correctly discards it as a matter of what the grammar
/// permits, not of malformed syntax it can repair - so it never reaches
/// `sheet.rules.0` and the AST walk in [`css_refs`] never sees it, no matter
/// how well-formed the `@import` itself is. Restores the raw-text recovery
/// the pre-parser scanner used to provide for exactly this shape.
fn raw_import_refs(css: &str, base_dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = css;
    while let Some(rel) = find_ci(cursor, "@import") {
        let tail = &cursor[rel + "@import".len()..];
        cursor = tail;
        let trimmed = tail.trim_start();
        let quote = trimmed.chars().next().filter(|c| *c == '"' || *c == '\'');
        if let Some(q) = quote {
            let rest = &trimmed[1..];
            let window = bounded_prefix(rest, MAX_RAW_SPAN);
            if let Some(end) = window.find(q) {
                let raw = &rest[..end];
                if !raw.contains(['{', '}', '\n', '\r'])
                    && let Some(p) = resolve(base_dir, raw)
                {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// Every path `css` names, via `url()` or `@import`.
///
/// This parses rather than scans for its primary pass. The previous
/// hand-rolled scanner searched for `url(` and took everything up to the next
/// `)`, so one malformed declaration consumed the following rule whole and
/// its target was pruned as unreferenced - a file the book genuinely used,
/// deleted. lightningcss runs in error-recovery mode here (the same mode
/// `css::subset` uses), so a malformed rule is skipped and the rest of the
/// sheet is still read.
///
/// That alone is not sufficient, though: see [`raw_url_refs`] and
/// [`raw_import_refs`] for the two shapes a spec-compliant AST walk cannot
/// see by construction - one a tokenizer quirk, the other a grammar rule -
/// and why each is restored here as a pure, deduplicated safety net rather
/// than trusted as the primary signal.
fn css_refs(css: &str, base_dir: &str) -> Vec<String> {
    use lightningcss::rules::CssRule;
    use lightningcss::stylesheet::{ParserOptions, StyleSheet};
    use lightningcss::visitor::Visit as _;

    let options = ParserOptions {
        error_recovery: true,
        ..ParserOptions::default()
    };

    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // `StyleSheet::parse` only returns `Err` when `error_recovery` is off;
    // with it on (set immediately above) every failure is recovered into a
    // skipped rule or declaration instead, so this branch is defensive - not
    // something normal, or even adversarial, input actually reaches - rather
    // than a live path this function relies on.
    if let Ok(mut sheet) = StyleSheet::parse(css, options) {
        let mut collector = UrlCollector { found: Vec::new() };
        let _ = sheet.visit(&mut collector);

        // `@import "file.css"` carries its target as a plain string field on
        // the rule (`ImportRule::url`, a `CowArcStr`) marked `#[skip_visit]`
        // in lightningcss's own source - the URLS visitor above never sees
        // it, so it is walked explicitly here.
        for rule in &sheet.rules.0 {
            if let CssRule::Import(import) = rule {
                collector.found.push(import.url.as_ref().to_string());
            }
        }

        for raw in &collector.found {
            if let Some(p) = resolve(base_dir, raw)
                && seen.insert(p.clone())
            {
                out.push(p);
            }
        }
    }

    // The two grammar-independent safety nets, folded in as pure additions
    // and deduplicated against everything found so far - the AST walk above
    // and each other alike - rather than only against the AST half.
    for extra in raw_url_refs(css, base_dir)
        .into_iter()
        .chain(raw_import_refs(css, base_dir))
    {
        if seen.insert(extra.clone()) {
            out.push(extra);
        }
    }

    out
}

/// Every path an NCX (`application/x-dtbncx+xml`) names through a
/// `<content src="...">` element - the way `navPoint`, `navTarget` and
/// `pageTarget` entries all point at spine documents (and occasionally at
/// resources the manifest never lists).
fn ncx_refs(ncx: &str, base_dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(doc) = roxmltree::Document::parse(ncx) else {
        return out;
    };
    for node in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "content")
    {
        if let Some(src) = node.attribute("src")
            && let Some(p) = resolve(base_dir, src)
        {
            out.push(p);
        }
    }
    out
}

/// Every path a SMIL media-overlay document (`application/smil+xml`) names
/// through an `<audio src>`, `<text src>` or similar `<par>`/`<seq>` child's
/// `src` - the way a read-aloud EPUB 3 book's audio track and per-fragment
/// text targets are named. `text@src` usually just repoints at a spine
/// document already reachable, but the audio track is reachable only from
/// here.
fn smil_refs(smil: &str, base_dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(doc) = roxmltree::Document::parse(smil) else {
        return out;
    };
    for node in doc.descendants().filter(|n| {
        n.is_element() && matches!(n.tag_name().name(), "audio" | "text" | "video" | "img")
    }) {
        if let Some(src) = node.attribute("src")
            && let Some(p) = resolve(base_dir, src)
        {
            out.push(p);
        }
    }
    out
}

/// Every path the OPF names through a manifest item's `media-overlay` or
/// `fallback` attribute (each an `idref` to another manifest item, not a
/// path). Ordinary `<item href>` membership is deliberately NOT walked here -
/// manifest membership is the wrong reachability test, the whole point of
/// this module - but `media-overlay` (the SMIL doc driving a spine chapter's
/// audio) and `fallback` (the resource a reading system falls back to for an
/// unsupported type) are real edges a spine/content walk alone never sees.
fn opf_refs(opf: &str, base_dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(doc) = roxmltree::Document::parse(opf) else {
        return out;
    };
    let items: Vec<_> = doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "item")
        .collect();
    let id_href: HashMap<&str, &str> = items
        .iter()
        .filter_map(|n| Some((n.attribute("id")?, n.attribute("href")?)))
        .collect();
    for item in &items {
        for attr in ["media-overlay", "fallback"] {
            if let Some(target_id) = item.attribute(attr)
                && let Some(href) = id_href.get(target_id)
                && let Some(p) = resolve(base_dir, href)
            {
                out.push(p);
            }
        }
    }
    out
}

/// Every path `resource` (whose directory is `dir`) names, dispatched by its
/// (normalized) media type. The graph walk's only extraction logic -
/// deliberately NOT reused by [`prune`]'s post-removal safety net, which
/// exists specifically to catch what this function fails to extract (see the
/// module docs).
fn refs_of(resource: &Resource, dir: &str) -> Vec<String> {
    match normalize_media_type(&resource.media_type).as_str() {
        "text/css" => match std::str::from_utf8(&resource.data) {
            Ok(text) => css_refs(text, dir),
            Err(_) => Vec::new(),
        },
        // XML formats, parsed separately rather than through the HTML5 tree
        // builder below, whose tag set/self-closing handling doesn't match
        // arbitrary XML.
        "application/x-dtbncx+xml" => match std::str::from_utf8(&resource.data) {
            Ok(text) => ncx_refs(text, dir),
            Err(_) => Vec::new(),
        },
        "application/smil+xml" => match std::str::from_utf8(&resource.data) {
            Ok(text) => smil_refs(text, dir),
            Err(_) => Vec::new(),
        },
        "application/oebps-package+xml" => match std::str::from_utf8(&resource.data) {
            Ok(text) => opf_refs(text, dir),
            Err(_) => Vec::new(),
        },
        "application/xhtml+xml" | "image/svg+xml" => {
            let Ok(doc) = parse_xhtml(&resource.data) else {
                return Vec::new();
            };
            let mut refs = Vec::new();
            // Every element, not a hand-picked tag allow-list: `<object
            // data>`, `<script src>`, `<track src>`, `<embed src>`, `<a
            // xlink:href>` inside an inline `<svg>`, ... all use the same
            // small set of reference-shaped attributes, so probing every
            // element is both simpler and safer than enumerating tag names -
            // over-collection here leaves harmless junk reachable,
            // under-collection deletes real content.
            for node in doc.inclusive_descendants() {
                if local_name(&node).is_none() {
                    continue;
                }
                for attr in REF_ATTRS {
                    // An element can carry more than one spelling of the same
                    // local name (`href` and `xlink:href` naming different
                    // targets) - every match is collected, not just the first.
                    for raw in get_attr_local(&node, attr) {
                        if let Some(p) = resolve(dir, &raw) {
                            refs.push(p);
                        }
                    }
                }
                for srcset in get_attr_local(&node, "srcset") {
                    refs.extend(srcset_refs(&srcset, dir));
                }
                // An inline `style="background:url(...)"` references assets
                // exactly like a `<style>` block does.
                for style in get_attr_local(&node, "style") {
                    refs.extend(css_refs(&style, dir));
                }
            }
            // `<style>` blocks reference assets too.
            for node in collect_by_name(&doc, "style") {
                refs.extend(css_refs(&node.text_contents(), dir));
            }
            refs
        }
        _ => Vec::new(),
    }
}

/// Drop every resource unreachable from the book's roots.
///
/// `srcset_by_document` supplies edges a caller resolved *before* this runs,
/// from a reference type that no longer exists in the book by the time
/// `prune` sees it - today, exactly the `<img srcset>` targets
/// `image::rewrite_refs` strips on every conversion (not just `generic`)
/// well before this walk starts. Keyed by the zip-absolute path of the
/// document each target was found in, and consulted only when the walk
/// actually visits that document: these targets stand in for attributes that
/// are no longer in the DOM, so they must behave exactly as those attributes
/// would have - reachable if and only if the document naming them is itself
/// reachable, never as roots in their own right. A target named only from a
/// document nothing else points at (an unreferenced non-spine chapter, say)
/// must be dropped along with that document, not kept regardless of it.
pub(crate) fn prune(
    book: &mut Book,
    transformations: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
    srcset_by_document: &HashMap<String, Vec<String>>,
) {
    let mut reachable: HashSet<String> = HashSet::new();
    let mut queue: Vec<String> = Vec::new();

    // Roots: the package document, the navigation document, the NCX, the cover
    // and every spine document. None of these is ever droppable.
    queue.push(book.opf_path.clone());
    queue.extend(book.nav_path.clone());
    queue.extend(book.ncx_path.clone());
    queue.extend(book.cover.clone());
    queue.extend(book.spine.iter().cloned());

    while let Some(path) = queue.pop() {
        if !reachable.insert(path.clone()) {
            continue;
        }
        if let Some(resource) = book.resources.get(&path) {
            let dir = parent_dir(&path);
            queue.extend(refs_of(resource, &dir));
        }
        if let Some(targets) = srcset_by_document.get(&path) {
            queue.extend(targets.iter().cloned());
        }
    }

    // Resource order (an `IndexMap` preserves it), not `HashSet` iteration
    // order: `Transformation` entries land in `ConvertReport` and, per
    // process, `HashSet`'s `RandomState` seed varies - iterating a set here
    // would make report order nondeterministic run to run even though the
    // EPUB bytes themselves are unaffected (`shift_remove` alone decides
    // those, and set membership is deterministic).
    let dropped_ordered: Vec<String> = book
        .resources
        .keys()
        .filter(|p| !reachable.contains(*p))
        .cloned()
        .collect();

    for path in &dropped_ordered {
        let size = book.resources[path].data.len();
        transformations.push(Transformation {
            kind: "generic-unreferenced".to_string(),
            detail: format!("dropped {size} bytes nothing references"),
            file: Some(path.clone()),
        });
    }
    for path in &dropped_ordered {
        book.resources.shift_remove(path);
    }

    // Independent safety net: for each dropped file's basename, a plain byte
    // search over every surviving textual document's raw bytes. See the
    // module docs for why this shares no code with the walk above - that
    // independence is what lets it catch an edge type `refs_of` itself
    // fails to extract, which a check built from `refs_of` provably cannot.
    // Cheap false positives (a basename mentioned in prose) are the accepted
    // cost; a silent deletion is not.
    for dropped_path in &dropped_ordered {
        let Some(basename) = dropped_path.rsplit('/').next().filter(|b| !b.is_empty()) else {
            continue;
        };
        let needle = basename.as_bytes();
        for (path, resource) in &book.resources {
            if !TEXTUAL_MEDIA_TYPES.contains(&normalize_media_type(&resource.media_type).as_str()) {
                continue;
            }
            if resource.data.len() >= needle.len()
                && resource.data.windows(needle.len()).any(|w| w == needle)
            {
                warnings.push(Warning {
                    message: format!(
                        "{path} still mentions {basename} (from {dropped_path}, dropped as \
                         unreferenced) - the reachability graph may be missing an edge"
                    ),
                    file: Some(path.clone()),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::epub::{Metadata, Resource};
    use indexmap::IndexMap;

    fn book_with(resources: Vec<(&str, &str, &[u8])>, spine: Vec<&str>) -> Book {
        let mut map = IndexMap::new();
        for (path, media_type, data) in resources {
            map.insert(
                path.to_string(),
                Resource {
                    data: data.to_vec(),
                    media_type: media_type.to_string(),
                },
            );
        }
        Book {
            metadata: Metadata::default(),
            resources: map,
            spine: spine.into_iter().map(String::from).collect(),
            toc: Vec::new(),
            cover: None,
            opf_path: "OEBPS/content.opf".to_string(),
            nav_path: None,
            ncx_path: None,
        }
    }

    fn prune_book(book: &mut Book) -> (Vec<Transformation>, Vec<Warning>) {
        prune_book_with_srcset(book, &HashMap::new())
    }

    fn prune_book_with_srcset(
        book: &mut Book,
        srcset_by_document: &HashMap<String, Vec<String>>,
    ) -> (Vec<Transformation>, Vec<Warning>) {
        let mut transformations = Vec::new();
        let mut warnings = Vec::new();
        prune(
            book,
            &mut transformations,
            &mut warnings,
            srcset_by_document,
        );
        (transformations, warnings)
    }

    #[test]
    fn drops_a_resource_nothing_references() {
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/chapter.xhtml",
                    "application/xhtml+xml",
                    b"<html><body><p>Hi</p></body></html>",
                ),
                ("OEBPS/stray.txt", "application/octet-stream", b"stray"),
            ],
            vec!["OEBPS/chapter.xhtml"],
        );
        let (transformations, warnings) = prune_book(&mut book);
        assert!(!book.resources.contains_key("OEBPS/stray.txt"));
        assert!(book.resources.contains_key("OEBPS/chapter.xhtml"));
        assert_eq!(transformations.len(), 1);
        assert!(warnings.is_empty());
    }

    #[test]
    fn keeps_a_file_reachable_only_through_css_import_without_url() {
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/chapter.xhtml",
                    "application/xhtml+xml",
                    br#"<html><head><link rel="stylesheet" href="main.css"/></head><body/></html>"#,
                ),
                ("OEBPS/main.css", "text/css", b"@import 'other.css';\n"),
                ("OEBPS/other.css", "text/css", b"body { color: red; }"),
            ],
            vec!["OEBPS/chapter.xhtml"],
        );
        prune_book(&mut book);
        assert!(book.resources.contains_key("OEBPS/other.css"));
    }

    #[test]
    fn keeps_a_chapter_named_only_by_the_ncx() {
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/toc.ncx",
                    "application/x-dtbncx+xml",
                    br#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap>
<navPoint><content src="landmark.xhtml"/></navPoint>
</navMap></ncx>"#,
                ),
                (
                    "OEBPS/landmark.xhtml",
                    "application/xhtml+xml",
                    b"<html><body><p>Extra</p></body></html>",
                ),
            ],
            vec![],
        );
        book.ncx_path = Some("OEBPS/toc.ncx".to_string());
        prune_book(&mut book);
        assert!(book.resources.contains_key("OEBPS/landmark.xhtml"));
    }

    #[test]
    fn does_not_follow_an_external_or_data_uri() {
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/chapter.xhtml",
                    "application/xhtml+xml",
                    br#"<html><body>
<img src="https://example.com/x.png"/>
<img src="data:image/png;base64,AAAA"/>
</body></html>"#,
                ),
            ],
            vec!["OEBPS/chapter.xhtml"],
        );
        // Must not panic or otherwise choke on the absolute/data refs.
        prune_book(&mut book);
        assert!(book.resources.contains_key("OEBPS/chapter.xhtml"));
    }

    #[test]
    fn keeps_an_svg_cover_raster_referenced_only_through_xlink_href() {
        // The classic cover-wrapper pattern (SVG 1.1): a standalone SVG,
        // itself the book's cover root, framing a raster via `xlink:href`.
        // html5ever's foreign-content adjustment stores that attribute in the
        // xlink namespace with local name `href`, which a null-namespace-only
        // lookup never matches - this pins the fix.
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/cover.svg",
                    "image/svg+xml",
                    br#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="cover.jpg"/></svg>"#,
                ),
                ("OEBPS/cover.jpg", "image/jpeg", b"\xFF\xD8fake\xFF\xD9"),
            ],
            vec![],
        );
        book.cover = Some("OEBPS/cover.svg".to_string());
        let (_, warnings) = prune_book(&mut book);
        assert!(
            book.resources.contains_key("OEBPS/cover.jpg"),
            "the raster an SVG cover wraps via xlink:href must survive"
        );
        assert!(
            warnings.is_empty(),
            "a correctly-walked edge warns of nothing"
        );
    }

    #[test]
    fn keeps_a_file_referenced_only_from_an_inline_style_attribute() {
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/chapter.xhtml",
                    "application/xhtml+xml",
                    br#"<html><body><div style="background-image:url(inline-bg.png)">Hi</div></body></html>"#,
                ),
                ("OEBPS/inline-bg.png", "image/png", b"\x89PNG"),
            ],
            vec!["OEBPS/chapter.xhtml"],
        );
        prune_book(&mut book);
        assert!(book.resources.contains_key("OEBPS/inline-bg.png"));
    }

    #[test]
    fn a_srcset_target_owned_by_a_reachable_document_survives_pruning() {
        // Pins the mechanism `image::rewrite_refs`'s `<img srcset>` targets
        // rely on: a target keyed under a document that IS reachable (here,
        // a spine chapter) survives via that document, even though nothing
        // in the DOM itself names it any more (the attribute was already
        // stripped by the time `prune` runs).
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/chapter.xhtml",
                    "application/xhtml+xml",
                    b"<html><body><p>Hi</p></body></html>",
                ),
                ("OEBPS/only.png", "image/png", b"\x89PNG"),
            ],
            vec!["OEBPS/chapter.xhtml"],
        );
        let mut srcset = HashMap::new();
        srcset.insert(
            "OEBPS/chapter.xhtml".to_string(),
            vec!["OEBPS/only.png".to_string()],
        );
        prune_book_with_srcset(&mut book, &srcset);
        assert!(book.resources.contains_key("OEBPS/only.png"));
    }

    #[test]
    fn a_srcset_target_owned_by_an_orphan_document_is_dropped_with_it() {
        // The regression this pins: a srcset target must NOT be a global
        // root. `orphan.xhtml` is manifested (present in `book.resources`)
        // but neither in the spine nor referenced from anywhere reachable,
        // so it is itself unreferenced and must be dropped - and `wm.png`,
        // named only through `orphan.xhtml`'s (already-stripped) `<img
        // srcset>`, must be dropped right along with it rather than
        // surviving as if it were a root in its own right.
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/chapter.xhtml",
                    "application/xhtml+xml",
                    b"<html><body><p>Hi</p></body></html>",
                ),
                (
                    "OEBPS/orphan.xhtml",
                    "application/xhtml+xml",
                    b"<html><body><p>Orphan</p></body></html>",
                ),
                ("OEBPS/wm.png", "image/png", b"\x89PNG"),
            ],
            vec!["OEBPS/chapter.xhtml"],
        );
        let mut srcset = HashMap::new();
        srcset.insert(
            "OEBPS/orphan.xhtml".to_string(),
            vec!["OEBPS/wm.png".to_string()],
        );
        prune_book_with_srcset(&mut book, &srcset);
        assert!(
            !book.resources.contains_key("OEBPS/orphan.xhtml"),
            "the orphan chapter itself must still be dropped"
        );
        assert!(
            !book.resources.contains_key("OEBPS/wm.png"),
            "a srcset target owned by a dropped, unreachable document must be dropped with it"
        );
    }

    #[test]
    fn keeps_a_file_referenced_only_through_srcset() {
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/chapter.xhtml",
                    "application/xhtml+xml",
                    br#"<html><body><picture><source srcset="hi.png 2x"/></picture></body></html>"#,
                ),
                ("OEBPS/hi.png", "image/png", b"\x89PNG"),
            ],
            vec!["OEBPS/chapter.xhtml"],
        );
        prune_book(&mut book);
        assert!(book.resources.contains_key("OEBPS/hi.png"));
    }

    #[test]
    fn keeps_targets_of_object_script_track_and_embed() {
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/chapter.xhtml",
                    "application/xhtml+xml",
                    br#"<html><body>
<object data="thing.pdf"></object>
<script src="app.js"></script>
<video><track src="subs.vtt"/></video>
<embed src="widget.swf"/>
</body></html>"#,
                ),
                ("OEBPS/thing.pdf", "application/pdf", b"pdf"),
                ("OEBPS/app.js", "application/octet-stream", b"js"),
                ("OEBPS/subs.vtt", "text/vtt", b"vtt"),
                ("OEBPS/widget.swf", "application/x-shockwave-flash", b"swf"),
            ],
            vec!["OEBPS/chapter.xhtml"],
        );
        prune_book(&mut book);
        assert!(book.resources.contains_key("OEBPS/thing.pdf"));
        assert!(book.resources.contains_key("OEBPS/app.js"));
        assert!(book.resources.contains_key("OEBPS/subs.vtt"));
        assert!(book.resources.contains_key("OEBPS/widget.swf"));
    }

    #[test]
    fn keeps_smil_audio_reached_through_a_manifest_media_overlay() {
        let mut book = book_with(
            vec![
                (
                    "OEBPS/content.opf",
                    "application/oebps-package+xml",
                    br#"<package xmlns="http://www.idpf.org/2007/opf">
<manifest>
<item id="ch1" href="chapter.xhtml" media-type="application/xhtml+xml" media-overlay="mo1"/>
<item id="mo1" href="chapter.smil" media-type="application/smil+xml"/>
</manifest>
</package>"#,
                ),
                (
                    "OEBPS/chapter.xhtml",
                    "application/xhtml+xml",
                    b"<html><body><p>Hi</p></body></html>",
                ),
                (
                    "OEBPS/chapter.smil",
                    "application/smil+xml",
                    br#"<smil xmlns="http://www.w3.org/ns/SMIL"><body><par>
<text src="chapter.xhtml"/><audio src="track.mp3"/>
</par></body></smil>"#,
                ),
                ("OEBPS/track.mp3", "audio/mpeg", b"mp3"),
            ],
            vec!["OEBPS/chapter.xhtml"],
        );
        prune_book(&mut book);
        assert!(book.resources.contains_key("OEBPS/chapter.smil"));
        assert!(book.resources.contains_key("OEBPS/track.mp3"));
    }

    #[test]
    fn keeps_a_fallback_chain_target() {
        let mut book = book_with(
            vec![
                (
                    "OEBPS/content.opf",
                    "application/oebps-package+xml",
                    br#"<package xmlns="http://www.idpf.org/2007/opf">
<manifest>
<item id="ch1" href="chapter.xhtml" media-type="application/xhtml+xml"/>
<item id="weird" href="weird.xml" media-type="application/x-weird+xml" fallback="fb"/>
<item id="fb" href="fallback.xhtml" media-type="application/xhtml+xml"/>
</manifest>
</package>"#,
                ),
                (
                    "OEBPS/chapter.xhtml",
                    "application/xhtml+xml",
                    b"<html><body><p>Hi</p></body></html>",
                ),
                ("OEBPS/weird.xml", "application/x-weird+xml", b"<w/>"),
                (
                    "OEBPS/fallback.xhtml",
                    "application/xhtml+xml",
                    b"<html><body><p>Fallback</p></body></html>",
                ),
            ],
            vec!["OEBPS/chapter.xhtml"],
        );
        prune_book(&mut book);
        assert!(book.resources.contains_key("OEBPS/fallback.xhtml"));
    }

    #[test]
    fn matches_a_media_type_with_parameters_and_odd_casing() {
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/chapter.xhtml",
                    "application/xhtml+xml",
                    br#"<html><head><link rel="stylesheet" href="main.css"/></head><body/></html>"#,
                ),
                (
                    "OEBPS/main.css",
                    "TEXT/CSS; charset=utf-8",
                    b"body { background: url(bg.png); }",
                ),
                ("OEBPS/bg.png", "image/png", b"\x89PNG"),
            ],
            vec!["OEBPS/chapter.xhtml"],
        );
        prune_book(&mut book);
        assert!(book.resources.contains_key("OEBPS/bg.png"));
    }

    #[test]
    fn an_unterminated_trailing_url_does_not_lose_the_earlier_match() {
        // The `url(` that opens here never closes anywhere in the rest of the
        // string (it is the last thing in the sheet). It must not take the
        // already-collected earlier match down with it - originally a guard
        // on the old scanner's `else { break }` on this exact shape, which
        // aborted the whole scan. That risk no longer applies with an AST
        // walk, but CSS Syntax Level 3 defines that hitting EOF while
        // scanning a `url(...)` token still returns a (parse-error-flagged)
        // url token with whatever was accumulated so far - verified directly
        // against this crate's pinned lightningcss - so `unterminated` itself
        // is now collected too. That is a genuine reference the old scanner's
        // `)`-only match could never find, not a regression: collecting more
        // is the safe direction (see the module docs), and `OEBPS/unterminated`
        // is harmless since no real resource ever has that path.
        let refs = css_refs(
            "a { background: url(good.png); } b { background: url(unterminated",
            "OEBPS",
        );
        assert_eq!(
            refs,
            vec![
                "OEBPS/good.png".to_string(),
                "OEBPS/unterminated".to_string()
            ]
        );
    }

    #[test]
    fn two_well_formed_urls_are_both_collected() {
        let refs = css_refs(
            "a { background: url(one.png); } b { background: url(two.png); }",
            "OEBPS",
        );
        assert_eq!(
            refs,
            vec!["OEBPS/one.png".to_string(), "OEBPS/two.png".to_string()]
        );
    }

    #[test]
    fn generic_unreferenced_transformations_are_reported_in_resource_order() {
        // Insertion order deliberately does not sort alphabetically (z, a,
        // m): a `HashSet`-driven push loop would reorder these
        // nondeterministically (a different `RandomState` seed per process),
        // which broke `ConvertReport` reproducibility even though the EPUB
        // bytes themselves were always fine.
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/chapter.xhtml",
                    "application/xhtml+xml",
                    b"<html><body><p>Hi</p></body></html>",
                ),
                ("OEBPS/z-stray.txt", "application/octet-stream", b"z"),
                ("OEBPS/a-stray.txt", "application/octet-stream", b"a"),
                ("OEBPS/m-stray.txt", "application/octet-stream", b"m"),
            ],
            vec!["OEBPS/chapter.xhtml"],
        );
        let (transformations, _) = prune_book(&mut book);
        let dropped_files: Vec<&str> = transformations
            .iter()
            .map(|t| t.file.as_deref().unwrap())
            .collect();
        assert_eq!(
            dropped_files,
            vec![
                "OEBPS/z-stray.txt",
                "OEBPS/a-stray.txt",
                "OEBPS/m-stray.txt"
            ],
            "transformation order must follow resource (insertion) order, not hash order"
        );
    }

    #[test]
    fn both_spellings_of_the_same_local_name_survive() {
        // `<image href="dual.png" xlink:href="dual2.png"/>`: two attributes,
        // same local name (`href`), different targets. `get_attr_local`
        // returning only the first match would keep one and silently delete
        // the other - deletion is the dangerous direction, so both must
        // survive even though this shape is vanishingly rare in practice.
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/cover.svg",
                    "image/svg+xml",
                    br#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image href="dual.png" xlink:href="dual2.png"/></svg>"#,
                ),
                ("OEBPS/dual.png", "image/png", b"\x89PNG"),
                ("OEBPS/dual2.png", "image/png", b"\x89PNG"),
            ],
            vec![],
        );
        book.cover = Some("OEBPS/cover.svg".to_string());
        prune_book(&mut book);
        assert!(book.resources.contains_key("OEBPS/dual.png"));
        assert!(book.resources.contains_key("OEBPS/dual2.png"));
    }

    #[test]
    fn a_malformed_rule_does_not_hide_the_next_url() {
        // The scanner this replaced consumed to the `)` of the LATER url(),
        // swallowing good.png with it. Every shape below deletes a file the
        // book genuinely uses.
        let cases = [
            "a{background:url(broken.png}\np.b{background:url(good.png)}",
            "a{background:url('unclosed}\np.b{background:url(good.png)}",
            "a{background:url(/*)*/}\np.b{background:url(good.png)}",
            "@media screen{a{background:url(broken.png}}\np.b{background:url(good.png)}",
        ];
        for css in cases {
            let refs = css_refs(css, "OEBPS");
            assert!(
                refs.contains(&"OEBPS/good.png".to_string()),
                "good.png must survive: {css}\ngot {refs:?}"
            );
        }
    }

    #[test]
    fn well_formed_urls_and_imports_are_still_collected() {
        let css = r#"@import "base.css";
@import url(more.css);
@font-face { src: url("fonts/body.woff2") format("woff2"); }
p { background: url('img/bg.png'); }"#;
        let refs = css_refs(css, "OEBPS");
        for want in [
            "OEBPS/base.css",
            "OEBPS/more.css",
            "OEBPS/fonts/body.woff2",
            "OEBPS/img/bg.png",
        ] {
            assert!(
                refs.contains(&want.to_string()),
                "missing {want} in {refs:?}"
            );
        }
    }

    #[test]
    fn raw_url_refs_finds_a_url_hidden_inside_an_earlier_unclosed_one() {
        // Pins the load-bearing invariant: after failing to close
        // `url(broken.png}...`, the scan must advance only past the `url(`
        // token it just matched, not past whatever the unmatched span
        // (mis)consumed - otherwise a later, independent `url(` inside that
        // same span is never found, exactly the old scanner's bug.
        let refs = raw_url_refs(
            "a{background:url(broken.png}\np.b{background:url(good.png)}",
            "OEBPS",
        );
        assert!(refs.contains(&"OEBPS/good.png".to_string()), "got {refs:?}");
    }

    #[test]
    fn raw_url_refs_is_case_insensitive() {
        // CSS function names are ASCII case-insensitive; `URL(` must be
        // found exactly like `url(`.
        let refs = raw_url_refs("a { background: URL(shout.png); }", "OEBPS");
        assert_eq!(refs, vec!["OEBPS/shout.png".to_string()]);
    }

    #[test]
    fn raw_url_refs_discards_a_capture_spanning_a_brace_or_newline() {
        // A captured value crossing a `{`, `}` or newline is always garbage
        // (no legitimate path contains one), so it must not be pushed even
        // when some later `)` eventually closes it.
        let refs = raw_url_refs("a{background:url(one{two)three}", "OEBPS");
        assert!(refs.is_empty(), "got {refs:?}");
    }

    #[test]
    fn raw_import_refs_recovers_an_import_after_another_rule() {
        // `@import` is only grammatically valid ahead of other rules, so
        // lightningcss's AST drops a misplaced one outright even under
        // `error_recovery` - restoring the pre-parser scanner's raw-text
        // recovery for exactly this shape.
        let refs = raw_import_refs("p{color:red}\n@import \"sub.css\";", "OEBPS");
        assert_eq!(refs, vec!["OEBPS/sub.css".to_string()]);
    }

    #[test]
    fn raw_import_refs_recovers_an_import_nested_in_a_media_block() {
        let refs = raw_import_refs("@media print{@import \"sub.css\";}", "OEBPS");
        assert_eq!(refs, vec!["OEBPS/sub.css".to_string()]);
    }

    #[test]
    fn css_refs_recovers_an_import_after_a_malformed_url_rule() {
        // The regression code review caught: a malformed `url()` rule ahead
        // of an otherwise well-formed `@import` must not cost the import
        // target either, the same guarantee
        // `a_malformed_rule_does_not_hide_the_next_url` already pins for a
        // later `url()`.
        let refs = css_refs(
            "a{background:url(broken.png}\n@import \"sub.css\";",
            "OEBPS",
        );
        assert!(
            refs.contains(&"OEBPS/sub.css".to_string()),
            "sub.css must survive: got {refs:?}"
        );
    }

    #[test]
    fn independent_substring_scan_warns_when_the_walk_misses_a_real_edge() {
        // `<meta name="og:image" content="...">` is a real, currently-true
        // gap in the attribute walk: `content` is not in `REF_ATTRS`, so
        // `refs_of` never sees it and `social.png` gets dropped as
        // "unreferenced" even though the chapter plainly still names it.
        // This is exactly the class of bug a `refs_of`-based check could
        // never catch (see the module docs) - the independent byte-substring
        // scan must catch it. If `content` is ever added to `REF_ATTRS`, this
        // stops being droppable and this test needs a different genuine gap
        // - do not delete the assertion that the walk actually drops it.
        let mut book = book_with(
            vec![
                ("OEBPS/content.opf", "application/oebps-package+xml", b""),
                (
                    "OEBPS/chapter.xhtml",
                    "application/xhtml+xml",
                    br#"<html><head><meta name="og:image" content="social.png"/></head><body><p>Hi</p></body></html>"#,
                ),
                ("OEBPS/social.png", "image/png", b"\x89PNG"),
            ],
            vec!["OEBPS/chapter.xhtml"],
        );
        let (_, warnings) = prune_book(&mut book);
        assert!(
            !book.resources.contains_key("OEBPS/social.png"),
            "precondition: the attribute walk genuinely does not see `content=`"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.message.contains("social.png") && w.message.contains("chapter.xhtml")),
            "the independent substring scan must catch what the walk missed: {warnings:?}"
        );
    }
}
