//! A structural lint against the CrossPoint-supported EPUB subset.
//!
//! Unlike [`crate::convert`], [`lint_epub`] never mutates or fixes anything -
//! it inspects an arbitrary `.epub` (one this tool produced, or any other) and
//! reports every way it falls outside what the device can actually read, so
//! the `check` CLI subcommand can validate a book without converting it, and
//! [`crate::convert`] can sanity-check its own output in debug builds.
//!
//! Every check below works directly off the raw zip bytes and a purpose-built,
//! tolerant reparse of the OPF/nav/NCX - not [`crate::epub::read::read_epub`],
//! which is a poor fit here: it bails on the first structural problem (exactly
//! wrong for a tool whose entire point is to enumerate every problem), and it
//! builds far more of the [`crate::epub::Book`] model than a lint needs. Where
//! the logic really is identical either way (DRM/ZIP64 classification, reading
//! every zip entry, finding the OPF path, which text resources must be UTF-8),
//! it is factored out of `read.rs` and reused rather than copied.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Seek};

use indexmap::IndexMap;
use roxmltree::{Document, Node, ParsingOptions};
use serde::Serialize;
use unicode_normalization::is_nfc;
use zip::{CompressionMethod, ZipArchive};

use crate::epub::model::normalize_href;
use crate::epub::read::{
    ArchiveEntries, EncryptionClass, EntryCollision, TEXT_MEDIA_TYPES, classify_encryption_xml,
    parse_container, read_all_entries, zip64_reason,
};
use crate::html::dom::get_attr;
use crate::html::parse_xhtml;
use crate::html::serialize::is_ncname;
use crate::image::sniff_raster_kind;
use crate::is_font;
use crate::profile::{DeviceCaps, Features};

/// How serious a [`LintFinding`] is. `Error` fails the `check` CLI's exit code
/// (and trips [`crate::convert`]'s debug-only self-check); `Warning` and
/// `Info` are advisory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// One structural problem (or note) [`lint_epub`] found.
#[derive(Debug, Clone, Serialize)]
pub struct LintFinding {
    pub severity: Severity,
    /// A stable, machine-matchable identifier for the kind of check that
    /// produced this finding (e.g. `"image-format"`), never the message text.
    pub code: &'static str,
    pub message: String,
    /// The zip-absolute path the finding is about, when it is about one
    /// specific resource.
    pub path: Option<String>,
}

impl LintFinding {
    fn new(severity: Severity, code: &'static str, message: String, path: Option<String>) -> Self {
        LintFinding {
            severity,
            code,
            message,
            path,
        }
    }

    fn error(code: &'static str, message: String, path: Option<String>) -> Self {
        Self::new(Severity::Error, code, message, path)
    }

    fn warning(code: &'static str, message: String, path: Option<String>) -> Self {
        Self::new(Severity::Warning, code, message, path)
    }
}

/// Lint an `.epub` archive's raw `bytes`, returning every finding (possibly
/// empty). Structural checks (archive shape, OPF/manifest sync, encoding,
/// DRM) always run; device checks (fonts, CSS caps, image formats/budgets)
/// run only for the transforms `features` enables, so a repair-only profile
/// gets a pure structural check. Never panics: a bad enough archive (not a
/// zip, missing `META-INF/container.xml`, unparsable OPF) simply stops early
/// with the findings collected so far, always including at least one `Error`.
pub fn lint_epub(bytes: &[u8], caps: &DeviceCaps, features: &Features) -> Vec<LintFinding> {
    let mut findings = Vec::new();

    let mut archive = match ZipArchive::new(Cursor::new(bytes)) {
        Ok(archive) => archive,
        Err(e) => {
            findings.push(LintFinding::error(
                "unreadable",
                format!("not a valid ZIP archive: {e}"),
                None,
            ));
            return findings;
        }
    };

    check_mimetype_first(&mut archive, &mut findings);
    check_drm(&mut archive, &mut findings);
    check_zip64(&mut archive, &mut findings);

    let ArchiveEntries {
        entries,
        collisions,
    } = match read_all_entries(&mut archive) {
        Ok(archive_entries) => archive_entries,
        Err(e) => {
            findings.push(LintFinding::error(
                "unreadable",
                format!("could not read zip entries: {e}"),
                None,
            ));
            return findings;
        }
    };
    for EntryCollision {
        kept_raw,
        dropped_raw,
        key,
    } in &collisions
    {
        findings.push(LintFinding::warning(
            "entry-collision",
            format!(
                "zip entries '{dropped_raw}' and '{kept_raw}' normalize to the same path '{key}' - kept the first"
            ),
            Some(key.clone()),
        ));
    }

    let Some(opf_path) = find_opf_path(&entries, &mut findings) else {
        return findings;
    };
    let Some(opf_data) = entries.get(&opf_path) else {
        findings.push(LintFinding::error(
            "manifest-sync",
            format!("OPF referenced at '{opf_path}' not found in archive"),
            Some(opf_path),
        ));
        return findings;
    };
    let opf_text = String::from_utf8_lossy(opf_data).into_owned();
    let Ok(opf_doc) = parse_xml(&opf_text) else {
        findings.push(LintFinding::error(
            "manifest-sync",
            format!("{opf_path} is not well-formed XML"),
            Some(opf_path),
        ));
        return findings;
    };

    let opf_dir = parent_dir(&opf_path);
    let opf = parse_opf_lint(&opf_doc, &opf_dir);

    check_spine_readable(&opf, &opf_path, &mut findings);
    check_manifest_sync(&entries, &opf, &opf_path, &mut findings);
    check_spine_toc_sync(&entries, &opf, &mut findings);
    check_content_wellformed(&entries, &opf, &mut findings);
    check_duplicate_ids(&entries, &opf, &mut findings);
    if features.transcode_images || features.rasterize_svg {
        check_image_format(&entries, &opf, &opf_path, caps, features, &mut findings);
    }
    if features.filter_css {
        check_css_caps(&entries, &opf, &opf_path, caps, &mut findings);
    }
    check_encoding(&entries, &opf, &opf_path, &mut findings);
    if features.strip_fonts {
        check_fonts(&entries, &opf, &mut findings);
    }

    findings
}

// ---------------------------------------------------------------------
// content-wellformed
// ---------------------------------------------------------------------

/// Scan XML `text` for element or attribute names that are not namespace-valid
/// QNames (`NCName` or `NCName:NCName`), returning the first offender and its
/// 1-based line.
///
/// roxmltree only enforces XML 1.0 well-formedness, where a name may contain
/// any number of colons - it even swallows `:xmlns` whole, as a namespace
/// declaration. The namespace-aware parsers inside actual EPUB readers reject
/// such names outright; libxml2 reports `Failed to parse QName ':xmlns'`,
/// which is exactly how the 0.4.0/0.4.1 corruption surfaced. This scan closes
/// that gap. Only call it on text roxmltree has already accepted:
/// well-formedness (no stray `<` in text or attribute values, quoted values,
/// terminated comments) is what keeps the scanner this simple.
pub fn find_invalid_qname(text: &str) -> Option<(String, usize)> {
    let mut offset = 0;
    while let Some(lt) = text[offset..].find('<') {
        offset += lt;
        let rest = &text[offset..];
        let skipped = if rest.starts_with("<!--") {
            rest.find("-->").map_or(rest.len(), |p| p + 3)
        } else if rest.starts_with("<![CDATA[") {
            rest.find("]]>").map_or(rest.len(), |p| p + 3)
        } else if rest.starts_with("<?") {
            rest.find("?>").map_or(rest.len(), |p| p + 2)
        } else if rest.starts_with("<!") {
            skip_doctype(rest)
        } else {
            let (len, bad) = scan_tag_names(rest);
            if let Some(name) = bad {
                let line = text[..offset].matches('\n').count() + 1;
                return Some((name, line));
            }
            len
        };
        offset += skipped.max(1).min(rest.len());
        if offset >= text.len() {
            break;
        }
    }
    None
}

/// Byte length of a `<!DOCTYPE ...>` at the start of `text`, stepping over an
/// internal subset (`[...]`, which may itself contain `<`/`>` markup).
fn skip_doctype(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut in_subset = false;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => in_subset = true,
            b']' => in_subset = false,
            b'>' if !in_subset => return i + 1,
            _ => {}
        }
        i += 1;
    }
    text.len()
}

/// Validate the names inside one start or end tag beginning at `tag` (which
/// starts with `<`). Returns the tag's byte length (through its closing `>`)
/// and the first invalid name found in it. Quoted attribute values are
/// skipped whole, since they may legally contain `>`.
fn scan_tag_names(tag: &str) -> (usize, Option<String>) {
    fn is_qname(name: &str) -> bool {
        match name.split_once(':') {
            None => is_ncname(name),
            Some((prefix, local)) => is_ncname(prefix) && is_ncname(local),
        }
    }
    let bytes = tag.as_bytes();
    let len = bytes.len();
    let mut i = 1; // past '<'
    if bytes.get(i) == Some(&b'/') {
        i += 1;
    }
    let name_start = i;
    while i < len && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>') {
        i += 1;
    }
    if !is_qname(&tag[name_start..i]) {
        return (i, Some(tag[name_start..i].to_string()));
    }
    loop {
        while i < len && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'/') {
            i += 1;
        }
        if i >= len || bytes[i] == b'>' {
            return ((i + 1).min(len), None);
        }
        let attr_start = i;
        while i < len && !matches!(bytes[i], b'=' | b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/') {
            i += 1;
        }
        if !is_qname(&tag[attr_start..i]) {
            return (i, Some(tag[attr_start..i].to_string()));
        }
        while i < len && !matches!(bytes[i], b'"' | b'\'' | b'>') {
            i += 1;
        }
        if i < len && (bytes[i] == b'"' || bytes[i] == b'\'') {
            let quote = bytes[i];
            i += 1;
            while i < len && bytes[i] != quote {
                i += 1;
            }
            i += 1;
        }
    }
}

/// Every XHTML content document must be well-formed XML: EPUB readers parse
/// them strictly, so one malformed document breaks the book on the device
/// even when everything else is fine. This is also the check that spots books
/// damaged by epub-tailor 0.4.0/0.4.1, whose serializer could write a
/// malformed `:xmlns` attribute name (readers report it as `Failed to parse
/// QName ':xmlns'`). Note the deliberate contrast with the rest of this
/// module: content documents are otherwise inspected with the lenient HTML
/// parser, which swallows exactly this class of damage.
fn check_content_wellformed(
    entries: &IndexMap<String, Vec<u8>>,
    opf: &OpfLint,
    findings: &mut Vec<LintFinding>,
) {
    let mut seen: HashSet<&str> = HashSet::new();
    for item in opf.manifest.values() {
        if item.media_type != "application/xhtml+xml" && item.media_type != "text/html" {
            continue;
        }
        if !seen.insert(item.href.as_str()) {
            continue;
        }
        // A missing entry is check_manifest_sync's finding, not ours.
        let Some(data) = entries.get(&item.href) else {
            continue;
        };
        let text = String::from_utf8_lossy(data);
        let detail = match parse_xml(&text) {
            Err(e) => Some(e.to_string()),
            Ok(_) => find_invalid_qname(&text)
                .map(|(name, line)| format!("name '{name}' on line {line} is not a valid QName")),
        };
        if let Some(detail) = detail {
            findings.push(LintFinding::error(
                "content-wellformed",
                format!(
                    "{} is not well-formed XML ({detail}); strict readers cannot open it",
                    item.href
                ),
                Some(item.href.clone()),
            ));
        }
    }
}

// ---------------------------------------------------------------------
// mimetype-first
// ---------------------------------------------------------------------

fn check_mimetype_first<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    findings: &mut Vec<LintFinding>,
) {
    if archive.is_empty() {
        findings.push(LintFinding::error(
            "mimetype-first",
            "archive has no entries".to_string(),
            None,
        ));
        return;
    }
    let mut file = match archive.by_index(0) {
        Ok(file) => file,
        Err(e) => {
            findings.push(LintFinding::error(
                "mimetype-first",
                format!("could not read the first zip entry: {e}"),
                None,
            ));
            return;
        }
    };
    if file.name() != "mimetype" {
        findings.push(LintFinding::error(
            "mimetype-first",
            format!("the first zip entry is '{}', not 'mimetype'", file.name()),
            None,
        ));
        return;
    }
    if file.compression() != CompressionMethod::Stored {
        findings.push(LintFinding::error(
            "mimetype-first",
            "the 'mimetype' entry is compressed; it must be stored uncompressed".to_string(),
            Some("mimetype".to_string()),
        ));
    }
    let mut data = Vec::new();
    if file.read_to_end(&mut data).is_err() {
        findings.push(LintFinding::error(
            "mimetype-first",
            "could not read the 'mimetype' entry's content".to_string(),
            Some("mimetype".to_string()),
        ));
        return;
    }
    if data != b"application/epub+zip" {
        findings.push(LintFinding::error(
            "mimetype-first",
            "the 'mimetype' entry's content is not exactly 'application/epub+zip'".to_string(),
            Some("mimetype".to_string()),
        ));
    }
}

// ---------------------------------------------------------------------
// drm
// ---------------------------------------------------------------------

fn check_drm<R: Read + Seek>(archive: &mut ZipArchive<R>, findings: &mut Vec<LintFinding>) {
    let mut data = Vec::new();
    match archive.by_name("META-INF/encryption.xml") {
        Ok(mut file) => {
            if file.read_to_end(&mut data).is_err() {
                findings.push(LintFinding::error(
                    "drm",
                    "could not read META-INF/encryption.xml".to_string(),
                    Some("META-INF/encryption.xml".to_string()),
                ));
                return;
            }
        }
        Err(_) => return,
    }

    let text = String::from_utf8_lossy(&data);
    match classify_encryption_xml(&text) {
        Ok(EncryptionClass::Drm) => findings.push(LintFinding::error(
            "drm",
            "META-INF/encryption.xml declares real encryption; encrypted content cannot \
             be read or transformed"
                .to_string(),
            Some("META-INF/encryption.xml".to_string()),
        )),
        Ok(EncryptionClass::FontObfuscationOnly) => findings.push(LintFinding::new(
            Severity::Info,
            "drm",
            "META-INF/encryption.xml uses font obfuscation only, not DRM; safe to process"
                .to_string(),
            Some("META-INF/encryption.xml".to_string()),
        )),
        Err(_) => findings.push(LintFinding::error(
            "drm",
            "META-INF/encryption.xml is not well-formed XML".to_string(),
            Some("META-INF/encryption.xml".to_string()),
        )),
    }
}

// ---------------------------------------------------------------------
// zip64
// ---------------------------------------------------------------------

fn check_zip64<R: Read + Seek>(archive: &mut ZipArchive<R>, findings: &mut Vec<LintFinding>) {
    match zip64_reason(archive) {
        Ok(Some(reason)) => findings.push(LintFinding::error(
            "zip64",
            format!(
                "archive requires ZIP64 ({reason}); many e-readers cannot open ZIP64 \
                 archives"
            ),
            None,
        )),
        Ok(None) => {}
        Err(e) => findings.push(LintFinding::error("zip64", format!("{e}"), None)),
    }
}

// ---------------------------------------------------------------------
// A minimal, tolerant OPF parse: just what the remaining checks need.
// ---------------------------------------------------------------------

struct ManifestItem {
    href: String,
    media_type: String,
    properties: Vec<String>,
}

struct OpfLint {
    /// Manifest id -> item.
    manifest: IndexMap<String, ManifestItem>,
    spine_idrefs: Vec<String>,
    /// The number of spine itemrefs whose `linear` attribute is NOT `"no"`
    /// (i.e. that the reader actually keeps). See [`check_spine_readable`].
    spine_linear_count: usize,
    nav_href: Option<String>,
    ncx_href: Option<String>,
    cover_href: Option<String>,
}

fn find_opf_path(
    entries: &IndexMap<String, Vec<u8>>,
    findings: &mut Vec<LintFinding>,
) -> Option<String> {
    let Some(container) = entries.get("META-INF/container.xml") else {
        findings.push(LintFinding::error(
            "manifest-sync",
            "missing META-INF/container.xml".to_string(),
            None,
        ));
        return None;
    };
    let text = String::from_utf8_lossy(container).into_owned();
    match parse_container(&text) {
        Ok(path) => Some(path),
        Err(e) => {
            findings.push(LintFinding::error("manifest-sync", format!("{e}"), None));
            None
        }
    }
}

/// Parse an XML document, tolerating a `<!DOCTYPE ...>` (real-world EPUB nav
/// documents and NCX files routinely carry one - our own generated nav.xhtml
/// among them - which `roxmltree::Document::parse`'s default options reject
/// outright).
fn parse_xml(text: &str) -> Result<Document<'_>, roxmltree::Error> {
    Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
}

fn child_named<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

fn parse_opf_lint(doc: &Document, opf_dir: &str) -> OpfLint {
    let package = doc.root_element();
    let mut manifest: IndexMap<String, ManifestItem> = IndexMap::new();
    if let Some(manifest_node) = child_named(package, "manifest") {
        for item in manifest_node
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "item")
        {
            let (Some(id), Some(href_raw)) = (item.attribute("id"), item.attribute("href")) else {
                continue;
            };
            let href = normalize_href(opf_dir, href_raw);
            let media_type = item.attribute("media-type").unwrap_or("").to_string();
            let properties = item
                .attribute("properties")
                .map(|p| p.split_whitespace().map(String::from).collect())
                .unwrap_or_default();
            manifest.insert(
                id.to_string(),
                ManifestItem {
                    href,
                    media_type,
                    properties,
                },
            );
        }
    }

    let mut spine_idrefs = Vec::new();
    let mut spine_linear_count = 0usize;
    let mut spine_toc_attr = None;
    if let Some(spine_node) = child_named(package, "spine") {
        spine_toc_attr = spine_node.attribute("toc").map(String::from);
        for itemref in spine_node
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "itemref")
        {
            if itemref.attribute("linear") != Some("no") {
                spine_linear_count += 1;
            }
            if let Some(idref) = itemref.attribute("idref") {
                spine_idrefs.push(idref.to_string());
            }
        }
    }

    let nav_href = manifest
        .values()
        .find(|item| item.properties.iter().any(|p| p == "nav"))
        .map(|item| item.href.clone());
    let mut ncx_href = manifest
        .values()
        .find(|item| item.media_type == "application/x-dtbncx+xml")
        .map(|item| item.href.clone());
    if ncx_href.is_none()
        && let Some(toc_id) = &spine_toc_attr
    {
        ncx_href = manifest.get(toc_id).map(|item| item.href.clone());
    }

    let mut cover_href = manifest
        .values()
        .find(|item| item.properties.iter().any(|p| p == "cover-image"))
        .map(|item| item.href.clone());
    if cover_href.is_none()
        && let Some(metadata_node) = child_named(package, "metadata")
        && let Some(id) = metadata_node
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "meta")
            .find(|n| n.attribute("name") == Some("cover"))
            .and_then(|n| n.attribute("content"))
    {
        cover_href = manifest.get(id).map(|item| item.href.clone());
    }

    OpfLint {
        manifest,
        spine_idrefs,
        spine_linear_count,
        nav_href,
        ncx_href,
        cover_href,
    }
}

/// The manifest-declared media type of `name`, if any (the OPF's own path is
/// treated as `application/oebps-package+xml`, since it is never listed in
/// its own manifest).
fn declared_media_type<'a>(opf: &'a OpfLint, opf_path: &str, name: &str) -> Option<&'a str> {
    if name == opf_path {
        return Some("application/oebps-package+xml");
    }
    opf.manifest
        .values()
        .find(|item| item.href == name)
        .map(|item| item.media_type.as_str())
}

// ---------------------------------------------------------------------
// spine-empty
// ---------------------------------------------------------------------

/// A book whose spine has no usable itemrefs (none at all, or every one lacking
/// an `idref`), or whose itemrefs are ALL `linear="no"`, shows nothing on the
/// device: [`crate::epub::read::read_epub`] skips every `linear="no"` itemref
/// (with its own per-item warning), so either case leaves the reader's
/// `book.spine` empty.
fn check_spine_readable(opf: &OpfLint, opf_path: &str, findings: &mut Vec<LintFinding>) {
    if opf.spine_idrefs.is_empty() {
        findings.push(LintFinding::error(
            "spine-empty",
            "the spine has no usable itemrefs; the book has no readable content".to_string(),
            Some(opf_path.to_string()),
        ));
    } else if opf.spine_linear_count == 0 {
        findings.push(LintFinding::error(
            "spine-empty",
            "every spine itemref is linear=\"no\"; the reader skips them all and the book \
             appears empty"
                .to_string(),
            Some(opf_path.to_string()),
        ));
    }
}

// ---------------------------------------------------------------------
// manifest-sync
// ---------------------------------------------------------------------

fn check_manifest_sync(
    entries: &IndexMap<String, Vec<u8>>,
    opf: &OpfLint,
    opf_path: &str,
    findings: &mut Vec<LintFinding>,
) {
    for (id, item) in &opf.manifest {
        if !entries.contains_key(&item.href) {
            findings.push(LintFinding::error(
                "manifest-sync",
                format!(
                    "manifest item '{id}' references '{}', which is not in the archive",
                    item.href
                ),
                Some(item.href.clone()),
            ));
        }
    }
    let manifest_hrefs: HashSet<&str> = opf.manifest.values().map(|i| i.href.as_str()).collect();
    for name in entries.keys() {
        if name == "mimetype" || name.starts_with("META-INF/") || name == opf_path {
            continue;
        }
        if !manifest_hrefs.contains(name.as_str()) {
            findings.push(LintFinding::error(
                "manifest-sync",
                format!("'{name}' is in the archive but not listed in the manifest"),
                Some(name.clone()),
            ));
        }
    }
}

// ---------------------------------------------------------------------
// spine-toc-sync
// ---------------------------------------------------------------------

fn check_spine_toc_sync(
    entries: &IndexMap<String, Vec<u8>>,
    opf: &OpfLint,
    findings: &mut Vec<LintFinding>,
) {
    for idref in &opf.spine_idrefs {
        if !opf.manifest.contains_key(idref) {
            findings.push(LintFinding::error(
                "spine-toc-sync",
                format!("spine itemref references unknown manifest id '{idref}'"),
                None,
            ));
        }
    }

    let spine_docs: HashSet<&str> = opf
        .spine_idrefs
        .iter()
        .filter_map(|idref| opf.manifest.get(idref))
        .map(|item| item.href.as_str())
        .collect();

    // Fragment-existence lookups are memoized per target document path: a
    // document with many incoming TOC/nav fragment refs (a long nav, or an
    // NCX with many navPoints into the same chapter) would otherwise be
    // re-parsed once per ref.
    let mut id_cache: HashMap<String, HashSet<String>> = HashMap::new();

    if let Some(nav_href) = &opf.nav_href
        && let Some(data) = entries.get(nav_href)
    {
        let nav_dir = parent_dir(nav_href);
        let text = String::from_utf8_lossy(data);
        match parse_xml(&text) {
            Ok(doc) => {
                for a in doc
                    .descendants()
                    .filter(|n| n.is_element() && n.tag_name().name() == "a")
                {
                    if let Some(href) = a.attribute("href") {
                        check_toc_target(
                            href,
                            &nav_dir,
                            &spine_docs,
                            entries,
                            nav_href,
                            &mut id_cache,
                            findings,
                        );
                    }
                }
            }
            Err(_) => findings.push(LintFinding::error(
                "spine-toc-sync",
                format!("{nav_href} is not well-formed XML"),
                Some(nav_href.clone()),
            )),
        }
    }

    if let Some(ncx_href) = &opf.ncx_href
        && let Some(data) = entries.get(ncx_href)
    {
        let ncx_dir = parent_dir(ncx_href);
        let text = String::from_utf8_lossy(data);
        match parse_xml(&text) {
            Ok(doc) => {
                for content in doc
                    .descendants()
                    .filter(|n| n.is_element() && n.tag_name().name() == "content")
                {
                    if let Some(src) = content.attribute("src") {
                        check_toc_target(
                            src,
                            &ncx_dir,
                            &spine_docs,
                            entries,
                            ncx_href,
                            &mut id_cache,
                            findings,
                        );
                    }
                }
            }
            Err(_) => findings.push(LintFinding::error(
                "spine-toc-sync",
                format!("{ncx_href} is not well-formed XML"),
                Some(ncx_href.clone()),
            )),
        }
    }
}

fn check_toc_target(
    href_raw: &str,
    base_dir: &str,
    spine_docs: &HashSet<&str>,
    entries: &IndexMap<String, Vec<u8>>,
    toc_path: &str,
    id_cache: &mut HashMap<String, HashSet<String>>,
    findings: &mut Vec<LintFinding>,
) {
    let resolved = normalize_href(base_dir, href_raw);
    let (path, fragment) = match resolved.split_once('#') {
        Some((p, f)) => (p.to_string(), Some(f.to_string())),
        None => (resolved.clone(), None),
    };
    if !spine_docs.contains(path.as_str()) {
        findings.push(LintFinding::error(
            "spine-toc-sync",
            format!("{toc_path} target '{href_raw}' does not resolve to a spine document"),
            Some(toc_path.to_string()),
        ));
        return;
    }
    let Some(frag) = fragment else { return };
    let Some(data) = entries.get(&path) else {
        return;
    };
    let ids = id_cache
        .entry(path.clone())
        .or_insert_with(|| document_id_set(data));
    if !ids.contains(&frag) {
        findings.push(LintFinding::error(
            "spine-toc-sync",
            format!(
                "{toc_path} target '{href_raw}' references id '{frag}', which does not exist \
                 in {path}"
            ),
            Some(toc_path.to_string()),
        ));
    }
}

/// Every `id` in `data`'s document, or an empty set if it does not parse -
/// preserving the pre-memoization behavior where an unparsable target document
/// makes every fragment reference into it a finding.
fn document_id_set(data: &[u8]) -> HashSet<String> {
    let Ok(doc) = parse_xhtml(data) else {
        return HashSet::new();
    };
    doc.inclusive_descendants()
        .filter_map(|n| get_attr(&n, "id"))
        .collect()
}

// ---------------------------------------------------------------------
// duplicate-id
// ---------------------------------------------------------------------

/// Every id that appears on more than one element in a spine document is an
/// `epubcheck` `RSC-005` ("Duplicate ID") error. This is exactly what a
/// naive parse of Gutenberg-style `<a id="..."/>` source produces: the HTML5
/// tree builder does not treat `<a .../>` as self-closing, so the anchor
/// stays open and gets cloned - `id` and all - across following block
/// boundaries (see [`crate::html::transform_chapter`]'s dedupe pass, which
/// fixes this in our own output). Only spine documents are checked, matching
/// what an EPUB reader actually renders.
fn check_duplicate_ids(
    entries: &IndexMap<String, Vec<u8>>,
    opf: &OpfLint,
    findings: &mut Vec<LintFinding>,
) {
    let spine_docs: HashSet<&str> = opf
        .spine_idrefs
        .iter()
        .filter_map(|idref| opf.manifest.get(idref))
        .map(|item| item.href.as_str())
        .collect();

    for name in spine_docs {
        let Some(data) = entries.get(name) else {
            continue;
        };
        let Ok(doc) = parse_xhtml(data) else {
            continue;
        };
        let mut seen: HashSet<String> = HashSet::new();
        let mut duplicated: Vec<String> = Vec::new();
        for node in doc.inclusive_descendants() {
            let Some(id) = get_attr(&node, "id") else {
                continue;
            };
            if !seen.insert(id.clone()) && !duplicated.contains(&id) {
                duplicated.push(id);
            }
        }
        for id in duplicated {
            findings.push(LintFinding::error(
                "duplicate-id",
                format!("id '{id}' appears on more than one element in {name}"),
                Some(name.to_string()),
            ));
        }
    }
}

// ---------------------------------------------------------------------
// image-format
// ---------------------------------------------------------------------

fn check_image_format(
    entries: &IndexMap<String, Vec<u8>>,
    opf: &OpfLint,
    opf_path: &str,
    profile: &DeviceCaps,
    features: &Features,
    findings: &mut Vec<LintFinding>,
) {
    for (name, data) in entries {
        if name == "mimetype" || name.starts_with("META-INF/") {
            continue;
        }
        let declared = declared_media_type(opf, opf_path, name);
        if !looks_like_image(declared, data) {
            continue;
        }
        check_one_image(name, data, opf, profile, features, declared, findings);
    }
}

fn looks_like_image(declared: Option<&str>, data: &[u8]) -> bool {
    if declared.is_some_and(|m| m.starts_with("image/")) {
        return true;
    }
    sniff_raster_kind(data).is_some() || looks_like_svg(data)
}

/// Whether `data` is an SVG *document*, not merely something that mentions
/// `<svg` somewhere in its first bytes: skipping any leading BOM, XML
/// declaration, DOCTYPE and comments, its content must open with a root
/// `<svg` element. A naive substring search would mislabel an XHTML chapter
/// (root `<html>`) that happens to carry an early inline `<svg>` - e.g. a
/// chart right after the opening `<body>` - as an SVG image resource.
fn looks_like_svg(data: &[u8]) -> bool {
    let head = &data[..data.len().min(4096)];
    let text = String::from_utf8_lossy(head);
    let mut rest = text.trim_start_matches('\u{feff}').trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("<?") {
            let Some(end) = after.find("?>") else {
                return false;
            };
            rest = after[end + 2..].trim_start();
        } else if let Some(after) = rest.strip_prefix("<!--") {
            let Some(end) = after.find("-->") else {
                return false;
            };
            rest = after[end + 3..].trim_start();
        } else if let Some(after) = rest.strip_prefix("<!") {
            let Some(end) = after.find('>') else {
                return false;
            };
            rest = after[end + 1..].trim_start();
        } else {
            break;
        }
    }
    let Some(after_tag) = rest.strip_prefix("<svg") else {
        return false;
    };
    after_tag
        .chars()
        .next()
        .is_none_or(|c| c.is_whitespace() || c == '>' || c == '/')
}

fn check_one_image(
    name: &str,
    data: &[u8],
    opf: &OpfLint,
    profile: &DeviceCaps,
    features: &Features,
    declared: Option<&str>,
    findings: &mut Vec<LintFinding>,
) {
    let Some((ext, _)) = sniff_raster_kind(data) else {
        if features.rasterize_svg && (declared == Some("image/svg+xml") || looks_like_svg(data)) {
            findings.push(LintFinding::error(
                "image-format",
                format!("{name} is SVG, which the device cannot decode"),
                Some(name.to_string()),
            ));
        }
        return;
    };
    if !features.transcode_images {
        return;
    }

    match ext {
        "jpg" => {
            if jpeg_is_progressive(data) {
                findings.push(LintFinding::error(
                    "image-format",
                    format!(
                        "{name} is a progressive JPEG, which the device cannot decode \
                         (baseline only)"
                    ),
                    Some(name.to_string()),
                ));
            }
        }
        "png" => {}
        other => {
            findings.push(LintFinding::error(
                "image-format",
                format!(
                    "{name} is {}, which the device cannot decode (baseline JPEG or PNG only)",
                    other.to_ascii_uppercase()
                ),
                Some(name.to_string()),
            ));
        }
    }

    if let Some((w, h)) = image_dimensions(data)
        && (w > profile.max_src_px.0 || h > profile.max_src_px.1)
    {
        findings.push(LintFinding::error(
            "image-format",
            format!(
                "{name} is {w}x{h}, over the device's {}x{} decode cap",
                profile.max_src_px.0, profile.max_src_px.1
            ),
            Some(name.to_string()),
        ));
    }

    let budget = if opf.cover_href.as_deref() == Some(name) {
        profile.cover_budget_bytes
    } else {
        profile.inline_budget_bytes
    };
    if data.len() > budget {
        findings.push(LintFinding::warning(
            "image-format",
            format!(
                "{name} is {}KB, over the {}KB byte budget",
                kb(data.len()),
                kb(budget)
            ),
            Some(name.to_string()),
        ));
    }
}

/// Whether a JPEG's marker structure declares a progressive coding process.
///
/// Walks the marker segments from SOI rather than scanning for `FF C2` bytes:
/// APPn/DQT/COM segments (ICC profiles, EXIF thumbnails, ...) are
/// length-prefixed and may legitimately contain those bytes, so a whole-file
/// byte scan would flag valid baseline files. Each length-prefixed segment is
/// skipped by its declared big-endian length; standalone markers (TEM, RSTn)
/// carry none; the walk stops at SOS (everything after it is entropy-coded
/// scan data, irrelevant to the frame header) or EOI.
///
/// Progressive iff an actual SOF marker for a progressive process appears:
/// SOF2 (the huffman progressive baseline every encoder emits) plus the rare
/// SOF6/SOF10/SOF14 progressive variants (differential/arithmetic), included
/// for completeness since the device decodes none of them. Malformed
/// structure (no SOI, truncated length, a non-marker byte where a marker
/// belongs) is treated as not progressive - the decode-based checks own real
/// garbage, and a structural guess must never produce a false Error.
fn jpeg_is_progressive(data: &[u8]) -> bool {
    if !data.starts_with(&[0xFF, 0xD8]) {
        return false;
    }
    let mut i = 2;
    loop {
        // A marker is 0xFF (plus any number of legal 0xFF fill bytes)
        // followed by its code byte.
        if data.get(i) != Some(&0xFF) {
            return false;
        }
        while data.get(i) == Some(&0xFF) {
            i += 1;
        }
        let Some(&code) = data.get(i) else {
            return false;
        };
        i += 1;
        match code {
            // SOF2/SOF6/SOF10/SOF14: progressive coding processes.
            0xC2 | 0xC6 | 0xCA | 0xCE => return true,
            // SOS (scan data follows) or EOI: no progressive frame header.
            0xDA | 0xD9 => return false,
            // Standalone markers with no length field: TEM, RSTn.
            0x01 | 0xD0..=0xD7 => {}
            // FF 00 is a stuffed byte, never a marker; seeing it in the
            // header section means the structure is malformed.
            0x00 => return false,
            // Every other marker carries a big-endian length that counts its
            // own two bytes.
            _ => {
                let (Some(&hi), Some(&lo)) = (data.get(i), data.get(i + 1)) else {
                    return false;
                };
                let len = u16::from_be_bytes([hi, lo]) as usize;
                if len < 2 {
                    return false;
                }
                i += len;
            }
        }
    }
}

/// Decode just the pixel dimensions of a sniffed raster, without decoding the
/// whole image.
fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

// ---------------------------------------------------------------------
// css-caps
// ---------------------------------------------------------------------

fn check_css_caps(
    entries: &IndexMap<String, Vec<u8>>,
    opf: &OpfLint,
    opf_path: &str,
    profile: &DeviceCaps,
    findings: &mut Vec<LintFinding>,
) {
    let mut total_rules = 0usize;
    for (name, data) in entries {
        let declared = declared_media_type(opf, opf_path, name);
        let is_css = declared == Some("text/css") || name.ends_with(".css");
        if !is_css {
            continue;
        }
        if data.len() > profile.css_max_bytes {
            findings.push(LintFinding::error(
                "css-caps",
                format!(
                    "{name} is {}KB, over the device's {}KB per-file CSS cap",
                    kb(data.len()),
                    kb(profile.css_max_bytes)
                ),
                Some(name.clone()),
            ));
        }
        total_rules += String::from_utf8_lossy(data).matches('}').count();
    }
    if total_rules > profile.css_max_rules {
        findings.push(LintFinding::warning(
            "css-caps",
            format!(
                "the book has {total_rules} CSS rules, over the device's {} cap; the device \
                 will drop the rules past the cap",
                profile.css_max_rules
            ),
            None,
        ));
    }
}

// ---------------------------------------------------------------------
// encoding
// ---------------------------------------------------------------------

fn check_encoding(
    entries: &IndexMap<String, Vec<u8>>,
    opf: &OpfLint,
    opf_path: &str,
    findings: &mut Vec<LintFinding>,
) {
    for (name, data) in entries {
        if name == "mimetype" {
            continue;
        }
        let declared = declared_media_type(opf, opf_path, name);
        if !looks_like_text_resource(declared, name) {
            continue;
        }
        match std::str::from_utf8(data) {
            Err(_) => findings.push(LintFinding::error(
                "encoding",
                format!("{name} is not valid UTF-8"),
                Some(name.clone()),
            )),
            Ok(text) => {
                if !is_nfc(text) {
                    findings.push(LintFinding::warning(
                        "encoding",
                        format!(
                            "{name} is not Unicode-normalized (NFC); the device does no \
                             normalization"
                        ),
                        Some(name.clone()),
                    ));
                }
            }
        }
    }
}

fn looks_like_text_resource(declared: Option<&str>, name: &str) -> bool {
    match declared {
        Some(mt) if TEXT_MEDIA_TYPES.contains(&mt) => true,
        Some(mt) if !mt.is_empty() => false,
        _ => {
            let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            matches!(ext.as_str(), "xhtml" | "html" | "css" | "ncx" | "opf")
        }
    }
}

// ---------------------------------------------------------------------
// fonts
// ---------------------------------------------------------------------

fn check_fonts(
    entries: &IndexMap<String, Vec<u8>>,
    opf: &OpfLint,
    findings: &mut Vec<LintFinding>,
) {
    for name in entries.keys() {
        if name == "mimetype" || name.starts_with("META-INF/") {
            continue;
        }
        let media_type = opf
            .manifest
            .values()
            .find(|item| item.href == *name)
            .map(|item| item.media_type.as_str())
            .unwrap_or("");
        if is_font(name, media_type) {
            findings.push(LintFinding::warning(
                "fonts",
                format!(
                    "{name} is an embedded font; the device ignores fonts, so it is dead weight"
                ),
                Some(name.clone()),
            ));
        }
    }
}

// ---------------------------------------------------------------------
// Small shared helpers
// ---------------------------------------------------------------------

/// Parent directory of a zip-absolute path (`""` if it has no `/`).
fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(idx) => path[..idx].to_string(),
        None => String::new(),
    }
}

/// Bytes rounded to the nearest kibibyte, for finding messages.
fn kb(bytes: usize) -> usize {
    (bytes + 512) / 1024
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    /// Build a ZIP archive from `entries` (path, raw bytes), in order.
    /// `mimetype` (if present) is written STORED; everything else DEFLATE.
    fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, data) in entries {
            let options = if *name == "mimetype" {
                stored
            } else {
                deflated
            };
            writer.start_file(*name, options).expect("start_file");
            writer.write_all(data).expect("write entry data");
        }
        writer.finish().expect("finish zip").into_inner()
    }

    const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    /// A minimal, clean EPUB3: nav doc, one chapter with two ids, a small CSS
    /// file, no images. Every check should come back clean against this.
    fn minimal_opf(extra_manifest: &str, extra_spine: &str) -> Vec<u8> {
        minimal_opf_with_spine(
            extra_manifest,
            &format!(
                r#"<spine>
    <itemref idref="ch1"/>
    {extra_spine}
  </spine>"#
            ),
        )
    }

    /// Like [`minimal_opf`] but takes the full `<spine>...</spine>` block
    /// verbatim, so a test can exercise an empty spine or an all-`linear="no"`
    /// spine without duplicating the rest of the OPF.
    fn minimal_opf_with_spine(extra_manifest: &str, spine_block: &str) -> Vec<u8> {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Sample</dc:title>
    <dc:language>en</dc:language>
    <dc:identifier id="pub-id">urn:uuid:12345678-1234-1234-1234-123456789012</dc:identifier>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="css" href="styles/main.css" media-type="text/css"/>
    {extra_manifest}
  </manifest>
  {spine_block}
</package>"#
        )
        .into_bytes()
    }

    const NAV_XHTML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol>
<li><a href="text/chapter1.xhtml">Chapter 1</a></li>
<li><a href="text/chapter1.xhtml#s2">Section 2</a></li>
</ol></nav></body></html>"#;

    const CHAPTER1: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body><h1 id="top">Chapter 1</h1><p>Text.</p><h2 id="s2">Section 2</h2><p>More.</p></body></html>"#;

    const MAIN_CSS: &[u8] = b".a{text-align:center}\n";

    fn clean_book(extra_entries: &[(&str, &[u8])]) -> Vec<u8> {
        let opf = minimal_opf("", "");
        let mut entries: Vec<(&str, &[u8])> = vec![
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", MAIN_CSS),
        ];
        entries.extend_from_slice(extra_entries);
        build_zip(&entries)
    }

    fn errors(findings: &[LintFinding]) -> Vec<&LintFinding> {
        findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .collect()
    }

    #[test]
    fn qname_scan_finds_corrupt_colon_xmlns_with_its_line() {
        let text = "<html xmlns=\"http://www.w3.org/1999/xhtml\">\n<body>\n<svg :xmlns=\"http://www.w3.org/2000/svg\"></svg>\n</body></html>";
        assert_eq!(find_invalid_qname(text), Some((":xmlns".to_string(), 3)));
    }

    #[test]
    fn qname_scan_accepts_a_clean_document() {
        let text = r#"<?xml version="1.0"?><!DOCTYPE html><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><p xml:lang="de" epub:type="pagebreak" data-x="1">a &amp; b</p><!-- a comment --></body></html>"#;
        assert_eq!(find_invalid_qname(text), None);
    }

    #[test]
    fn qname_scan_ignores_lookalikes_in_comments_cdata_and_values() {
        let text = "<html><body><!-- <svg :xmlns=\"x\"> --><p title=\"a :xmlns= <b\">t</p><script><![CDATA[ <x :xmlns='y'> ]]></script></body></html>";
        assert_eq!(find_invalid_qname(text), None);
    }

    #[test]
    fn qname_scan_flags_a_double_colon_element_name() {
        assert_eq!(
            find_invalid_qname("<html><body><a:b:c/></body></html>"),
            Some(("a:b:c".to_string(), 1))
        );
    }

    #[test]
    fn qname_scan_steps_over_a_doctype_internal_subset() {
        let text =
            "<!DOCTYPE html [ <!ENTITY nbsp \"&#160;\"> ]>\n<html><body><p>ok</p></body></html>";
        assert_eq!(find_invalid_qname(text), None);
    }

    #[test]
    fn corrupt_content_doc_yields_content_wellformed_error() {
        // The `:xmlns` malformation epub-tailor 0.4.0/0.4.1 wrote: lenient
        // HTML parsing swallows it, so only a strict XML check can flag it.
        const CORRUPT: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>C</title></head>
<body><svg :xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"></svg></body></html>"#;
        let opf = minimal_opf("", "");
        let entries: Vec<(&str, &[u8])> = vec![
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", CORRUPT),
            ("OEBPS/styles/main.css", MAIN_CSS),
        ];
        let bytes = build_zip(&entries);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        let finding = findings
            .iter()
            .find(|f| f.code == "content-wellformed")
            .expect("a content-wellformed finding");
        assert_eq!(finding.severity, Severity::Error);
        assert_eq!(finding.path.as_deref(), Some("OEBPS/text/chapter1.xhtml"));
        assert!(
            finding.message.contains("not well-formed XML"),
            "got: {}",
            finding.message
        );
    }

    #[test]
    fn clean_book_has_zero_error_findings() {
        let bytes = clean_book(&[]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            errors(&findings).is_empty(),
            "expected zero errors, got {:#?}",
            errors(&findings)
        );
    }

    #[test]
    fn mimetype_not_first_is_an_error() {
        let opf = minimal_opf("", "");
        let bytes = build_zip(&[
            ("META-INF/container.xml", CONTAINER_XML),
            ("mimetype", b"application/epub+zip"),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", MAIN_CSS),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings
                .iter()
                .any(|f| f.code == "mimetype-first" && f.severity == Severity::Error),
            "got {findings:#?}"
        );
    }

    #[test]
    fn drm_encryption_xml_is_an_error() {
        const ENC: &[u8] = br#"<?xml version="1.0"?>
<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#">
<EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes256-cbc"/>
</EncryptedData>
</encryption>"#;
        let bytes = clean_book(&[("META-INF/encryption.xml", ENC)]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings
                .iter()
                .any(|f| f.code == "drm" && f.severity == Severity::Error),
            "got {findings:#?}"
        );
    }

    #[test]
    fn font_obfuscation_only_is_info_not_error() {
        const ENC: &[u8] = br#"<?xml version="1.0"?>
<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#">
<EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/>
</EncryptedData>
</encryption>"#;
        let bytes = clean_book(&[("META-INF/encryption.xml", ENC)]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings
                .iter()
                .any(|f| f.code == "drm" && f.severity == Severity::Info),
            "got {findings:#?}"
        );
        assert!(
            !findings
                .iter()
                .any(|f| f.code == "drm" && f.severity == Severity::Error)
        );
    }

    #[test]
    fn manifest_orphan_href_is_an_error() {
        let opf = minimal_opf(
            r#"<item id="ghost" href="text/missing.xhtml" media-type="application/xhtml+xml"/>"#,
            "",
        );
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", MAIN_CSS),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings
                .iter()
                .any(|f| f.code == "manifest-sync" && f.message.contains("missing.xhtml")),
            "got {findings:#?}"
        );
    }

    #[test]
    fn file_not_in_manifest_is_an_error() {
        let bytes = clean_book(&[("OEBPS/text/orphan.xhtml", b"<html/>")]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings
                .iter()
                .any(|f| f.code == "manifest-sync" && f.message.contains("orphan.xhtml")),
            "got {findings:#?}"
        );
    }

    #[test]
    fn broken_toc_fragment_is_an_error() {
        const NAV_BROKEN: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol>
<li><a href="text/chapter1.xhtml#does-not-exist">Broken</a></li>
</ol></nav></body></html>"#;
        let opf = minimal_opf("", "");
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_BROKEN),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", MAIN_CSS),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings
                .iter()
                .any(|f| f.code == "spine-toc-sync" && f.message.contains("does-not-exist")),
            "got {findings:#?}"
        );
    }

    #[test]
    fn two_fragment_refs_into_the_same_doc_yield_exactly_one_finding() {
        // Both refs target text/chapter1.xhtml: one a valid id ("s2"), one
        // broken ("nope"). Regardless of how the id lookup is implemented
        // (memoized or not), each ref is independent - exactly one finding,
        // for the broken one, and it must not be duplicated or lost.
        const NAV_TWO_FRAGS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol>
<li><a href="text/chapter1.xhtml#s2">Valid</a></li>
<li><a href="text/chapter1.xhtml#nope">Broken</a></li>
</ol></nav></body></html>"#;
        let opf = minimal_opf("", "");
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_TWO_FRAGS),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", MAIN_CSS),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        let sync_findings: Vec<&LintFinding> = findings
            .iter()
            .filter(|f| f.code == "spine-toc-sync")
            .collect();
        assert_eq!(
            sync_findings.len(),
            1,
            "expected exactly one finding, got {sync_findings:#?}"
        );
        assert!(sync_findings[0].message.contains("nope"));
    }

    #[test]
    fn toc_target_outside_the_spine_is_an_error() {
        const NAV_OUTSIDE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol>
<li><a href="text/not-in-spine.xhtml">Not in spine</a></li>
</ol></nav></body></html>"#;
        let opf = minimal_opf(
            r#"<item id="extra" href="text/not-in-spine.xhtml" media-type="application/xhtml+xml"/>"#,
            "",
        );
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_OUTSIDE),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/text/not-in-spine.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", MAIN_CSS),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings
                .iter()
                .any(|f| f.code == "spine-toc-sync" && f.message.contains("not-in-spine")),
            "got {findings:#?}"
        );
    }

    #[test]
    fn duplicate_id_in_spine_document_is_an_error() {
        const NAV_SIMPLE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Nav</title></head>
<body><nav epub:type="toc"><ol>
<li><a href="text/chapter1.xhtml">Chapter 1</a></li>
</ol></nav></body></html>"#;
        const CHAPTER_DUP: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter 1</title></head>
<body><h1 id="dup">Chapter 1</h1><p>Text.</p><h2 id="dup">Section 2</h2><p>More.</p></body></html>"#;
        let opf = minimal_opf("", "");
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_SIMPLE),
            ("OEBPS/text/chapter1.xhtml", CHAPTER_DUP),
            ("OEBPS/styles/main.css", MAIN_CSS),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings.iter().any(|f| f.code == "duplicate-id"
                && f.severity == Severity::Error
                && f.message.contains("dup")
                && f.message.contains("chapter1.xhtml")),
            "got {findings:#?}"
        );
    }

    #[test]
    fn unique_ids_produce_no_duplicate_id_finding() {
        // The existing clean_book fixture has two distinct ids ("top", "s2");
        // regression guard so the new check does not false-positive on it.
        let bytes = clean_book(&[]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            !findings.iter().any(|f| f.code == "duplicate-id"),
            "got {findings:#?}"
        );
    }

    #[test]
    fn svg_image_resource_is_an_image_format_error() {
        const SVG: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10"/></svg>"#;
        let opf = minimal_opf(
            r#"<item id="img" href="images/pic.svg" media-type="image/svg+xml"/>"#,
            "",
        );
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", MAIN_CSS),
            ("OEBPS/images/pic.svg", SVG),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings.iter().any(|f| f.code == "image-format"
                && f.severity == Severity::Error
                && f.message.contains("SVG")),
            "got {findings:#?}"
        );
    }

    #[test]
    fn inline_svg_early_in_a_chapter_is_not_flagged_as_an_svg_image() {
        // A chapter with an inline chart <svg> near the very start of its
        // bytes must not sniff as an SVG *image* resource - it is an XHTML
        // document whose root element is <html>, not <svg>.
        const CHAPTER_WITH_INLINE_SVG: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chart</title></head>
<body><p><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10"/></svg></p></body></html>"#;
        let bytes = clean_book(&[("OEBPS/text/chapter2.xhtml", CHAPTER_WITH_INLINE_SVG)]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            !findings
                .iter()
                .any(|f| f.code == "image-format" && f.message.contains("chapter2.xhtml")),
            "an XHTML chapter with an early inline <svg> must not be flagged as an SVG image: \
             {findings:#?}"
        );
    }

    #[test]
    fn gif_image_is_an_image_format_error() {
        const GIF: &[u8] = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;";
        let opf = minimal_opf(
            r#"<item id="img" href="images/pic.gif" media-type="image/gif"/>"#,
            "",
        );
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", MAIN_CSS),
            ("OEBPS/images/pic.gif", GIF),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings.iter().any(|f| f.code == "image-format"
                && f.severity == Severity::Error
                && f.message.contains("GIF")),
            "got {findings:#?}"
        );
    }

    /// A hand-crafted, minimal progressive JPEG: a valid JPEG header/quant
    /// table followed by a `SOF2` (progressive) marker instead of `SOF0`. The
    /// `image` crate cannot construct one directly (it only encodes
    /// baseline), so the marker bytes are spliced in by hand, matching how
    /// `image/mod.rs`'s own baseline-vs-progressive tests already probe for
    /// the `0xFFC0`/`0xFFC2` markers.
    #[test]
    fn progressive_jpeg_is_an_image_format_error() {
        // SOI, APP0/JFIF, a minimal DQT, then SOF2 (0xFFC2) instead of SOF0,
        // 2x2 single-component, then EOI. Never decoded, only sniffed and
        // marker-scanned, so it need not be a fully valid bitstream past the
        // headers.
        let jpeg: Vec<u8> = vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
            0x00, 0x01, 0x00, 0x00, // APP0
            0xFF, 0xDB, 0x00, 0x43, 0x00, // DQT marker + 64 zeroed entries
        ]
        .into_iter()
        .chain(std::iter::repeat_n(0u8, 64))
        .chain(vec![
            0xFF, 0xC2, 0x00, 0x0B, 0x08, 0x00, 0x02, 0x00, 0x02, 0x01, 0x01, 0x11,
            0x00, // SOF2
            0xFF, 0xD9, // EOI
        ])
        .collect();

        let opf = minimal_opf(
            r#"<item id="img" href="images/pic.jpg" media-type="image/jpeg"/>"#,
            "",
        );
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", MAIN_CSS),
            ("OEBPS/images/pic.jpg", &jpeg),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings.iter().any(|f| f.code == "image-format"
                && f.severity == Severity::Error
                && f.message.contains("progressive")),
            "got {findings:#?}"
        );
    }

    #[test]
    fn oversized_css_file_is_an_error() {
        let big_css = format!(".a{{{}}}", "margin-left:1em;".repeat(20_000));
        let opf = minimal_opf("", "");
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", big_css.as_bytes()),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings
                .iter()
                .any(|f| f.code == "css-caps" && f.severity == Severity::Error),
            "got {findings:#?}"
        );
    }

    #[test]
    fn too_many_css_rules_is_a_warning() {
        let mut css = String::new();
        for i in 0..1600 {
            css.push_str(&format!(".c{i}{{margin-left:1em}}"));
        }
        let opf = minimal_opf("", "");
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", css.as_bytes()),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings
                .iter()
                .any(|f| f.code == "css-caps" && f.severity == Severity::Warning),
            "got {findings:#?}"
        );
    }

    #[test]
    fn font_resource_is_a_warning() {
        let opf = minimal_opf(
            r#"<item id="font" href="fonts/a.ttf" media-type="font/ttf"/>"#,
            "",
        );
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", MAIN_CSS),
            ("OEBPS/fonts/a.ttf", b"not a real font"),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings
                .iter()
                .any(|f| f.code == "fonts" && f.severity == Severity::Warning),
            "got {findings:#?}"
        );
    }

    #[test]
    fn invalid_utf8_text_resource_is_an_error() {
        // Not valid UTF-8 anywhere: a lone continuation byte.
        let bad = [b"<html", &[0xFFu8, 0xFEu8][..], b"/>"].concat();
        let opf = minimal_opf("", "");
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", &bad),
            ("OEBPS/styles/main.css", MAIN_CSS),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings
                .iter()
                .any(|f| f.code == "encoding" && f.severity == Severity::Error),
            "got {findings:#?}"
        );
    }

    #[test]
    fn non_nfc_text_resource_is_a_warning() {
        // "e" + U+0301 COMBINING ACUTE ACCENT: decomposed, not NFC.
        let decomposed = "cafe\u{0301}";
        let chapter = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>C</title></head>
<body><p>{decomposed}</p></body></html>"#
        );
        let opf = minimal_opf("", "");
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", chapter.as_bytes()),
            ("OEBPS/styles/main.css", MAIN_CSS),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings
                .iter()
                .any(|f| f.code == "encoding" && f.severity == Severity::Warning),
            "got {findings:#?}"
        );
    }

    #[test]
    fn jpeg_is_progressive_detects_the_sof2_marker() {
        assert!(jpeg_is_progressive(&[0xFF, 0xD8, 0xFF, 0xC2, 0x00]));
        assert!(!jpeg_is_progressive(&[0xFF, 0xD8, 0xFF, 0xC0, 0x00]));
    }

    /// A valid baseline JPEG whose APP2 payload (e.g. an ICC profile or EXIF
    /// thumbnail) happens to contain the bytes `FF C2`. APPn/DQT/COM segments
    /// are length-prefixed, not byte-stuffed, so those bytes are legitimate
    /// there - only an actual SOF2 MARKER means progressive. A whole-file
    /// byte scan false-positives on this and would fail `check` (exit 1) on
    /// a perfectly valid third-party book.
    fn baseline_jpeg_with_ff_c2_in_app2() -> Vec<u8> {
        vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xE2, 0x00, 0x06, 0xFF, 0xC2, 0xAB, 0xCD, // APP2, payload holds FF C2
            0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x02, 0x00, 0x02, 0x01, 0x01, 0x11,
            0x00, // SOF0 (baseline)
            0xFF, 0xD9, // EOI
        ]
    }

    #[test]
    fn baseline_jpeg_with_ff_c2_bytes_in_an_app_segment_is_not_progressive() {
        assert!(
            !jpeg_is_progressive(&baseline_jpeg_with_ff_c2_in_app2()),
            "FF C2 inside a length-prefixed APP2 payload is not a SOF2 marker"
        );
    }

    #[test]
    fn baseline_jpeg_with_ff_c2_in_app_segment_gets_no_image_format_error() {
        let jpeg = baseline_jpeg_with_ff_c2_in_app2();
        let opf = minimal_opf(
            r#"<item id="img" href="images/pic.jpg" media-type="image/jpeg"/>"#,
            "",
        );
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", MAIN_CSS),
            ("OEBPS/images/pic.jpg", &jpeg),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            !findings
                .iter()
                .any(|f| f.code == "image-format" && f.message.contains("progressive")),
            "a baseline JPEG must not be flagged progressive: {findings:#?}"
        );
    }

    #[test]
    fn jpeg_marker_walk_survives_truncated_and_garbage_input() {
        // Malformed structure never panics and never claims progressive; the
        // decode-based checks own real garbage.
        for bytes in [
            &[][..],
            &[0xFF][..],
            &[0xFF, 0xD8][..],                         // bare SOI
            &[0xFF, 0xD8, 0xFF][..],                   // truncated marker
            &[0xFF, 0xD8, 0xFF, 0xE2][..],             // APPn with no length
            &[0xFF, 0xD8, 0xFF, 0xE2, 0x00][..],       // APPn with half a length
            &[0xFF, 0xD8, 0xFF, 0xE2, 0x00, 0x01][..], // length below its own 2 bytes
            &[0xFF, 0xD8, 0x12, 0x34][..],             // junk where a marker belongs
            &[0xFF, 0xD8, 0xFF, 0x00][..],             // stuffed byte outside scan data
            &[0x00, 0x11, 0x22][..],                   // no SOI at all
            b"GIF89a not even a jpeg"[..].as_ref(),
        ] {
            assert!(
                !jpeg_is_progressive(bytes),
                "malformed input {bytes:02X?} must not be flagged progressive"
            );
        }
    }

    #[test]
    fn empty_spine_is_a_spine_empty_error() {
        let opf = minimal_opf_with_spine("", "<spine>\n  </spine>");
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", MAIN_CSS),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        let spine_empty: Vec<&LintFinding> = findings
            .iter()
            .filter(|f| f.code == "spine-empty")
            .collect();
        assert_eq!(
            spine_empty.len(),
            1,
            "expected exactly one spine-empty finding, got {findings:#?}"
        );
        assert_eq!(spine_empty[0].severity, Severity::Error);
        assert_eq!(
            spine_empty[0].message,
            "the spine has no usable itemrefs; the book has no readable content"
        );
    }

    #[test]
    fn all_itemrefs_linear_no_is_a_spine_empty_error_mentioning_linear() {
        let opf = minimal_opf_with_spine(
            "",
            r#"<spine>
    <itemref idref="ch1" linear="no"/>
  </spine>"#,
        );
        let bytes = build_zip(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf),
            ("OEBPS/nav.xhtml", NAV_XHTML),
            ("OEBPS/text/chapter1.xhtml", CHAPTER1),
            ("OEBPS/styles/main.css", MAIN_CSS),
        ]);
        let findings = lint_epub(&bytes, &DeviceCaps::x4(), &Features::all_on());
        assert!(
            findings.iter().any(|f| f.code == "spine-empty"
                && f.severity == Severity::Error
                && f.message.contains("linear")),
            "got {findings:#?}"
        );
    }

    #[test]
    fn unreadable_bytes_report_a_single_error() {
        let findings = lint_epub(
            b"not a zip file at all",
            &DeviceCaps::x4(),
            &Features::all_on(),
        );
        assert_eq!(errors(&findings).len(), 1);
        assert_eq!(errors(&findings)[0].code, "unreadable");
    }
}
