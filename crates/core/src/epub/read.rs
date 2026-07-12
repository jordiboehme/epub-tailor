//! Reading an `.epub` archive (bytes) into the shared [`super::model::Book`].

use std::io::{Cursor, Read, Seek};

use indexmap::IndexMap;
use roxmltree::{Document, Node};
use zip::ZipArchive;

use crate::epub::model::{
    Book, Creator, Identifier, Metadata, Resource, Series, TocEntry, normalize_entry_name,
    normalize_href,
};
use crate::error::ConvertError;
use crate::report::Warning;

/// The result of successfully reading an EPUB: the parsed [`Book`] plus any
/// non-fatal warnings encountered along the way.
#[derive(Debug)]
pub struct ReadEpub {
    /// The parsed book.
    pub book: Book,
    /// Non-fatal issues encountered while reading (missing metadata,
    /// transcoded encodings, dropped files, dangling references, ...).
    pub warnings: Vec<Warning>,
}

/// Font-obfuscation algorithms that `META-INF/encryption.xml` may declare
/// without the book being treated as DRM-protected (the fonts are stripped
/// anyway, so de-obfuscating them would be pointless work).
const FONT_OBFUSCATION_ALGORITHMS: [&str; 2] = [
    "http://www.idpf.org/2008/embedding",
    "http://ns.adobe.com/pdf/enc#RC",
];

/// Media types normalized to UTF-8 per step 8 of the read pipeline. Also used
/// by [`crate::validate`]'s `encoding` lint to pick out which resources must
/// be valid UTF-8/NFC.
pub(crate) const TEXT_MEDIA_TYPES: [&str; 5] = [
    "application/xhtml+xml",
    "text/html",
    "text/css",
    "application/x-dtbncx+xml",
    "application/oebps-package+xml",
];

/// Read an `.epub` file's raw bytes into a [`Book`].
///
/// # Errors
/// Returns [`ConvertError::InvalidEpub`] for a corrupt/non-ZIP input or a
/// structurally invalid EPUB, [`ConvertError::DrmProtected`] if the book is
/// DRM-protected (font obfuscation alone is not treated as DRM), and
/// [`ConvertError::Zip64Unsupported`] for ZIP64 archives.
pub fn read_epub(bytes: &[u8]) -> Result<ReadEpub, ConvertError> {
    let mut warnings = Vec::new();
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| ConvertError::InvalidEpub(format!("not a valid ZIP archive: {e}")))?;

    check_drm(&mut archive, &mut warnings)?;
    check_zip64(&mut archive)?;

    let ArchiveEntries {
        entries: all_entries,
        collisions,
    } = read_all_entries(&mut archive)?;
    for EntryCollision {
        kept_raw,
        dropped_raw,
        key,
    } in &collisions
    {
        warnings.push(Warning {
            message: format!(
                "zip entries '{dropped_raw}' and '{kept_raw}' normalize to the same path '{key}' - kept the first"
            ),
            file: None,
        });
    }

    let container_bytes = all_entries
        .get("META-INF/container.xml")
        .ok_or_else(|| ConvertError::InvalidEpub("missing META-INF/container.xml".to_string()))?;
    let container_text = String::from_utf8_lossy(container_bytes).into_owned();
    let opf_path = parse_container(&container_text)?;

    // Partition entries into resources (in zip order) vs. dropped META-INF
    // extras, excluding `mimetype` and `META-INF/` itself.
    let mut dropped_meta_inf = Vec::new();
    let mut resources_raw: IndexMap<String, Vec<u8>> = IndexMap::new();
    for (name, data) in all_entries {
        if name == "mimetype" {
            continue;
        }
        if let Some(rest) = name.strip_prefix("META-INF/") {
            if rest != "container.xml" && rest != "encryption.xml" {
                dropped_meta_inf.push(name);
            }
            continue;
        }
        resources_raw.insert(name, data);
    }
    if !dropped_meta_inf.is_empty() {
        warnings.push(Warning {
            message: format!(
                "unsupported META-INF file(s) dropped: {}",
                dropped_meta_inf.join(", ")
            ),
            file: None,
        });
    }

    let opf_dir = parent_dir(&opf_path);
    let opf_raw = resources_raw.get(&opf_path).ok_or_else(|| {
        ConvertError::InvalidEpub(format!(
            "OPF referenced at '{opf_path}' not found in archive"
        ))
    })?;
    let opf_utf8 = normalize_text_bytes(&opf_path, opf_raw, &mut warnings);
    let opf_text =
        String::from_utf8(opf_utf8).expect("normalize_text_bytes always returns valid UTF-8");
    let opf_doc = Document::parse(&opf_text)
        .map_err(|e| ConvertError::InvalidEpub(format!("malformed OPF: {e}")))?;

    let parsed_opf = parse_opf(&opf_doc, &opf_dir, &opf_path, &mut warnings)?;

    // Cache of text resources already normalized to UTF-8 while parsing the
    // OPF/nav/NCX above, so the resource-building loop below never re-decodes
    // (and re-warns about) the same file.
    let mut normalized_texts: IndexMap<String, Vec<u8>> = IndexMap::new();
    normalized_texts.insert(opf_path.clone(), opf_text.into_bytes());

    let toc = build_toc(
        &parsed_opf.nav_path,
        &parsed_opf.ncx_path,
        &resources_raw,
        &mut normalized_texts,
        &mut warnings,
    );

    let href_media_types: std::collections::HashMap<&str, &str> = parsed_opf
        .manifest
        .values()
        .map(|item| (item.href.as_str(), item.media_type.as_str()))
        .collect();

    let mut resources = IndexMap::new();
    for (path, data) in resources_raw {
        let media_type = href_media_types
            .get(path.as_str())
            .filter(|mt| !mt.is_empty())
            .map(|mt| mt.to_string())
            .unwrap_or_else(|| guess_media_type(&path));
        let final_data = if TEXT_MEDIA_TYPES.contains(&media_type.as_str()) {
            match normalized_texts.swap_remove(&path) {
                Some(cached) => cached,
                None => normalize_text_bytes(&path, &data, &mut warnings),
            }
        } else {
            data
        };
        resources.insert(
            path,
            Resource {
                data: final_data,
                media_type,
            },
        );
    }

    let book = Book {
        metadata: parsed_opf.metadata,
        resources,
        spine: parsed_opf.spine,
        toc,
        cover: parsed_opf.cover,
        opf_path,
        nav_path: parsed_opf.nav_path,
        ncx_path: parsed_opf.ncx_path,
    };

    Ok(ReadEpub { book, warnings })
}

// ---------------------------------------------------------------------
// DRM check
// ---------------------------------------------------------------------

/// What `META-INF/encryption.xml` declares, once at least one `EncryptedData`
/// entry is present (an absent file is not represented here; callers check
/// presence first).
pub(crate) enum EncryptionClass {
    /// Every `EncryptedData` entry uses a font-obfuscation algorithm: not
    /// DRM (the fonts are stripped anyway, so de-obfuscating them would be
    /// pointless work), but worth a nod since the book does carry the file.
    FontObfuscationOnly,
    /// At least one entry uses a real encryption algorithm: the book is
    /// DRM-protected.
    Drm,
}

/// Classify a decoded `META-INF/encryption.xml` document's `EncryptedData`
/// entries. Shared by [`check_drm`] (which turns [`EncryptionClass::Drm`] into
/// a hard [`ConvertError::DrmProtected`]) and `crate::validate`'s `drm` lint
/// (which turns it into an `Error`-severity finding instead).
pub(crate) fn classify_encryption_xml(text: &str) -> Result<EncryptionClass, ConvertError> {
    let doc = Document::parse(text).map_err(|e| {
        ConvertError::InvalidEpub(format!("malformed META-INF/encryption.xml: {e}"))
    })?;

    let mut non_font_algorithm = false;
    for enc_data in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "EncryptedData")
    {
        let algorithm = enc_data
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "EncryptionMethod")
            .and_then(|n| n.attribute("Algorithm"));
        match algorithm {
            Some(algo) if FONT_OBFUSCATION_ALGORITHMS.contains(&algo) => {}
            _ => non_font_algorithm = true,
        }
    }

    if non_font_algorithm {
        Ok(EncryptionClass::Drm)
    } else {
        Ok(EncryptionClass::FontObfuscationOnly)
    }
}

fn check_drm<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    warnings: &mut Vec<Warning>,
) -> Result<(), ConvertError> {
    let mut enc_bytes = Vec::new();
    match archive.by_name("META-INF/encryption.xml") {
        Ok(mut file) => {
            file.read_to_end(&mut enc_bytes).map_err(|e| {
                ConvertError::InvalidEpub(format!("could not read META-INF/encryption.xml: {e}"))
            })?;
        }
        Err(zip::result::ZipError::FileNotFound) => return Ok(()),
        Err(e) => {
            return Err(ConvertError::InvalidEpub(format!(
                "could not read META-INF/encryption.xml: {e}"
            )));
        }
    }

    let text = String::from_utf8_lossy(&enc_bytes);
    match classify_encryption_xml(&text)? {
        EncryptionClass::Drm => Err(ConvertError::DrmProtected),
        EncryptionClass::FontObfuscationOnly => {
            warnings.push(Warning {
                message: "obfuscated fonts detected (META-INF/encryption.xml uses font \
                          obfuscation only); fonts are stripped anyway"
                    .to_string(),
                file: None,
            });
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------
// ZIP64 check
// ---------------------------------------------------------------------

/// Whether `archive` is (or contains an entry that requires) ZIP64: more than
/// 65,535 entries, or any entry whose size or header offset does not fit a
/// 32-bit field. Shared by [`check_zip64`] (a hard error for `read_epub`) and
/// `crate::validate`'s `zip64` lint (an `Error`-severity finding instead).
pub(crate) fn zip64_reason<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Option<&'static str>, ConvertError> {
    if archive.len() > 65_535 {
        return Ok(Some("more than 65,535 entries"));
    }
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| ConvertError::InvalidEpub(format!("corrupt zip entry {i}: {e}")))?;
        if file.size() >= u32::MAX as u64
            || file.compressed_size() >= u32::MAX as u64
            || file.header_start() >= u32::MAX as u64
        {
            return Ok(Some("an entry's size or offset does not fit 32 bits"));
        }
    }
    Ok(None)
}

fn check_zip64<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<(), ConvertError> {
    match zip64_reason(archive)? {
        Some(_) => Err(ConvertError::Zip64Unsupported),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------
// Entry gathering
// ---------------------------------------------------------------------

/// The result of reading every entry of an archive: the bytes keyed by
/// normalized path, plus any raw-name collisions that were dropped.
pub(crate) struct ArchiveEntries {
    /// Every non-directory entry, keyed by [`normalize_entry_name`] of its raw
    /// zip name, in zip order. First entry wins on a normalized-key collision.
    pub entries: IndexMap<String, Vec<u8>>,
    /// One record per raw name whose normalized key already existed. The first
    /// entry to claim a key is kept.
    pub collisions: Vec<EntryCollision>,
}

/// A normalized-key collision between two raw zip entry names: the first entry
/// (`kept_raw`) won and the later one (`dropped_raw`) was discarded; both
/// normalize to `key`. Carrying the kept RAW name - not just the shared key -
/// lets the message name both entries distinctly even when the kept entry is
/// itself the messy one (`./OEBPS/a.xhtml` kept, `OEBPS/a.xhtml` dropped).
pub(crate) struct EntryCollision {
    /// The raw name of the first entry to claim the key (kept).
    pub kept_raw: String,
    /// The raw name of the later entry that normalized to the same key (dropped).
    pub dropped_raw: String,
    /// The normalized key both raw names share.
    pub key: String,
}

/// Read every non-directory entry of `archive` into memory, keyed by its
/// normalized zip entry name (see [`normalize_entry_name`]), in zip order.
/// Shared by [`read_epub`] and `crate::validate`, which both need the raw bytes
/// of every file in the archive under keys that match normalized manifest
/// hrefs. On a normalized-key collision the FIRST entry wins and the later raw
/// name is recorded in `collisions` rather than overwriting it.
pub(crate) fn read_all_entries<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<ArchiveEntries, ConvertError> {
    let mut entries = IndexMap::new();
    // The raw name of the first entry to claim each normalized key, so a later
    // collision can name the kept entry as well as the dropped one.
    let mut kept_raw_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut collisions = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| ConvertError::InvalidEpub(format!("corrupt zip entry {i}: {e}")))?;
        if file.is_dir() {
            continue;
        }
        let raw_name = file.name().to_string();
        let key = normalize_entry_name(&raw_name);
        let mut data = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut data).map_err(ConvertError::Io)?;
        // First wins: `IndexMap::insert` would overwrite, so guard with
        // `contains_key` and record the collision (kept + dropped) instead.
        if entries.contains_key(&key) {
            let kept_raw = kept_raw_names
                .get(&key)
                .cloned()
                .unwrap_or_else(|| key.clone());
            collisions.push(EntryCollision {
                kept_raw,
                dropped_raw: raw_name,
                key,
            });
        } else {
            kept_raw_names.insert(key.clone(), raw_name);
            entries.insert(key, data);
        }
    }
    Ok(ArchiveEntries {
        entries,
        collisions,
    })
}

// ---------------------------------------------------------------------
// container.xml
// ---------------------------------------------------------------------

/// Parse `META-INF/container.xml`, returning the OPF's zip-absolute path.
/// Shared by [`read_epub`] and `crate::validate`.
pub(crate) fn parse_container(text: &str) -> Result<String, ConvertError> {
    let doc = Document::parse(text)
        .map_err(|e| ConvertError::InvalidEpub(format!("malformed META-INF/container.xml: {e}")))?;
    let rootfile = doc
        .descendants()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == "rootfile"
                && n.attribute("media-type") == Some("application/oebps-package+xml")
        })
        .ok_or_else(|| {
            ConvertError::InvalidEpub(
                "META-INF/container.xml has no rootfile with \
                 media-type=\"application/oebps-package+xml\""
                    .to_string(),
            )
        })?;
    let full_path = rootfile.attribute("full-path").ok_or_else(|| {
        ConvertError::InvalidEpub("container.xml rootfile is missing full-path".to_string())
    })?;
    Ok(normalize_href("", full_path))
}

// ---------------------------------------------------------------------
// OPF parsing
// ---------------------------------------------------------------------

struct ManifestItem {
    href: String,
    media_type: String,
    properties: Vec<String>,
}

struct ParsedOpf {
    metadata: Metadata,
    manifest: IndexMap<String, ManifestItem>,
    spine: Vec<String>,
    cover: Option<String>,
    nav_path: Option<String>,
    ncx_path: Option<String>,
}

fn parse_opf(
    doc: &Document,
    opf_dir: &str,
    opf_path: &str,
    warnings: &mut Vec<Warning>,
) -> Result<ParsedOpf, ConvertError> {
    let package = doc.root_element();
    let unique_identifier = package.attribute("unique-identifier");

    let metadata_node = package
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "metadata")
        .ok_or_else(|| ConvertError::InvalidEpub("OPF missing <metadata>".to_string()))?;

    let metadata = parse_metadata(metadata_node, unique_identifier, warnings);

    let manifest_node = package
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "manifest")
        .ok_or_else(|| ConvertError::InvalidEpub("OPF missing <manifest>".to_string()))?;
    let manifest = parse_manifest(manifest_node, opf_dir);

    let spine_node = package
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "spine")
        .ok_or_else(|| ConvertError::InvalidEpub("OPF missing <spine>".to_string()))?;
    let spine_toc_attr = spine_node.attribute("toc");
    let spine = parse_spine(spine_node, &manifest, opf_path, warnings);

    let cover = find_cover(metadata_node, &manifest);
    let nav_path = manifest
        .values()
        .find(|item| item.properties.iter().any(|p| p == "nav"))
        .map(|item| item.href.clone());
    let mut ncx_path = manifest
        .values()
        .find(|item| item.media_type == "application/x-dtbncx+xml")
        .map(|item| item.href.clone());
    if ncx_path.is_none()
        && let Some(toc_id) = spine_toc_attr
    {
        ncx_path = manifest.get(toc_id).map(|item| item.href.clone());
    }

    Ok(ParsedOpf {
        metadata,
        manifest,
        spine,
        cover,
        nav_path,
        ncx_path,
    })
}

/// The text of the first `<dc:NAME>` element, or `None` when it is absent or
/// blank.
fn first_dc(metadata_node: Node, name: &str) -> Option<String> {
    metadata_node
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == name)
        .map(|n| collapse_whitespace(&collect_text(n)))
        .filter(|s| !s.is_empty())
}

/// Every `<dc:NAME>` element's text, in document order, blanks skipped.
fn all_dc(metadata_node: Node, name: &str) -> Vec<String> {
    metadata_node
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == name)
        .map(|n| collapse_whitespace(&collect_text(n)))
        .filter(|s| !s.is_empty())
        .collect()
}

/// The value of an EPUB3 refinement: `<meta refines="#id" property="P">value</meta>`.
///
/// EPUB3 moved what EPUB2 put in attributes (`opf:file-as`, `opf:role`,
/// `opf:scheme`) into these standoff elements, so a reader that ignores them -
/// as this one did until 0.2 - loses author sort keys, roles and ISBN schemes
/// on every modern book.
fn refinement(metadata_node: Node, id: &str, property: &str) -> Option<String> {
    let target = format!("#{id}");
    metadata_node
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "meta")
        .find(|n| {
            n.attribute("refines") == Some(target.as_str())
                && n.attribute("property") == Some(property)
        })
        .map(|n| collapse_whitespace(&collect_text(n)))
        .filter(|s| !s.is_empty())
}

/// Read a `dc:creator` / `dc:contributor`, taking its sort key and role from
/// either the EPUB2 attributes or the EPUB3 refinements.
fn parse_creator(metadata_node: Node, node: Node) -> Option<Creator> {
    let name = collapse_whitespace(&collect_text(node));
    if name.is_empty() {
        return None;
    }
    // `opf:file-as` is namespaced, but roxmltree's `attribute()` matches on the
    // local name only when given a plain string, so try both spellings.
    let attr = |names: [&str; 2]| {
        names
            .iter()
            .find_map(|n| node.attribute(*n))
            .map(collapse_whitespace)
            .filter(|s| !s.is_empty())
    };
    let by_id = |property: &str| {
        node.attribute("id")
            .and_then(|id| refinement(metadata_node, id, property))
    };
    Some(Creator {
        name,
        file_as: attr(["file-as", "opf:file-as"]).or_else(|| by_id("file-as")),
        role: attr(["role", "opf:role"]).or_else(|| by_id("role")),
    })
}

/// The series, from the EPUB3 `belongs-to-collection` refinement or Calibre's
/// `<meta name="calibre:series">`. Calibre's spelling is worth honoring because
/// a large share of the world's sideloaded EPUBs came out of Calibre.
fn parse_series(metadata_node: Node) -> Option<Series> {
    // EPUB3: <meta property="belongs-to-collection" id="c1">Name</meta>
    //        <meta refines="#c1" property="group-position">2</meta>
    let epub3 = metadata_node
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "meta")
        .find(|n| n.attribute("property") == Some("belongs-to-collection"))
        .and_then(|n| {
            let name = collapse_whitespace(&collect_text(n));
            if name.is_empty() {
                return None;
            }
            let index = n
                .attribute("id")
                .and_then(|id| refinement(metadata_node, id, "group-position"));
            Some(Series { name, index })
        });
    if epub3.is_some() {
        return epub3;
    }

    // Calibre: <meta name="calibre:series" content="Name"/>
    //          <meta name="calibre:series_index" content="2"/>
    let named = |want: &str| {
        metadata_node
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "meta")
            .find(|n| n.attribute("name") == Some(want))
            .and_then(|n| n.attribute("content"))
            .map(collapse_whitespace)
            .filter(|s| !s.is_empty())
    };
    named("calibre:series").map(|name| Series {
        name,
        index: named("calibre:series_index"),
    })
}

fn parse_metadata(
    metadata_node: Node,
    unique_identifier: Option<&str>,
    warnings: &mut Vec<Warning>,
) -> Metadata {
    let title = first_dc(metadata_node, "title").unwrap_or_else(|| {
        warnings.push(Warning {
            message: "OPF has no dc:title; using \"Untitled\"".to_string(),
            file: None,
        });
        "Untitled".to_string()
    });

    let creators = |name: &str| -> Vec<Creator> {
        metadata_node
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == name)
            .filter_map(|n| parse_creator(metadata_node, n))
            .collect()
    };

    let language = first_dc(metadata_node, "language").unwrap_or_else(|| {
        warnings.push(Warning {
            message: "OPF has no dc:language; using \"en\"".to_string(),
            file: None,
        });
        "en".to_string()
    });

    // The unique identifier is the one the package points at; everything else is
    // a secondary identifier (an ISBN, usually) and is kept as such.
    let id_nodes: Vec<Node> = metadata_node
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "identifier")
        .collect();
    let unique_node = unique_identifier
        .and_then(|uid| id_nodes.iter().find(|n| n.attribute("id") == Some(uid)))
        .or_else(|| id_nodes.first())
        .copied();
    let identifier = unique_node
        .map(|n| collapse_whitespace(&collect_text(n)))
        .filter(|s| !s.is_empty());

    let identifiers: Vec<Identifier> = id_nodes
        .iter()
        .filter(|n| Some(**n) != unique_node)
        .filter_map(|n| {
            let value = collapse_whitespace(&collect_text(*n));
            if value.is_empty() {
                return None;
            }
            let scheme = ["scheme", "opf:scheme"]
                .iter()
                .find_map(|a| n.attribute(*a))
                .map(collapse_whitespace)
                .or_else(|| {
                    n.attribute("id")
                        .and_then(|id| refinement(metadata_node, id, "identifier-type"))
                })
                .filter(|s| !s.is_empty());
            Some(Identifier { value, scheme })
        })
        .collect();

    Metadata {
        title,
        authors: creators("creator"),
        contributors: creators("contributor"),
        language,
        identifier,
        identifiers,
        description: first_dc(metadata_node, "description"),
        publisher: first_dc(metadata_node, "publisher"),
        subjects: all_dc(metadata_node, "subject"),
        date: first_dc(metadata_node, "date"),
        rights: first_dc(metadata_node, "rights"),
        series: parse_series(metadata_node),
    }
}

fn parse_manifest(manifest_node: Node, opf_dir: &str) -> IndexMap<String, ManifestItem> {
    let mut manifest = IndexMap::new();
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
    manifest
}

fn parse_spine(
    spine_node: Node,
    manifest: &IndexMap<String, ManifestItem>,
    opf_path: &str,
    warnings: &mut Vec<Warning>,
) -> Vec<String> {
    let itemrefs: Vec<Node> = spine_node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "itemref")
        .collect();

    // A resolvable itemref is one whose idref names a manifest item; a linear
    // one additionally lacks `linear="no"`. When the spine has resolvable
    // itemrefs but not one is linear, dropping them all would leave an empty
    // book. Rescue that case: keep every resolvable itemref in document order
    // and warn once, instead of skipping each with its own warning.
    let resolvable = |itemref: &Node| {
        itemref
            .attribute("idref")
            .is_some_and(|id| manifest.contains_key(id))
    };
    let any_linear = itemrefs
        .iter()
        .any(|r| r.attribute("linear") != Some("no") && resolvable(r));
    let any_resolvable = itemrefs.iter().any(resolvable);
    let rescue = any_resolvable && !any_linear;

    let mut spine = Vec::new();
    for itemref in &itemrefs {
        let idref = itemref.attribute("idref");
        let Some(item) = idref.and_then(|id| manifest.get(id)) else {
            warnings.push(Warning {
                message: format!(
                    "spine itemref references unknown manifest id '{}'; skipped",
                    idref.unwrap_or("")
                ),
                file: None,
            });
            continue;
        };
        // In the rescue case keep every resolvable itemref, `linear="no"`
        // included; otherwise skip `linear="no"` items with a per-item warning
        // (behavior unchanged when at least one linear itemref exists).
        if !rescue && itemref.attribute("linear") == Some("no") {
            warnings.push(Warning {
                message: "spine item has linear=\"no\"; skipped".to_string(),
                file: Some(item.href.clone()),
            });
            continue;
        }
        spine.push(item.href.clone());
    }
    if rescue {
        warnings.push(Warning {
            message: "the spine has no readable linear items - keeping the non-linear ones so \
                      the book is not empty"
                .to_string(),
            file: Some(opf_path.to_string()),
        });
    }
    spine
}

fn find_cover(metadata_node: Node, manifest: &IndexMap<String, ManifestItem>) -> Option<String> {
    let meta_cover_id = metadata_node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "meta")
        .find(|n| n.attribute("name") == Some("cover"))
        .and_then(|n| n.attribute("content"));
    if let Some(id) = meta_cover_id
        && let Some(item) = manifest.get(id)
        && item.media_type.starts_with("image/")
    {
        return Some(item.href.clone());
    }
    manifest
        .values()
        .find(|item| item.properties.iter().any(|p| p == "cover-image"))
        .map(|item| item.href.clone())
}

// ---------------------------------------------------------------------
// TOC (nav doc, falling back to NCX)
// ---------------------------------------------------------------------

/// Build the table of contents from the nav doc (falling back to the NCX),
/// normalizing each file's bytes to UTF-8 exactly once and stashing the
/// result in `normalized_texts` so the caller's resource-building pass can
/// reuse it instead of decoding (and re-warning about) the file again.
fn build_toc(
    nav_path: &Option<String>,
    ncx_path: &Option<String>,
    resources_raw: &IndexMap<String, Vec<u8>>,
    normalized_texts: &mut IndexMap<String, Vec<u8>>,
    warnings: &mut Vec<Warning>,
) -> Vec<TocEntry> {
    if let Some(nav_path) = nav_path
        && let Some(raw) = resources_raw.get(nav_path)
    {
        let utf8 = normalize_text_bytes(nav_path, raw, warnings);
        let text = String::from_utf8(utf8).expect("normalize_text_bytes returns valid UTF-8");
        let toc = Document::parse(&text)
            .ok()
            .and_then(|doc| parse_nav_toc(&doc, &parent_dir(nav_path)));
        normalized_texts.insert(nav_path.clone(), text.into_bytes());
        if let Some(toc) = toc {
            return toc;
        }
    }
    if let Some(ncx_path) = ncx_path
        && let Some(raw) = resources_raw.get(ncx_path)
    {
        let utf8 = normalize_text_bytes(ncx_path, raw, warnings);
        let text = String::from_utf8(utf8).expect("normalize_text_bytes returns valid UTF-8");
        let toc = Document::parse(&text).ok().and_then(|doc| {
            let toc = parse_ncx_toc(&doc, &parent_dir(ncx_path));
            (!toc.is_empty()).then_some(toc)
        });
        normalized_texts.insert(ncx_path.clone(), text.into_bytes());
        if let Some(toc) = toc {
            return toc;
        }
    }
    warnings.push(Warning {
        message: "no navigation document or NCX found; table of contents is empty".to_string(),
        file: None,
    });
    Vec::new()
}

fn parse_nav_toc(doc: &Document, nav_dir: &str) -> Option<Vec<TocEntry>> {
    let navs: Vec<Node> = doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "nav")
        .collect();
    let toc_nav = navs
        .iter()
        .find(|n| {
            n.attributes()
                .any(|a| a.name() == "type" && a.value() == "toc")
        })
        .or_else(|| {
            navs.iter().find(|n| {
                n.descendants()
                    .any(|d| d.is_element() && d.tag_name().name() == "ol")
            })
        })?;
    let root_ol = toc_nav
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "ol")?;
    let mut toc = Vec::new();
    parse_nav_list(root_ol, 1, nav_dir, &mut toc);
    Some(toc)
}

fn parse_nav_list(ol: Node, level: u8, nav_dir: &str, out: &mut Vec<TocEntry>) {
    for li in ol
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "li")
    {
        if let Some(a) = li
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "a")
        {
            let title = collapse_whitespace(&collect_text(a));
            let href_raw = a.attribute("href").unwrap_or("");
            let href = normalize_href(nav_dir, href_raw);
            out.push(TocEntry { title, href, level });
        }
        if let Some(nested_ol) = li
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "ol")
        {
            parse_nav_list(nested_ol, level + 1, nav_dir, out);
        }
    }
}

fn parse_ncx_toc(doc: &Document, ncx_dir: &str) -> Vec<TocEntry> {
    let mut toc = Vec::new();
    if let Some(nav_map) = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "navMap")
    {
        parse_nav_points(nav_map, 1, ncx_dir, &mut toc);
    }
    toc
}

fn parse_nav_points(parent: Node, level: u8, ncx_dir: &str, out: &mut Vec<TocEntry>) {
    for nav_point in parent
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "navPoint")
    {
        let title = nav_point
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "navLabel")
            .map(|n| collapse_whitespace(&collect_text(n)))
            .unwrap_or_default();
        let src = nav_point
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "content")
            .and_then(|n| n.attribute("src"))
            .unwrap_or("");
        let href = normalize_href(ncx_dir, src);
        out.push(TocEntry { title, href, level });
        parse_nav_points(nav_point, level + 1, ncx_dir, out);
    }
}

// ---------------------------------------------------------------------
// Media type guessing
// ---------------------------------------------------------------------

fn guess_media_type(path: &str) -> String {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "xhtml" | "html" => "application/xhtml+xml",
        "css" => "text/css",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ncx" => "application/x-dtbncx+xml",
        "opf" => "application/oebps-package+xml",
        _ => "application/octet-stream",
    }
    .to_string()
}

// ---------------------------------------------------------------------
// Encoding normalization
// ---------------------------------------------------------------------

/// Normalize a text resource's bytes to UTF-8 per the pipeline's step 8:
/// strip a UTF-8 BOM, transcode UTF-16 (BOM-detected) or any other
/// non-UTF-8-valid encoding (sniffed from the XML declaration/meta charset,
/// falling back to windows-1252), and rewrite the declared encoding to UTF-8
/// when a transcode happened.
fn normalize_text_bytes(path: &str, data: &[u8], warnings: &mut Vec<Warning>) -> Vec<u8> {
    if data.starts_with(&[0xFF, 0xFE]) || data.starts_with(&[0xFE, 0xFF]) {
        let (decoded, _enc, _had_errors) = encoding_rs::UTF_16LE.decode(data);
        let rewritten = rewrite_declared_encoding(&decoded);
        warnings.push(Warning {
            message: format!("{path}: re-encoded from UTF-16 to UTF-8"),
            file: Some(path.to_string()),
        });
        return rewritten.into_bytes();
    }

    let data = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);

    match std::str::from_utf8(data) {
        Ok(_) => data.to_vec(),
        Err(_) => {
            let label = sniff_declared_encoding(data);
            let encoding = label
                .as_deref()
                .and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()))
                .unwrap_or(encoding_rs::WINDOWS_1252);
            let (decoded, _had_errors) = encoding.decode_without_bom_handling(data);
            let source_name = encoding.name();
            let rewritten = rewrite_declared_encoding(&decoded);
            warnings.push(Warning {
                message: format!("{path}: re-encoded from {source_name} to UTF-8"),
                file: Some(path.to_string()),
            });
            rewritten.into_bytes()
        }
    }
}

/// Look for a declared encoding in an XML declaration (`encoding="..."`) or
/// an HTML `<meta charset>`/`http-equiv` `content` attribute, scanning only
/// the (ASCII, always-decodable) head of the document.
fn sniff_declared_encoding(data: &[u8]) -> Option<String> {
    let window = &data[..data.len().min(2048)];
    find_attr_value(window, b"encoding=").or_else(|| find_attr_value(window, b"charset="))
}

fn find_attr_value(haystack: &[u8], needle: &[u8]) -> Option<String> {
    let pos = find_subslice(haystack, needle)?;
    let mut i = pos + needle.len();
    let quote = haystack
        .get(i)
        .copied()
        .filter(|b| *b == b'"' || *b == b'\'');
    if quote.is_some() {
        i += 1;
    }
    let start = i;
    while i < haystack.len() {
        let c = haystack[i];
        let stop = match quote {
            Some(q) => c == q,
            None => c == b'"' || c == b'\'' || c == b';' || c == b' ' || c == b'>',
        };
        if stop {
            break;
        }
        i += 1;
    }
    std::str::from_utf8(&haystack[start..i])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Rewrite an already-decoded document's declared encoding to `UTF-8`: the
/// XML declaration's `encoding="..."` attribute, and any HTML
/// `charset="..."` (bare `<meta charset>` or inside a `http-equiv` `content`
/// attribute).
fn rewrite_declared_encoding(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let rest = &input[i..];
        let keyword_len = if rest.starts_with("encoding=") {
            Some("encoding=".len())
        } else if rest.starts_with("charset=") {
            Some("charset=".len())
        } else {
            None
        };
        if let Some(keyword_len) = keyword_len {
            out.push_str(&rest[..keyword_len]);
            i += keyword_len;
            let bytes = input.as_bytes();
            match bytes.get(i).copied() {
                Some(q @ (b'"' | b'\'')) => {
                    out.push(q as char);
                    i += 1;
                    if let Some(rel_close) = input[i..].find(q as char) {
                        out.push_str("UTF-8");
                        i += rel_close;
                        out.push(q as char);
                        i += 1;
                    }
                }
                _ => {
                    let start = i;
                    let mut j = i;
                    while j < bytes.len() && !matches!(bytes[j], b'"' | b'\'' | b';' | b' ' | b'>')
                    {
                        j += 1;
                    }
                    out.push_str("UTF-8");
                    i = j.max(start);
                }
            }
            continue;
        }
        let ch = rest.chars().next().expect("i < input.len()");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

// ---------------------------------------------------------------------
// Small shared helpers
// ---------------------------------------------------------------------

/// Parent directory of a zip-absolute path (`""` if `path` has no `/`).
fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(idx) => path[..idx].to_string(),
        None => String::new(),
    }
}

/// Concatenate all descendant text nodes of `node` (handles mixed content
/// like `<a>Chapter <em>One</em></a>`, which `Node::text()` alone can't).
fn collect_text(node: Node) -> String {
    let mut s = String::new();
    for desc in node.descendants() {
        if desc.is_text() {
            s.push_str(desc.text().unwrap_or(""));
        }
    }
    s
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_declared_encoding_rewrites_xml_decl() {
        let input = r#"<?xml version="1.0" encoding="windows-1252"?><p>x</p>"#;
        let out = rewrite_declared_encoding(input);
        assert_eq!(out, r#"<?xml version="1.0" encoding="UTF-8"?><p>x</p>"#);
    }

    #[test]
    fn rewrite_declared_encoding_rewrites_meta_charset() {
        let input = r#"<meta charset="iso-8859-1"/>"#;
        let out = rewrite_declared_encoding(input);
        assert_eq!(out, r#"<meta charset="UTF-8"/>"#);
    }

    #[test]
    fn rewrite_declared_encoding_rewrites_http_equiv_content() {
        let input =
            r#"<meta http-equiv="Content-Type" content="text/html; charset=windows-1252"/>"#;
        let out = rewrite_declared_encoding(input);
        assert_eq!(
            out,
            r#"<meta http-equiv="Content-Type" content="text/html; charset=UTF-8"/>"#
        );
    }

    #[test]
    fn sniff_declared_encoding_finds_xml_decl_value() {
        let input = br#"<?xml version="1.0" encoding="windows-1252"?><p/>"#;
        assert_eq!(
            sniff_declared_encoding(input),
            Some("windows-1252".to_string())
        );
    }

    #[test]
    fn sniff_declared_encoding_none_when_absent() {
        let input = br#"<p>hello</p>"#;
        assert_eq!(sniff_declared_encoding(input), None);
    }

    #[test]
    fn guess_media_type_matches_known_extensions() {
        assert_eq!(guess_media_type("a.xhtml"), "application/xhtml+xml");
        assert_eq!(guess_media_type("a.html"), "application/xhtml+xml");
        assert_eq!(guess_media_type("a.css"), "text/css");
        assert_eq!(guess_media_type("a.jpg"), "image/jpeg");
        assert_eq!(guess_media_type("a.jpeg"), "image/jpeg");
        assert_eq!(guess_media_type("a.png"), "image/png");
        assert_eq!(guess_media_type("a.gif"), "image/gif");
        assert_eq!(guess_media_type("a.svg"), "image/svg+xml");
        assert_eq!(guess_media_type("a.webp"), "image/webp");
        assert_eq!(guess_media_type("a.ttf"), "font/ttf");
        assert_eq!(guess_media_type("a.otf"), "font/otf");
        assert_eq!(guess_media_type("a.woff"), "font/woff");
        assert_eq!(guess_media_type("a.woff2"), "font/woff2");
        assert_eq!(guess_media_type("a.ncx"), "application/x-dtbncx+xml");
        assert_eq!(guess_media_type("a.opf"), "application/oebps-package+xml");
        assert_eq!(guess_media_type("a.bin"), "application/octet-stream");
    }
}
