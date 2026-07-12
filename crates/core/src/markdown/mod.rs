//! The Markdown frontend: turn a single `.md` file (plus its local images)
//! into a [`Book`], the same in-memory shape the EPUB reader produces. From
//! here on [`crate::convert`] runs the identical finalize pipeline regardless
//! of where the book came from.
//!
//! The pipeline is: comrak parses the whole source (frontmatter delimiter
//! recognized, so it never gets treated as body content) -> [`split`] slices
//! the AST into chapters on heading boundaries -> each chapter is rendered to
//! HTML independently (so GFM footnotes number and resolve correctly within
//! their own chapter) -> wrapped in the [`ChapterTemplate`] -> parsed back into
//! a DOM -> [`render::sanitize_chapter_dom`] strips unsafe raw HTML -> heading
//! slugs are stamped on -> local images are resolved and their `<img src>`
//! rewritten -> serialized as a chapter resource.

mod assets;
mod frontmatter;
pub(crate) mod render;
mod split;

pub use assets::{AssetResolver, FsResolver};

use askama::Template;
use comrak::nodes::NodeValue;
use comrak::{Arena, format_html, parse_document};
use indexmap::IndexMap;
use kuchikiki::NodeRef;

use crate::epub::model::{Book, Metadata, Resource, TocEntry};
use crate::error::ConvertError;
use crate::html::dom::{collect_by_name, get_attr, set_attr};
use crate::html::{parse_xhtml, serialize_xhtml};
use crate::options::ConvertOptions;
use crate::report::Warning;
use assets::ImageRegistry;
use frontmatter::Frontmatter;

/// Parse `text` as a Markdown book and build the [`Book`] [`crate::convert`]
/// then runs its shared finalize pipeline over. `assets` resolves every local
/// image reference (see [`AssetResolver`]).
///
/// # Errors
/// Returns [`ConvertError::InvalidMarkdown`] if the frontmatter YAML is
/// malformed, or any error a chapter template/serialize step raises.
pub fn build_book(
    text: &str,
    assets: &dyn AssetResolver,
    opts: &ConvertOptions,
) -> Result<(Book, Vec<Warning>), ConvertError> {
    let mut warnings = Vec::new();
    let comrak_opts = render::comrak_options();
    let arena = Arena::new();
    let root = parse_document(&arena, text, &comrak_opts);

    let frontmatter = match leading_front_matter_text(root) {
        Some(raw) => frontmatter::parse_frontmatter(&raw)?,
        None => Frontmatter::default(),
    };

    let title = resolve_title(&frontmatter, root, &mut warnings);
    let language = frontmatter
        .language
        .clone()
        .unwrap_or_else(|| "en".to_string());

    let mut chunks = split::split_chapters(root, opts.split_level, &mut warnings);
    if chunks.is_empty() {
        chunks.push(split::empty_front_chunk());
    }

    let opf_path = "OEBPS/content.opf".to_string();
    let mut resources: IndexMap<String, Resource> = IndexMap::new();
    // A placeholder: `write_epub` always regenerates the OPF from `Book`'s
    // fields and ignores this entry's bytes, but it only ever writes a
    // `book.opf_path` zip entry at all if `resources` already has one at that
    // key (true for EPUB input, where the OPF is a real zip entry; nav/NCX get
    // their own "write if missing" fallback in the writer, the OPF does not).
    resources.insert(
        opf_path.clone(),
        Resource {
            data: Vec::new(),
            media_type: "application/oebps-package+xml".to_string(),
        },
    );
    let mut spine = Vec::new();
    let mut toc = Vec::new();
    let mut images = ImageRegistry::new(assets);

    for (index, chunk) in chunks.iter().enumerate() {
        let chapter_path = format!("OEBPS/ch-{:03}.xhtml", index + 1);
        let chapter_title = if chunk.is_front() {
            title.clone()
        } else {
            chunk.title.clone()
        };

        let chunk_doc = arena.alloc(NodeValue::Document.into());
        for node in &chunk.nodes {
            chunk_doc.append(node);
        }
        let mut body_html = String::new();
        format_html(chunk_doc, &comrak_opts, &mut body_html).map_err(fmt_err)?;

        let wrapped = ChapterTemplate {
            title: chapter_title.clone(),
            lang: language.clone(),
            body_html,
        }
        .render()
        .map_err(render_err)?;

        let doc = parse_xhtml(wrapped.as_bytes())?;
        render::sanitize_chapter_dom(&doc, &mut warnings, &chapter_path);
        split::assign_heading_ids(&doc, &chunk.heading_ids_in_order(), opts.split_level);
        resolve_chapter_images(
            &doc,
            &mut images,
            &mut resources,
            &mut warnings,
            &chapter_path,
        );

        let bytes = serialize_xhtml(&doc);
        resources.insert(
            chapter_path.clone(),
            Resource {
                data: bytes,
                media_type: "application/xhtml+xml".to_string(),
            },
        );
        spine.push(chapter_path.clone());

        toc.push(TocEntry {
            title: chapter_title,
            href: chapter_path.clone(),
            level: 1,
        });
        for sub in &chunk.sub_headings {
            toc.push(TocEntry {
                title: sub.title.clone(),
                href: format!("{chapter_path}#{}", sub.slug),
                level: 2,
            });
        }
    }

    let cover = frontmatter
        .cover
        .as_ref()
        .and_then(|href| images.resolve(href, &mut resources, &mut warnings, "frontmatter"));

    let book = Book {
        metadata: Metadata {
            title,
            authors: frontmatter.authors,
            language,
            identifier: None,
        },
        resources,
        spine,
        toc,
        cover,
        opf_path,
        nav_path: None,
        ncx_path: None,
    };

    Ok((book, warnings))
}

/// The raw text of the document's `NodeValue::FrontMatter` node, if comrak's
/// `front_matter` extension recognized a leading block.
fn leading_front_matter_text(root: comrak::Node<'_>) -> Option<String> {
    root.children().next().and_then(|node| {
        if let NodeValue::FrontMatter(raw) = &node.data.borrow().value {
            Some(raw.clone())
        } else {
            None
        }
    })
}

/// Resolve the book title: the frontmatter's, else the first H1 in the
/// document, else `"Untitled"` with a warning.
fn resolve_title(
    frontmatter: &Frontmatter,
    root: comrak::Node<'_>,
    warnings: &mut Vec<Warning>,
) -> String {
    if let Some(title) = &frontmatter.title
        && !title.trim().is_empty()
    {
        return title.clone();
    }
    if let Some(h1) = split::first_h1_text(root) {
        return h1;
    }
    warnings.push(Warning {
        message: "no title in frontmatter and no H1 heading found; using \"Untitled\"".to_string(),
        file: None,
    });
    "Untitled".to_string()
}

/// Resolve every local `<img src>` in a chapter's DOM, rewriting it to the
/// resolved resource's relative path (left unchanged, with a warning already
/// recorded by [`ImageRegistry`], when unresolvable).
fn resolve_chapter_images(
    doc: &NodeRef,
    images: &mut ImageRegistry,
    resources: &mut IndexMap<String, Resource>,
    warnings: &mut Vec<Warning>,
    chapter_path: &str,
) {
    for img in collect_by_name(doc, "img") {
        let Some(src) = get_attr(&img, "src") else {
            continue;
        };
        if let Some(resource_path) = images.resolve(&src, resources, warnings, chapter_path) {
            set_attr(
                &img,
                "src",
                &assets::rewrite_src(chapter_path, &resource_path),
            );
        }
    }
}

#[derive(Template)]
#[template(path = "chapter.xhtml", escape = "html")]
struct ChapterTemplate {
    title: String,
    lang: String,
    /// Pre-rendered, already-escaped chapter body HTML from comrak.
    body_html: String,
}

fn render_err(e: askama::Error) -> ConvertError {
    ConvertError::Io(std::io::Error::other(e))
}

fn fmt_err(e: std::fmt::Error) -> ConvertError {
    ConvertError::Io(std::io::Error::other(e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MapResolver(HashMap<String, Vec<u8>>);

    impl AssetResolver for MapResolver {
        fn resolve(&self, href: &str) -> Option<Vec<u8>> {
            self.0.get(href).cloned()
        }
    }

    fn resolver(entries: &[(&str, &[u8])]) -> MapResolver {
        MapResolver(
            entries
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_vec()))
                .collect(),
        )
    }

    fn chapter_bytes<'a>(book: &'a Book, path: &str) -> &'a str {
        std::str::from_utf8(&book.resources[path].data).expect("chapter is utf8")
    }

    #[test]
    fn frontmatter_title_and_author_flow_into_metadata() {
        let md = "---\ntitle: My Book\nauthor:\n  - Jane Doe\n  - John Smith\nlanguage: de\n---\n\n# Chapter One\n\nHello.\n";
        let (book, warnings) =
            build_book(md, &resolver(&[]), &ConvertOptions::default()).expect("builds");
        assert_eq!(book.metadata.title, "My Book");
        assert_eq!(
            book.metadata.authors,
            vec!["Jane Doe".to_string(), "John Smith".to_string()]
        );
        assert_eq!(book.metadata.language, "de");
        assert!(
            warnings.iter().all(|w| !w.message.contains("Untitled")),
            "no fallback warning expected: {warnings:?}"
        );
    }

    #[test]
    fn missing_title_falls_back_to_first_h1_with_no_warning() {
        let md = "Some intro.\n\n# The Real Title\n\nBody.\n";
        let (book, warnings) =
            build_book(md, &resolver(&[]), &ConvertOptions::default()).expect("builds");
        assert_eq!(book.metadata.title, "The Real Title");
        assert!(warnings.iter().all(|w| !w.message.contains("Untitled")));
    }

    #[test]
    fn missing_title_and_no_h1_falls_back_to_untitled_with_a_warning() {
        let md = "Just a paragraph, no headings at all.\n";
        let (book, warnings) =
            build_book(md, &resolver(&[]), &ConvertOptions::default()).expect("builds");
        assert_eq!(book.metadata.title, "Untitled");
        assert!(warnings.iter().any(|w| w.message.contains("Untitled")));
    }

    #[test]
    fn default_language_is_en() {
        let md = "# Chapter One\n\nBody.\n";
        let (book, _) = build_book(md, &resolver(&[]), &ConvertOptions::default()).expect("builds");
        assert_eq!(book.metadata.language, "en");
    }

    #[test]
    fn malformed_frontmatter_yaml_propagates_as_invalid_markdown() {
        let md = "---\ntitle: [oops\n---\n\n# Chapter\n";
        let err = build_book(md, &resolver(&[]), &ConvertOptions::default())
            .expect_err("malformed YAML must error");
        assert!(matches!(err, ConvertError::InvalidMarkdown(_)));
    }

    #[test]
    fn chapters_split_into_separate_resources_with_a_spine_and_toc() {
        let md =
            "# Chapter One\n\nText one.\n\n## Section A\n\nMore.\n\n# Chapter Two\n\nText two.\n";
        let (book, _) = build_book(md, &resolver(&[]), &ConvertOptions::default()).expect("builds");
        assert_eq!(book.spine, vec!["OEBPS/ch-001.xhtml", "OEBPS/ch-002.xhtml"]);
        assert_eq!(book.opf_path, "OEBPS/content.opf");
        assert_eq!(book.nav_path, None);
        assert_eq!(book.ncx_path, None);

        assert_eq!(book.toc.len(), 3, "chapter 1, its sub-heading, chapter 2");
        assert_eq!(book.toc[0].level, 1);
        assert_eq!(book.toc[0].href, "OEBPS/ch-001.xhtml");
        assert_eq!(book.toc[1].level, 2);
        assert_eq!(book.toc[1].href, "OEBPS/ch-001.xhtml#section-a");
        assert_eq!(book.toc[2].level, 1);
        assert_eq!(book.toc[2].href, "OEBPS/ch-002.xhtml");

        let ch1 = chapter_bytes(&book, "OEBPS/ch-001.xhtml");
        assert!(ch1.contains(r#"<h1 id="chapter-one">"#), "got: {ch1}");
        assert!(ch1.contains(r#"<h2 id="section-a">"#), "got: {ch1}");
    }

    #[test]
    fn front_chapter_is_titled_with_the_book_title() {
        let md = "---\ntitle: The Book\n---\n\nIntro paragraph.\n\n# Chapter One\n\nBody.\n";
        let (book, _) = build_book(md, &resolver(&[]), &ConvertOptions::default()).expect("builds");
        assert_eq!(book.spine.len(), 2);
        assert_eq!(book.toc[0].title, "The Book");
        assert_eq!(book.toc[0].href, "OEBPS/ch-001.xhtml");
    }

    #[test]
    fn empty_document_still_produces_one_chapter() {
        let (book, _) = build_book("", &resolver(&[]), &ConvertOptions::default()).expect("builds");
        assert_eq!(book.spine.len(), 1);
    }

    #[test]
    fn local_image_is_resolved_rewritten_and_stored() {
        let md = "# Chapter One\n\n![alt text](art/pic.png)\n";
        let (book, warnings) = build_book(
            md,
            &resolver(&[("art/pic.png", b"\x89PNG\r\n")]),
            &ConvertOptions::default(),
        )
        .expect("builds");
        assert!(warnings.is_empty(), "got: {warnings:?}");
        assert!(book.resources.contains_key("OEBPS/images/art-pic.png"));
        let ch1 = chapter_bytes(&book, "OEBPS/ch-001.xhtml");
        assert!(ch1.contains(r#"src="images/art-pic.png""#), "got: {ch1}");
    }

    #[test]
    fn missing_local_image_warns_and_leaves_the_reference_unchanged() {
        let md = "# Chapter One\n\n![alt](missing.png)\n";
        let (book, warnings) =
            build_book(md, &resolver(&[]), &ConvertOptions::default()).expect("builds");
        assert!(
            warnings
                .iter()
                .any(|w| w.message.contains("could not resolve"))
        );
        let ch1 = chapter_bytes(&book, "OEBPS/ch-001.xhtml");
        assert!(ch1.contains(r#"src="missing.png""#), "got: {ch1}");
    }

    #[test]
    fn remote_image_warns_and_is_left_as_is() {
        let md = "# Chapter One\n\n![alt](https://example.com/pic.png)\n";
        let (book, warnings) =
            build_book(md, &resolver(&[]), &ConvertOptions::default()).expect("builds");
        assert!(
            warnings
                .iter()
                .any(|w| w.message.contains("remote image not fetched"))
        );
        let ch1 = chapter_bytes(&book, "OEBPS/ch-001.xhtml");
        assert!(
            ch1.contains(r#"src="https://example.com/pic.png""#),
            "got: {ch1}"
        );
    }

    #[test]
    fn frontmatter_cover_is_resolved_into_book_cover() {
        let md = "---\ncover: images/cover.jpg\n---\n\n# Chapter One\n\nBody.\n";
        let (book, _) = build_book(
            md,
            &resolver(&[("images/cover.jpg", b"jpegbytes")]),
            &ConvertOptions::default(),
        )
        .expect("builds");
        assert_eq!(book.cover.as_deref(), Some("OEBPS/images/images-cover.jpg"));
        assert!(book.resources.contains_key("OEBPS/images/images-cover.jpg"));
    }

    #[test]
    fn script_tag_in_raw_html_is_removed_with_a_warning() {
        let md = "# Chapter One\n\n<script>alert(1)</script>\n\nText.\n";
        let (book, warnings) =
            build_book(md, &resolver(&[]), &ConvertOptions::default()).expect("builds");
        let ch1 = chapter_bytes(&book, "OEBPS/ch-001.xhtml");
        assert!(!ch1.contains("script"), "got: {ch1}");
        assert!(warnings.iter().any(|w| w.message.contains("script")));
    }

    #[test]
    fn gfm_extensions_render_tables_tasklists_and_footnotes() {
        let md = "# Chapter One\n\n\
                   | A | B |\n|---|---|\n| 1 | 2 |\n\n\
                   - [x] done\n- [ ] not done\n\n\
                   Here is a note.[^1]\n\n[^1]: The footnote body.\n";
        let (book, _) = build_book(md, &resolver(&[]), &ConvertOptions::default()).expect("builds");
        let ch1 = chapter_bytes(&book, "OEBPS/ch-001.xhtml");
        assert!(ch1.contains("<table>"), "table should render: {ch1}");
        assert!(
            ch1.contains(r#"type="checkbox""#),
            "task list should render: {ch1}"
        );
        assert!(
            ch1.contains("footnote-ref") && ch1.contains("class=\"footnotes\""),
            "footnotes should render: {ch1}"
        );
    }

    #[test]
    fn split_level_two_is_honored() {
        let opts = ConvertOptions {
            split_level: 2,
            ..ConvertOptions::default()
        };
        let md = "# Part One\n\n## Section A\n\nText.\n\n# Part Two\n\nText.\n";
        let (book, _) = build_book(md, &resolver(&[]), &opts).expect("builds");
        assert_eq!(book.spine.len(), 3, "Part One, Section A, Part Two");
    }
}
