//! The image pipeline: turn whatever raster an EPUB carries into something the
//! target device can actually decode and render well.
//!
//! Every raster is sniffed from its magic bytes (EPUBs lie about extensions),
//! decoded, uprighted per its EXIF orientation, flattened onto white, reduced to
//! the panel's color space, classified as line art or photo, contrast-stretched
//! (grayscale photos only), downscaled to the reading area, and re-encoded as a
//! baseline JPEG (photos) or an 8-bit PNG (line art) inside a byte budget.
//! Broken input is left untouched with a warning: one bad image must never fail
//! a whole conversion.
//!
//! The panel decides the color space (see [`canvas::Canvas`] and
//! [`crate::profile::Panel`]): a grayscale e-ink device gets 8-bit luma, which
//! is all it can show and a third of the bytes, while a Kaleido-class color
//! panel keeps its images in RGB the whole way through. Alpha is composited onto
//! white for both - no target device renders transparency, and Amazon says so
//! outright.
//!
//! Formats are chosen for the worst decoder in the family: the CrossPoint
//! firmware only reads baseline JPEG and 8-bit PNG (a progressive JPEG blurs;
//! GIF/WebP/TIFF/BMP/SVG render as nothing) and aborts past 2048x1536 px, so
//! everything is normalized to baseline JPEG or PNG regardless of target.
//!
//! SVG has no decoder on several of these devices; it is handled in [`svg`],
//! which either unwraps a single-`<image>` wrapper (letting its raster payload
//! flow through this pipeline) or rasterizes real vector art and hands the
//! result to [`encode_rendered`], reusing the same classify/encode/budget path.

mod autocontrast;
mod canvas;
mod classify;
mod encode;
mod resize;
mod split;
pub mod svg;

use std::collections::HashMap;
use std::io::Cursor;

use image::metadata::Orientation;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use kuchikiki::{NodeData, NodeRef};

use crate::epub::model::normalize_href;
use crate::epub::relative_href;
use crate::html::dom::{
    collect_by_name, element, get_attr, is_named, remove_attr, replace_with, set_attr,
};
use crate::profile::{DeviceCaps, Panel};
use crate::report::Warning;

use canvas::Canvas;

/// Whether an image is laid out inline (in the reading flow) or is the cover.
/// The two get different fit targets and byte budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageRole {
    /// An image in the reading flow.
    Inline,
    /// The book cover.
    Cover,
}

/// The output encoding chosen for a processed image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutFormat {
    /// Baseline grayscale JPEG (photos).
    Jpeg,
    /// 8-bit grayscale PNG (line art).
    Png,
}

impl OutFormat {
    /// The lowercase file extension (no dot) for this format.
    pub(crate) fn ext(self) -> &'static str {
        match self {
            OutFormat::Jpeg => "jpg",
            OutFormat::Png => "png",
        }
    }

    /// The media (MIME) type for this format.
    pub(crate) fn media_type(self) -> &'static str {
        match self {
            OutFormat::Jpeg => "image/jpeg",
            OutFormat::Png => "image/png",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            OutFormat::Jpeg => "jpeg",
            OutFormat::Png => "png",
        }
    }
}

/// The result of [`process_image`] for a single raster.
pub enum ImageOutcome {
    /// The image was decoded and re-encoded for the device.
    Processed {
        /// The encoded output bytes.
        data: Vec<u8>,
        /// The chosen output format.
        format: OutFormat,
        /// Output width in pixels.
        width: u32,
        /// Output height in pixels.
        height: u32,
        /// A human-readable summary of the transformation (dimensions, formats,
        /// quality and sizes), used to build the report line.
        note: String,
    },
    /// The image could not be processed and was left byte-for-byte unchanged.
    /// A [`Warning`] describing why has already been recorded.
    Unchanged {
        /// Why the image was left untouched.
        reason: String,
    },
}

/// Raster formats the device cannot decode natively but we can transcode from.
/// Kept separate from the media-type list so sniffing never depends on the
/// (untrustworthy) declared extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InFormat {
    Jpeg,
    Png,
    Gif,
    Webp,
    Tiff,
    Bmp,
}

impl InFormat {
    fn label(self) -> &'static str {
        match self {
            InFormat::Jpeg => "jpeg",
            InFormat::Png => "png",
            InFormat::Gif => "gif",
            InFormat::Webp => "webp",
            InFormat::Tiff => "tiff",
            InFormat::Bmp => "bmp",
        }
    }

    fn image_format(self) -> ImageFormat {
        match self {
            InFormat::Jpeg => ImageFormat::Jpeg,
            InFormat::Png => ImageFormat::Png,
            InFormat::Gif => ImageFormat::Gif,
            InFormat::Webp => ImageFormat::WebP,
            InFormat::Tiff => ImageFormat::Tiff,
            InFormat::Bmp => ImageFormat::Bmp,
        }
    }

    /// The lowercase file extension (no dot) and media type for this format,
    /// used to name a raster payload extracted from an SVG data URI before the
    /// pipeline re-sniffs and processes it.
    fn ext_and_media_type(self) -> (&'static str, &'static str) {
        match self {
            InFormat::Jpeg => ("jpg", "image/jpeg"),
            InFormat::Png => ("png", "image/png"),
            InFormat::Gif => ("gif", "image/gif"),
            InFormat::Webp => ("webp", "image/webp"),
            InFormat::Tiff => ("tiff", "image/tiff"),
            InFormat::Bmp => ("bmp", "image/bmp"),
        }
    }
}

/// Sniff a raster payload's format from its magic bytes, returning a suitable
/// `(extension, media_type)` for it, or `None` if it is not a recognized raster.
pub(crate) fn sniff_raster_kind(data: &[u8]) -> Option<(&'static str, &'static str)> {
    sniff(data).map(InFormat::ext_and_media_type)
}

/// Media types that mark a resource as a raster we should process. `image/svg+xml`
/// is deliberately excluded: SVG is handled earlier by the [`svg`] pass, which
/// either unwraps it or rasterizes it into one of these formats.
const RASTER_MEDIA_TYPES: [&str; 6] = [
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "image/tiff",
    "image/bmp",
];

/// Whether a resource should be fed through the image pipeline: any resource
/// whose declared media type is a raster image, or whose bytes sniff as one.
/// SVG is never a candidate.
pub(crate) fn is_image_candidate(media_type: &str, data: &[u8]) -> bool {
    if media_type == "image/svg+xml" {
        return false;
    }
    RASTER_MEDIA_TYPES.contains(&media_type) || sniff(data).is_some()
}

/// Process one raster for the device. Sniffs the format from magic bytes (never
/// the extension), decodes and normalizes it, and re-encodes it within the byte
/// budget for its role. Undecodable input yields [`ImageOutcome::Unchanged`]
/// plus a recorded [`Warning`], so a single broken image never fails a
/// conversion.
pub fn process_image(
    data: &[u8],
    role: ImageRole,
    profile: &DeviceCaps,
    quality: u8,
    warnings: &mut Vec<Warning>,
    path: &str,
) -> ImageOutcome {
    let Some(prepared) = sniff_and_prepare(data, profile.panel, warnings, path) else {
        return ImageOutcome::Unchanged {
            reason: "unsupported or undecodable image".to_string(),
        };
    };
    finish(prepared, role, profile, quality, data.len(), warnings, path)
}

/// A crate-internal richer entry point used by `convert`: like [`process_image`]
/// but able to split a tall inline image into page tiles when requested.
pub(crate) fn process_for_convert(
    data: &[u8],
    role: ImageRole,
    profile: &DeviceCaps,
    quality: u8,
    split_tall: bool,
    warnings: &mut Vec<Warning>,
    path: &str,
) -> PipelineResult {
    let Some(prepared) = sniff_and_prepare(data, profile.panel, warnings, path) else {
        return PipelineResult::Single(ImageOutcome::Unchanged {
            reason: "unsupported or undecodable image".to_string(),
        });
    };
    if split_tall && role == ImageRole::Inline {
        match split::run(prepared, profile, quality, warnings, path) {
            split::Outcome::Tiles(tiles) => return PipelineResult::Split(tiles),
            split::Outcome::NotTall(prepared) => {
                return PipelineResult::Single(finish(
                    prepared,
                    role,
                    profile,
                    quality,
                    data.len(),
                    warnings,
                    path,
                ));
            }
        }
    }
    PipelineResult::Single(finish(
        prepared,
        role,
        profile,
        quality,
        data.len(),
        warnings,
        path,
    ))
}

/// What the `convert`-facing pipeline produced for one source image.
pub(crate) enum PipelineResult {
    /// A single output (processed or left unchanged).
    Single(ImageOutcome),
    /// A tall image split into ordered page tiles.
    Split(Vec<split::Tile>),
}

/// A decoded, normalized image plus what we learned about it.
struct Prepared {
    canvas: Canvas,
    line_art: bool,
    in_w: u32,
    in_h: u32,
    in_fmt: InFormat,
}

/// Sniff, decode and normalize `data`, recording a warning and returning `None`
/// if the format is unrecognized or the bytes cannot be decoded.
fn sniff_and_prepare(
    data: &[u8],
    panel: Panel,
    warnings: &mut Vec<Warning>,
    path: &str,
) -> Option<Prepared> {
    let Some(format) = sniff(data) else {
        warnings.push(Warning {
            message: format!("could not identify {path} as a supported image; left it unchanged"),
            file: Some(path.to_string()),
        });
        return None;
    };
    if is_animated(format, data) {
        warnings.push(Warning {
            message: format!(
                "{path} is an animated {}; used its first frame only",
                format.label()
            ),
            file: Some(path.to_string()),
        });
    }
    match decode(format, data) {
        Ok(img) => Some(prepare(img, format, panel)),
        Err(reason) => {
            warnings.push(Warning {
                message: format!(
                    "could not decode {path} as {} ({reason}); left it unchanged",
                    format.label()
                ),
                file: Some(path.to_string()),
            });
            None
        }
    }
}

/// Identify a raster format from its leading magic bytes. Returns `None` for
/// anything not in the supported set (including SVG, which has no magic number
/// and is never processed here).
fn sniff(data: &[u8]) -> Option<InFormat> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(InFormat::Jpeg);
    }
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(InFormat::Png);
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Some(InFormat::Gif);
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some(InFormat::Webp);
    }
    if data.starts_with(&[0x49, 0x49, 0x2A, 0x00]) || data.starts_with(&[0x4D, 0x4D, 0x00, 0x2A]) {
        return Some(InFormat::Tiff);
    }
    if data.starts_with(b"BM") {
        return Some(InFormat::Bmp);
    }
    None
}

/// Whether a GIF or WebP holds more than one frame (so we can warn that only the
/// first is used). Other formats are never animated.
fn is_animated(format: InFormat, data: &[u8]) -> bool {
    match format {
        InFormat::Gif => image::codecs::gif::GifDecoder::new(Cursor::new(data))
            .map(|d| d.into_frames().take(2).count() > 1)
            .unwrap_or(false),
        InFormat::Webp => image::codecs::webp::WebPDecoder::new(Cursor::new(data))
            .map(|d| d.has_animation())
            .unwrap_or(false),
        _ => false,
    }
}

/// Decode `data` as `format`, applying EXIF orientation so rotated photos land
/// upright. Animated inputs decode to their first frame.
fn decode(format: InFormat, data: &[u8]) -> Result<DynamicImage, String> {
    let mut reader = ImageReader::new(Cursor::new(data));
    reader.set_format(format.image_format());
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// Flatten alpha onto white, reduce to the panel's color space, classify, and
/// (for photos on a grayscale panel) autocontrast.
fn prepare(img: DynamicImage, format: InFormat, panel: Panel) -> Prepared {
    let (in_w, in_h) = (img.width(), img.height());
    let (canvas, line_art) = to_device_canvas(img, panel);
    Prepared {
        canvas,
        line_art,
        in_w,
        in_h,
        in_fmt: format,
    }
}

/// Composite alpha onto white, reduce to the panel's color space, classify
/// line-art-vs-photo, and contrast-stretch photos: the device-normalization
/// every raster shares, whether decoded from a file or rendered from an SVG.
///
/// Classification always reasons about luminance, so a color image is
/// classified through a gray view of itself and keeps its RGB pixels either
/// way. The contrast stretch is grayscale-only: on a 4-level panel a flat
/// mid-tone photo turns to mush without it, but stretching a color photo would
/// shift its hues, and a color panel has the tonal range to not need it.
fn to_device_canvas(img: DynamicImage, panel: Panel) -> (Canvas, bool) {
    let flattened = flatten_alpha_onto_white(img);
    let mut canvas = Canvas::from_flattened(flattened, panel.is_color());
    let line_art = classify::classify(&canvas.to_luma()) == classify::Kind::LineArt;
    if !line_art && let Canvas::Gray(gray) = &mut canvas {
        autocontrast::autocontrast(gray);
    }
    (canvas, line_art)
}

/// Composite any alpha channel onto a white background, matching how every
/// target device flattens PNG alpha (Amazon states outright that Kindle does
/// not support transparency at all).
fn flatten_alpha_onto_white(img: DynamicImage) -> DynamicImage {
    if !img.color().has_alpha() {
        return img;
    }
    let mut rgba = img.into_rgba8();
    for px in rgba.pixels_mut() {
        let alpha = px.0[3] as u32;
        let inv = 255 - alpha;
        for channel in &mut px.0[0..3] {
            *channel = ((*channel as u32 * alpha + 255 * inv) / 255) as u8;
        }
        px.0[3] = 255;
    }
    DynamicImage::ImageRgba8(rgba)
}

/// Fit, encode and budget-check a prepared image for its role, producing the
/// final [`ImageOutcome`] with its report note.
fn finish(
    prepared: Prepared,
    role: ImageRole,
    profile: &DeviceCaps,
    quality: u8,
    in_len: usize,
    warnings: &mut Vec<Warning>,
    path: &str,
) -> ImageOutcome {
    let enc = encode_prepared(
        &prepared.canvas,
        prepared.line_art,
        role,
        profile,
        quality,
        warnings,
        path,
    );
    let note = note_str(
        &prepared,
        enc.width,
        enc.height,
        enc.format,
        enc.quality,
        in_len,
        enc.data.len(),
    );
    ImageOutcome::Processed {
        data: enc.data,
        format: enc.format,
        width: enc.width,
        height: enc.height,
        note,
    }
}

/// A device-encoded raster: the chosen bytes, format, final dimensions, and (for
/// a JPEG) the quality it was encoded at, for building a report line.
pub(crate) struct Encoded {
    pub data: Vec<u8>,
    pub format: OutFormat,
    pub width: u32,
    pub height: u32,
    pub quality: Option<u8>,
}

/// Fit an image to its role's target, then encode it within the byte budget: a
/// lossless PNG for line art (falling back to a higher-quality JPEG only if the
/// PNG blows the budget), a baseline JPEG for photos. The canvas's color space
/// carries through: a color panel gets RGB out, a grayscale panel gets luma.
fn encode_prepared(
    canvas: &Canvas,
    line_art: bool,
    role: ImageRole,
    profile: &DeviceCaps,
    quality: u8,
    warnings: &mut Vec<Warning>,
    path: &str,
) -> Encoded {
    let (target, budget, photo_quality) = match role {
        ImageRole::Cover => (
            profile.cover_max,
            profile.cover_budget_bytes,
            quality.max(85),
        ),
        ImageRole::Inline => (profile.inline_max, profile.inline_budget_bytes, quality),
    };

    let fitted = resize::fit(canvas, target.0, target.1);
    let (fw, fh) = fitted.dimensions();

    if line_art {
        let png = encode::encode_png(&fitted);
        if png.len() <= budget {
            return Encoded {
                data: png,
                format: OutFormat::Png,
                width: fw,
                height: fh,
                quality: None,
            };
        }
        let line_art_quality = quality.saturating_add(6).min(95);
        let fit = encode::fit_to_budget(fitted, line_art_quality, budget);
        warn_if_over_budget(&fit, budget, warnings, path);
        return Encoded {
            data: fit.data,
            format: OutFormat::Jpeg,
            width: fit.width,
            height: fit.height,
            quality: Some(fit.quality),
        };
    }

    let fit = encode::fit_to_budget(fitted, photo_quality, budget);
    warn_if_over_budget(&fit, budget, warnings, path);
    Encoded {
        data: fit.data,
        format: OutFormat::Jpeg,
        width: fit.width,
        height: fit.height,
        quality: Some(fit.quality),
    }
}

/// How [`encode_rendered`] should treat a synthetically rendered image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenderHint {
    /// Classify line-art-vs-photo from the histogram (autocontrast for photos),
    /// for renders whose content is unknown (an SVG that embeds a raster photo
    /// or uses a gradient/pattern/filter - see `svg::svg_render_hint`).
    Auto,
    /// The caller knows the render is line art by construction: a table we
    /// laid out ourselves (black text and rules on white), or a vector SVG
    /// whose source markup proves it has no continuous-tone construct (see
    /// `svg::svg_render_hint`). Skips classification and autocontrast and goes
    /// straight to the crisp PNG-first encode - the classifier would misread
    /// the antialiasing of a supersampled downscale as continuous tone and
    /// soften it into a JPEG.
    LineArt,
}

/// Normalize and device-encode an already-decoded (e.g. SVG-rendered) image,
/// reusing the same grayscale/fit/budget path as a raster decoded from a file.
/// `hint` decides between histogram classification ([`RenderHint::Auto`]) and a
/// caller-asserted line-art encode ([`RenderHint::LineArt`]); the PNG-over-
/// budget JPEG fallback applies either way. The caller builds its own report
/// line.
pub(crate) fn encode_rendered(
    img: DynamicImage,
    hint: RenderHint,
    role: ImageRole,
    profile: &DeviceCaps,
    quality: u8,
    warnings: &mut Vec<Warning>,
    path: &str,
) -> Encoded {
    let (canvas, line_art) = match hint {
        RenderHint::Auto => to_device_canvas(img, profile.panel),
        RenderHint::LineArt => (
            Canvas::from_flattened(flatten_alpha_onto_white(img), profile.panel.is_color()),
            true,
        ),
    };
    encode_prepared(&canvas, line_art, role, profile, quality, warnings, path)
}

fn warn_if_over_budget(
    fit: &encode::BudgetFit,
    budget: usize,
    warnings: &mut Vec<Warning>,
    path: &str,
) {
    if fit.over_budget {
        warnings.push(Warning {
            message: format!(
                "could not fit {path} within its {budget}-byte budget; kept the smallest version ({}x{})",
                fit.width, fit.height
            ),
            file: Some(path.to_string()),
        });
    }
}

/// Build the "1600x2400 png -> 480x730 jpeg q82 312KB -> 64KB"-style note.
fn note_str(
    prepared: &Prepared,
    out_w: u32,
    out_h: u32,
    out_fmt: OutFormat,
    quality: Option<u8>,
    in_len: usize,
    out_len: usize,
) -> String {
    let q = match quality {
        Some(q) => format!(" q{q}"),
        None => String::new(),
    };
    format!(
        "{}x{} {} -> {}x{} {}{} {}KB -> {}KB",
        prepared.in_w,
        prepared.in_h,
        prepared.in_fmt.label(),
        out_w,
        out_h,
        out_fmt.label(),
        q,
        kb(in_len),
        kb(out_len),
    )
}

/// Bytes rounded to the nearest kibibyte for display.
fn kb(bytes: usize) -> usize {
    (bytes + 512) / 1024
}

// ---------------------------------------------------------------------
// Reference rewriting in chapter documents
// ---------------------------------------------------------------------

/// Rewrite image references in a parsed chapter after the resources have been
/// processed: strip `width`/`height` and `srcset` from every `<img>`, repoint
/// `src`/`xlink:href` at renamed resources, and replace a split image's single
/// `<img>` with one `<p class="et-img"><img/></p>` per page tile, in order.
///
/// `chapter_dir` is the zip-absolute directory of the chapter, used to resolve
/// and re-relativize hrefs. `renames` maps an old zip-absolute image path to its
/// new one; `splits` maps an old path to its ordered tile paths.
pub(crate) fn rewrite_refs(
    doc: &NodeRef,
    chapter_dir: &str,
    renames: &HashMap<String, String>,
    splits: &HashMap<String, Vec<String>>,
) {
    for img in collect_by_name(doc, "img") {
        // Stale intrinsic dimensions mislead the device scaler; drop them.
        remove_attr(&img, "width");
        remove_attr(&img, "height");
        remove_attr(&img, "srcset");

        let Some(src) = get_attr(&img, "src") else {
            continue;
        };
        let (path, suffix) = split_href_suffix(&src);
        let target = normalize_href(chapter_dir, path);

        if let Some(tiles) = splits.get(&target) {
            let alt = get_attr(&img, "alt");
            let replacements = tiles
                .iter()
                .map(|tile| tile_paragraph(chapter_dir, tile, alt.as_deref()))
                .collect();
            place_split_tiles(&img, replacements);
        } else if let Some(new_path) = renames.get(&target) {
            set_attr(
                &img,
                "src",
                &format!("{}{suffix}", relative_href(chapter_dir, new_path)),
            );
        }
    }

    // SVG `<image>` elements that reference a renamed raster.
    for image in collect_by_name(doc, "image") {
        for attr in ["xlink:href", "href"] {
            if let Some(href) = get_attr(&image, attr) {
                let (path, suffix) = split_href_suffix(&href);
                let target = normalize_href(chapter_dir, path);
                if let Some(new_path) = renames.get(&target) {
                    set_attr(
                        &image,
                        attr,
                        &format!("{}{suffix}", relative_href(chapter_dir, new_path)),
                    );
                }
            }
        }
    }

    // A plain `<a href>` pointing directly at a renamed image (the common
    // "view full-size image" link, not wrapping an `<img>` at all) dangles
    // just like an unrewritten `src` would unless it follows the same rename.
    for anchor in collect_by_name(doc, "a") {
        let Some(href) = get_attr(&anchor, "href") else {
            continue;
        };
        let (path, suffix) = split_href_suffix(&href);
        let target = normalize_href(chapter_dir, path);
        if let Some(new_path) = renames.get(&target) {
            set_attr(
                &anchor,
                "href",
                &format!("{}{suffix}", relative_href(chapter_dir, new_path)),
            );
        }
    }
}

/// One `<p class="et-img"><img src="tile" alt="..."/></p>` wrapper for a split tile.
fn tile_paragraph(chapter_dir: &str, tile_path: &str, alt: Option<&str>) -> NodeRef {
    let href = relative_href(chapter_dir, tile_path);
    let img = if let Some(alt) = alt {
        element("img", &[("src", &href), ("alt", alt)])
    } else {
        element("img", &[("src", &href)])
    };
    let para = element("p", &[("class", "et-img")]);
    para.append(img);
    para
}

/// Inline (phrasing-content) wrappers a split image is commonly nested in
/// (`<p><a href="big.png"><img/></a></p>` is the classic linked image). The
/// block-ancestor search climbs through these; block tiles must never be
/// spliced inside one when it sits in a `<p>`.
const INLINE_WRAPPERS: &[&str] = &[
    "a", "span", "em", "i", "b", "strong", "u", "small", "sub", "sup",
];

/// Whether `node` is one of the [`INLINE_WRAPPERS`].
fn is_inline_wrapper(node: &NodeRef) -> bool {
    matches!(node.data(), NodeData::Element(e) if INLINE_WRAPPERS.contains(&e.name.local.as_ref()))
}

/// Insert a split image's tile paragraphs where its original `<img>` was.
///
/// EPUB's content model forbids block content inside a `<p>`. Swapping the
/// tiles in for the img (fine under `<body>`, `<div>`, `<li>`, ...) would
/// produce `<p><p class="et-img">...</p></p>` for the very common source
/// pattern `<p><img/></p>` - an epubcheck error - and the same one level
/// removed for a linked image `<p><a><img/></a></p>`, where the tiles would
/// also violate the `<a>`'s inline content model. So the img's ancestry is
/// climbed through any inline wrappers to the nearest block-level ancestor;
/// when that ancestor is a `<p>`:
///
/// - img effectively the paragraph's only content (each hop the sole element
///   child of its parent, whitespace-only text ignored): the whole paragraph
///   is replaced by the tiles.
/// - anything else in the paragraph (real text, another element): the
///   paragraph keeps its content, the tiles are inserted as siblings right
///   after it, and the img plus any inline wrapper it leaves empty are
///   dropped.
fn place_split_tiles(img: &NodeRef, replacements: Vec<NodeRef>) {
    let mut top = img.clone();
    while let Some(parent) = top.parent() {
        if !is_inline_wrapper(&parent) {
            break;
        }
        top = parent;
    }

    // Only a <p> ancestor forbids block content; any other block-level
    // ancestor takes the tiles at the img's own position.
    let Some(paragraph) = top.parent().filter(|block| is_named(block, "p")) else {
        replace_with(img, replacements);
        return;
    };

    if is_sole_content_chain(&paragraph, img) {
        if let Some(first_tile) = replacements.first() {
            adopt_paragraph_attrs(&paragraph, first_tile);
        }
        replace_with(&paragraph, replacements);
    } else {
        insert_after(&paragraph, replacements);
        detach_and_prune_wrappers(img);
    }
}

/// Carry the original split paragraph's identity onto the first replacement
/// tile when the whole `<p>` is swapped out, so an in-book anchor
/// (`<p id="fig1">`) or author styling does not dangle. `id` is copied as-is;
/// `class` is merged - the original's tokens first, then `et-img` appended if
/// it is not already one of them, with duplicate tokens dropped.
fn adopt_paragraph_attrs(original: &NodeRef, first_tile: &NodeRef) {
    if let Some(id) = get_attr(original, "id") {
        set_attr(first_tile, "id", &id);
    }

    let original_class = get_attr(original, "class");
    let mut tokens: Vec<&str> = Vec::new();
    if let Some(class) = &original_class {
        for token in class.split_whitespace() {
            if !tokens.contains(&token) {
                tokens.push(token);
            }
        }
    }
    if !tokens.contains(&"et-img") {
        tokens.push("et-img");
    }
    set_attr(first_tile, "class", &tokens.join(" "));
}

/// Whether `img` is effectively `block`'s only content: every hop from the
/// img up to `block` is the sole element child of its parent (whitespace-only
/// text siblings ignored).
fn is_sole_content_chain(block: &NodeRef, img: &NodeRef) -> bool {
    let mut node = img.clone();
    while let Some(parent) = node.parent() {
        if !is_only_element_child(&parent, &node) {
            return false;
        }
        if &parent == block {
            return true;
        }
        node = parent;
    }
    false
}

/// Detach `img`, then drop each inline wrapper it leaves empty (no element
/// children, no non-whitespace text), climbing until a wrapper still holds
/// content or the block ancestor is reached.
fn detach_and_prune_wrappers(img: &NodeRef) {
    let mut parent = img.parent();
    img.detach();
    while let Some(node) = parent {
        if !is_inline_wrapper(&node) || has_content(&node) {
            break;
        }
        parent = node.parent();
        node.detach();
    }
}

/// Whether `node` still holds anything worth keeping: an element child or
/// non-whitespace text.
fn has_content(node: &NodeRef) -> bool {
    node.children().any(|c| match c.data() {
        NodeData::Element(_) => true,
        NodeData::Text(t) => !t.borrow().trim().is_empty(),
        _ => false,
    })
}

/// Whether `child` is `parent`'s only element child, ignoring any
/// whitespace-only text siblings. A real text sibling or another element
/// means the parent has other content that must be preserved.
///
/// A comment sibling (`<p><!--c--><img/></p>`) is likewise ignored and falls
/// through to the catch-all arm below: comments are non-rendering, so a
/// comment-only paragraph is still "the img's only content" and takes the
/// whole-paragraph replacement path, dropping the comment along with the
/// paragraph. This is deliberate, not an oversight.
fn is_only_element_child(parent: &NodeRef, child: &NodeRef) -> bool {
    parent.children().all(|c| {
        &c == child
            || match c.data() {
                NodeData::Element(_) => false,
                NodeData::Text(t) => t.borrow().trim().is_empty(),
                _ => true,
            }
    })
}

/// Insert `nodes`, in order, as siblings immediately after `anchor`.
fn insert_after(anchor: &NodeRef, nodes: Vec<NodeRef>) {
    match anchor.next_sibling() {
        Some(next) => {
            for node in nodes {
                next.insert_before(node);
            }
        }
        None => {
            if let Some(parent) = anchor.parent() {
                for node in nodes {
                    parent.append(node);
                }
            }
        }
    }
}

/// Split `href` into its path portion and the suffix starting at the first
/// `#` or `?` (an empty string if there is neither).
fn split_href_suffix(href: &str) -> (&str, &str) {
    let end = href.find(['#', '?']).unwrap_or(href.len());
    (&href[..end], &href[end..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Warning;
    use image::codecs::bmp::BmpEncoder;
    use image::codecs::gif::GifEncoder;
    use image::codecs::png::PngEncoder;
    use image::codecs::tiff::TiffEncoder;
    use image::codecs::webp::WebPEncoder;
    use image::{ExtendedColorType, Frame, ImageEncoder, RgbImage, RgbaImage};
    use jpeg_encoder::{ColorType, Encoder};
    use std::io::Cursor;

    /// A grayscale-gradient RGB image the pipeline classifies as a photo.
    fn photo_rgb(w: u32, h: u32) -> RgbImage {
        let mut img = RgbImage::new(w, h);
        for (x, _y, px) in img.enumerate_pixels_mut() {
            let v = ((x * 255) / w.max(1)) as u8;
            *px = image::Rgb([v, v, v]);
        }
        img
    }

    fn png_bytes(img: &RgbImage) -> Vec<u8> {
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(
                img.as_raw(),
                img.width(),
                img.height(),
                ExtendedColorType::Rgb8,
            )
            .unwrap();
        out
    }

    fn process(data: &[u8], role: ImageRole) -> (ImageOutcome, Vec<Warning>) {
        let mut warnings = Vec::new();
        let outcome = process_image(data, role, &DeviceCaps::x4(), 82, &mut warnings, "images/x");
        (outcome, warnings)
    }

    #[test]
    fn photo_becomes_baseline_grayscale_jpeg() {
        let (outcome, _) = process(&png_bytes(&photo_rgb(300, 200)), ImageRole::Inline);
        let ImageOutcome::Processed { data, format, .. } = outcome else {
            panic!("expected a processed image");
        };
        assert_eq!(format, OutFormat::Jpeg, "a smooth gradient is a photo");
        assert_eq!(sniff(&data), Some(InFormat::Jpeg));
        assert!(data.windows(2).any(|w| w == [0xFF, 0xC0]), "baseline SOF0");
        assert!(
            !data.windows(2).any(|w| w == [0xFF, 0xC2]),
            "no progressive SOF2"
        );
    }

    #[test]
    fn checkerboard_stays_crisp_png() {
        let mut img = RgbImage::new(40, 40);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let v = if (x + y) % 2 == 0 { 0 } else { 255 };
            *px = image::Rgb([v, v, v]);
        }
        let (outcome, _) = process(&png_bytes(&img), ImageRole::Inline);
        let ImageOutcome::Processed {
            data,
            format,
            width,
            height,
            ..
        } = outcome
        else {
            panic!("expected a processed image");
        };
        assert_eq!(format, OutFormat::Png, "line art stays PNG");
        assert_eq!((width, height), (40, 40), "small line art is not resized");
        // Pixel-crisp: exactly the two original tones survive.
        let decoded = image::load_from_memory(&data).unwrap().into_luma8();
        assert!(decoded.pixels().all(|p| p.0[0] == 0 || p.0[0] == 255));
    }

    #[test]
    fn fits_long_axis_and_never_upscales() {
        // 1600x2400 -> fits inside 480x730 (width binds -> 480x720).
        let (big, _) = process(&png_bytes(&photo_rgb(1600, 2400)), ImageRole::Inline);
        let ImageOutcome::Processed { width, height, .. } = big else {
            panic!("expected processed");
        };
        assert_eq!((width, height), (480, 720));

        // 200x300 stays exactly 200x300 (never upscales).
        let (small, _) = process(&png_bytes(&photo_rgb(200, 300)), ImageRole::Inline);
        let ImageOutcome::Processed { width, height, .. } = small else {
            panic!("expected processed");
        };
        assert_eq!((width, height), (200, 300));
    }

    #[test]
    fn alpha_is_composited_onto_white() {
        // Transparent background, opaque black square. After compositing the
        // corner is white; two tones classify as line art -> lossless PNG.
        let mut img = RgbaImage::from_pixel(30, 30, image::Rgba([0, 0, 0, 0]));
        for (x, y, px) in img.enumerate_pixels_mut() {
            if (10..20).contains(&x) && (10..20).contains(&y) {
                *px = image::Rgba([0, 0, 0, 255]);
            }
        }
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(img.as_raw(), 30, 30, ExtendedColorType::Rgba8)
            .unwrap();
        let (outcome, _) = process(&png, ImageRole::Inline);
        let ImageOutcome::Processed { data, format, .. } = outcome else {
            panic!("expected processed");
        };
        assert_eq!(format, OutFormat::Png);
        let decoded = image::load_from_memory(&data).unwrap().into_luma8();
        assert_eq!(
            decoded.get_pixel(0, 0).0[0],
            255,
            "transparent corner is white"
        );
    }

    #[test]
    fn exif_orientation_six_uprights_dimensions() {
        // 4x2 grayscale JPEG, tagged EXIF orientation=6, becomes 2x4 upright.
        let mut wide = image::GrayImage::new(4, 2);
        for (x, _y, px) in wide.enumerate_pixels_mut() {
            px.0[0] = (x * 60) as u8;
        }
        let mut base = Vec::new();
        Encoder::new(&mut base, 82)
            .encode(wide.as_raw(), 4, 2, ColorType::Luma)
            .unwrap();
        // APP1 EXIF: little-endian TIFF, one Orientation=6 SHORT entry.
        let app1: &[u8] = &[
            0xFF, 0xE1, 0x00, 0x22, b'E', b'x', b'i', b'f', 0x00, 0x00, 0x49, 0x49, 0x2A, 0x00,
            0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0x12, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let mut jpeg = Vec::new();
        jpeg.extend_from_slice(&base[..2]);
        jpeg.extend_from_slice(app1);
        jpeg.extend_from_slice(&base[2..]);

        let (outcome, _) = process(&jpeg, ImageRole::Inline);
        let ImageOutcome::Processed { width, height, .. } = outcome else {
            panic!("expected processed");
        };
        assert_eq!((width, height), (2, 4), "orientation 6 swaps dimensions");
    }

    #[test]
    fn budget_is_respected_for_a_noisy_photo() {
        // High-entropy noise cannot compress to nothing; the loop must still land
        // at or under the 100KB inline budget (or warn that it could not).
        let mut img = RgbImage::new(1200, 1600);
        let mut state = 0x9E37_79B9u32;
        for px in img.pixels_mut() {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let v = (state >> 24) as u8;
            *px = image::Rgb([v, v, v]);
        }
        let (outcome, warnings) = process(&png_bytes(&img), ImageRole::Inline);
        let ImageOutcome::Processed { data, .. } = outcome else {
            panic!("expected processed");
        };
        let budget = DeviceCaps::x4().inline_budget_bytes;
        let over = warnings.iter().any(|w| w.message.contains("budget"));
        assert!(
            data.len() <= budget || over,
            "output {} bytes exceeds {budget} with no warning",
            data.len()
        );
    }

    #[test]
    fn processing_is_deterministic() {
        // Same input and options must yield byte-identical output.
        let png = png_bytes(&photo_rgb(500, 400));
        let (a, _) = process(&png, ImageRole::Inline);
        let (b, _) = process(&png, ImageRole::Inline);
        let (ImageOutcome::Processed { data: da, .. }, ImageOutcome::Processed { data: db, .. }) =
            (a, b)
        else {
            panic!("expected processed");
        };
        assert_eq!(da, db, "identical input must produce identical bytes");
    }

    #[test]
    fn dead_formats_are_transcoded_away() {
        let photo = photo_rgb(64, 48);

        // GIF (palette line art typically -> PNG).
        let mut gif = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut gif);
            enc.encode_frame(Frame::new(
                image::DynamicImage::ImageRgb8(photo.clone()).into_rgba8(),
            ))
            .unwrap();
        }

        // WebP (lossless).
        let mut webp = Vec::new();
        WebPEncoder::new_lossless(&mut webp)
            .encode(photo.as_raw(), 64, 48, ExtendedColorType::Rgb8)
            .unwrap();

        // TIFF.
        let mut tiff = Vec::new();
        TiffEncoder::new(Cursor::new(&mut tiff))
            .write_image(photo.as_raw(), 64, 48, ExtendedColorType::Rgb8)
            .unwrap();

        // BMP.
        let mut bmp = Vec::new();
        BmpEncoder::new(&mut bmp)
            .encode(photo.as_raw(), 64, 48, ExtendedColorType::Rgb8)
            .unwrap();

        for (name, bytes) in [("gif", gif), ("webp", webp), ("tiff", tiff), ("bmp", bmp)] {
            let (outcome, _) = process(&bytes, ImageRole::Inline);
            let ImageOutcome::Processed { data, format, .. } = outcome else {
                panic!("{name} should be transcoded, not left unchanged");
            };
            assert!(
                matches!(format, OutFormat::Jpeg | OutFormat::Png),
                "{name} must become JPEG or PNG"
            );
            // The dead format's magic bytes must be gone.
            assert!(
                matches!(sniff(&data), Some(InFormat::Jpeg) | Some(InFormat::Png)),
                "{name} output must be a live format"
            );
        }
    }

    #[test]
    fn sniff_recognizes_supported_formats() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(InFormat::Jpeg));
        assert_eq!(
            sniff(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
            Some(InFormat::Png)
        );
        assert_eq!(sniff(b"GIF89a...."), Some(InFormat::Gif));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(InFormat::Webp));
        assert_eq!(sniff(&[0x49, 0x49, 0x2A, 0x00]), Some(InFormat::Tiff));
        assert_eq!(sniff(b"BM\0\0"), Some(InFormat::Bmp));
    }

    #[test]
    fn sniff_rejects_unknown_and_svg() {
        assert_eq!(sniff(b"<svg xmlns=..."), None);
        assert_eq!(sniff(b"not an image"), None);
        assert_eq!(sniff(&[]), None);
    }

    #[test]
    fn is_candidate_covers_declared_and_sniffed_but_not_svg() {
        assert!(is_image_candidate("image/png", b""));
        assert!(is_image_candidate(
            "application/octet-stream",
            &[0xFF, 0xD8, 0xFF, 0xE0]
        ));
        assert!(!is_image_candidate("image/svg+xml", b"<svg/>"));
        assert!(!is_image_candidate("text/css", b".x{}"));
    }

    #[test]
    fn undecodable_bytes_stay_unchanged_with_warning() {
        // JPEG magic but truncated garbage: sniffs as JPEG, fails to decode.
        let mut warnings = Vec::new();
        let outcome = process_image(
            &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A],
            ImageRole::Inline,
            &DeviceCaps::x4(),
            82,
            &mut warnings,
            "images/broken.jpg",
        );
        assert!(matches!(outcome, ImageOutcome::Unchanged { .. }));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("could not decode"));
    }

    #[test]
    fn unknown_format_stays_unchanged_with_warning() {
        let mut warnings: Vec<Warning> = Vec::new();
        let outcome = process_image(
            b"totally not an image",
            ImageRole::Inline,
            &DeviceCaps::x4(),
            82,
            &mut warnings,
            "images/mystery.bin",
        );
        assert!(matches!(outcome, ImageOutcome::Unchanged { .. }));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("could not identify"));
    }

    #[test]
    fn split_href_suffix_separates_path_from_fragment_and_query() {
        assert_eq!(split_href_suffix("a/b.png#x"), ("a/b.png", "#x"));
        assert_eq!(split_href_suffix("a/b.png?v=2"), ("a/b.png", "?v=2"));
        assert_eq!(split_href_suffix("a/b.png"), ("a/b.png", ""));
    }

    #[test]
    fn a_href_pointing_directly_at_a_renamed_image_is_rewritten() {
        // A text link straight to the image file (not wrapping an <img>) is
        // a common "view full-size image" pattern; it must follow the same
        // rename an <img src> would.
        let doc = crate::html::testutil::doc_from_body(
            r#"<p><a href="../images/pic.png">full size</a></p>"#,
        );
        let mut renames = HashMap::new();
        renames.insert("images/pic.png".to_string(), "images/pic.jpg".to_string());
        rewrite_refs(&doc, "text", &renames, &HashMap::new());
        let out = crate::html::testutil::serialize(&doc);
        assert!(
            out.contains(r#"href="../images/pic.jpg""#),
            "the link should follow the rename: {out}"
        );
    }

    #[test]
    fn a_href_fragment_and_query_survive_the_rewrite() {
        let doc = crate::html::testutil::doc_from_body(
            r#"<p><a href="../images/pic.png?v=2#note">full size</a></p>"#,
        );
        let mut renames = HashMap::new();
        renames.insert("images/pic.png".to_string(), "images/pic.jpg".to_string());
        rewrite_refs(&doc, "text", &renames, &HashMap::new());
        let out = crate::html::testutil::serialize(&doc);
        assert!(
            out.contains(r#"href="../images/pic.jpg?v=2#note""#),
            "the query/fragment must survive the rewrite: {out}"
        );
    }

    #[test]
    fn img_src_fragment_and_query_survive_the_rewrite() {
        let doc = crate::html::testutil::doc_from_body(
            r#"<p><img src="../images/pic.png?v=2#note" alt="x"/></p>"#,
        );
        let mut renames = HashMap::new();
        renames.insert("images/pic.png".to_string(), "images/pic.jpg".to_string());
        rewrite_refs(&doc, "text", &renames, &HashMap::new());
        let out = crate::html::testutil::serialize(&doc);
        assert!(
            out.contains(r#"src="../images/pic.jpg?v=2#note""#),
            "the query/fragment must survive the rewrite: {out}"
        );
    }

    #[test]
    fn svg_image_href_fragment_and_query_survive_the_rewrite() {
        let doc = crate::html::testutil::doc_from_body(
            r#"<svg><image href="../images/pic.png?v=2#note"/></svg>"#,
        );
        let mut renames = HashMap::new();
        renames.insert("images/pic.png".to_string(), "images/pic.jpg".to_string());
        rewrite_refs(&doc, "text", &renames, &HashMap::new());
        let out = crate::html::testutil::serialize(&doc);
        assert!(
            out.contains(r#"href="../images/pic.jpg?v=2#note""#),
            "the query/fragment must survive the rewrite: {out}"
        );
    }

    #[test]
    fn a_href_not_in_the_rename_map_is_left_alone() {
        let doc = crate::html::testutil::doc_from_body(
            r#"<p><a href="../images/other.png">elsewhere</a></p>"#,
        );
        let mut renames = HashMap::new();
        renames.insert("images/pic.png".to_string(), "images/pic.jpg".to_string());
        rewrite_refs(&doc, "text", &renames, &HashMap::new());
        let out = crate::html::testutil::serialize(&doc);
        assert!(
            out.contains(r#"href="../images/other.png""#),
            "an unrelated href must be untouched: {out}"
        );
    }

    fn tile_splits() -> HashMap<String, Vec<String>> {
        let mut splits = HashMap::new();
        splits.insert(
            "images/tall.png".to_string(),
            vec![
                "images/tall-p1.jpg".to_string(),
                "images/tall-p2.jpg".to_string(),
            ],
        );
        splits
    }

    /// Assert by DOM walk (attribute-robust, unlike a `<p><p` substring
    /// check) that no `<p>` and no inline wrapper contains a `<p>` descendant.
    fn assert_no_p_nesting(doc: &NodeRef) {
        for name in ["p", "a", "span", "em", "strong"] {
            for node in collect_by_name(doc, name) {
                assert!(
                    !node.descendants().any(|d| is_named(&d, "p")),
                    "a <{name}> contains a <p>: {}",
                    crate::html::testutil::serialize(doc)
                );
            }
        }
    }

    #[test]
    fn split_img_alone_in_a_paragraph_replaces_the_whole_paragraph() {
        // The very common source pattern: an <img> that is its <p>'s only
        // content. Swapping tiles in at the img's position would nest a <p>
        // (block content) inside a <p>, which is an epubcheck error; the
        // paragraph itself must be replaced instead.
        let doc = crate::html::testutil::doc_from_body(
            r#"<p><img src="images/tall.png" alt="tall"/></p>"#,
        );
        rewrite_refs(&doc, "", &HashMap::new(), &tile_splits());
        assert_no_p_nesting(&doc);
        let out = crate::html::testutil::serialize(&doc);
        assert_eq!(
            out.matches(r#"class="et-img""#).count(),
            2,
            "two tile paragraphs at the former <p>'s position: {out}"
        );
        assert!(out.contains("tall-p1.jpg") && out.contains("tall-p2.jpg"));
    }

    #[test]
    fn split_paragraph_attrs_are_adopted_by_the_first_tile() {
        // The original <p>'s id/class carry an in-book anchor and author
        // styling; dropping them when the whole paragraph is swapped out for
        // tiles leaves a dangling anchor. The id and classes must land on
        // the first tile; later tiles stay plain et-img.
        let doc = crate::html::testutil::doc_from_body(
            r#"<p id="fig1" class="illus"><img src="images/tall.png" alt="tall"/></p>"#,
        );
        rewrite_refs(&doc, "", &HashMap::new(), &tile_splits());
        let tiles = collect_by_name(&doc, "p");
        assert_eq!(tiles.len(), 2, "two tile paragraphs expected");
        assert_eq!(
            get_attr(&tiles[0], "id").as_deref(),
            Some("fig1"),
            "the original id must move to the first tile"
        );
        assert_eq!(
            get_attr(&tiles[0], "class").as_deref(),
            Some("illus et-img"),
            "the original classes must be kept, et-img appended"
        );
        assert_eq!(
            get_attr(&tiles[1], "id"),
            None,
            "only the first tile adopts the id"
        );
        assert_eq!(get_attr(&tiles[1], "class").as_deref(), Some("et-img"));
    }

    #[test]
    fn split_paragraph_id_only_is_adopted_with_cp_img_as_the_sole_class() {
        // The original <p> has an id but no class: the id must still move to
        // the first tile, and et-img becomes the sole class (pins the id-only
        // path of adopt_paragraph_attrs; the id+class path is tested above).
        let doc = crate::html::testutil::doc_from_body(
            r#"<p id="fig9"><img src="images/tall.png" alt="tall"/></p>"#,
        );
        rewrite_refs(&doc, "", &HashMap::new(), &tile_splits());
        let tiles = collect_by_name(&doc, "p");
        assert_eq!(tiles.len(), 2, "two tile paragraphs expected");
        assert_eq!(
            get_attr(&tiles[0], "id").as_deref(),
            Some("fig9"),
            "the original id must move to the first tile"
        );
        assert_eq!(
            get_attr(&tiles[0], "class").as_deref(),
            Some("et-img"),
            "with no original class, et-img is the sole class"
        );
    }

    #[test]
    fn split_img_with_comment_sibling_still_replaces_whole_paragraph() {
        // Comments are non-rendering; a comment sibling must not stop the
        // whole-paragraph replacement path the way a real text sibling
        // would (pinning the deliberate behavior documented on
        // `is_only_element_child`).
        let doc = crate::html::testutil::doc_from_body(
            r#"<p><!--c--><img src="images/tall.png" alt="tall"/></p>"#,
        );
        rewrite_refs(&doc, "", &HashMap::new(), &tile_splits());
        assert_no_p_nesting(&doc);
        let out = crate::html::testutil::serialize(&doc);
        assert!(!out.contains("<!--"), "the comment must be gone: {out}");
        assert_eq!(
            out.matches(r#"class="et-img""#).count(),
            2,
            "two tile paragraphs at the former <p>'s position: {out}"
        );
    }

    #[test]
    fn split_img_with_caption_text_keeps_the_paragraph_and_appends_tiles_after() {
        // The img shares its paragraph with real text (a caption). The
        // paragraph must survive with its text; the tiles land as siblings
        // right after it rather than nesting inside it.
        let doc = crate::html::testutil::doc_from_body(
            r#"<p>Caption text <img src="images/tall.png" alt="tall"/></p>"#,
        );
        rewrite_refs(&doc, "", &HashMap::new(), &tile_splits());
        assert_no_p_nesting(&doc);
        let out = crate::html::testutil::serialize(&doc);
        assert!(out.contains("Caption text"), "caption kept: {out}");
        let caption_p_end = out.find("</p>").expect("caption paragraph closes");
        let first_tile = out.find("tall-p1.jpg").expect("first tile present");
        assert!(
            caption_p_end < first_tile,
            "tiles must follow the caption paragraph, not sit inside it: {out}"
        );
    }

    #[test]
    fn split_img_linked_in_a_paragraph_replaces_the_whole_paragraph() {
        // A linked image, one inline level down: <p><a><img/></a></p>.
        // Splicing block tiles in at the img's position would nest them
        // inside the <a> inside the <p> - the same epubcheck error class
        // plus an inline-content-model violation. The whole paragraph must
        // be replaced, dropping the now-pointless link.
        let doc = crate::html::testutil::doc_from_body(
            r#"<p><a href="images/tall.png"><img src="images/tall.png" alt="tall"/></a></p>"#,
        );
        rewrite_refs(&doc, "", &HashMap::new(), &tile_splits());
        assert_no_p_nesting(&doc);
        assert!(
            collect_by_name(&doc, "a").is_empty(),
            "no <a> may remain: {}",
            crate::html::testutil::serialize(&doc)
        );
        let out = crate::html::testutil::serialize(&doc);
        assert_eq!(
            out.matches(r#"class="et-img""#).count(),
            2,
            "two tile paragraphs at the former <p>'s position: {out}"
        );
    }

    #[test]
    fn split_img_in_nested_inline_wrappers_replaces_the_whole_paragraph() {
        // Two inline levels down: <p><a><span><img/></span></a></p>. The
        // climb must pass through every inline wrapper to the <p>.
        let doc = crate::html::testutil::doc_from_body(
            r#"<p><a href="x"><span><img src="images/tall.png" alt="tall"/></span></a></p>"#,
        );
        rewrite_refs(&doc, "", &HashMap::new(), &tile_splits());
        assert_no_p_nesting(&doc);
        let out = crate::html::testutil::serialize(&doc);
        assert!(
            collect_by_name(&doc, "a").is_empty() && collect_by_name(&doc, "span").is_empty(),
            "no emptied inline wrapper may remain: {out}"
        );
        assert_eq!(
            out.matches(r#"class="et-img""#).count(),
            2,
            "two tile paragraphs at the former <p>'s position: {out}"
        );
    }

    #[test]
    fn split_img_in_link_with_text_keeps_paragraph_and_drops_the_emptied_link() {
        // The linked img shares its paragraph with real text. The paragraph
        // must survive with its text, the emptied <a> is dropped, and the
        // tiles follow as siblings after the paragraph.
        let doc = crate::html::testutil::doc_from_body(
            r#"<p>See <a href="x"><img src="images/tall.png" alt="tall"/></a> here</p>"#,
        );
        rewrite_refs(&doc, "", &HashMap::new(), &tile_splits());
        assert_no_p_nesting(&doc);
        let out = crate::html::testutil::serialize(&doc);
        assert!(
            out.contains("See") && out.contains("here"),
            "paragraph text kept: {out}"
        );
        assert!(
            collect_by_name(&doc, "a").is_empty(),
            "the emptied link is dropped: {out}"
        );
        let caption_p_end = out.find("</p>").expect("text paragraph closes");
        let first_tile = out.find("tall-p1.jpg").expect("first tile present");
        assert!(
            caption_p_end < first_tile,
            "tiles must follow the text paragraph, not sit inside it: {out}"
        );
    }
}
