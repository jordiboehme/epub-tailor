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

/// Every path `css` names through `url(...)` or a `url()`-less
/// `@import "file.css"` / `@import 'file.css'`.
fn css_refs(css: &str, base_dir: &str) -> Vec<String> {
    let mut out = Vec::new();

    // `url(...)`: covers both a plain `url()` value and `@import url(...)`.
    // A `url(` with no matching `)` skips just that occurrence and keeps
    // scanning - one malformed declaration must not hide every later
    // `url()` in the sheet.
    let mut rest = css;
    while let Some(start) = rest.find("url(") {
        let after = &rest[start + 4..];
        match after.find(')') {
            Some(end) => {
                let raw = after[..end].trim().trim_matches(['"', '\'']);
                if let Some(p) = resolve(base_dir, raw) {
                    out.push(p);
                }
                rest = &after[end + 1..];
            }
            None => rest = after,
        }
    }

    // `@import "file.css"`: the string-literal form, with no `url(...)`
    // wrapper. `cursor` always advances past the `@import` keyword it just
    // matched, so the loop terminates even when nothing after it parses.
    let mut cursor = css;
    while let Some(start) = cursor.find("@import") {
        let tail = &cursor[start + "@import".len()..];
        cursor = tail;
        let trimmed = tail.trim_start();
        let quote = trimmed.chars().next().filter(|c| *c == '"' || *c == '\'');
        if let Some(q) = quote
            && let Some(end) = trimmed[1..].find(q)
        {
            let raw = &trimmed[1..1 + end];
            if let Some(p) = resolve(base_dir, raw) {
                out.push(p);
            }
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
/// `extra_roots` seeds the walk with paths a caller resolved *before* this
/// runs, from a reference type that no longer exists in the book by the time
/// `prune` sees it - today, exactly the `<img srcset>` targets
/// `image::rewrite_refs` strips on every conversion (not just `generic`)
/// well before this walk starts. Without them, the walk simply cannot see
/// the edge: it is not a gap in [`refs_of`]'s extraction, it is the
/// reference no longer being there to extract.
pub(crate) fn prune(
    book: &mut Book,
    transformations: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
    extra_roots: &[String],
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
    queue.extend(extra_roots.iter().cloned());

    while let Some(path) = queue.pop() {
        if !reachable.insert(path.clone()) {
            continue;
        }
        let Some(resource) = book.resources.get(&path) else {
            continue;
        };
        let dir = parent_dir(&path);
        queue.extend(refs_of(resource, &dir));
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
        prune_book_with_roots(book, &[])
    }

    fn prune_book_with_roots(
        book: &mut Book,
        extra_roots: &[String],
    ) -> (Vec<Transformation>, Vec<Warning>) {
        let mut transformations = Vec::new();
        let mut warnings = Vec::new();
        prune(book, &mut transformations, &mut warnings, extra_roots);
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
    fn an_extra_root_survives_pruning_even_though_nothing_in_the_dom_names_it() {
        // Pins the mechanism `image::rewrite_refs`'s `<img srcset>` targets
        // rely on: a path handed in as an extra root must survive even
        // though, by construction here, nothing in any surviving document
        // references it at all.
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
        prune_book_with_roots(&mut book, &["OEBPS/only.png".to_string()]);
        assert!(book.resources.contains_key("OEBPS/only.png"));
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
    fn an_unterminated_trailing_url_does_not_panic_or_lose_earlier_matches() {
        // The `url(` that opens here never closes anywhere in the rest of the
        // string (it is the last thing in the sheet) - the old code's `else
        // { break }` on this exact shape aborted the whole scan; it must not
        // take the already-collected earlier match down with it.
        let refs = css_refs(
            "a { background: url(good.png); } b { background: url(unterminated",
            "OEBPS",
        );
        assert_eq!(refs, vec!["OEBPS/good.png".to_string()]);
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
