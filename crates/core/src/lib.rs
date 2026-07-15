//! Core of `epub-tailor`, a tool that cleans, fixes and transforms EPUB (and
//! Markdown) books, driven by composable JSON profiles.
//!
//! This crate provides the profile model ([`profile`]: device caps, feature
//! switches, content filter rules), the shared options/error/report types,
//! the EPUB reader, the Markdown frontend ([`markdown::build_book`]), the
//! strict-XHTML parser/serializer, the CSS/image pipelines, and the EPUB
//! writer - tied together by the [`convert`] pipeline. Both input doors
//! ([`Input::Epub`] and [`Input::Markdown`]) produce the same [`epub::Book`],
//! which `convert` then runs through one shared finalize path: content
//! filters, profile-gated HTML transforms (table linearization, list
//! numbering, code preservation, box degradation, link/anchor normalization,
//! text hygiene), CSS filtering, SVG/image processing, and the
//! epubcheck-clean EPUB writer. Archive and metadata repair runs
//! unconditionally; everything device-specific is a profile feature.

mod chapter_split;
pub mod css;
pub mod epub;
pub mod error;
pub mod filter;
pub mod html;
pub mod image;
pub mod markdown;
pub mod metadata;
pub mod options;
pub mod profile;
pub mod report;
pub mod validate;

use std::collections::{HashMap, HashSet};

use kuchikiki::{NodeData, NodeRef};

pub use crate::image::{ImageOutcome, ImageRole, OutFormat, process_image};
pub use css::{FilteredCss, filter_css, filter_inline_style};
pub use epub::{
    Book, Creator, Identifier, Metadata, ReadEpub, Resource, Series, StampInfo, TocEntry,
    read_epub, read_stamp, read_stamp_info, relative_href, write_epub,
};
pub use error::ConvertError;
pub use filter::{FilterAction, FilterRule, FilterTarget};
pub use html::{
    AliasMap, ChapterTransform, apply_anchor_aliases, apply_toc_aliases, parse_xhtml,
    serialize_xhtml, transform_chapter,
};
pub use markdown::{AssetResolver, FsResolver};
pub use metadata::{ClearField, MergeMode, MetadataDoc};
pub use options::{ConvertOptions, CoverImage, TableMode};
pub use profile::{DeviceCaps, Features, Profile, ProfileError};
pub use report::{ConvertReport, ConvertStats, Transformation, Warning};
pub use validate::{LintFinding, Severity, find_invalid_qname, lint_epub};

use crate::image::svg::{Rasterized, WrapperTarget};
use crate::image::{
    PipelineResult, RenderHint, encode_rendered, is_image_candidate, process_for_convert,
    sniff_raster_kind,
};
use css::caps;
use epub::model::normalize_href;
use html::cap_ids;
use html::dom::{collect_by_name, element, find_head, get_attr, remove_attr, replace_with};

/// The generated helper stylesheet, kept strictly within the device's supported
/// CSS grammar (single class selectors, supported properties only). It is added
/// to a converted book only when a transform actually introduced one of these
/// `et-*` classes.
///
/// Stored already minified so it is a fixed point of the CSS subset filter
/// (`filter_css(ET_STYLES) == ET_STYLES`, proven by a test); it is therefore not
/// itself run through the filter during conversion.
const ET_STYLES: &str = ".et-box-title{margin-top:1em}.et-caption{text-align:center;font-style:italic}.et-table-caption{text-align:center}.et-table-cell{margin-left:1em;text-indent:0}.et-table-row{margin-left:1em}.et-table-row-sep{text-align:center}.et-ol-item{text-indent:0}.et-ol-nested{margin-left:1em}.et-ol-cont{margin-left:2em}.et-code{text-align:left;margin-left:1em;text-indent:0}.et-dt{margin-top:0.5em}.et-dd{margin-left:1.5em}";

/// A source book handed to [`convert`]: either an `.epub` archive's raw bytes,
/// or a Markdown document plus a resolver for its local image references.
pub enum Input {
    /// Raw bytes of an `.epub` archive.
    Epub(Vec<u8>),
    /// A Markdown document (frontmatter, GFM body) and a resolver for the
    /// local image references it contains.
    Markdown {
        /// The Markdown source text.
        text: String,
        /// Resolves each local (schemeless) image reference to its bytes.
        assets: Box<dyn AssetResolver>,
    },
}

/// The result of a successful [`convert`] call.
pub struct Converted {
    /// The optimized EPUB archive.
    ///
    /// Empty when [`ConvertOptions::dry_run`] is set: the conversion still runs
    /// in full (so `report` reflects what would happen), but no output bytes
    /// are produced.
    pub epub: Vec<u8>,
    /// A summary of what the conversion did.
    pub report: ConvertReport,
}

/// Convert `input` into an EPUB tuned for a CrossPoint device.
///
/// The book is read, then every spine document is parsed, run through the
/// [`html::transform_chapter`] device transforms, cross-linked in a second pass
/// ([`apply_anchor_aliases`]) so anchor aliases work across chapters, capped at
/// the device anchor limit, and re-serialized as strict XHTML. Non-spine XHTML
/// resources (e.g. the nav document) are only re-serialized. A generated
/// `et-styles.css` is added when the transforms introduced any helper class.
///
/// # Errors
/// Propagates any [`ConvertError`] from reading the input, parsing a document,
/// or writing the output.
pub fn convert(input: Input, opts: &ConvertOptions) -> Result<Converted, ConvertError> {
    let (
        bytes_in,
        ReadEpub {
            mut book,
            mut warnings,
        },
    ) = match input {
        Input::Epub(bytes) => {
            let bytes_in = bytes.len() as u64;
            (bytes_in, read_epub(&bytes)?)
        }
        Input::Markdown { text, assets } => {
            let bytes_in = text.len() as u64;
            let (book, warnings) = markdown::build_book(&text, assets.as_ref(), opts)?;
            (bytes_in, ReadEpub { book, warnings })
        }
    };

    // A spine with no readable content (no itemrefs at all, or none resolvable)
    // leaves nothing to convert; fail cleanly rather than emit a nav/NCX that
    // points at a non-spine resource. An all-`linear="no"` spine is rescued in
    // `parse_spine`, so it stays non-empty and never reaches here.
    if book.spine.is_empty() {
        return Err(ConvertError::EmptySpine);
    }

    let mut transformations = Vec::new();

    // User-supplied metadata lands first, so the filters below see the finished
    // article: a watermark in a description we just filled is still a watermark.
    // Nothing here reaches the network - the document was already fetched, by a
    // separate command, before this one ever ran.
    if !opts.metadata.is_empty() {
        metadata::apply(
            &opts.metadata,
            &mut book.metadata,
            opts.metadata_merge,
            &mut transformations,
            &mut warnings,
        );
    }
    // Explicit clears run after the merge: `--clear` is a flag, and a flag is
    // the most specific thing the user can say, so a cleared field stays
    // cleared whatever the document carries - and a `fill` document can never
    // quietly refill a field the user just asked to remove.
    if !opts.metadata_clears.is_empty() {
        metadata::apply_clears(
            &opts.metadata_clears,
            &mut book.metadata,
            &mut transformations,
        );
    }
    // A supplied cover arrives as bytes, not a path: this crate never touches
    // the filesystem (the same reason the Markdown frontend takes an
    // `AssetResolver`), so the caller has already read it.
    if let Some(cover) = &opts.cover_image {
        set_cover(&mut book, cover, &mut transformations);
    }

    // Content filters see the metadata and every chapter before any device
    // transform reshapes them.
    if !opts.filters.is_empty() {
        filter::apply_metadata_filters(
            &mut book.metadata,
            &mut book.toc,
            &opts.filters,
            &mut transformations,
            &mut warnings,
        );
        filter::apply_resource_filters(&mut book, &opts.filters, &mut transformations);
    }

    // Strip embedded fonts (a device that never loads them wastes the bytes)
    // up front, so their paths are known when we prune the `<link>`s that
    // point at them.
    let stripped_fonts = if opts.features.strip_fonts {
        strip_fonts(&mut book, &mut transformations, &mut warnings)
    } else {
        HashSet::new()
    };

    // Rework every source stylesheet in place, before chapters are serialized.
    // Two passes, and a book can ask for either:
    //
    // - `sanitize_css` keeps the sheet whole and removes only what makes Adobe
    //   RMSDK throw all of it away (Kobo, PocketBook, tolino in RMSDK mode).
    // - `filter_css` demolishes it down to CrossPoint's dozen properties.
    //
    // Sanitizing runs first so that a profile with both on (nobody's, today)
    // still behaves: filtering a sanitized sheet is the same as filtering the
    // original, since everything sanitize drops, filter drops too.
    let mut total_css_rules = 0usize;
    if opts.features.sanitize_css || opts.features.filter_css {
        let css_paths: Vec<String> = book
            .resources
            .iter()
            .filter(|(_, resource)| resource.media_type == "text/css")
            .map(|(path, _)| path.clone())
            .collect();
        for path in css_paths {
            let source = String::from_utf8_lossy(&book.resources[&path].data).into_owned();

            let source = if opts.features.sanitize_css {
                let sanitized = css::sanitize_css(&source, &path, &mut warnings);
                if sanitized.decls_dropped > 0 || sanitized.rules_dropped > 0 {
                    transformations.push(Transformation {
                        kind: "css-sanitized".to_string(),
                        detail: format!(
                            "removed {} declaration(s) and {} rule(s) Adobe RMSDK cannot parse",
                            sanitized.decls_dropped, sanitized.rules_dropped
                        ),
                        file: Some(path.clone()),
                    });
                }
                sanitized.css
            } else {
                source
            };

            let filtered = if opts.features.filter_css {
                filter_css(&source, &path, &mut warnings)
            } else {
                FilteredCss {
                    css: source,
                    rules_kept: 0,
                    rules_dropped: 0,
                    decls_dropped: 0,
                }
            };
            total_css_rules += filtered.rules_kept;
            store_filtered_css(
                &mut book,
                &path,
                filtered,
                opts,
                &mut transformations,
                &mut warnings,
            );
        }
    }

    // SVGs first (the device has no SVG decoder): unwrap single-`<image>`
    // wrappers so their raster payload flows through the raster pass, and
    // rasterize real vector art. Runs before the raster pass so its outputs
    // share the same rename map, ref rewriting and manifest regeneration.
    let SvgPlan {
        renames: svg_renames,
        finalized: svg_finalized,
        rasterized: svg_rasterized,
    } = if opts.features.rasterize_svg {
        process_svgs(&mut book, opts, &mut transformations, &mut warnings)
    } else {
        SvgPlan {
            renames: HashMap::new(),
            finalized: HashSet::new(),
            rasterized: 0,
        }
    };

    // Transcode/fit/re-encode every raster image, renaming resources whose
    // format changed (and splitting tall ones when asked). Rasters the SVG pass
    // already finalized are skipped. The resulting rename and split maps drive
    // the per-chapter reference rewriting below.
    let ImagePlan {
        renames: image_renames,
        splits,
        images_processed,
    } = if opts.features.transcode_images {
        process_images(
            &mut book,
            opts,
            &svg_finalized,
            &mut transformations,
            &mut warnings,
        )
    } else {
        ImagePlan {
            renames: HashMap::new(),
            splits: HashMap::new(),
            images_processed: 0,
        }
    };

    // Compose the maps so a chapter ref to an original SVG resolves to its final
    // raster (svg -> intermediate payload -> raster-pass rename).
    let renames = compose_renames(&svg_renames, &image_renames);
    let mut images_processed = images_processed + svg_rasterized;

    // Phase 1: transform every spine chapter, collecting per-chapter anchor
    // aliases into one book-wide map and each chapter's lifted `<style>` CSS.
    //
    // Inline `<svg>` elements and rasterized tables are handled here too, before
    // ref rewriting, so any `<img>` they become is repointed/stripped like the
    // rest. Each produces a NEW resource; these are buffered and added to the
    // book after the loop, which borrows `book` immutably throughout.
    let mut reserved: HashSet<String> = book.resources.keys().cloned().collect();
    let mut generated_resources: Vec<(String, Resource)> = Vec::new();
    let mut inline_svg_rasterized = 0u32;
    let mut tables_rasterized = 0u32;
    let mut chapters: Vec<(String, NodeRef)> = Vec::new();
    let mut aliases: AliasMap = HashMap::new();
    let mut relocated_styles: Vec<(String, String)> = Vec::new();
    // 1-based index over chapters that actually yielded non-empty head/body
    // `<style>` CSS, used to scope each contributor's relocated rules to its own
    // chapter (see [`css::scope`]).
    let mut relocated_scope_idx = 0usize;
    for path in &book.spine {
        let Some(resource) = book.resources.get(path) else {
            continue;
        };
        if !is_xhtml(&resource.media_type) {
            continue;
        }
        let doc = parse_xhtml(&resource.data)?;
        if !opts.filters.is_empty() {
            filter::apply_chapter_filters(&doc, &opts.filters, &mut transformations, path);
        }
        let ChapterTransform {
            aliases: chapter_aliases,
            extracted_css,
        } = transform_chapter(&doc, opts, &mut transformations, &mut warnings, path);
        aliases.extend(chapter_aliases);
        if !extracted_css.trim().is_empty() {
            // Scope this chapter's lifted CSS to itself (tagging its own DOM) so
            // it cannot bleed onto other chapters via the shared external sheet.
            // The counter advances per contributing chapter even if every rule
            // dies, keeping each chapter's scope index stable.
            relocated_scope_idx += 1;
            let scoped = css::scope::scope_relocated_css(
                &doc,
                &extracted_css,
                relocated_scope_idx,
                path,
                &mut warnings,
            );
            if !scoped.is_empty() {
                relocated_styles.push((path.clone(), scoped));
            }
        }
        remove_font_links(&doc, path, &stripped_fonts);
        if opts.features.rasterize_svg {
            inline_svg_rasterized += rasterize_inline_svgs(
                &doc,
                path,
                opts,
                &mut reserved,
                &mut generated_resources,
                &mut transformations,
                &mut warnings,
            );
        }
        if opts.features.linearize_tables {
            tables_rasterized += rasterize_tables(
                &doc,
                path,
                opts,
                &mut reserved,
                &mut generated_resources,
                &mut transformations,
                &mut warnings,
            );
        }
        crate::image::rewrite_refs(&doc, &parent_dir(path), &renames, &splits);
        chapters.push((path.clone(), doc));
    }

    // Add the resources generated inside the chapter loop (inline vector `<svg>`
    // rasters and rendered-table images) now that its borrow of `book` has
    // ended. They are already device-encoded, so the raster pass (which ran
    // earlier) never sees them.
    for (path, resource) in generated_resources {
        book.resources.insert(path, resource);
    }
    images_processed += inline_svg_rasterized + tables_rasterized;

    // Relocate the lifted head/body `<style>` CSS: concatenate it (marked by
    // source chapter), filter it, and - if anything survives - write it to an
    // external sheet the device does read, linked from each contributing chapter.
    if !relocated_styles.is_empty() {
        let mut combined = String::new();
        for (path, css) in &relocated_styles {
            combined.push_str(&format!("/* from {path} */\n"));
            combined.push_str(css);
            combined.push('\n');
        }
        let relocated_path = join_dir(&parent_dir(&book.opf_path), "et-relocated.css");
        // Only a filtering profile runs the relocated CSS through the subset
        // filter; otherwise it is relocated verbatim.
        let filtered = if opts.features.filter_css {
            filter_css(&combined, &relocated_path, &mut warnings)
        } else {
            FilteredCss {
                css: combined.clone(),
                rules_kept: 0,
                rules_dropped: 0,
                decls_dropped: 0,
            }
        };
        if !filtered.css.is_empty() {
            total_css_rules += filtered.rules_kept;
            for (path, _) in &relocated_styles {
                if let Some((_, doc)) = chapters.iter().find(|(p, _)| p == path)
                    && let Some(head) = find_head(doc)
                {
                    let href = relative_href(&parent_dir(path), &relocated_path);
                    head.append(element(
                        "link",
                        &[("rel", "stylesheet"), ("type", "text/css"), ("href", &href)],
                    ));
                    transformations.push(Transformation {
                        kind: "head-style-relocated".to_string(),
                        detail: "moved head/body <style> CSS into et-relocated.css".to_string(),
                        file: Some(path.clone()),
                    });
                }
            }
            store_filtered_css(
                &mut book,
                &relocated_path,
                filtered,
                opts,
                &mut transformations,
                &mut warnings,
            );
        }
    }

    // Phase 2: fix up cross-document references, then enforce the anchor cap
    // using the set of ids actually referenced anywhere in the book.
    if opts.features.relocate_anchors {
        for (path, doc) in &chapters {
            apply_anchor_aliases(doc, &aliases, path);
        }
        // The TOC (nav.xhtml/toc.ncx, regenerated verbatim from `book.toc`)
        // must follow the same relocations before the cap-protection pass
        // below reads `book.toc`'s fragments as "referenced" ids.
        apply_toc_aliases(&mut book.toc, &aliases);
        let referenced = referenced_fragments(&chapters, &book);
        for (path, doc) in &chapters {
            cap_ids(doc, &referenced, &mut warnings, path);
        }
    }

    // Add the generated stylesheet (and link it) for any chapter that gained a
    // helper class, then serialize the transformed chapters back out.
    let css_path = join_dir(&parent_dir(&book.opf_path), "et-styles.css");
    let mut css_needed = false;
    for (path, doc) in &chapters {
        if doc_has_cp_class(doc)
            && let Some(head) = find_head(doc)
        {
            css_needed = true;
            let href = relative_href(&parent_dir(path), &css_path);
            head.append(element(
                "link",
                &[("rel", "stylesheet"), ("type", "text/css"), ("href", &href)],
            ));
        }
    }
    if css_needed {
        // et-styles.css is already device-conformant, so it is not filtered, but
        // its rules still count toward the book-wide rule cap.
        total_css_rules += ET_STYLES.matches('}').count();
        book.resources.insert(
            css_path.clone(),
            Resource {
                data: ET_STYLES.as_bytes().to_vec(),
                media_type: "text/css".to_string(),
            },
        );
        transformations.push(Transformation {
            kind: "stylesheet-added".to_string(),
            detail: "added et-styles.css for the generated helper classes".to_string(),
            file: Some(css_path),
        });
    }

    if opts.features.filter_css
        && let Some(warning) = caps::rule_cap_warning(total_css_rules, opts.device.css_max_rules)
    {
        warnings.push(warning);
    }

    // Split any spine chapter over the device's per-file byte cap into
    // `<stem>-1.xhtml`, `<stem>-2.xhtml`, ... parts at block boundaries,
    // retargeting the spine, table of contents and every chapter's internal
    // hrefs so nothing dangles. Runs last among the content transforms, once
    // every chapter's final DOM (anchors, images, CSS links) is settled.
    let chapters_split = if opts.features.chapter_split {
        chapter_split::split_oversize_chapters(
            &mut book,
            &mut chapters,
            opts,
            &mut transformations,
            &mut warnings,
        )
    } else {
        0
    };

    for (path, doc) in &chapters {
        let bytes = serialize_xhtml(doc);
        book.resources.insert(
            path.clone(),
            Resource {
                data: bytes,
                media_type: "application/xhtml+xml".to_string(),
            },
        );
    }

    // Non-spine XHTML (e.g. the nav document): normalize to strict XHTML only.
    // Computed from the (possibly split-expanded) spine so a chapter's split
    // parts are never mistaken for a non-spine document.
    let spine: HashSet<&str> = book.spine.iter().map(String::as_str).collect();
    let non_spine: Vec<String> = book
        .resources
        .iter()
        .filter(|(path, resource)| !spine.contains(path.as_str()) && is_xhtml(&resource.media_type))
        .map(|(path, _)| path.clone())
        .collect();
    for path in non_spine {
        let doc = parse_xhtml(&book.resources[&path].data)?;
        crate::image::rewrite_refs(&doc, &parent_dir(&path), &renames, &splits);
        let bytes = serialize_xhtml(&doc);
        let resource = &mut book.resources[&path];
        resource.data = bytes;
        resource.media_type = "application/xhtml+xml".to_string();
    }

    // The writer regenerates the OPF, nav document and NCX from
    // `book.metadata` and `book.toc`, which chapter-text hygiene never sees -
    // normalize them under the same feature, or a decomposed TOC title keeps
    // the `encoding` lint finding alive through every repair run.
    if opts.features.unicode_hygiene {
        normalize_model_strings(&mut book, &mut transformations);
    }

    let chapter_count = book.spine.len() as u32;
    ensure_output_wellformed(&book)?;
    let epub = write_epub(
        &book,
        opts.output_stamp.as_deref(),
        opts.output_profile.as_deref(),
    )?;
    let bytes_out = epub.len() as u64;

    // Our own output must never carry a structural error the device would
    // choke on that we did not already warn the user about - only checked in
    // debug builds, since a full lint pass on every release conversion would
    // be wasted work once this is trusted. "Already warned about" matters: a
    // handful of transforms (e.g. a malformed source SVG) deliberately leave
    // one bad resource untouched, with a warning, rather than fail the whole
    // conversion - that is by design, not a regression, so a lint finding
    // whose path matches an existing warning's file is not treated as a bug.
    #[cfg(debug_assertions)]
    {
        // The firmware cannot render a `<table>` at all, so - in every table
        // mode - none may survive a table-linearizing profile: they are
        // linearized or rasterized away, and the `data-et-table-render`
        // sentinel goes with them.
        if opts.features.linearize_tables {
            for (path, doc) in &chapters {
                debug_assert!(
                    collect_by_name(doc, "table").is_empty(),
                    "convert() left a <table> in {path}"
                );
            }
        }
        let findings = validate::lint_epub(&epub, &opts.device, &opts.features);
        let warned_paths: HashSet<&str> =
            warnings.iter().filter_map(|w| w.file.as_deref()).collect();
        let unexplained_errors: Vec<&validate::LintFinding> = findings
            .iter()
            .filter(|f| f.severity == validate::Severity::Error)
            .filter(|f| !f.path.as_deref().is_some_and(|p| warned_paths.contains(p)))
            .collect();
        debug_assert!(
            unexplained_errors.is_empty(),
            "convert() produced an EPUB with lint errors not covered by any warning: \
             {unexplained_errors:?}"
        );
    }

    let warning_count = warnings.len() as u32;
    let report = ConvertReport {
        transformations,
        warnings,
        stats: ConvertStats {
            bytes_in,
            bytes_out,
            images_processed,
            chapters: chapter_count,
            chapters_split,
            warnings: warning_count,
        },
    };

    let epub = if opts.dry_run { Vec::new() } else { epub };
    Ok(Converted { epub, report })
}

/// Whether a media type is an (X)HTML content document.
fn is_xhtml(media_type: &str) -> bool {
    media_type == "application/xhtml+xml" || media_type == "text/html"
}

/// Refuse to ship a content document that is not well-formed XML.
///
/// The serializer guarantees well-formedness by construction; this gate turns
/// any future serializer bug into a hard conversion error instead of a
/// corrupted book. It runs in release builds too: 0.4.0/0.4.1 wrote malformed
/// `:xmlns` attributes into whole libraries precisely because the only output
/// check stood behind `#[cfg(debug_assertions)]`. One strict parse per
/// content document is trivial next to the parsing and image work already
/// done, and because every write path (single file, batch, in-place) only
/// touches disk on `Ok`, a failure here can never overwrite an original.
fn ensure_output_wellformed(book: &Book) -> Result<(), ConvertError> {
    for (path, resource) in &book.resources {
        if !is_xhtml(&resource.media_type) {
            continue;
        }
        let text = String::from_utf8_lossy(&resource.data);
        let options = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        };
        // roxmltree checks XML 1.0 well-formedness; the QName scan catches
        // what it tolerates but namespace-aware reader parsers reject (the
        // 0.4.0/0.4.1 bug wrote `:xmlns`, which roxmltree swallows whole).
        let detail = match roxmltree::Document::parse_with_options(&text, options) {
            Err(e) => Some(e.to_string()),
            Ok(_) => validate::find_invalid_qname(&text)
                .map(|(name, line)| format!("name '{name}' on line {line} is not a valid QName")),
        };
        if let Some(detail) = detail {
            return Err(ConvertError::MalformedOutput {
                path: path.clone(),
                detail,
            });
        }
    }
    Ok(())
}

/// NFC-normalize and sanitize every model string the writer regenerates into
/// the OPF, nav document and NCX: metadata display fields and TOC titles.
/// Without this, chapter-text hygiene alone never converges - a decomposed
/// TOC title or `dc:title` reappears verbatim in the regenerated files and
/// `lint_epub` flags them again after every repair run.
///
/// Deliberately untouched: the unique identifier (reading systems key their
/// library and bookmarks off its exact bytes) and every href/path (they must
/// keep byte-matching zip entry names and fragment ids).
fn normalize_model_strings(book: &mut Book, transformations: &mut Vec<Transformation>) {
    fn clean(s: &mut String) -> bool {
        let cleaned = html::clean_string(s);
        if cleaned == *s {
            return false;
        }
        *s = cleaned;
        true
    }
    fn clean_opt(s: &mut Option<String>) -> bool {
        s.as_mut().is_some_and(clean)
    }
    fn clean_creators(creators: &mut [Creator]) -> bool {
        let mut changed = false;
        for creator in creators {
            changed |= clean(&mut creator.name);
            changed |= clean_opt(&mut creator.file_as);
            changed |= clean_opt(&mut creator.role);
        }
        changed
    }

    let m = &mut book.metadata;
    let mut changed_fields: Vec<&str> = Vec::new();
    if clean(&mut m.title) {
        changed_fields.push("title");
    }
    if clean_creators(&mut m.authors) {
        changed_fields.push("authors");
    }
    if clean_creators(&mut m.contributors) {
        changed_fields.push("contributors");
    }
    if clean(&mut m.language) {
        changed_fields.push("language");
    }
    if m.identifiers.iter_mut().fold(false, |changed, id| {
        changed | clean(&mut id.value) | clean_opt(&mut id.scheme)
    }) {
        changed_fields.push("identifiers");
    }
    if clean_opt(&mut m.description) {
        changed_fields.push("description");
    }
    if clean_opt(&mut m.publisher) {
        changed_fields.push("publisher");
    }
    if m.subjects.iter_mut().fold(false, |c, s| c | clean(s)) {
        changed_fields.push("subjects");
    }
    if clean_opt(&mut m.date) {
        changed_fields.push("date");
    }
    if clean_opt(&mut m.rights) {
        changed_fields.push("rights");
    }
    if m.series
        .as_mut()
        .is_some_and(|s| clean(&mut s.name) | clean_opt(&mut s.index))
    {
        changed_fields.push("series");
    }
    for field in changed_fields {
        transformations.push(Transformation {
            kind: "metadata-nfc".to_string(),
            detail: format!("normalized {field} to NFC and stripped invalid characters"),
            file: None,
        });
    }

    let mut toc_changed = 0usize;
    for entry in &mut book.toc {
        if clean(&mut entry.title) {
            toc_changed += 1;
        }
    }
    if toc_changed > 0 {
        transformations.push(Transformation {
            kind: "toc-nfc".to_string(),
            detail: format!("normalized {toc_changed} table-of-contents title(s) to NFC"),
            file: None,
        });
    }
}

/// Remove every embedded font resource from `book`, returning the set of
/// stripped zip paths. The device never loads embedded fonts, and `@font-face`
/// is already dropped by the CSS filter, so the bytes are pure waste.
/// Embed a supplied cover image and point the book at it.
///
/// Placed next to the package document so it lands inside the content
/// directory, and given a name that cannot collide with an existing resource.
/// The image itself then flows through the normal pipeline: under a device
/// profile it is fitted to the panel and re-encoded like any other cover.
fn set_cover(book: &mut Book, cover: &CoverImage, transformations: &mut Vec<Transformation>) {
    let dir = parent_dir(&book.opf_path);
    let mut path = join_dir(&dir, &cover.file_name);
    // Never clobber a resource that is already there.
    let mut n = 2;
    while book.resources.contains_key(&path) && book.cover.as_deref() != Some(path.as_str()) {
        let (stem, ext) = cover
            .file_name
            .rsplit_once('.')
            .unwrap_or((cover.file_name.as_str(), ""));
        let name = if ext.is_empty() {
            format!("{stem}-{n}")
        } else {
            format!("{stem}-{n}.{ext}")
        };
        path = join_dir(&dir, &name);
        n += 1;
    }

    book.resources.insert(
        path.clone(),
        Resource {
            data: cover.data.clone(),
            media_type: cover.media_type.clone(),
        },
    );
    let replaced = book.cover.is_some();
    book.cover = Some(path.clone());
    transformations.push(Transformation {
        kind: "cover-set".to_string(),
        detail: if replaced {
            "replaced the cover with the supplied image".to_string()
        } else {
            "added the supplied cover image".to_string()
        },
        file: Some(path),
    });
}

fn strip_fonts(
    book: &mut Book,
    transformations: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
) -> HashSet<String> {
    let font_paths: Vec<String> = book
        .resources
        .iter()
        .filter(|(path, resource)| is_font(path, &resource.media_type))
        .map(|(path, _)| path.clone())
        .collect();
    if font_paths.is_empty() {
        return HashSet::new();
    }
    for path in &font_paths {
        book.resources.shift_remove(path);
    }
    transformations.push(Transformation {
        kind: "fonts-stripped".to_string(),
        detail: format!("removed {} embedded font file(s)", font_paths.len()),
        file: None,
    });
    warnings.push(Warning {
        message: format!(
            "stripped {} embedded font file(s) the device cannot load: {}",
            font_paths.len(),
            font_paths.join(", ")
        ),
        file: None,
    });
    font_paths.into_iter().collect()
}

/// Whether a resource is an embedded font, by media type or file extension.
/// Also used by `validate`'s `fonts` lint.
pub(crate) fn is_font(path: &str, media_type: &str) -> bool {
    let media_type = media_type.to_ascii_lowercase();
    if media_type.starts_with("font/")
        || media_type == "application/vnd.ms-opentype"
        || media_type == "application/font-woff"
        || media_type.starts_with("application/x-font-")
    {
        return true;
    }
    let path = path.to_ascii_lowercase();
    [".ttf", ".otf", ".woff", ".woff2", ".eot"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

/// Detach any `<link>` in `doc` whose href resolves to a stripped font path.
fn remove_font_links(doc: &NodeRef, chapter_path: &str, stripped_fonts: &HashSet<String>) {
    if stripped_fonts.is_empty() {
        return;
    }
    let chapter_dir = parent_dir(chapter_path);
    for link in collect_by_name(doc, "link") {
        let Some(href) = get_attr(&link, "href") else {
            continue;
        };
        let path_part = href.split('#').next().unwrap_or(&href);
        if stripped_fonts.contains(&normalize_href(&chapter_dir, path_part)) {
            link.detach();
        }
    }
}

/// The outcome of running the image pipeline over every raster in the book.
struct ImagePlan {
    /// Old zip-absolute image path -> its new path (only when the path changed).
    renames: HashMap<String, String>,
    /// Old zip-absolute image path -> its ordered page-tile paths.
    splits: HashMap<String, Vec<String>>,
    /// How many source images were transformed (processed or split).
    images_processed: u32,
}

/// Process every raster resource: transcode, fit and re-encode it, renaming the
/// resource when its output format (extension) changed and splitting tall images
/// when requested. Updates `book.resources` and `book.cover` in place and
/// returns the rename/split maps that drive per-chapter reference rewriting.
fn process_images(
    book: &mut Book,
    opts: &ConvertOptions,
    skip: &HashSet<String>,
    transformations: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
) -> ImagePlan {
    let mut renames = HashMap::new();
    let mut splits = HashMap::new();
    let mut images_processed = 0u32;

    let candidates: Vec<String> = book
        .resources
        .iter()
        .filter(|(path, resource)| {
            !skip.contains(path.as_str())
                && is_image_candidate(&resource.media_type, &resource.data)
        })
        .map(|(path, _)| path.clone())
        .collect();

    // Reserve every existing resource path so a rename or tile never collides.
    let mut reserved: HashSet<String> = book.resources.keys().cloned().collect();

    for path in candidates {
        let data = book.resources[&path].data.clone();
        let role = if book.cover.as_deref() == Some(path.as_str()) {
            ImageRole::Cover
        } else {
            ImageRole::Inline
        };
        let result = process_for_convert(
            &data,
            role,
            &opts.device,
            opts.jpeg_quality,
            opts.split_tall_images,
            warnings,
            &path,
        );
        match result {
            // Left byte-for-byte in place; the pipeline already warned.
            PipelineResult::Single(ImageOutcome::Unchanged { .. }) => {}
            PipelineResult::Single(ImageOutcome::Processed {
                data: out,
                format,
                note,
                ..
            }) => {
                let new_path = with_extension(&path, format.ext());
                if new_path == path {
                    let resource = &mut book.resources[&path];
                    resource.data = out;
                    resource.media_type = format.media_type().to_string();
                } else {
                    let unique = reserve_unique(
                        &mut reserved,
                        &parent_dir(&path),
                        stem_of(&path),
                        format.ext(),
                    );
                    book.resources.shift_remove(&path);
                    book.resources.insert(
                        unique.clone(),
                        Resource {
                            data: out,
                            media_type: format.media_type().to_string(),
                        },
                    );
                    if book.cover.as_deref() == Some(path.as_str()) {
                        book.cover = Some(unique.clone());
                    }
                    renames.insert(path.clone(), unique);
                }
                transformations.push(Transformation {
                    kind: "image-optimized".to_string(),
                    detail: format!("{} {note}", basename(&path)),
                    file: Some(path.clone()),
                });
                images_processed += 1;
            }
            PipelineResult::Split(tiles) => {
                let tile_count = tiles.len();
                let dir = parent_dir(&path);
                let stem = stem_of(&path).to_string();
                book.resources.shift_remove(&path);
                let mut tile_paths = Vec::with_capacity(tile_count);
                for (index, tile) in tiles.into_iter().enumerate() {
                    let tile_stem = format!("{stem}-p{}", index + 1);
                    let unique = reserve_unique(&mut reserved, &dir, &tile_stem, "jpg");
                    book.resources.insert(
                        unique.clone(),
                        Resource {
                            data: tile.data,
                            media_type: "image/jpeg".to_string(),
                        },
                    );
                    tile_paths.push(unique);
                }
                if book.cover.as_deref() == Some(path.as_str())
                    && let Some(first) = tile_paths.first()
                {
                    book.cover = Some(first.clone());
                }
                transformations.push(Transformation {
                    kind: "image-split".to_string(),
                    detail: format!("{} split into {tile_count} page tile(s)", basename(&path)),
                    file: Some(path.clone()),
                });
                splits.insert(path.clone(), tile_paths);
                images_processed += 1;
            }
        }
    }

    ImagePlan {
        renames,
        splits,
        images_processed,
    }
}

/// The outcome of the SVG pass.
struct SvgPlan {
    /// Original SVG path -> the raster resource that replaced it. For a wrapper
    /// this is its unwrapped payload; for vector art its rasterized output.
    renames: HashMap<String, String>,
    /// Rasterized-vector outputs: already device-encoded, so the raster pass
    /// must not process them again.
    finalized: HashSet<String>,
    /// How many SVG resources were rasterized (added to `images_processed`).
    rasterized: u32,
}

/// What to do with one SVG resource.
enum SvgAction {
    /// Drop the SVG and repoint refs at this existing raster resource.
    UnwrapHref(String),
    /// Extract this decoded raster payload (`ext`, `media_type`) as a resource.
    Extract(Vec<u8>, &'static str, &'static str),
    /// Not a wrapper (or a broken one): rasterize the vector art.
    Rasterize,
}

/// Handle every `image/svg+xml` resource: unwrap single-`<image>` wrappers and
/// rasterize real vector art, updating `book.resources`/`book.cover` in place and
/// returning the maps that drive reference rewriting. Malformed SVGs are left
/// byte-for-byte unchanged (with a warning) so one bad image never fails a
/// conversion.
fn process_svgs(
    book: &mut Book,
    opts: &ConvertOptions,
    transformations: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
) -> SvgPlan {
    let mut renames = HashMap::new();
    let mut finalized = HashSet::new();
    let mut rasterized = 0u32;

    let svg_paths: Vec<String> = book
        .resources
        .iter()
        .filter(|(_, resource)| resource.media_type == "image/svg+xml")
        .map(|(path, _)| path.clone())
        .collect();

    // Reserve every existing path so a new payload/raster never collides.
    let mut reserved: HashSet<String> = book.resources.keys().cloned().collect();

    for path in svg_paths {
        let data = book.resources[&path].data.clone();
        let svg = String::from_utf8_lossy(&data);
        let dir = parent_dir(&path);
        let is_cover = book.cover.as_deref() == Some(path.as_str());

        let action = match crate::image::svg::as_image_wrapper(&svg) {
            Some(WrapperTarget::Href(href)) => {
                let target = normalize_href(&dir, path_part(&href));
                if book.resources.contains_key(&target) {
                    SvgAction::UnwrapHref(target)
                } else {
                    // Wrapper points at nothing we hold (missing or an
                    // external URL); render it instead, which - since the
                    // frame is normally empty apart from the wrapped image -
                    // usually yields a blank raster. Warn so that is not
                    // silent.
                    warnings.push(Warning {
                        message: format!(
                            "{} wraps {href} which could not be resolved to a book resource - \
                             rasterizing the SVG frame instead, likely producing a blank image",
                            basename(&path)
                        ),
                        file: Some(path.clone()),
                    });
                    SvgAction::Rasterize
                }
            }
            Some(WrapperTarget::DataUri(bytes)) => match sniff_raster_kind(&bytes) {
                Some((ext, media_type)) => SvgAction::Extract(bytes, ext, media_type),
                None => SvgAction::Rasterize,
            },
            None => SvgAction::Rasterize,
        };

        match action {
            SvgAction::UnwrapHref(target) => {
                book.resources.shift_remove(&path);
                if is_cover {
                    book.cover = Some(target.clone());
                }
                renames.insert(path.clone(), target.clone());
                transformations.push(Transformation {
                    kind: "svg-unwrapped".to_string(),
                    detail: format!(
                        "{} wraps {} - dropped the SVG frame",
                        basename(&path),
                        basename(&target)
                    ),
                    file: Some(path.clone()),
                });
            }
            SvgAction::Extract(bytes, ext, media_type) => {
                let new_path = reserve_unique(&mut reserved, &dir, stem_of(&path), ext);
                book.resources.shift_remove(&path);
                book.resources.insert(
                    new_path.clone(),
                    Resource {
                        data: bytes,
                        media_type: media_type.to_string(),
                    },
                );
                if is_cover {
                    book.cover = Some(new_path.clone());
                }
                renames.insert(path.clone(), new_path.clone());
                transformations.push(Transformation {
                    kind: "svg-unwrapped".to_string(),
                    detail: format!(
                        "{} wraps an embedded {} - extracted to {}",
                        basename(&path),
                        ext,
                        basename(&new_path)
                    ),
                    file: Some(path.clone()),
                });
            }
            SvgAction::Rasterize => {
                // A cover gets the full cover box; anything else the inline box.
                let max_box = if is_cover {
                    opts.device.cover_max
                } else {
                    opts.device.inline_max
                };
                let Some(Rasterized {
                    image,
                    intrinsic_w,
                    intrinsic_h,
                }) = crate::image::svg::rasterize_sized(&svg, max_box, warnings, &path)
                else {
                    // Malformed: keep the resource as-is (warning already recorded).
                    continue;
                };
                let role = if is_cover {
                    ImageRole::Cover
                } else {
                    ImageRole::Inline
                };
                let enc = encode_rendered(
                    image,
                    crate::image::svg::svg_render_hint(&svg),
                    role,
                    &opts.device,
                    opts.jpeg_quality,
                    warnings,
                    &path,
                );
                let detail = format!(
                    "{} vector {intrinsic_w}x{intrinsic_h} -> {}x{} {} {}KB",
                    basename(&path),
                    enc.width,
                    enc.height,
                    enc.format.label(),
                    kb(enc.data.len()),
                );
                let new_path =
                    reserve_unique(&mut reserved, &dir, stem_of(&path), enc.format.ext());
                book.resources.shift_remove(&path);
                book.resources.insert(
                    new_path.clone(),
                    Resource {
                        data: enc.data,
                        media_type: enc.format.media_type().to_string(),
                    },
                );
                finalized.insert(new_path.clone());
                if is_cover {
                    book.cover = Some(new_path.clone());
                }
                renames.insert(path.clone(), new_path.clone());
                transformations.push(Transformation {
                    kind: "svg-rasterized".to_string(),
                    detail,
                    file: Some(path.clone()),
                });
                rasterized += 1;
            }
        }
    }

    SvgPlan {
        renames,
        finalized,
        rasterized,
    }
}

/// Rasterize inline `<svg>` elements in one chapter document, in place.
///
/// A wrapper `<svg>` (single `<image>`) becomes a plain `<img>` pointing at the
/// same href (later repointed by the rename map); real vector art is rasterized
/// into a NEW `<chapter-stem>-svg-N.<ext>` resource - buffered into
/// `new_resources` for the caller to insert - and the `<svg>` becomes an `<img>`
/// referencing it. Malformed inline SVGs are left in place. Returns how many
/// inline vectors were rasterized.
fn rasterize_inline_svgs(
    doc: &NodeRef,
    chapter_path: &str,
    opts: &ConvertOptions,
    reserved: &mut HashSet<String>,
    new_resources: &mut Vec<(String, Resource)>,
    transformations: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
) -> u32 {
    let chapter_dir = parent_dir(chapter_path);
    let chapter_stem = stem_of(chapter_path).to_string();
    let mut rasterized = 0u32;
    let mut counter = 0u32;

    for svg_node in collect_by_name(doc, "svg") {
        // A nested `<svg>` inside an outer one already handled is now detached;
        // skip anything no longer attached to the live document.
        if !is_attached(&svg_node) {
            continue;
        }
        let svg_str = crate::image::svg::serialize_svg_subtree(&svg_node);

        if let Some(WrapperTarget::Href(href)) = crate::image::svg::as_image_wrapper(&svg_str) {
            replace_with(&svg_node, vec![element("img", &[("src", &href)])]);
            transformations.push(Transformation {
                kind: "svg-unwrapped".to_string(),
                detail: format!("inline SVG wraps {href} - replaced with an <img>"),
                file: Some(chapter_path.to_string()),
            });
            continue;
        }

        let Some(Rasterized {
            image,
            intrinsic_w,
            intrinsic_h,
        }) = crate::image::svg::rasterize_sized(
            &svg_str,
            opts.device.inline_max,
            warnings,
            chapter_path,
        )
        else {
            // Malformed inline SVG: leave it as-is (warning already recorded).
            continue;
        };
        let enc = encode_rendered(
            image,
            crate::image::svg::svg_render_hint(&svg_str),
            ImageRole::Inline,
            &opts.device,
            opts.jpeg_quality,
            warnings,
            chapter_path,
        );
        counter += 1;
        let new_path = reserve_unique(
            reserved,
            &chapter_dir,
            &format!("{chapter_stem}-svg-{counter}"),
            enc.format.ext(),
        );
        let detail = format!(
            "inline vector {intrinsic_w}x{intrinsic_h} -> {}x{} {} {}KB",
            enc.width,
            enc.height,
            enc.format.label(),
            kb(enc.data.len()),
        );
        let src = relative_href(&chapter_dir, &new_path);
        replace_with(&svg_node, vec![element("img", &[("src", &src)])]);
        new_resources.push((
            new_path,
            Resource {
                data: enc.data,
                media_type: enc.format.media_type().to_string(),
            },
        ));
        transformations.push(Transformation {
            kind: "svg-rasterized".to_string(),
            detail,
            file: Some(chapter_path.to_string()),
        });
        rasterized += 1;
    }

    rasterized
}

/// Render every table marked `data-et-table-render` in one chapter to a
/// rasterized image, in place.
///
/// Mirrors [`rasterize_inline_svgs`]: each marked table is collapsed to a
/// [`html::table_render::TableModel`], laid out as a deterministic SVG, and
/// rasterized/encoded through the shared image path into a NEW
/// `<chapter-stem>-table-N.<ext>` resource - buffered into `new_resources` for
/// the caller to insert - with the `<table>` (and everything nested in it)
/// replaced by a single `<p class="et-img"><img/></p>`. On any
/// rasterize/encode failure the sentinel is removed and the table is linearized
/// instead (plus a warning): a `<table>` must never survive to serialization.
/// Returns how many tables were rasterized.
fn rasterize_tables(
    doc: &NodeRef,
    chapter_path: &str,
    opts: &ConvertOptions,
    reserved: &mut HashSet<String>,
    new_resources: &mut Vec<(String, Resource)>,
    transformations: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
) -> u32 {
    let chapter_dir = parent_dir(chapter_path);
    let chapter_stem = stem_of(chapter_path).to_string();
    let mut rasterized = 0u32;
    let mut counter = 0u32;

    for table in collect_by_name(doc, "table") {
        if get_attr(&table, "data-et-table-render").as_deref() != Some("1") {
            continue;
        }
        counter += 1;
        let model = html::tables::build_table_model(&table, chapter_path, warnings);
        let (rows, cols) = model_dimensions(&model);
        let nested = if model.rows.iter().flatten().any(|c| c.sub.is_some()) {
            " (nested)"
        } else {
            ""
        };
        let svg = html::table_render::render_table_svg(&model, opts.device.inline_max.0);

        let Some(Rasterized { image, .. }) = crate::image::svg::rasterize_sized(
            &svg,
            opts.device.inline_max,
            warnings,
            chapter_path,
        ) else {
            // Rendering a table we built ourselves should not fail; if it
            // somehow does, drop the sentinel and fall back to linearization so
            // no `<table>` reaches the output.
            remove_attr(&table, "data-et-table-render");
            html::tables::linearize_table_node(
                &table,
                transformations,
                chapter_path,
                Some("table rasterization failed"),
            );
            warnings.push(Warning {
                message: format!(
                    "could not rasterize a table in {chapter_path}; linearized it instead"
                ),
                file: Some(chapter_path.to_string()),
            });
            continue;
        };

        // A rendered table is black text and rules on white by construction:
        // encode it as crisp line-art PNG, never a photo-classified JPEG.
        let enc = encode_rendered(
            image,
            RenderHint::LineArt,
            ImageRole::Inline,
            &opts.device,
            opts.jpeg_quality,
            warnings,
            chapter_path,
        );
        let detail = format!(
            "{rows}x{cols} table{nested} -> {}x{} {} {}KB",
            enc.width,
            enc.height,
            enc.format.label(),
            kb(enc.data.len()),
        );
        let new_path = reserve_unique(
            reserved,
            &chapter_dir,
            &format!("{chapter_stem}-table-{counter}"),
            enc.format.ext(),
        );
        let alt = model
            .caption
            .clone()
            .unwrap_or_else(|| format!("Table {counter}"));
        let src = relative_href(&chapter_dir, &new_path);
        let img = element("img", &[("src", &src), ("alt", &alt)]);
        let para = element("p", &[("class", "et-img")]);
        para.append(img);
        replace_with(&table, vec![para]);
        new_resources.push((
            new_path,
            Resource {
                data: enc.data,
                media_type: enc.format.media_type().to_string(),
            },
        ));
        transformations.push(Transformation {
            kind: "table-rasterized".to_string(),
            detail,
            file: Some(chapter_path.to_string()),
        });
        rasterized += 1;
    }

    rasterized
}

/// Grid dimensions (rows including a header, columns) of a [`TableModel`], for
/// the "3x4 table -> ..." report line.
fn model_dimensions(model: &crate::html::table_render::TableModel) -> (usize, usize) {
    let mut cols = model.headers.as_ref().map_or(0, Vec::len);
    for row in &model.rows {
        cols = cols.max(row.len());
    }
    let rows = model.rows.len() + usize::from(model.headers.is_some());
    (rows, cols)
}

/// Whether `node` is still attached to its document root (as opposed to a
/// subtree already detached by an earlier replacement).
fn is_attached(node: &NodeRef) -> bool {
    let mut current = node.clone();
    loop {
        match current.parent() {
            Some(parent) => current = parent,
            None => {
                return matches!(
                    current.data(),
                    NodeData::Document(_) | NodeData::DocumentFragment
                );
            }
        }
    }
}

/// Compose the SVG and raster rename maps: an original SVG resolves through its
/// intermediate replacement to that replacement's final (raster-pass) name.
fn compose_renames(
    svg: &HashMap<String, String>,
    image: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut composed = image.clone();
    for (from, intermediate) in svg {
        let final_path = image
            .get(intermediate)
            .cloned()
            .unwrap_or_else(|| intermediate.clone());
        composed.insert(from.clone(), final_path);
    }
    composed
}

/// Bytes rounded to the nearest kibibyte, for report lines.
fn kb(bytes: usize) -> usize {
    (bytes + 512) / 1024
}

/// The path portion of an href, dropping any `#fragment` or `?query`.
fn path_part(href: &str) -> &str {
    let end = href.find(['#', '?']).unwrap_or(href.len());
    &href[..end]
}

/// The file name (last path segment) of a zip-absolute path.
fn basename(path: &str) -> &str {
    match path.rfind('/') {
        Some(idx) => &path[idx + 1..],
        None => path,
    }
}

/// The file name of a zip-absolute path without its extension.
fn stem_of(path: &str) -> &str {
    let name = basename(path);
    match name.rfind('.') {
        Some(idx) if idx > 0 => &name[..idx],
        _ => name,
    }
}

/// `path` with its extension replaced by `ext`, keeping directory and stem.
fn with_extension(path: &str, ext: &str) -> String {
    join_dir(&parent_dir(path), &format!("{}.{ext}", stem_of(path)))
}

/// A unique zip path `dir/stem.ext`, appending `-1`, `-2`, ... on collision and
/// reserving the chosen path so later images never reuse it.
fn reserve_unique(reserved: &mut HashSet<String>, dir: &str, stem: &str, ext: &str) -> String {
    let mut candidate = join_dir(dir, &format!("{stem}.{ext}"));
    let mut n = 1;
    while reserved.contains(&candidate) {
        candidate = join_dir(dir, &format!("{stem}-{n}.{ext}"));
        n += 1;
    }
    reserved.insert(candidate.clone());
    candidate
}

/// The zip path for the next available split part of `path`, starting the
/// search at `from_part` and bumping past any part number an existing
/// resource already occupies. A book that happens to already contain
/// `main-2.css` (its own unrelated resource) must not collide with a
/// generated split part of `main.css` - one of the two would silently
/// overwrite the other. Returns the chosen path and the part number used, so
/// the caller can resume the next search just past it.
fn next_unique_split_path(book: &Book, path: &str, from_part: usize) -> (String, usize) {
    let mut part = from_part;
    loop {
        let candidate = caps::split_path(path, part);
        if !book.resources.contains_key(&candidate) {
            return (candidate, part);
        }
        part += 1;
    }
}

/// Store one filtered stylesheet into `book` at `path`, splitting it into
/// `<stem>-2.css`, `<stem>-3.css`, ... when it exceeds the device's per-file
/// byte cap. All parts stay as `text/css` resources (the device zip-scans them
/// all). Inserting at an existing key updates it in place, preserving order.
/// Generated part names are uniquified against every existing resource key
/// (see [`next_unique_split_path`]), so a pre-existing same-named resource
/// bumps the generated name instead of one silently clobbering the other.
fn store_filtered_css(
    book: &mut Book,
    path: &str,
    filtered: FilteredCss,
    opts: &ConvertOptions,
    transformations: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
) {
    let chunks = caps::split_css(&filtered.css, opts.device.css_max_bytes);
    book.resources.insert(
        path.to_string(),
        Resource {
            data: chunks[0].clone().into_bytes(),
            media_type: "text/css".to_string(),
        },
    );
    let mut next_part = 2usize;
    for chunk in chunks.iter().skip(1) {
        let (part_path, used_part) = next_unique_split_path(book, path, next_part);
        book.resources.insert(
            part_path,
            Resource {
                data: chunk.clone().into_bytes(),
                media_type: "text/css".to_string(),
            },
        );
        next_part = used_part + 1;
    }
    if chunks.len() > 1 {
        transformations.push(Transformation {
            kind: "css-split".to_string(),
            detail: format!("split into {} files under the CSS byte cap", chunks.len()),
            file: Some(path.to_string()),
        });
        warnings.push(Warning {
            message: format!(
                "split {path} into {} files to stay under the {}-byte CSS cap",
                chunks.len(),
                opts.device.css_max_bytes
            ),
            file: Some(path.to_string()),
        });
    }
}

/// Whether any element in `doc` carries a generated `et-*` helper class.
fn doc_has_cp_class(doc: &NodeRef) -> bool {
    doc.inclusive_descendants().any(|node| {
        node.as_element().is_some_and(|e| {
            e.attributes
                .borrow()
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|cls| cls.starts_with("et-")))
        })
    })
}

/// Collect every fragment id referenced by an internal `<a href>` across the
/// transformed chapters, plus every fragment in the book's table of contents.
fn referenced_fragments(chapters: &[(String, NodeRef)], book: &Book) -> HashSet<String> {
    let mut referenced = HashSet::new();
    for (_, doc) in chapters {
        for anchor in doc.inclusive_descendants() {
            if let Some(elem) = anchor.as_element()
                && elem.name.local.as_ref() == "a"
                && let Some(href) = elem.attributes.borrow().get("href")
                && let Some((_, fragment)) = href.split_once('#')
            {
                referenced.insert(fragment.to_string());
            }
        }
    }
    for entry in &book.toc {
        if let Some((_, fragment)) = entry.href.split_once('#') {
            referenced.insert(fragment.to_string());
        }
    }
    referenced
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-resource book for exercising the output well-formedness gate.
    fn gate_book(data: &[u8], media_type: &str) -> crate::epub::model::Book {
        use crate::epub::model::{Book, Metadata, Resource};
        let mut resources = indexmap::IndexMap::new();
        resources.insert(
            "OEBPS/text/chapter1.xhtml".to_string(),
            Resource {
                data: data.to_vec(),
                media_type: media_type.to_string(),
            },
        );
        Book {
            metadata: Metadata::default(),
            resources,
            spine: vec!["OEBPS/text/chapter1.xhtml".to_string()],
            toc: Vec::new(),
            cover: None,
            opf_path: "OEBPS/content.opf".to_string(),
            nav_path: None,
            ncx_path: None,
        }
    }

    #[test]
    fn malformed_content_doc_fails_the_output_gate() {
        let book = gate_book(
            br#"<?xml version="1.0"?><html><body><svg :xmlns="x"></svg></body></html>"#,
            "application/xhtml+xml",
        );
        let err = ensure_output_wellformed(&book).expect_err("malformed output must be refused");
        assert_eq!(err.code(), "malformed-output");
        assert!(err.to_string().contains("chapter1.xhtml"), "got: {err}");
    }

    #[test]
    fn wellformed_content_doc_passes_the_output_gate() {
        let book = gate_book(
            br#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><body><p>ok</p></body></html>"#,
            "application/xhtml+xml",
        );
        ensure_output_wellformed(&book).expect("well-formed output must pass");
    }

    #[test]
    fn non_xhtml_resources_are_not_gated() {
        let book = gate_book(b"not xml at all { }", "text/css");
        ensure_output_wellformed(&book).expect("non-XHTML resources are not XML-checked");
    }

    /// A book whose metadata and TOC carry the given strings, for exercising
    /// model-string normalization.
    fn model_book(metadata: crate::epub::model::Metadata, toc: Vec<TocEntry>) -> Book {
        let mut book = gate_book(
            br#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><body><p>ok</p></body></html>"#,
            "application/xhtml+xml",
        );
        book.metadata = metadata;
        book.toc = toc;
        book
    }

    #[test]
    fn normalize_model_strings_precomposes_metadata_and_toc_titles() {
        use crate::epub::model::{Creator, Metadata, Series};

        let mut book = model_book(
            Metadata {
                title: "Die Ru\u{308}ckkehr der Jediritter".to_string(),
                authors: vec![Creator {
                    name: "Bjo\u{308}rn Borg".to_string(),
                    file_as: Some("Borg, Bjo\u{308}rn".to_string()),
                    role: None,
                }],
                subjects: vec!["Ma\u{308}rchen".to_string()],
                series: Some(Series {
                    name: "Jediritter-Bu\u{308}cher".to_string(),
                    index: Some("2".to_string()),
                }),
                ..Metadata::default()
            },
            vec![
                TocEntry {
                    title: "U\u{308}ber Endor".to_string(),
                    href: "OEBPS/text/chapter1.xhtml".to_string(),
                    level: 1,
                },
                TocEntry {
                    title: "Clean entry".to_string(),
                    href: "OEBPS/text/chapter1.xhtml#s2".to_string(),
                    level: 1,
                },
            ],
        );

        let mut transformations = Vec::new();
        normalize_model_strings(&mut book, &mut transformations);

        assert_eq!(book.metadata.title, "Die Rückkehr der Jediritter");
        assert_eq!(book.metadata.authors[0].name, "Björn Borg");
        assert_eq!(
            book.metadata.authors[0].file_as.as_deref(),
            Some("Borg, Björn")
        );
        assert_eq!(book.metadata.subjects[0], "Märchen");
        assert_eq!(
            book.metadata.series.as_ref().unwrap().name,
            "Jediritter-Bücher"
        );
        assert_eq!(book.toc[0].title, "Über Endor");
        assert_eq!(book.toc[1].title, "Clean entry");

        let metadata_details: Vec<&str> = transformations
            .iter()
            .filter(|t| t.kind == "metadata-nfc")
            .map(|t| t.detail.as_str())
            .collect();
        for field in ["title", "authors", "subjects", "series"] {
            assert!(
                metadata_details.iter().any(|d| d.contains(field)),
                "a metadata-nfc transformation must name {field}, got: {metadata_details:?}"
            );
        }
        assert_eq!(
            metadata_details.len(),
            4,
            "one push per changed field, got: {metadata_details:?}"
        );

        let toc_details: Vec<&str> = transformations
            .iter()
            .filter(|t| t.kind == "toc-nfc")
            .map(|t| t.detail.as_str())
            .collect();
        assert_eq!(toc_details.len(), 1, "got: {toc_details:?}");
        assert!(
            toc_details[0].contains("1 table-of-contents title"),
            "got: {toc_details:?}"
        );
    }

    #[test]
    fn normalize_model_strings_never_touches_identifier_or_hrefs() {
        use crate::epub::model::Metadata;

        // A decomposed unique identifier stays byte-exact: reading systems key
        // bookmarks off it. Hrefs must keep matching zip entry names.
        let identifier = "urn:custom:u\u{308}nique";
        let href = "OEBPS/text/u\u{308}ber.xhtml";
        let mut book = model_book(
            Metadata {
                title: "Clean Title".to_string(),
                identifier: Some(identifier.to_string()),
                ..Metadata::default()
            },
            vec![TocEntry {
                title: "Clean entry".to_string(),
                href: href.to_string(),
                level: 1,
            }],
        );

        let mut transformations = Vec::new();
        normalize_model_strings(&mut book, &mut transformations);

        assert_eq!(book.metadata.identifier.as_deref(), Some(identifier));
        assert_eq!(book.toc[0].href, href);
        assert!(
            transformations.is_empty(),
            "nothing normalizable changed, got: {transformations:?}"
        );
    }

    #[test]
    fn normalize_model_strings_is_a_noop_on_an_nfc_book() {
        use crate::epub::model::{Creator, Metadata};

        let mut book = model_book(
            Metadata {
                title: "Käpt'n Blaubär".to_string(),
                authors: vec![Creator::new("Jane Author")],
                description: Some("Eine Gutenachtgeschichte.".to_string()),
                ..Metadata::default()
            },
            vec![TocEntry {
                title: "Über Bord".to_string(),
                href: "OEBPS/text/chapter1.xhtml".to_string(),
                level: 1,
            }],
        );
        let before_title = book.metadata.title.clone();

        let mut transformations = Vec::new();
        normalize_model_strings(&mut book, &mut transformations);

        assert_eq!(book.metadata.title, before_title);
        assert!(
            transformations.is_empty(),
            "an already-NFC book records nothing, got: {transformations:?}"
        );
    }

    /// The generated helper stylesheet must already be device-conformant: the
    /// CSS filter is a no-op on it (and idempotent), which is why `convert` does
    /// not run it through the filter.
    #[test]
    fn et_styles_is_a_fixed_point_of_the_css_filter() {
        let mut warnings = Vec::new();
        let filtered = filter_css(ET_STYLES, "et-styles.css", &mut warnings);
        assert_eq!(
            filtered.css, ET_STYLES,
            "filter_css must not change et-styles.css"
        );
        assert!(
            warnings.is_empty(),
            "et-styles.css must filter cleanly: {warnings:?}"
        );
        // 12 helper rules, none dropped.
        assert_eq!(filtered.rules_kept, ET_STYLES.matches('}').count());
        assert_eq!(filtered.rules_dropped, 0);
        assert_eq!(filtered.decls_dropped, 0);
    }

    /// A book already carrying a `styles/main-2.css` resource unrelated to
    /// splitting; storing an oversized `styles/main.css` must not let its
    /// generated part silently overwrite (or be shadowed by) it.
    #[test]
    fn split_css_part_name_dodges_an_existing_resource_collision() {
        let mut resources = indexmap::IndexMap::new();
        resources.insert(
            "styles/main.css".to_string(),
            Resource {
                data: Vec::new(),
                media_type: "text/css".to_string(),
            },
        );
        resources.insert(
            "styles/main-2.css".to_string(),
            Resource {
                data: b"/* pre-existing, unrelated */".to_vec(),
                media_type: "text/css".to_string(),
            },
        );
        let mut book = Book {
            metadata: Metadata::default(),
            resources,
            spine: Vec::new(),
            toc: Vec::new(),
            cover: None,
            opf_path: "content.opf".to_string(),
            nav_path: None,
            ncx_path: None,
        };

        let opts = ConvertOptions {
            device: DeviceCaps {
                css_max_bytes: 10,
                ..DeviceCaps::x4()
            },
            ..ConvertOptions::default()
        };
        // Three rules, each individually over the 10-byte cap -> three chunks.
        let filtered = FilteredCss {
            css: "a{color:red}b{color:blue}c{color:green}".to_string(),
            rules_kept: 3,
            rules_dropped: 0,
            decls_dropped: 0,
        };
        let mut transformations = Vec::new();
        let mut warnings = Vec::new();
        store_filtered_css(
            &mut book,
            "styles/main.css",
            filtered,
            &opts,
            &mut transformations,
            &mut warnings,
        );

        // The pre-existing resource must survive untouched.
        assert_eq!(
            book.resources["styles/main-2.css"].data,
            b"/* pre-existing, unrelated */"
        );
        // The generated part must have landed at a distinct name, not
        // overwritten the pre-existing one.
        let generated: Vec<&str> = book
            .resources
            .keys()
            .filter(|k| k.as_str() != "styles/main.css" && k.as_str() != "styles/main-2.css")
            .map(String::as_str)
            .collect();
        assert_eq!(
            generated.len(),
            2,
            "both remaining split chunks should have landed at distinct names: {:?}",
            book.resources.keys().collect::<Vec<_>>()
        );
        for path in book.resources.keys() {
            assert!(
                path.ends_with(".css"),
                "every generated part must still be a .css resource: {path}"
            );
        }
    }

    #[test]
    fn is_font_matches_by_media_type_and_extension() {
        assert!(is_font("f.otf", "application/octet-stream"));
        assert!(is_font("f.woff2", "application/octet-stream"));
        assert!(is_font("x", "font/ttf"));
        assert!(is_font("x", "application/vnd.ms-opentype"));
        assert!(is_font("x", "application/x-font-ttf"));
        assert!(!is_font("cover.jpg", "image/jpeg"));
        assert!(!is_font("main.css", "text/css"));
    }
}
