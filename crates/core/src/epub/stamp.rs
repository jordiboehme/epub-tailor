//! The provenance stamp: reading back the `tailor:fitted` marker that
//! [`super::write::write_epub`] puts into a fitted book's OPF, so a folder
//! scan can tell a product from a source without converting anything.

use std::io::{Cursor, Read};

use zip::ZipArchive;

use crate::epub::model::normalize_entry_name;
use crate::epub::read::parse_container;

/// The IRI the `tailor` prefix is declared against on the `<package>` element.
pub const STAMP_PREFIX_IRI: &str = "https://github.com/jordiboehme/epub-tailor#";
/// The `property` of the provenance `<meta>`.
pub const STAMP_PROPERTY: &str = "tailor:fitted";

/// Cheap probe: the provenance stamp a previous `fit` wrote into `bytes`,
/// if any. Unzips only `META-INF/container.xml` and the OPF, never the full
/// [`super::read::read_epub`] parse. Any failure (not a zip, no container,
/// malformed OPF) returns `None`: an unprobeable file is treated as
/// unstamped, and whatever is actually wrong with it surfaces later, from
/// the code path that can report it properly.
pub fn read_stamp(bytes: &[u8]) -> Option<String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).ok()?;
    let container = read_entry(&mut archive, "META-INF/container.xml")?;
    let opf_path = parse_container(&container).ok()?;
    let opf = read_entry(&mut archive, &opf_path).or_else(|| {
        // A foreign archive may store the entry under a name that only
        // matches after normalization (leading ./, backslashes).
        let raw = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
            .find(|name| normalize_entry_name(name) == opf_path)?;
        read_entry(&mut archive, &raw)
    })?;

    let doc = roxmltree::Document::parse(&opf).ok()?;
    doc.descendants()
        .find(|n| n.has_tag_name("meta") && n.attribute("property") == Some(STAMP_PROPERTY))
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn read_entry(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut text = String::new();
    archive.by_name(name).ok()?.read_to_string(&mut text).ok()?;
    Some(text)
}
