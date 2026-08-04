//! Writing a [`Book`] back out as an epubcheck-clean EPUB.
//!
//! The container, package document (OPF), navigation document and NCX are all
//! regenerated from the model via [`askama`] templates; every other resource
//! is echoed verbatim. The ZIP is assembled by hand so the `mimetype` entry
//! comes first, STORED and with no extra field, as the OCF spec (and
//! epubcheck) require.

use std::collections::HashSet;
use std::io::{Cursor, Write};

use askama::Template;
use time::OffsetDateTime;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::epub::model::{Book, Creator, Metadata, TocEntry};
use crate::error::ConvertError;
use crate::html::escape::{escape_attr, escape_text};
use crate::report::Warning;

/// The `dcterms:modified` value the generic profile pins, so two copies of the
/// same edition converge. EPUB 3 requires the field, so it cannot be omitted;
/// a fixed epoch is the only value that converges even when the vendor stamped
/// the source field per copy.
pub const GENERIC_MODIFIED: &str = "1970-01-01T00:00:00Z";

/// Serialize a [`Book`] as a complete EPUB archive.
///
/// The OPF, navigation document and NCX are regenerated (the corresponding
/// bytes in `book.resources` are ignored); every other resource is written
/// unchanged. A navigation document and/or NCX are synthesized from
/// `book.toc` if the book lacks them.
///
/// `stamp` is the provenance marker written as
/// `<meta property="tailor:fitted">` (with its prefix declaration); `None`
/// leaves the OPF byte-identical to an unstamped write. `stamp_profile` names
/// the profile behind the stamp as a sibling `<meta property="tailor:profile">`;
/// it is ignored without a `stamp`. `fixed_modified` overrides the
/// `dcterms:modified` value with a caller-supplied string instead of the
/// current wall-clock time; `None` stamps `now_utc_iso8601()` as before.
///
/// Every manifest item's id is reassigned here (see [`IdAllocator`]), so a
/// `media-overlay`/`fallback` linkage carried on a [`crate::epub::model::Resource`]
/// (itself a resource path, already resolved from the source OPF's idref) is
/// re-resolved against the *new* ids rather than copied verbatim. A linkage
/// whose target is no longer a manifest path - pruned as unreferenced
/// upstream, or one that never resolved when the book was read - is omitted
/// (never emitted as a dangling reference) and reported to `warnings`.
///
/// # Errors
/// Returns [`ConvertError::Io`] if a template fails to render or a ZIP entry
/// cannot be written (neither is expected for well-formed model data).
pub fn write_epub(
    book: &Book,
    stamp: Option<&str>,
    stamp_profile: Option<&str>,
    fixed_modified: Option<&str>,
    warnings: &mut Vec<Warning>,
) -> Result<Vec<u8>, ConvertError> {
    let opf_dir = parent_dir(&book.opf_path);
    let nav_path = effective_nav_path(book);
    let ncx_path = book
        .ncx_path
        .clone()
        .unwrap_or_else(|| join_dir(&opf_dir, "toc.ncx"));
    let identifier = book
        .metadata
        .identifier
        .clone()
        .unwrap_or_else(|| synth_identifier(&book.metadata));

    // Manifest paths: every resource except the OPF (a package document is not
    // listed in its own manifest), plus synthesized nav/NCX if they are new.
    let mut manifest_paths: Vec<String> = book
        .resources
        .keys()
        .filter(|p| **p != book.opf_path)
        .cloned()
        .collect();
    if !book.resources.contains_key(&nav_path) {
        manifest_paths.push(nav_path.clone());
    }
    if !book.resources.contains_key(&ncx_path) {
        manifest_paths.push(ncx_path.clone());
    }

    // Pass 1: assign every manifest path a fresh, unique id up front. This
    // has to happen as its own pass, before any item is built, because a
    // `media-overlay`/`fallback` target can be *any* other manifest path -
    // including one that has not been reached yet in path order - and the
    // new id it needs to remap to only exists once that path's own
    // allocation has run.
    let mut allocator = IdAllocator::default();
    let ids: Vec<String> = manifest_paths
        .iter()
        .map(|p| allocator.allocate(p))
        .collect();
    let path_ids: std::collections::HashMap<&str, &str> = manifest_paths
        .iter()
        .zip(ids.iter())
        .map(|(p, id)| (p.as_str(), id.as_str()))
        .collect();
    let ncx_id = manifest_paths
        .iter()
        .position(|p| *p == ncx_path)
        .map(|i| ids[i].clone())
        .unwrap_or_default();

    // Pass 2: build the item list, resolving each resource's carried-over
    // media-overlay/fallback linkage (a resource path, already resolved from
    // the source idref by the reader) through the id map pass 1 just built.
    let mut items: Vec<OpfItem> = Vec::new();
    let mut cover_id = String::new();
    let mut media_durations: Vec<OpfMediaDuration> = Vec::new();
    for (path, id) in manifest_paths.iter().zip(ids.iter()) {
        let media_type = if *path == nav_path {
            "application/xhtml+xml".to_string()
        } else if *path == ncx_path {
            "application/x-dtbncx+xml".to_string()
        } else {
            book.resources
                .get(path)
                .map(|r| r.media_type.clone())
                .unwrap_or_else(|| "application/octet-stream".to_string())
        };
        let mut properties: Vec<&str> = Vec::new();
        if *path == nav_path {
            properties.push("nav");
        }
        if book.cover.as_deref() == Some(path.as_str()) {
            properties.push("cover-image");
            cover_id = id.clone();
        }
        let resource = book.resources.get(path);
        let media_overlay = resource
            .and_then(|r| r.media_overlay.as_deref())
            .map(|target| resolve_linkage(path, "media-overlay", target, &path_ids, warnings))
            .unwrap_or_default();
        let fallback = resource
            .and_then(|r| r.fallback.as_deref())
            .map(|target| resolve_linkage(path, "fallback", target, &path_ids, warnings))
            .unwrap_or_default();
        // This item's own duration refinement (it names itself via `id`, so
        // - unlike media-overlay/fallback - there is no cross-reference to
        // remap: the item either survived pass 1 and got `id`, or it isn't
        // here to iterate over at all.
        if let Some(value) = resource.and_then(|r| r.media_duration.clone()) {
            media_durations.push(OpfMediaDuration {
                id: id.clone(),
                value,
            });
        }
        items.push(OpfItem {
            id: id.clone(),
            href: relative_href(&opf_dir, path),
            media_type,
            properties: properties.join(" "),
            media_overlay,
            fallback,
        });
    }

    // Spine idrefs, resolved from the item ids assigned above.
    let spine: Vec<String> = book
        .spine
        .iter()
        .filter_map(|p| path_ids.get(p.as_str()).map(|id| (*id).to_string()))
        .collect();

    // Build the (nested) TOC once; render it relative to each document's dir.
    let toc_tree = build_toc_tree_or_synthesize(book);
    let depth = tree_depth(&toc_tree).max(1);

    // Every real input path already defaults an absent/blank language to
    // "en" (with a warning); this is a last-resort guard so a hand-built
    // `Book` can never round-trip an empty `dc:language`, which epubcheck
    // flags. Deliberately still silent even though `write_epub` now has a
    // `warnings` channel: a hand-built `Book` bypassing the reader is the
    // only way to hit this, and the reader's own warning already covers the
    // real-input path.
    let opf_language = if book.metadata.language.trim().is_empty() {
        "en".to_string()
    } else {
        book.metadata.language.clone()
    };

    let meta = &book.metadata;
    // Each creator and secondary identifier needs a stable id so its EPUB3
    // refinements (`file-as`, `role`, `identifier-type`) have something to point
    // at. They are positional and local to this document, so a plain counter is
    // enough - and it keeps the output deterministic.
    let creators = |prefix: &str, list: &[Creator]| -> Vec<OpfCreator> {
        list.iter()
            .enumerate()
            .map(|(i, c)| OpfCreator {
                id: format!("{prefix}{}", i + 1),
                name: c.name.clone(),
                file_as: c.file_as.clone().unwrap_or_default(),
                role: c.role.clone().unwrap_or_default(),
            })
            .collect()
    };
    let series = meta.series.clone().unwrap_or_default();

    let opf_bytes = OpfTemplate {
        identifier: identifier.clone(),
        identifiers: meta
            .identifiers
            .iter()
            .enumerate()
            .map(|(i, id)| OpfIdentifier {
                id: format!("id{}", i + 1),
                value: id.value.clone(),
                scheme: id.scheme.clone().unwrap_or_default(),
            })
            .collect(),
        title: meta.title.clone(),
        authors: creators("creator", &meta.authors),
        contributors: creators("contrib", &meta.contributors),
        language: opf_language,
        publisher: meta.publisher.clone().unwrap_or_default(),
        description: meta.description.clone().unwrap_or_default(),
        subjects: meta.subjects.clone(),
        date: meta.date.clone().unwrap_or_default(),
        rights: meta.rights.clone().unwrap_or_default(),
        series: series.name,
        series_index: series.index.unwrap_or_default(),
        modified: fixed_modified
            .map(str::to_string)
            .unwrap_or_else(now_utc_iso8601),
        stamp: stamp.unwrap_or_default().to_string(),
        stamp_profile: stamp.and(stamp_profile).unwrap_or_default().to_string(),
        cover_id,
        ncx_id,
        items,
        spine,
        media_duration: meta.media_duration.clone().unwrap_or_default(),
        media_durations,
    }
    .render()
    .map_err(render_err)?
    .into_bytes();

    let nav_bytes = NavTemplate {
        title: book.metadata.title.clone(),
        lang: book.metadata.language.clone(),
        ol_html: render_nav_ol(&toc_tree, &parent_dir(&nav_path)),
    }
    .render()
    .map_err(render_err)?
    .into_bytes();

    let ncx_bytes = NcxTemplate {
        identifier,
        title: book.metadata.title.clone(),
        depth,
        navpoints_html: render_ncx_points(&toc_tree, &parent_dir(&ncx_path)),
    }
    .render()
    .map_err(render_err)?
    .into_bytes();

    let container_bytes = ContainerTemplate {
        opf_path: percent_encode(&book.opf_path, true),
    }
    .render()
    .map_err(render_err)?
    .into_bytes();

    // Assemble the ZIP: mimetype first (STORED), then the container, then every
    // resource (DEFLATE) with the regenerated bytes swapped in.
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file("mimetype", stored).map_err(zip_err)?;
    zip.write_all(b"application/epub+zip")?;

    zip.start_file("META-INF/container.xml", deflated)
        .map_err(zip_err)?;
    zip.write_all(&container_bytes)?;

    for (path, resource) in &book.resources {
        let bytes: &[u8] = if *path == book.opf_path {
            &opf_bytes
        } else if *path == nav_path {
            &nav_bytes
        } else if *path == ncx_path {
            &ncx_bytes
        } else {
            &resource.data
        };
        zip.start_file(path, deflated).map_err(zip_err)?;
        zip.write_all(bytes)?;
    }
    if !book.resources.contains_key(&nav_path) {
        zip.start_file(&nav_path, deflated).map_err(zip_err)?;
        zip.write_all(&nav_bytes)?;
    }
    if !book.resources.contains_key(&ncx_path) {
        zip.start_file(&ncx_path, deflated).map_err(zip_err)?;
        zip.write_all(&ncx_bytes)?;
    }

    let cursor = zip.finish().map_err(zip_err)?;
    Ok(cursor.into_inner())
}

// ---------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------

#[derive(Template)]
#[template(path = "container.xml", escape = "html")]
struct ContainerTemplate {
    opf_path: String,
}

#[derive(Template)]
#[template(path = "content.opf", escape = "html")]
struct OpfTemplate {
    identifier: String,
    /// Secondary identifiers (ISBNs and the like). The unique one is above.
    identifiers: Vec<OpfIdentifier>,
    title: String,
    authors: Vec<OpfCreator>,
    contributors: Vec<OpfCreator>,
    language: String,
    publisher: String,
    description: String,
    subjects: Vec<String>,
    date: String,
    rights: String,
    series: String,
    series_index: String,
    modified: String,
    /// The `tailor:fitted` provenance value; empty writes no stamp and no
    /// prefix declaration, keeping the OPF byte-identical to pre-stamp output.
    stamp: String,
    /// The `tailor:profile` value naming the fitting profile; only ever
    /// non-empty together with [`Self::stamp`].
    stamp_profile: String,
    cover_id: String,
    ncx_id: String,
    items: Vec<OpfItem>,
    spine: Vec<String>,
    /// The book-wide `<meta property="media:duration">`, empty when the
    /// source had none. EPUB 3 requires it whenever any `items` entry
    /// carries `media-overlay` (see [`OpfItem::media_overlay`]).
    media_duration: String,
    /// One `<meta refines="#id" property="media:duration">` per SMIL item
    /// whose duration survived the rebuild - `id` already the item's
    /// *regenerated* manifest id, not the source's.
    media_durations: Vec<OpfMediaDuration>,
}

/// One SMIL item's read-aloud duration refinement: `id` is that item's own
/// regenerated manifest id (the `refines` target), `value` the duration text.
struct OpfMediaDuration {
    id: String,
    value: String,
}

/// A `dc:creator`/`dc:contributor` as the template needs it: an id to hang the
/// refinements off, and empty strings rather than `Option`s (askama's `if` is
/// happier with `!x.is_empty()`, which is how the rest of this template reads).
struct OpfCreator {
    id: String,
    name: String,
    file_as: String,
    role: String,
}

/// A secondary `dc:identifier` plus the id its `identifier-type` refines.
struct OpfIdentifier {
    id: String,
    value: String,
    scheme: String,
}

/// One `<item>` in the OPF manifest.
struct OpfItem {
    id: String,
    href: String,
    media_type: String,
    /// Space-separated `properties` value, empty when the item has none.
    properties: String,
    /// The new id of this item's media-overlay SMIL document, empty when the
    /// item has none (see [`resolve_linkage`]).
    media_overlay: String,
    /// The new id of this item's fallback target, empty when the item has
    /// none (see [`resolve_linkage`]).
    fallback: String,
}

#[derive(Template)]
#[template(path = "nav.xhtml", escape = "html")]
struct NavTemplate {
    title: String,
    lang: String,
    /// Pre-rendered, already-escaped nested `<ol>` markup.
    ol_html: String,
}

#[derive(Template)]
#[template(path = "toc.ncx", escape = "html")]
struct NcxTemplate {
    identifier: String,
    title: String,
    depth: u32,
    /// Pre-rendered, already-escaped `<navPoint>` markup.
    navpoints_html: String,
}

// ---------------------------------------------------------------------
// Table of contents tree
// ---------------------------------------------------------------------

/// A nested table-of-contents entry, derived from the flat, level-tagged
/// [`TocEntry`] list.
struct TocNode {
    title: String,
    href: String,
    children: Vec<TocNode>,
}

/// Build the nested TOC, synthesizing a single entry (book title → first spine
/// document) when the book has no table of contents.
fn build_toc_tree_or_synthesize(book: &Book) -> Vec<TocNode> {
    if !book.toc.is_empty() {
        let mut pos = 0;
        return build_toc_tree(&book.toc, &mut pos, 0);
    }
    let target = book
        .spine
        .first()
        .cloned()
        .or_else(|| {
            book.resources
                .keys()
                .find(|p| **p != book.opf_path)
                .cloned()
        })
        .unwrap_or_default();
    vec![TocNode {
        title: fallback_title(&book.metadata.title),
        href: target,
        children: Vec::new(),
    }]
}

/// Fold the flat, level-tagged entries into a tree: an entry's children are the
/// immediately following entries with a strictly greater level.
fn build_toc_tree(entries: &[TocEntry], pos: &mut usize, parent_level: u8) -> Vec<TocNode> {
    let mut nodes = Vec::new();
    while let Some(entry) = entries.get(*pos) {
        if entry.level <= parent_level {
            break;
        }
        let my_level = entry.level;
        *pos += 1;
        let children = build_toc_tree(entries, pos, my_level);
        nodes.push(TocNode {
            title: entry.title.clone(),
            href: entry.href.clone(),
            children,
        });
    }
    nodes
}

fn tree_depth(nodes: &[TocNode]) -> u32 {
    nodes
        .iter()
        .map(|n| 1 + tree_depth(&n.children))
        .max()
        .unwrap_or(0)
}

/// Render the nested `<ol>` for the navigation document, hrefs relative to
/// `nav_dir`. Returns an empty string for an empty tree.
fn render_nav_ol(nodes: &[TocNode], nav_dir: &str) -> String {
    let mut out = String::new();
    render_nav_ol_into(nodes, nav_dir, &mut out);
    out
}

fn render_nav_ol_into(nodes: &[TocNode], nav_dir: &str, out: &mut String) {
    if nodes.is_empty() {
        return;
    }
    out.push_str("<ol>\n");
    for node in nodes {
        out.push_str("<li><a href=\"");
        out.push_str(&escape_attr(&relative_href(nav_dir, &node.href)));
        out.push_str("\">");
        out.push_str(&escape_text(&fallback_title(&node.title)));
        out.push_str("</a>");
        if !node.children.is_empty() {
            out.push('\n');
            render_nav_ol_into(&node.children, nav_dir, out);
        }
        out.push_str("</li>\n");
    }
    out.push_str("</ol>");
}

/// Render the `<navPoint>` tree for the NCX, srcs relative to `ncx_dir`, with
/// sequential depth-first `playOrder`.
fn render_ncx_points(nodes: &[TocNode], ncx_dir: &str) -> String {
    let mut out = String::new();
    let mut counter = 0u32;
    render_ncx_points_into(nodes, ncx_dir, &mut counter, &mut out);
    out
}

fn render_ncx_points_into(nodes: &[TocNode], ncx_dir: &str, counter: &mut u32, out: &mut String) {
    for node in nodes {
        *counter += 1;
        let order = *counter;
        out.push_str("<navPoint id=\"navPoint-");
        out.push_str(&order.to_string());
        out.push_str("\" playOrder=\"");
        out.push_str(&order.to_string());
        out.push_str("\">\n");
        out.push_str("<navLabel><text>");
        out.push_str(&escape_text(&fallback_title(&node.title)));
        out.push_str("</text></navLabel>\n");
        out.push_str("<content src=\"");
        out.push_str(&escape_attr(&relative_href(ncx_dir, &node.href)));
        out.push_str("\"/>\n");
        render_ncx_points_into(&node.children, ncx_dir, counter, out);
        out.push_str("</navPoint>\n");
    }
}

fn fallback_title(title: &str) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

// ---------------------------------------------------------------------
// Manifest id allocation
// ---------------------------------------------------------------------

/// Resolve `target` - a resource path, already resolved from a source OPF
/// idref by the reader (see [`crate::epub::read`]) - to the *new* id
/// `path_ids` assigned it, for the `media-overlay`/`fallback` linkage `path`
/// (also a resource path, for the warning message) carries.
///
/// `target` no longer naming a manifest path is expected, not a bug: the
/// resource it pointed at may have been pruned as unreferenced by an earlier
/// pipeline stage, or (defensively, since a hand-built [`Book`] need not have
/// gone through the reader at all) never named a real manifest item to begin
/// with. Either way this returns `""` - the template's empty-string-means-
/// absent convention, matching [`OpfItem::properties`] - rather than a
/// dangling reference, which epubcheck rejects; the caller loses linkage
/// info it cannot safely emit, so this also records why.
fn resolve_linkage(
    path: &str,
    kind: &str,
    target: &str,
    path_ids: &std::collections::HashMap<&str, &str>,
    warnings: &mut Vec<Warning>,
) -> String {
    match path_ids.get(target) {
        Some(id) => (*id).to_string(),
        None => {
            warnings.push(Warning {
                message: format!(
                    "{path}: {kind} target '{target}' is not in the rebuilt manifest; \
                     dropping the {kind} linkage"
                ),
                file: Some(path.to_string()),
            });
            String::new()
        }
    }
}

/// Allocates XML-safe, unique manifest ids derived from resource paths.
#[derive(Default)]
struct IdAllocator {
    used: HashSet<String>,
}

impl IdAllocator {
    /// Derive an id from `path`: non-alphanumeric characters become `-`, a
    /// leading non-letter is prefixed with `id-`, and collisions get a numeric
    /// suffix.
    fn allocate(&mut self, path: &str) -> String {
        let mut base: String = path
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        if !base.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            base = format!("id-{base}");
        }
        let mut candidate = base.clone();
        let mut n = 2;
        while self.used.contains(&candidate) {
            candidate = format!("{base}-{n}");
            n += 1;
        }
        self.used.insert(candidate.clone());
        candidate
    }
}

// ---------------------------------------------------------------------
// href helpers
// ---------------------------------------------------------------------

/// Compute an href from `from_dir` to `to_path`, both zip-absolute normalized
/// paths (as produced by [`crate::epub::model::normalize_href`]). `to_path`
/// may carry a `#fragment`. The result is relative, percent-encoded for use in
/// an href, and forms the inverse of `normalize_href`.
pub fn relative_href(from_dir: &str, to_path: &str) -> String {
    let (path, fragment) = match to_path.split_once('#') {
        Some((p, f)) => (p, Some(f)),
        None => (to_path, None),
    };
    let mut out = percent_encode(&relative_path(from_dir, path), true);
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(&percent_encode(fragment, false));
    }
    out
}

/// Relative path from directory `from_dir` to file `to_path` (both
/// `/`-separated, no leading slash), using `..` to ascend as needed.
fn relative_path(from_dir: &str, to_path: &str) -> String {
    let from: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let to: Vec<&str> = to_path.split('/').filter(|s| !s.is_empty()).collect();

    // Never treat the target's own filename (last segment) as a shared dir.
    let max_common = from.len().min(to.len().saturating_sub(1));
    let mut common = 0;
    while common < max_common && from[common] == to[common] {
        common += 1;
    }

    let mut segments: Vec<&str> = Vec::new();
    segments.extend(std::iter::repeat_n("..", from.len() - common));
    segments.extend(&to[common..]);
    segments.join("/")
}

const HEX: [u8; 16] = *b"0123456789ABCDEF";

/// Percent-encode `s` for use in an href, leaving the URL-unreserved set
/// (`A-Za-z0-9-._~`) and, when `keep_slash`, path separators intact.
fn percent_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let keep = b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~')
            || (keep_slash && b == b'/');
        if keep {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
    }
    out
}

// ---------------------------------------------------------------------
// Misc helpers
// ---------------------------------------------------------------------

/// The path [`write_epub`] will write the nav document to: `book.nav_path`
/// when the book had one, otherwise `<opf_dir>/nav.xhtml`.
///
/// Shared rather than inlined because three callers outside the writer need
/// to know which resource the writer is going to overwrite: the invisible-
/// character scrub, the reachability walk's roots and the resource filter's
/// protected list. Guarding on `nav_path`
/// alone misses the fallback: an EPUB 2 book has `nav_path: None` but may
/// still carry a non-spine XHTML at exactly `<opf_dir>/nav.xhtml`, and that
/// file's stored bytes never ship - the writer regenerates them from
/// `book.metadata` and `book.toc`. Any work done on it is dead work that
/// still reports a transformation.
pub(crate) fn effective_nav_path(book: &Book) -> String {
    book.nav_path
        .clone()
        .unwrap_or_else(|| join_dir(&parent_dir(&book.opf_path), "nav.xhtml"))
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

/// Synthesize a deterministic `urn:epub-tailor:<hex>` identifier by hashing the
/// title and authors (FNV-1a), for books whose metadata has no identifier.
fn synth_identifier(metadata: &Metadata) -> String {
    let mut data = metadata.title.clone();
    for author in &metadata.authors {
        data.push('\u{0}');
        data.push_str(&author.name);
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("urn:epub-tailor:{hash:016x}")
}

/// Current UTC time as an EPUB `dcterms:modified` value (`CCYY-MM-DDThh:mm:ssZ`,
/// no fractional seconds).
fn now_utc_iso8601() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
    )
}

fn render_err(e: askama::Error) -> ConvertError {
    ConvertError::Io(std::io::Error::other(e))
}

fn zip_err(e: zip::result::ZipError) -> ConvertError {
    ConvertError::Io(std::io::Error::other(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_href_same_directory() {
        assert_eq!(
            relative_href("OEBPS", "OEBPS/text/ch1.xhtml"),
            "text/ch1.xhtml"
        );
    }

    #[test]
    fn relative_href_sibling_file() {
        assert_eq!(relative_href("OEBPS", "OEBPS/nav.xhtml"), "nav.xhtml");
    }

    #[test]
    fn relative_href_ascends_to_sibling_directory() {
        assert_eq!(
            relative_href("OEBPS/text", "OEBPS/images/cover.jpg"),
            "../images/cover.jpg"
        );
    }

    #[test]
    fn relative_href_from_root_dir() {
        assert_eq!(relative_href("", "OEBPS/content.opf"), "OEBPS/content.opf");
    }

    #[test]
    fn relative_href_preserves_fragment() {
        assert_eq!(
            relative_href("OEBPS", "OEBPS/text/ch1.xhtml#s2"),
            "text/ch1.xhtml#s2"
        );
    }

    #[test]
    fn relative_href_percent_encodes_spaces() {
        assert_eq!(relative_href("OEBPS", "OEBPS/ch 1.xhtml"), "ch%201.xhtml");
    }

    #[test]
    fn relative_href_is_inverse_of_normalize_href() {
        use crate::epub::model::normalize_href;
        let rel = relative_href("OEBPS/text", "OEBPS/images/a b.jpg");
        assert_eq!(normalize_href("OEBPS/text", &rel), "OEBPS/images/a b.jpg");
    }

    #[test]
    fn ids_are_unique_and_start_with_a_letter() {
        let mut alloc = IdAllocator::default();
        let a = alloc.allocate("OEBPS/text/ch1.xhtml");
        let b = alloc.allocate("OEBPS/text/ch1.xhtml");
        assert_ne!(a, b, "duplicate paths must get distinct ids");
        assert!(a.starts_with(|c: char| c.is_ascii_alphabetic()));
        assert!(b.starts_with(|c: char| c.is_ascii_alphabetic()));
        let leading_digit = alloc.allocate("123.xhtml");
        assert!(leading_digit.starts_with(|c: char| c.is_ascii_alphabetic()));
    }

    #[test]
    fn synth_identifier_is_deterministic() {
        let meta = Metadata {
            title: "Book".to_string(),
            authors: vec![Creator::new("Author")],
            language: "en".to_string(),
            ..Metadata::default()
        };
        assert_eq!(synth_identifier(&meta), synth_identifier(&meta));
        assert!(synth_identifier(&meta).starts_with("urn:epub-tailor:"));
    }

    /// Write a hand-built `Book` carrying `language` and return its regenerated
    /// OPF text, for pinning the last-resort `dc:language` guard and the stamp.
    fn opf_for(language: &str, stamp: Option<&str>) -> String {
        opf_for_stamped(language, stamp, None)
    }

    fn opf_for_stamped(language: &str, stamp: Option<&str>, stamp_profile: Option<&str>) -> String {
        use std::io::Read;

        let opf_path = "OEBPS/content.opf".to_string();
        let mut resources = indexmap::IndexMap::new();
        // The writer only emits an entry for `opf_path` if it is present in
        // `resources` (its bytes get swapped for the regenerated OPF); a
        // placeholder is enough since the content is always regenerated.
        resources.insert(
            opf_path.clone(),
            crate::epub::model::Resource {
                data: Vec::new(),
                media_type: "application/oebps-package+xml".to_string(),
                ..Default::default()
            },
        );
        let book = Book {
            metadata: Metadata {
                title: "T".to_string(),
                language: language.to_string(),
                ..Metadata::default()
            },
            resources,
            spine: vec![],
            toc: vec![],
            cover: None,
            opf_path,
            nav_path: None,
            ncx_path: None,
        };
        let bytes = write_epub(&book, stamp, stamp_profile, None, &mut Vec::new())
            .expect("write should succeed");

        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("output is a valid zip");
        let mut opf = String::new();
        archive
            .by_name("OEBPS/content.opf")
            .expect("opf entry present")
            .read_to_string(&mut opf)
            .expect("entry is UTF-8");
        opf
    }

    #[test]
    fn empty_language_defaults_to_en_in_the_opf() {
        let opf = opf_for("", None);
        assert!(
            opf.contains("<dc:language>en</dc:language>"),
            "an empty language must default to en: {opf}"
        );
    }

    #[test]
    fn whitespace_only_language_defaults_to_en_in_the_opf() {
        // The guard trims before testing for emptiness, so a whitespace-only
        // language must default to "en" the same as an empty one.
        let opf = opf_for("   ", None);
        assert!(
            opf.contains("<dc:language>en</dc:language>"),
            "a whitespace-only language must default to en: {opf}"
        );
    }

    #[test]
    fn stamp_template_matches_the_probe_constants() {
        // askama templates cannot reference Rust consts, so this pins the
        // template against `stamp::STAMP_PROPERTY`/`STAMP_PREFIX_IRI` - if
        // either side drifts, the probe stops finding what the writer wrote.
        let stamped = opf_for("en", Some("x4 1.2.3"));
        assert!(
            stamped.contains(crate::epub::stamp::STAMP_PROPERTY),
            "got: {stamped}"
        );
        assert!(
            stamped.contains(crate::epub::stamp::STAMP_PREFIX_IRI),
            "got: {stamped}"
        );
        assert!(stamped.contains(">x4 1.2.3</meta>"), "got: {stamped}");

        let unstamped = opf_for("en", None);
        assert!(!unstamped.contains("prefix="), "got: {unstamped}");
        assert!(!unstamped.contains("tailor:fitted"), "got: {unstamped}");
    }

    #[test]
    fn profile_template_matches_the_probe_constant() {
        let profiled = opf_for_stamped("en", Some("x4 1.2.3"), Some("x4"));
        assert!(
            profiled.contains(crate::epub::stamp::PROFILE_PROPERTY),
            "got: {profiled}"
        );
        assert!(
            profiled.contains(r#"<meta property="tailor:profile">x4</meta>"#),
            "got: {profiled}"
        );

        // A profile is only provenance detail on the stamp: without a stamp it
        // must not appear, keeping unstamped output byte-identical.
        let profile_only = opf_for_stamped("en", None, Some("x4"));
        assert!(
            !profile_only.contains("tailor:profile"),
            "got: {profile_only}"
        );
        assert!(
            !profile_only.contains("tailor:fitted"),
            "got: {profile_only}"
        );
        assert!(!profile_only.contains("prefix="), "got: {profile_only}");
    }

    #[test]
    fn now_utc_iso8601_has_expected_shape() {
        let ts = now_utc_iso8601();
        assert_eq!(ts.len(), 20, "expected CCYY-MM-DDThh:mm:ssZ, got {ts}");
        assert!(ts.ends_with('Z'));
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[10..11], "T");
    }
}
