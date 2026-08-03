//! Unreferenced-resource removal by reachability.
//!
//! Manifest membership is the wrong test. Files referenced only from a CSS
//! `@font-face` or an HTML `src` while missing from the manifest are a spec
//! violation *and* common in the wild, and the writer's keep-and-promote
//! behaviour is the correct repair for them. So the graph is walked from real
//! roots along real references, and only what nothing points at is dropped.

use std::collections::HashSet;

use crate::epub::Book;
use crate::epub::model::normalize_href;
use crate::html::dom::{collect_by_name, get_attr};
use crate::html::parse::parse_xhtml;
use crate::report::Transformation;

/// Attributes that can name another resource.
const REF_ATTRS: &[&str] = &["src", "href", "poster", "xlink:href"];

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

/// Every path `css` names through `url(...)` or a `url()`-less
/// `@import "file.css"` / `@import 'file.css'`.
fn css_refs(css: &str, base_dir: &str) -> Vec<String> {
    let mut out = Vec::new();

    // `url(...)`: covers both a plain `url()` value and `@import url(...)`.
    let mut rest = css;
    while let Some(start) = rest.find("url(") {
        let after = &rest[start + 4..];
        let Some(end) = after.find(')') else { break };
        let raw = after[..end].trim().trim_matches(['"', '\'']);
        if let Some(p) = resolve(base_dir, raw) {
            out.push(p);
        }
        rest = &after[end + 1..];
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

/// Drop every resource unreachable from the book's roots.
pub(crate) fn prune(book: &mut Book, transformations: &mut Vec<Transformation>) {
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
        let Some(resource) = book.resources.get(&path) else {
            continue;
        };
        let dir = parent_dir(&path);
        let found: Vec<String> = match resource.media_type.as_str() {
            "text/css" => match std::str::from_utf8(&resource.data) {
                Ok(text) => css_refs(text, &dir),
                Err(_) => Vec::new(),
            },
            // The NCX is XML, not HTML: parsed separately rather than through
            // the HTML5 tree builder below, whose tag set it does not share.
            "application/x-dtbncx+xml" => match std::str::from_utf8(&resource.data) {
                Ok(text) => ncx_refs(text, &dir),
                Err(_) => Vec::new(),
            },
            "application/xhtml+xml" | "image/svg+xml" => {
                let Ok(doc) = parse_xhtml(&resource.data) else {
                    continue;
                };
                let mut refs = Vec::new();
                for name in [
                    "img", "image", "link", "a", "source", "video", "audio", "use",
                ] {
                    for node in collect_by_name(&doc, name) {
                        for attr in REF_ATTRS {
                            if let Some(raw) = get_attr(&node, attr)
                                && let Some(p) = resolve(&dir, &raw)
                            {
                                refs.push(p);
                            }
                        }
                    }
                }
                // Inline `<style>` blocks reference assets too.
                for node in collect_by_name(&doc, "style") {
                    refs.extend(css_refs(&node.text_contents(), &dir));
                }
                refs
            }
            _ => Vec::new(),
        };
        queue.extend(found);
    }

    let dropped: Vec<String> = book
        .resources
        .keys()
        .filter(|p| !reachable.contains(*p))
        .cloned()
        .collect();
    for path in dropped {
        let size = book.resources[&path].data.len();
        book.resources.shift_remove(&path);
        transformations.push(Transformation {
            kind: "generic-unreferenced".to_string(),
            detail: format!("dropped {size} bytes nothing references"),
            file: Some(path),
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
        let mut transformations = Vec::new();
        prune(&mut book, &mut transformations);
        assert!(!book.resources.contains_key("OEBPS/stray.txt"));
        assert!(book.resources.contains_key("OEBPS/chapter.xhtml"));
        assert_eq!(transformations.len(), 1);
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
        let mut transformations = Vec::new();
        prune(&mut book, &mut transformations);
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
        let mut transformations = Vec::new();
        prune(&mut book, &mut transformations);
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
        let mut transformations = Vec::new();
        // Must not panic or otherwise choke on the absolute/data refs.
        prune(&mut book, &mut transformations);
        assert!(book.resources.contains_key("OEBPS/chapter.xhtml"));
    }
}
