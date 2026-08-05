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
//! the walk at all: [`scan_for_dangling_basenames`] searches each surviving
//! textual document's raw bytes for each dropped file's basename as a plain
//! substring - no parsing, no re-derivation of the walk's extraction. Cruder,
//! but independent - which is the entire point of a safety net.
//!
//! That scan is also bounded to one pass per surviving document, regardless
//! of how many files were dropped. See [`scan_for_dangling_basenames`]'s docs
//! for the blowup a per-dropped-file rescan produced on adversarial input,
//! and for why the fix had to preserve substring semantics exactly.

use std::collections::HashMap;
use std::collections::HashSet;

use aho_corasick::AhoCorasick;

use super::normalize_media_type;
use crate::epub::Book;
use crate::epub::model::normalize_href;
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

/// A dropped path's basename, or `None` when it has none (a path ending in
/// `/`). Shared by [`scan_for_dangling_basenames`] and the warning it feeds
/// so the two cannot disagree about what a basename is.
fn basename_of(path: &str) -> Option<&str> {
    path.rsplit('/').next().filter(|b| !b.is_empty())
}

/// [`prune`]'s independent safety net, factored out so it can be driven by a
/// plain `&[(&str, &[u8])]` in tests. For each `dropped` path's basename,
/// finds every document in `docs` whose raw bytes contain that basename as a
/// substring. See the module docs for why this shares no code with the
/// reachability walk, and must stay that way.
///
/// Bounded to one pass per document, not one substring search per dropped
/// file. The loop this replaced re-ran a `windows(needle.len())` search over
/// every surviving document for *each* dropped file, i.e. O(dropped × total
/// textual bytes): thousands of dropped stray files alongside tens of MB of
/// surviving text - a shape an arbitrary downloaded EPUB can present - is on
/// the order of hundreds of GB of byte comparisons for one conversion. An
/// Aho-Corasick automaton searches for every basename at once in a single
/// pass, so the byte-scanning cost is O(total textual bytes + match events)
/// however long `dropped` is. Two smaller terms remain, both far below the
/// old one: pairing hits back to dropped paths is O(dropped × docs) hash
/// lookups, no byte scanning; and `match events` is not bounded by the byte
/// count when needles nest inside one another, so a document of 5 MB of `a`
/// against needles `a`, `aa`, `aaa`... would still be slow. Real basenames
/// carry an extension and a stem, so they do not nest that way.
///
/// The semantics stay *exactly* those of the nested `windows()` loop -
/// unanchored, byte-for-byte substring containment. That equality is the
/// point, and `scan_agrees_with_a_naive_substring_search` pins it: an earlier
/// attempt at this bound instead tokenized each document on bytes that cannot
/// appear in a filename and compared whole tokens, which silently stopped
/// matching any basename containing a space, a parenthesis or an ampersand -
/// and `normalize_entry_name` percent-*decodes* zip entry names, so
/// `OEBPS/my%20pic.png` becomes the key `OEBPS/my pic.png` and spaces in keys
/// are routine. A safety net that exists to prevent silent deletion must not
/// itself lose matches to buy speed.
///
/// Returns `(document_path, dropped_path)` pairs in a fixed, deterministic
/// order: outer by `dropped`'s order, inner by `docs`'s order - the same
/// order the original nested loop produced, and never derived from a
/// `HashSet`'s iteration order, so a caller building a report from these
/// stays byte-identical run to run.
///
/// Note: like the loop it replaces, this only ever matches a basename's
/// literal bytes - a reference spelled with percent-encoding (`my%20pic.png`)
/// will not match a dropped file named `my pic.png`. That is an existing,
/// accepted limitation of a raw byte scan, not something this change alters.
pub(crate) fn scan_for_dangling_basenames(
    dropped: &[String],
    docs: &[(&str, &[u8])],
) -> Vec<(String, String)> {
    // Deduped, because two dropped paths in different directories can share a
    // basename and the automaton only needs one pattern per distinct needle.
    // The map back from basename to pattern index keeps the pairing below a
    // hash lookup per dropped path rather than a linear search, so nothing
    // here is quadratic in `dropped.len()` either.
    let mut needles: Vec<&str> = Vec::new();
    let mut pattern_of: HashMap<&str, usize> = HashMap::new();
    for path in dropped {
        if let Some(basename) = basename_of(path) {
            pattern_of.entry(basename).or_insert_with(|| {
                needles.push(basename);
                needles.len() - 1
            });
        }
    }
    if needles.is_empty() {
        return Vec::new();
    }
    // `new` only fails on a pattern set too large for the automaton. A net
    // that cannot be built reports nothing rather than aborting the
    // conversion: it is advisory over a walk that is already correct.
    let Ok(automaton) = AhoCorasick::new(&needles) else {
        return Vec::new();
    };

    // One pass per document, whatever `dropped.len()` is. Overlapping search
    // (not leftmost) because a non-overlapping walk steps past the remainder
    // of each match, which would hide a needle nested inside another one -
    // `a.png` inside `data.png` - and the loop this replaced, being a
    // per-needle `windows()` scan, found both.
    let found_per_doc: Vec<HashSet<usize>> = docs
        .iter()
        .map(|(_, data)| {
            automaton
                .find_overlapping_iter(data)
                .map(|m| m.pattern().as_usize())
                .collect()
        })
        .collect();

    let mut hits = Vec::new();
    for dropped_path in dropped {
        let Some(pattern) = basename_of(dropped_path).and_then(|b| pattern_of.get(b)) else {
            continue;
        };
        for ((path, _), found) in docs.iter().zip(&found_per_doc) {
            if found.contains(pattern) {
                hits.push(((*path).to_string(), dropped_path.clone()));
            }
        }
    }
    hits
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

/// Every path a resource of `media_type` (whose directory is `dir`) names.
/// The graph walk's only extraction logic - deliberately NOT reused by
/// [`prune`]'s post-removal safety net, which exists specifically to catch
/// what this function fails to extract (see the module docs).
///
/// Takes the media type and bytes rather than a `&Resource` so `validate`'s
/// standalone reachability lint can walk a freshly-read archive with the same
/// extraction the destructive pass uses. One implementation, two callers: the
/// module docs record what a second one cost last time (a dead `xlink:href`
/// lookup that silently deleted real cover art).
pub(crate) fn refs_of(media_type: &str, data: &[u8], dir: &str) -> Vec<String> {
    match normalize_media_type(media_type).as_str() {
        "text/css" => match std::str::from_utf8(data) {
            Ok(text) => css_refs(text, dir),
            Err(_) => Vec::new(),
        },
        // XML formats, parsed separately rather than through the HTML5 tree
        // builder below, whose tag set/self-closing handling doesn't match
        // arbitrary XML.
        "application/x-dtbncx+xml" => match std::str::from_utf8(data) {
            Ok(text) => ncx_refs(text, dir),
            Err(_) => Vec::new(),
        },
        "application/smil+xml" => match std::str::from_utf8(data) {
            Ok(text) => smil_refs(text, dir),
            Err(_) => Vec::new(),
        },
        "application/oebps-package+xml" => match std::str::from_utf8(data) {
            Ok(text) => opf_refs(text, dir),
            Err(_) => Vec::new(),
        },
        "application/xhtml+xml" | "image/svg+xml" => {
            let Ok(doc) = parse_xhtml(data) else {
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
    //
    queue.push(book.opf_path.clone());
    match &book.nav_path {
        // A real nav: walked like any other root, because its links are the
        // book's own navigation and can reach resources the spine does not.
        Some(nav) => queue.push(nav.clone()),
        // No nav of its own, but the writer still emits `<opf_dir>/nav.xhtml`.
        // Whatever happens to sit at that path must not be dropped and
        // reported as unreferenced when the output contains that path - but
        // its stored bytes are discarded and regenerated, so its links must
        // not keep anything else alive either. Marked reachable without ever
        // being queued, which protects the path without walking it.
        None => {
            reachable.insert(crate::epub::write::effective_nav_path(book));
        }
    }
    queue.extend(book.ncx_path.clone());
    queue.extend(book.cover.clone());
    queue.extend(book.spine.iter().cloned());

    while let Some(path) = queue.pop() {
        if !reachable.insert(path.clone()) {
            continue;
        }
        if let Some(resource) = book.resources.get(&path) {
            let dir = parent_dir(&path);
            queue.extend(refs_of(&resource.media_type, &resource.data, &dir));
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

    // Independent safety net: see `scan_for_dangling_basenames`'s docs, and
    // the module docs, for why this shares no code with the walk above -
    // that independence is what lets it catch an edge type `refs_of` itself
    // fails to extract, which a check built from `refs_of` provably cannot.
    // Cheap false positives (a basename mentioned in prose) are the accepted
    // cost; a silent deletion is not.
    //
    // The three documents the writer regenerates are excluded, because their
    // stored bytes never reach the output at all: `write_epub` substitutes
    // freshly built bytes for the package document, the nav and the NCX and
    // discards `resource.data` for each. A mention in bytes that are thrown
    // away cannot dangle in the output, so scanning them can only produce
    // false alarms.
    //
    // For the OPF that is not a rare edge case but a guarantee: a manifest
    // declares every resource it owns, including the ones being dropped here,
    // so scanning it warned on *every* unreferenced file this pass has ever
    // removed. `prune` already applies exactly this reasoning to a
    // synthesized nav above ("its stored bytes are discarded and regenerated,
    // so its links must not keep anything else alive either"); this extends it
    // to the two documents that reasoning always applied to as well.
    let nav_path = crate::epub::write::effective_nav_path(book);
    let regenerated: [Option<&str>; 3] = [
        Some(book.opf_path.as_str()),
        Some(nav_path.as_str()),
        book.ncx_path.as_deref(),
    ];
    let textual_docs: Vec<(&str, &[u8])> = book
        .resources
        .iter()
        .filter(|(path, _)| !regenerated.contains(&Some(path.as_str())))
        .filter(|(_, resource)| {
            TEXTUAL_MEDIA_TYPES.contains(&normalize_media_type(&resource.media_type).as_str())
        })
        .map(|(path, resource)| (path.as_str(), resource.data.as_slice()))
        .collect();
    for (path, dropped_path) in scan_for_dangling_basenames(&dropped_ordered, &textual_docs) {
        // Always `Some`: the scan only pairs a `dropped_path` whose basename
        // it already resolved through this same helper, so a `None` here
        // would mean the two had drifted - skip rather than invent a name.
        let Some(basename) = basename_of(&dropped_path) else {
            continue;
        };
        warnings.push(Warning {
            message: format!(
                "{path} still mentions {basename} (from {dropped_path}, dropped as \
                 unreferenced) - the reachability graph may be missing an edge"
            ),
            file: Some(path.clone()),
        });
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
                    ..Default::default()
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

    /// The scan this replaced, written the obvious way: one `windows()` pass
    /// per dropped file per document. Slow by construction - which is the
    /// whole reason it was replaced - but its semantics are the contract, so
    /// it stands here as the reference the fast version is checked against.
    fn naive_scan(dropped: &[String], docs: &[(&str, &[u8])]) -> Vec<(String, String)> {
        let mut hits = Vec::new();
        for dropped_path in dropped {
            let Some(basename) = basename_of(dropped_path) else {
                continue;
            };
            let needle = basename.as_bytes();
            for (path, data) in docs {
                if data.len() >= needle.len() && data.windows(needle.len()).any(|w| w == needle) {
                    hits.push(((*path).to_string(), dropped_path.clone()));
                }
            }
        }
        hits
    }

    #[test]
    fn scan_agrees_with_a_naive_substring_search() {
        // The bound must not cost coverage. An earlier attempt tokenized each
        // document on bytes that cannot appear in a filename and compared
        // whole tokens, which silently stopped matching every basename
        // containing a space, a parenthesis, an ampersand or an apostrophe -
        // and stopped matching plain names abutting non-ASCII punctuation.
        // Every case below is one the tokenizing version got wrong and the
        // scan it replaced got right, so this fails against that version and
        // passes against both the old loop and the automaton.
        let dropped: Vec<String> = [
            "OEBPS/my pic.png",     // space: routine, %20 is decoded into the key
            "OEBPS/image(1).png",   // parentheses
            "OEBPS/Q&A.png",        // ampersand
            "OEBPS/it's.png",       // apostrophe
            "OEBPS/fig 1, rev.png", // space and comma
            "OEBPS/图1.png",        // non-ASCII, abutting non-ASCII punctuation
            "OEBPS/stray.png",      // plain, abutting typographic quotes
            "OEBPS/a.png",          // nested inside another needle's match
            "OEBPS/data.png",       // contains `a.png`
            "OEBPS/never-mentioned.png",
            "OEBPS/", // no basename at all
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        let docs: Vec<(&str, &[u8])> = vec![
            (
                "OEBPS/c1.xhtml",
                r#"<meta content="my pic.png"/><img src="image(1).png">"#.as_bytes(),
            ),
            (
                "OEBPS/c2.xhtml",
                "Q&A.png and it's.png and fig 1, rev.png".as_bytes(),
            ),
            (
                "OEBPS/c3.xhtml",
                "见 图1.png。 und „stray.png“ hier".as_bytes(),
            ),
            ("OEBPS/c4.xhtml", r#"<img src="data.png">"#.as_bytes()),
            ("OEBPS/c5.xhtml", "nothing at all".as_bytes()),
        ];

        let expected = naive_scan(&dropped, &docs);
        assert_eq!(scan_for_dangling_basenames(&dropped, &docs), expected);
        // Guard against the reference and the real thing agreeing on nothing:
        // an all-empty comparison would pass no matter how broken either was.
        assert!(
            expected.len() >= 9,
            "the fixture must actually produce hits, got {expected:?}"
        );
    }

    #[test]
    fn scan_finds_a_needle_nested_inside_another_needles_match() {
        // `data.png` contains `a.png`. A leftmost non-overlapping search
        // consumes `data.png` and never reports `a.png`; the per-needle
        // `windows()` loop reported both, so the automaton must too.
        let dropped = vec!["OEBPS/a.png".to_string(), "OEBPS/data.png".to_string()];
        let doc = br#"<img src="data.png">"#;
        let hits = scan_for_dangling_basenames(&dropped, &[("OEBPS/c.xhtml", doc)]);
        assert_eq!(
            hits,
            vec![
                ("OEBPS/c.xhtml".to_string(), "OEBPS/a.png".to_string()),
                ("OEBPS/c.xhtml".to_string(), "OEBPS/data.png".to_string()),
            ]
        );
    }

    #[test]
    fn the_safety_net_stays_linear_in_the_number_of_dropped_files() {
        // Measured against `naive_scan` on the SAME input rather than against
        // a fixed wall-clock bound. A constant like "under 5 seconds" encodes
        // this machine's speed and is the assertion most likely to flake on a
        // contended runner - and when it does, the failure reads like a real
        // regression. A ratio cancels machine speed out: whatever the runner,
        // one pass over the bytes must beat 2000 passes by a wide margin, and
        // a per-needle loop hidden inside the per-document one would land at
        // a ratio near 1 on any hardware.
        // Sized so the naive side stays well under a second: the ratio is what
        // discriminates, and a slow reference only makes the suite slow.
        let dropped: Vec<String> = (0..500).map(|i| format!("OEBPS/s{i}-unused.png")).collect();
        let doc = "<p>this paragraph names none of them</p>".repeat(4_000);
        let docs: &[(&str, &[u8])] = &[("OEBPS/c.xhtml", doc.as_bytes())];

        let start = std::time::Instant::now();
        let hits = scan_for_dangling_basenames(&dropped, docs);
        let fast = start.elapsed();

        let start = std::time::Instant::now();
        let reference = naive_scan(&dropped, docs);
        let slow = start.elapsed();

        assert!(hits.is_empty());
        assert_eq!(
            hits, reference,
            "and it must still agree with the reference"
        );
        assert!(
            fast * 10 < slow,
            "the automaton took {fast:?} against the naive scan's {slow:?} over {} bytes; \
             a bounded scan should win by far more than 10x",
            doc.len()
        );
    }

    #[test]
    fn the_safety_net_reports_every_dropped_path_sharing_a_basename() {
        // Two different dropped files can share a basename (same file name,
        // different directory). The automaton carries one pattern per
        // distinct needle, so the pairing step must still fan that single
        // match back out to one hit per dropped path.
        let dropped = vec![
            "OEBPS/images/dup.png".to_string(),
            "OEBPS/other/dup.png".to_string(),
        ];
        let doc = br#"<img src="dup.png">"#;
        let hits = scan_for_dangling_basenames(&dropped, &[("OEBPS/c.xhtml", doc)]);
        assert_eq!(
            hits,
            vec![
                (
                    "OEBPS/c.xhtml".to_string(),
                    "OEBPS/images/dup.png".to_string()
                ),
                (
                    "OEBPS/c.xhtml".to_string(),
                    "OEBPS/other/dup.png".to_string()
                ),
            ],
            "both dropped paths sharing a basename must be reported, in dropped order"
        );
    }

    #[test]
    fn raw_import_refs_is_case_insensitive() {
        // Mirrors `raw_url_refs_is_case_insensitive`: CSS at-rule keywords
        // are ASCII case-insensitive exactly like function names, so
        // `@IMPORT` and `@Import` must be found just like `@import`. Placed
        // after another rule so only this raw safety net - not the AST walk,
        // which lightningcss correctly discards a misplaced `@import` from
        // under `error_recovery` - can ever find them.
        let refs = raw_import_refs(
            "p{color:red}\n@IMPORT \"shout.css\";\n@Import 'whisper.css';",
            "OEBPS",
        );
        assert_eq!(
            refs,
            vec![
                "OEBPS/shout.css".to_string(),
                "OEBPS/whisper.css".to_string()
            ]
        );
    }

    #[test]
    fn raw_url_refs_does_not_search_past_max_raw_span() {
        // Pins MAX_RAW_SPAN's effect, not its value: a well-formed url()
        // value long enough that its closing `)` sits past the bound is not
        // captured - the safety net's search window ends before it ever
        // reaches that `)`. A short value comfortably inside the bound is
        // captured normally in the same call. If the bound were removed
        // (e.g. MAX_RAW_SPAN = usize::MAX), the window would span the whole
        // remaining string, the long value's `)` would be found, and this
        // assertion would fail - which is exactly the quadratic-blowup
        // regression the bound exists to prevent.
        let long_value = "a".repeat(3000);
        let css = format!("a{{background:url({long_value})}}");
        let refs = raw_url_refs(&css, "OEBPS");
        assert!(
            refs.is_empty(),
            "a url() value whose closing paren sits past MAX_RAW_SPAN must not be captured: got {refs:?}"
        );

        let short_refs = raw_url_refs("a{background:url(short.png)}", "OEBPS");
        assert_eq!(short_refs, vec!["OEBPS/short.png".to_string()]);
    }
}
