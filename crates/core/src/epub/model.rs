//! The in-memory book model. This is the shared data structure produced by
//! every book source (the EPUB reader here; the Markdown frontend in a later
//! milestone) and consumed by every later stage (HTML/CSS/image transforms,
//! and the EPUB writer).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// A book ready for transformation and re-writing as a CrossPoint-optimized
/// EPUB.
#[derive(Debug, Clone)]
pub struct Book {
    /// Everything the OPF `<metadata>` element carries.
    pub metadata: Metadata,
    /// Every retained resource (XHTML, CSS, images, fonts, the OPF/nav/NCX,
    /// ...), keyed by its zip-absolute path, in original zip insertion order.
    pub resources: IndexMap<String, Resource>,
    /// Reading order: resource paths, each a key of `resources`.
    pub spine: Vec<String>,
    /// Table of contents entries, in document order.
    pub toc: Vec<TocEntry>,
    /// Resource path of the cover image, if one was found.
    pub cover: Option<String>,
    /// Zip path of the package document (OPF). The writer regenerates the OPF
    /// at this path.
    pub opf_path: String,
    /// Zip path of the EPUB3 navigation document, if present.
    pub nav_path: Option<String>,
    /// Zip path of the EPUB2 NCX document, if present.
    pub ncx_path: Option<String>,
}

/// Book-level metadata: everything the OPF `<metadata>` element carries, or
/// what Markdown front-matter and `--metadata` supply.
///
/// Until 0.2 this held four fields and the rest of `<metadata>` was not merely
/// dropped on the rewrite - it was never read at all, silently. A book that
/// arrived with a publisher, a description and ten subjects went out with none
/// of them and no warning. Everything here is now read, carried through the
/// pipeline, and written back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    /// The book's title (falls back to `"Untitled"` with a warning if absent).
    pub title: String,
    /// Authors, in document order (from all `dc:creator` elements).
    pub authors: Vec<Creator>,
    /// Editors, translators, illustrators and the like (`dc:contributor`).
    pub contributors: Vec<Creator>,
    /// BCP 47-ish language tag (falls back to `"en"` with a warning if absent).
    pub language: String,
    /// The book's unique identifier - the `dc:identifier` the package's
    /// `unique-identifier` points at.
    ///
    /// Never overwritten by a lookup or an override: a reading system keys its
    /// library and its reading position off this, so swapping it silently
    /// orphans the user's bookmarks. New identifiers are *added* to
    /// [`Self::identifiers`] instead.
    pub identifier: Option<String>,
    /// Every *other* `dc:identifier`: ISBNs, DOIs, vendor ids.
    pub identifiers: Vec<Identifier>,
    /// `dc:description` - the blurb.
    pub description: Option<String>,
    /// `dc:publisher`.
    pub publisher: Option<String>,
    /// `dc:subject` - keywords/BISAC categories, in document order.
    pub subjects: Vec<String>,
    /// `dc:date` - the publication date.
    pub date: Option<String>,
    /// `dc:rights`.
    pub rights: Option<String>,
    /// The series this book belongs to, from an EPUB3
    /// `belongs-to-collection` refinement or Calibre's `calibre:series` meta.
    pub series: Option<Series>,
}

/// A person: a `dc:creator` or `dc:contributor`, with the EPUB3 refinements
/// worth keeping.
///
/// Deserializes from a bare string *or* an object, so a hand-written metadata
/// document can say `author: Jane Author` and only reach for the long form when
/// it needs to:
///
/// ```yaml
/// authors:
///   - Jane Author                                  # the common case
///   - { name: Bill Writer, file_as: "Writer, Bill", role: edt }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "CreatorField", into = "CreatorField")]
pub struct Creator {
    /// The display name, as printed on the cover.
    pub name: String,
    /// `opf:file-as` - how a library sorts this person ("Writer, Bill"). Worth
    /// carrying: it is what a Kobo orders your shelf by.
    pub file_as: Option<String>,
    /// A MARC relator code (`aut`, `edt`, `trl`, `ill`, ...).
    pub role: Option<String>,
}

impl Creator {
    /// A creator with just a name, which is the overwhelmingly common case.
    pub fn new(name: impl Into<String>) -> Self {
        Creator {
            name: name.into(),
            file_as: None,
            role: None,
        }
    }
}

/// The on-the-wire shape of a [`Creator`]: a bare string, or the full object.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum CreatorField {
    Name(String),
    Full {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        file_as: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
    },
}

impl From<CreatorField> for Creator {
    fn from(field: CreatorField) -> Self {
        match field {
            CreatorField::Name(name) => Creator::new(name),
            CreatorField::Full {
                name,
                file_as,
                role,
            } => Creator {
                name,
                file_as,
                role,
            },
        }
    }
}

impl From<Creator> for CreatorField {
    fn from(creator: Creator) -> Self {
        // Round-trip to the short form when there is nothing else to say, so a
        // fetched document stays readable.
        if creator.file_as.is_none() && creator.role.is_none() {
            CreatorField::Name(creator.name)
        } else {
            CreatorField::Full {
                name: creator.name,
                file_as: creator.file_as,
                role: creator.role,
            }
        }
    }
}

/// A `dc:identifier` that is not the book's unique one: an ISBN, a DOI, a
/// vendor id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identifier {
    /// The identifier itself, e.g. `9780261102217`.
    pub value: String,
    /// What kind it is, e.g. `ISBN`, from `opf:scheme` or an EPUB3
    /// `identifier-type` refinement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
}

/// A series, and this book's place in it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Series {
    /// The series name, e.g. `The Lord of the Rings`.
    pub name: String,
    /// Position within the series. A string, not a number: real books are
    /// numbered `2`, `2.5` and `II`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<String>,
}

/// A single retained file: its raw bytes plus a declared or guessed media
/// type. Text resources have already been normalized to UTF-8.
#[derive(Debug, Clone)]
pub struct Resource {
    /// The file's raw bytes.
    pub data: Vec<u8>,
    /// The resource's media (MIME) type.
    pub media_type: String,
}

/// One entry in the table of contents.
#[derive(Debug, Clone)]
pub struct TocEntry {
    /// The entry's display title, whitespace-collapsed.
    pub title: String,
    /// A spine resource path, plus an optional `#fragment`.
    pub href: String,
    /// Nesting depth, starting at 1 for top-level entries.
    pub level: u8,
}

/// Normalize an href found in an EPUB document into a zip-absolute path:
/// percent-decoded, resolved relative to `base_dir` (the directory containing
/// the document the href was found in, e.g. the OPF's directory for manifest
/// hrefs), with `.`/`..` segments collapsed. No leading `/`, forward slashes
/// only. A trailing `#fragment` (also percent-decoded) is preserved as-is.
pub fn normalize_href(base_dir: &str, href: &str) -> String {
    let (path_part, fragment) = match href.split_once('#') {
        Some((p, f)) => (p, Some(f)),
        None => (href, None),
    };
    let decoded_path = percent_decode(path_part);
    let combined = if base_dir.is_empty() || decoded_path.starts_with('/') {
        decoded_path
    } else {
        format!("{base_dir}/{decoded_path}")
    };
    let normalized = normalize_path_segments(&combined);
    match fragment {
        Some(f) => format!("{normalized}#{}", percent_decode(f)),
        None => normalized,
    }
}

/// Normalize a raw zip entry name to the same shape [`normalize_href`] gives
/// manifest hrefs: backslashes to forward slashes, percent-decoded, leading
/// slash stripped, `./..` collapsed. Case is deliberately untouched -
/// lowercasing would corrupt a case-correct book, and a case-mismatched
/// manifest stays a genuine lint finding.
///
/// Accepted tradeoff: a file literally named with `%20` on disk that the
/// manifest references as `%2520` is sacrificed. Percent-decoding entry names
/// aligns the overwhelmingly common real-world case, where entry names and
/// hrefs are percent-encoded the same way, at the cost of that rare inversion.
pub(crate) fn normalize_entry_name(name: &str) -> String {
    normalize_href("", &name.replace('\\', "/"))
}

/// Percent-decode `%XX` escapes; invalid UTF-8 produced by the decode is
/// replaced with the Unicode replacement character rather than failing (hrefs
/// are display strings, never treated as trusted paths outside this crate).
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push(hi * 16 + lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Collapse `.`/`..` segments and stray/leading/trailing slashes out of a
/// `/`-separated path.
fn normalize_path_segments(path: &str) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            s => stack.push(s),
        }
    }
    stack.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_relative_href_to_base_dir() {
        assert_eq!(
            normalize_href("OEBPS", "chapter1.xhtml"),
            "OEBPS/chapter1.xhtml"
        );
    }

    #[test]
    fn joins_nested_relative_href() {
        assert_eq!(
            normalize_href("OEBPS", "text/chapter1.xhtml"),
            "OEBPS/text/chapter1.xhtml"
        );
    }

    #[test]
    fn root_base_dir_leaves_href_unprefixed() {
        assert_eq!(normalize_href("", "OEBPS/content.opf"), "OEBPS/content.opf");
    }

    #[test]
    fn percent_decodes_spaces() {
        assert_eq!(
            normalize_href("OEBPS", "chapter%201.xhtml"),
            "OEBPS/chapter 1.xhtml"
        );
    }

    #[test]
    fn percent_decodes_multibyte_utf8() {
        assert_eq!(
            normalize_href("OEBPS", "caf%C3%A9.xhtml"),
            "OEBPS/café.xhtml"
        );
    }

    #[test]
    fn resolves_parent_segment() {
        assert_eq!(
            normalize_href("OEBPS/text", "../images/cover.jpg"),
            "OEBPS/images/cover.jpg"
        );
    }

    #[test]
    fn collapses_current_dir_segment() {
        assert_eq!(
            normalize_href("OEBPS", "./chapter1.xhtml"),
            "OEBPS/chapter1.xhtml"
        );
    }

    #[test]
    fn strips_leading_slash_treating_href_as_zip_absolute() {
        assert_eq!(
            normalize_href("OEBPS", "/OEBPS/chapter1.xhtml"),
            "OEBPS/chapter1.xhtml"
        );
    }

    #[test]
    fn preserves_fragment_after_normalizing_path() {
        assert_eq!(
            normalize_href("OEBPS", "text/chapter1.xhtml#section2"),
            "OEBPS/text/chapter1.xhtml#section2"
        );
    }

    #[test]
    fn preserves_fragment_with_percent_encoded_href() {
        assert_eq!(
            normalize_href("OEBPS", "text/ch%201.xhtml#s2"),
            "OEBPS/text/ch 1.xhtml#s2"
        );
    }

    #[test]
    fn normalize_entry_name_matrix() {
        assert_eq!(normalize_entry_name("./OEBPS/a.xhtml"), "OEBPS/a.xhtml");
        assert_eq!(normalize_entry_name("OEBPS\\a.xhtml"), "OEBPS/a.xhtml");
        assert_eq!(normalize_entry_name("OEBPS/im%20g.png"), "OEBPS/im g.png");
        assert_eq!(normalize_entry_name("mimetype"), "mimetype");
        assert_eq!(
            normalize_entry_name("META-INF/container.xml"),
            "META-INF/container.xml"
        );
        assert_eq!(normalize_entry_name("/OEBPS/a.xhtml"), "OEBPS/a.xhtml");
    }
}
