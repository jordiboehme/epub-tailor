//! Resolving local image references from a Markdown source into `Book`
//! resources.
//!
//! Every local (schemeless) image reference in the Markdown - inline `<img>`
//! and the frontmatter `cover` - is read once via an [`AssetResolver`], stored
//! as a resource flattened under `OEBPS/images/`, and the reference is
//! rewritten to point at it. A reference that cannot be resolved (missing
//! file, path escapes the resolver's root, or a remote URL) is left pointing
//! at its original text and warned about; the device just shows a broken
//! image placeholder rather than failing the whole conversion.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use indexmap::IndexMap;

use crate::epub::model::Resource;
use crate::epub::relative_href;
use crate::report::Warning;

/// Reads the bytes a Markdown-relative image reference points at. Implemented
/// by [`FsResolver`] for the `md` CLI; tests and other embedders can supply
/// their own (e.g. an in-memory map).
pub trait AssetResolver {
    /// Resolve `href` (a schemeless, Markdown-relative reference, exactly as
    /// written in the source) to its bytes, or `None` if it cannot be read.
    fn resolve(&self, href: &str) -> Option<Vec<u8>>;
}

/// An [`AssetResolver`] rooted at a directory on disk (the Markdown file's own
/// directory, for the `md` CLI). Refuses to resolve anything outside that
/// root, so a crafted `../../etc/passwd`-style reference cannot read files the
/// book has no business touching.
pub struct FsResolver {
    root: PathBuf,
}

impl FsResolver {
    /// Root every resolved reference at `root`. `root` is canonicalized
    /// eagerly (falling back to the given path, unchanged, if that fails -
    /// every real reference will then simply fail the under-root check and
    /// resolve to `None`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let root = root.canonicalize().unwrap_or(root);
        FsResolver { root }
    }
}

impl AssetResolver for FsResolver {
    fn resolve(&self, href: &str) -> Option<Vec<u8>> {
        let candidate = self.root.join(href);
        let canonical = candidate.canonicalize().ok()?;
        if !canonical.starts_with(&self.root) {
            return None;
        }
        std::fs::read(&canonical).ok()
    }
}

/// Resolves and dedupes every local image an Markdown chapter (or the
/// frontmatter cover) references, accumulating new `OEBPS/images/...`
/// resources as it goes.
pub(crate) struct ImageRegistry<'a> {
    resolver: &'a dyn AssetResolver,
    /// Original Markdown href -> the resource path it was resolved to
    /// (dedupes a reference used more than once).
    resolved: HashMap<String, String>,
    /// Every resource path already claimed, so a flattened-name collision
    /// between two different hrefs gets a unique suffix instead of colliding.
    reserved: HashSet<String>,
}

impl<'a> ImageRegistry<'a> {
    pub(crate) fn new(resolver: &'a dyn AssetResolver) -> Self {
        ImageRegistry {
            resolver,
            resolved: HashMap::new(),
            reserved: HashSet::new(),
        }
    }

    /// Resolve `href`, inserting a new `resources` entry the first time it is
    /// seen. Returns the zip-absolute resource path on success. A remote
    /// (scheme'd) URL or an unresolvable reference records a [`Warning`] and
    /// returns `None`, leaving the caller free to keep the original text.
    pub(crate) fn resolve(
        &mut self,
        href: &str,
        resources: &mut IndexMap<String, Resource>,
        warnings: &mut Vec<Warning>,
        source_path: &str,
    ) -> Option<String> {
        if let Some(existing) = self.resolved.get(href) {
            return Some(existing.clone());
        }
        if has_scheme(href) {
            warnings.push(Warning {
                message: format!("remote image not fetched: {href}"),
                file: Some(source_path.to_string()),
            });
            return None;
        }
        let Some(data) = self.resolver.resolve(href) else {
            warnings.push(Warning {
                message: format!(
                    "could not resolve local image '{href}'; left the reference unchanged"
                ),
                file: Some(source_path.to_string()),
            });
            return None;
        };

        let flattened = href.replace('/', "-");
        let path = reserve_unique_image_path(&mut self.reserved, &flattened);
        let media_type = guess_image_media_type(&path);
        resources.insert(path.clone(), Resource { data, media_type });
        self.resolved.insert(href.to_string(), path.clone());
        Some(path)
    }
}

/// A `src` rewritten to point at a resolved image resource, relative to the
/// chapter that references it.
pub(crate) fn rewrite_src(chapter_path: &str, resource_path: &str) -> String {
    relative_href(&parent_dir(chapter_path), resource_path)
}

fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(idx) => path[..idx].to_string(),
        None => String::new(),
    }
}

/// A unique `OEBPS/images/<flattened>` path, appending `-2`, `-3`, ... on a
/// name collision between two different original hrefs.
fn reserve_unique_image_path(reserved: &mut HashSet<String>, flattened: &str) -> String {
    let (stem, ext) = split_ext(flattened);
    let mut candidate = join(&stem, &ext);
    let mut n = 2;
    while reserved.contains(&candidate) {
        candidate = join(&format!("{stem}-{n}"), &ext);
        n += 1;
    }
    reserved.insert(candidate.clone());
    candidate
}

fn join(stem: &str, ext: &str) -> String {
    if ext.is_empty() {
        format!("OEBPS/images/{stem}")
    } else {
        format!("OEBPS/images/{stem}.{ext}")
    }
}

/// Split a flattened file name into `(stem, extension)`; the extension is
/// empty if there is none.
fn split_ext(name: &str) -> (String, String) {
    match name.rfind('.') {
        Some(idx) if idx > 0 => (name[..idx].to_string(), name[idx + 1..].to_string()),
        _ => (name.to_string(), String::new()),
    }
}

fn guess_image_media_type(path: &str) -> String {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Whether an href's path part starts with a URL scheme (RFC 3986:
/// `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"`), i.e. is a remote
/// reference rather than a path relative to the Markdown source.
fn has_scheme(href: &str) -> bool {
    let Some(colon) = href.find(':') else {
        return false;
    };
    let scheme = &href[..colon];
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MapResolver(HashMap<String, Vec<u8>>);

    impl AssetResolver for MapResolver {
        fn resolve(&self, href: &str) -> Option<Vec<u8>> {
            self.0.get(href).cloned()
        }
    }

    fn map_resolver(entries: &[(&str, &[u8])]) -> MapResolver {
        MapResolver(
            entries
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_vec()))
                .collect(),
        )
    }

    #[test]
    fn resolves_and_flattens_a_nested_path() {
        let resolver = map_resolver(&[("art/pic.png", b"\x89PNG")]);
        let mut registry = ImageRegistry::new(&resolver);
        let mut resources = IndexMap::new();
        let mut warnings = Vec::new();
        let path = registry
            .resolve(
                "art/pic.png",
                &mut resources,
                &mut warnings,
                "OEBPS/ch-001.xhtml",
            )
            .expect("resolves");
        assert_eq!(path, "OEBPS/images/art-pic.png");
        assert!(resources.contains_key("OEBPS/images/art-pic.png"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn reusing_the_same_href_returns_the_same_resource_and_does_not_duplicate() {
        let resolver = map_resolver(&[("pic.png", b"data")]);
        let mut registry = ImageRegistry::new(&resolver);
        let mut resources = IndexMap::new();
        let mut warnings = Vec::new();
        let a = registry
            .resolve(
                "pic.png",
                &mut resources,
                &mut warnings,
                "OEBPS/ch-001.xhtml",
            )
            .unwrap();
        let b = registry
            .resolve(
                "pic.png",
                &mut resources,
                &mut warnings,
                "OEBPS/ch-002.xhtml",
            )
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(resources.len(), 1);
    }

    #[test]
    fn colliding_flattened_names_get_deduped() {
        let resolver = map_resolver(&[("art/pic.png", b"1"), ("art-pic.png", b"2")]);
        let mut registry = ImageRegistry::new(&resolver);
        let mut resources = IndexMap::new();
        let mut warnings = Vec::new();
        let a = registry
            .resolve("art/pic.png", &mut resources, &mut warnings, "ch.xhtml")
            .unwrap();
        let b = registry
            .resolve("art-pic.png", &mut resources, &mut warnings, "ch.xhtml")
            .unwrap();
        assert_ne!(a, b);
        assert_eq!(resources.len(), 2);
    }

    #[test]
    fn missing_reference_warns_and_returns_none() {
        let resolver = map_resolver(&[]);
        let mut registry = ImageRegistry::new(&resolver);
        let mut resources = IndexMap::new();
        let mut warnings = Vec::new();
        let result = registry.resolve("missing.png", &mut resources, &mut warnings, "ch.xhtml");
        assert!(result.is_none());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("could not resolve"));
    }

    #[test]
    fn remote_url_warns_and_is_left_unresolved() {
        let resolver = map_resolver(&[]);
        let mut registry = ImageRegistry::new(&resolver);
        let mut resources = IndexMap::new();
        let mut warnings = Vec::new();
        let result = registry.resolve(
            "https://example.com/pic.png",
            &mut resources,
            &mut warnings,
            "ch.xhtml",
        );
        assert!(result.is_none());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("remote image not fetched"));
    }

    #[test]
    fn media_type_is_guessed_from_extension() {
        assert_eq!(guess_image_media_type("OEBPS/images/a.jpg"), "image/jpeg");
        assert_eq!(guess_image_media_type("OEBPS/images/a.png"), "image/png");
        assert_eq!(
            guess_image_media_type("OEBPS/images/a.svg"),
            "image/svg+xml"
        );
        assert_eq!(
            guess_image_media_type("OEBPS/images/a.bin"),
            "application/octet-stream"
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("epub-tailor-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn fs_resolver_reads_a_file_under_its_root() {
        let root = temp_dir("fs-resolver-basic");
        std::fs::create_dir_all(root.join("images")).unwrap();
        std::fs::write(root.join("images/cover.jpg"), b"jpegbytes").unwrap();
        let resolver = FsResolver::new(&root);
        assert_eq!(
            resolver.resolve("images/cover.jpg"),
            Some(b"jpegbytes".to_vec())
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn fs_resolver_refuses_to_escape_its_root() {
        let root = temp_dir("fs-resolver-traversal");
        let sibling_secret = root
            .parent()
            .expect("temp dir has a parent")
            .join("epub-tailor-test-fs-resolver-secret.txt");
        std::fs::write(&sibling_secret, b"root:x:0:0").unwrap();
        let resolver = FsResolver::new(&root);
        assert_eq!(
            resolver.resolve("../epub-tailor-test-fs-resolver-secret.txt"),
            None,
            "must not read outside the root"
        );
        std::fs::remove_file(&sibling_secret).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn fs_resolver_returns_none_for_a_missing_file() {
        let root = temp_dir("fs-resolver-missing");
        let resolver = FsResolver::new(&root);
        assert_eq!(resolver.resolve("nope.png"), None);
        std::fs::remove_dir_all(&root).ok();
    }
}
