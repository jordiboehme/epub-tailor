//! SVG handling: the device has no SVG decoder at all, so an SVG resource or an
//! inline `<svg>` renders as nothing (see `docs/device-constraints.md`). Two
//! cases are handled here:
//!
//! 1. **Wrapper SVGs** - an `<svg>` whose only real content is a single
//!    `<image>` (the classic cover-page pattern). [`as_image_wrapper`] detects
//!    them; the caller drops the SVG frame and lets the raster payload flow
//!    through the normal image pipeline.
//! 2. **Real vector art** - rectangles, paths, text, ... [`rasterize_svg`]
//!    renders it with resvg at 2x supersampling over a white background and
//!    hands the result to the image pipeline's encode/budget path.
//!
//! Text is rendered with one bundled font (DejaVu Sans) and no system fonts, so
//! rendering is deterministic and offline.

use std::sync::{Arc, OnceLock};

use image::{DynamicImage, RgbaImage};
use kuchikiki::{NodeData, NodeRef};
use resvg::tiny_skia;
use resvg::usvg;

use super::RenderHint;
use crate::profile::DeviceCaps;
use crate::report::Warning;

/// The bundled font used for `<text>` in SVGs. One permissive (Bitstream Vera)
/// TrueType face, embedded so rendering never touches system fonts. See
/// `assets/fonts/DEJAVU-LICENSE.txt`.
const DEJAVU_SANS: &[u8] = include_bytes!("../../assets/fonts/DejaVuSans.ttf");

/// The font family name of [`DEJAVU_SANS`], used as every generic family so any
/// `font-family` in an SVG resolves to the one bundled face.
const DEJAVU_FAMILY: &str = "DejaVu Sans";

/// What a wrapper SVG frames: either an inline base64 `data:` payload (decoded
/// to its raw bytes) or a relative path to another resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WrapperTarget {
    /// A `data:image/...;base64,...` payload, already base64-decoded.
    DataUri(Vec<u8>),
    /// A relative href to another (raster) resource.
    Href(String),
}

/// Detect a "wrapper" SVG: one whose root `<svg>` contains exactly one
/// `<image>` element, possibly nested inside a chain of sole-child `<g>`
/// wrappers (up to [`MAX_WRAPPER_DEPTH`] levels deep), ignoring
/// `title`/`desc`/`defs`/`metadata` and whitespace at every level. Returns
/// what the single `<image>` points at, or `None` if the SVG is anything else
/// (real vector art, multiple images, malformed, too deeply nested, ...).
pub fn as_image_wrapper(svg: &str) -> Option<WrapperTarget> {
    let doc = roxmltree::Document::parse(svg).ok()?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return None;
    }

    let image = find_sole_image(root, 0)?;

    // `href` (SVG2) or `xlink:href` (SVG1.1); both have local name `href`.
    let href = image
        .attributes()
        .find(|a| a.name() == "href")
        .map(|a| a.value().trim())
        .filter(|h| !h.is_empty())?;

    if let Some(rest) = href.strip_prefix("data:") {
        // data:[<mediatype>][;base64],<payload>
        let (meta, payload) = rest.split_once(',')?;
        if !meta.contains("base64") {
            return None;
        }
        let bytes = decode_base64(payload).filter(|b| !b.is_empty())?;
        return Some(WrapperTarget::DataUri(bytes));
    }
    Some(WrapperTarget::Href(href.to_string()))
}

/// How many levels of sole-child `<g>` wrapping [`find_sole_image`] will
/// descend through before giving up. Real-world "cover page" wrappers seen in
/// the wild nest 0-2 `<g>` levels (from editing-tool export boilerplate);
/// four is generous headroom while still bounding the recursion.
const MAX_WRAPPER_DEPTH: usize = 4;

/// The sole rendering element at this level, descending through sole-child
/// `<g>` wrappers up to `MAX_WRAPPER_DEPTH` levels. `title`/`desc`/`defs`/
/// `metadata` are ignored at every level; a second rendering element at any
/// level - or any element that is neither `g` nor `image` nor ignorable -
/// means "not a wrapper" and the whole descent fails.
///
/// Deliberate tradeoff: any `transform` on a traversed `<g>` is ignored. The
/// extracted raster does not carry it forward - it re-enters the normal
/// fit-to-device pipeline, where a `<g>`'s scale/translate would have been
/// irrelevant anyway (the image is refit to the device box from scratch), but
/// a `rotate(...)` or `skew(...)` on the `<g>` would be silently lost. The
/// status quo for a `<g>`-wrapped image is a blank rendered frame (resvg
/// cannot load an external raster `href`, so rasterizing the wrapper produces
/// nothing), so unwrapping - and losing a hypothetical rotation - strictly
/// dominates shipping a blank page.
fn find_sole_image<'a, 'i>(
    node: roxmltree::Node<'a, 'i>,
    depth: usize,
) -> Option<roxmltree::Node<'a, 'i>> {
    let mut found = None;
    for child in node.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            // Non-rendering metadata: ignored when deciding "just one image".
            "title" | "desc" | "defs" | "metadata" => continue,
            "image" if found.is_none() => found = Some(child),
            "g" if found.is_none() && depth < MAX_WRAPPER_DEPTH => {
                // The `?` returns None for the WHOLE descent when the child
                // group is not a sole-image wrapper - one failed level means
                // "not a wrapper", full stop.
                let image = find_sole_image(child, depth + 1)?;
                found = Some(image);
            }
            // A second rendering element, a non-image/non-g first element, or
            // a `<g>` beyond the depth cap: not a wrapper.
            _ => return None,
        }
    }
    found
}

/// Local names of elements that make an SVG's rendered content continuous-tone
/// rather than flat vector art: an embedded raster, a paint server, or a
/// filter.
const CONTINUOUS_TONE_MARKERS: [&str; 5] = [
    "image",
    "linearGradient",
    "radialGradient",
    "pattern",
    "filter",
];

/// Decide how a rasterized SVG should be device-encoded, from the SVG source
/// markup rather than the rendered pixels.
///
/// This does its own `roxmltree` parse of `svg` - a second, cheap XML parse is
/// noise next to the `resvg` render that follows it, and keeps this decision
/// independent of (and safe to make before) that render.
///
/// If any element anywhere in the document - including nested inside `defs` -
/// has local name `image`, `linearGradient`, `radialGradient`, `pattern` or
/// `filter`, the render may contain continuous tone and is classified the
/// usual way ([`RenderHint::Auto`]). Otherwise the SVG is flat vector art by
/// construction and is encoded as line art ([`RenderHint::LineArt`]).
///
/// Rationale: an embedded raster's pixels and a paint server's/filter's
/// blending are the only SVG constructs that can produce continuous tone, and
/// all of them must exist as an in-document element to have any effect -
/// there is no way to smuggle continuous tone past this scan. Everything else
/// (rects, paths, text, solid fills, ...) renders as flat color regions, so it
/// is line art by construction. Choosing `LineArt` also correctly skips the
/// autocontrast stretch (a vector's colors are authorial, not a scan to
/// correct), and a pathologically detailed vector that renders to an
/// oversized PNG still falls back to JPEG via `encode_prepared`'s existing
/// over-budget path, so this cannot make a huge line-art render un-encodable.
///
/// A parse failure yields `Auto`; it does not matter which hint is returned
/// since [`rasterize_sized`] will fail its own parse the same way, warn, and
/// leave the resource untouched.
pub(crate) fn svg_render_hint(svg: &str) -> RenderHint {
    let Ok(doc) = roxmltree::Document::parse(svg) else {
        return RenderHint::Auto;
    };
    let has_marker = doc
        .descendants()
        .filter(roxmltree::Node::is_element)
        .any(|n| CONTINUOUS_TONE_MARKERS.contains(&n.tag_name().name()));
    if has_marker {
        RenderHint::Auto
    } else {
        RenderHint::LineArt
    }
}

/// Rasterize a real vector SVG for the device. See [`rasterize_sized`]; this is
/// the public entry point that drops the intrinsic-size bookkeeping.
///
/// Returns `None` (with a [`Warning`] recorded) when the SVG cannot be parsed or
/// has no positive intrinsic size, so a single bad image never fails a whole
/// conversion (the resource is left untouched by the caller).
pub fn rasterize_svg(
    svg: &str,
    profile: &DeviceCaps,
    warnings: &mut Vec<Warning>,
    path: &str,
) -> Option<DynamicImage> {
    rasterize_sized(svg, profile.inline_max, warnings, path).map(|r| r.image)
}

/// A rasterized SVG plus its (rounded) intrinsic size, for the caller's report
/// line ("vector 800x600 -> ...").
pub(crate) struct Rasterized {
    pub image: DynamicImage,
    pub intrinsic_w: u32,
    pub intrinsic_h: u32,
}

/// Rasterize a real vector SVG at 2x supersampling over a white background.
///
/// The target size is the SVG's intrinsic size fitted inside `max_box` (the
/// caller picks the device box: cover_max for a cover, inline_max otherwise)
/// preserving aspect ratio, never upscaled beyond intrinsic - except tiny icons
/// (intrinsic width < 100px), which may grow up to 2x so they are not stamp-sized
/// on a 220ppi screen. The SVG is rendered into a Pixmap at twice the target size
/// and downscaled with Lanczos3 for crisp edges.
pub(crate) fn rasterize_sized(
    svg: &str,
    max_box: (u32, u32),
    warnings: &mut Vec<Warning>,
    path: &str,
) -> Option<Rasterized> {
    let opt = options();
    let tree = match usvg::Tree::from_str(svg, &opt) {
        Ok(tree) => tree,
        Err(e) => {
            warnings.push(Warning {
                message: format!("could not render {path} as SVG ({e}); left it unchanged"),
                file: Some(path.to_string()),
            });
            return None;
        }
    };

    let size = tree.size();
    let (iw, ih) = (size.width(), size.height());
    if !(iw > 0.0 && ih > 0.0) {
        warnings.push(Warning {
            message: format!("{path} has no usable SVG size; left it unchanged"),
            file: Some(path.to_string()),
        });
        return None;
    }

    let (target_w, target_h) = target_size(iw, ih, max_box);
    let (render_w, render_h) = (target_w * 2, target_h * 2);

    let mut pixmap = tiny_skia::Pixmap::new(render_w, render_h)?;
    pixmap.fill(tiny_skia::Color::WHITE);
    let transform = tiny_skia::Transform::from_scale(render_w as f32 / iw, render_h as f32 / ih);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // The Pixmap is fully opaque (rendered over white), so its premultiplied
    // RGBA equals straight RGBA and drops cleanly to an image buffer.
    let supersampled = RgbaImage::from_raw(render_w, render_h, pixmap.data().to_vec())?;
    let image = DynamicImage::ImageRgba8(supersampled).resize_exact(
        target_w,
        target_h,
        image::imageops::FilterType::Lanczos3,
    );

    Some(Rasterized {
        image,
        intrinsic_w: iw.round().max(1.0) as u32,
        intrinsic_h: ih.round().max(1.0) as u32,
    })
}

/// Fit an intrinsic `iw` x `ih` inside `max_box` preserving aspect, never
/// upscaling - except a tiny icon (`iw < 100`), allowed up to 2x.
fn target_size(iw: f32, ih: f32, max_box: (u32, u32)) -> (u32, u32) {
    let (max_w, max_h) = max_box;
    let fit = f32::min(max_w as f32 / iw, max_h as f32 / ih);
    let upscale_cap = if iw < 100.0 { 2.0 } else { 1.0 };
    let scale = fit.min(upscale_cap);
    let tw = (iw * scale).round().max(1.0) as u32;
    let th = (ih * scale).round().max(1.0) as u32;
    (tw.min(max_w), th.min(max_h))
}

/// Serialize a kuchikiki `<svg>` subtree to a standalone SVG string usvg can
/// parse. The HTML5 parser strips the `xmlns`/`xmlns:xlink` declarations into
/// namespaces, so they are re-injected on the root; camelCase attribute names
/// (`viewBox`, ...) and the `xlink:` prefix are preserved as parsed.
pub(crate) fn serialize_svg_subtree(svg: &NodeRef) -> String {
    let mut out = String::new();
    write_svg_element(svg, true, &mut out);
    out
}

fn write_svg_element(node: &NodeRef, is_root: bool, out: &mut String) {
    let NodeData::Element(elem) = node.data() else {
        return;
    };
    let name = elem.name.local.as_ref();
    out.push('<');
    out.push_str(name);
    if is_root {
        out.push_str(
            " xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"",
        );
    }
    for (key, attr) in &elem.attributes.borrow().map {
        let local = key.local.as_ref();
        // Never re-emit a namespace declaration; the root gets ours injected.
        if local == "xmlns" || attr.prefix.as_ref().is_some_and(|p| p.as_ref() == "xmlns") {
            continue;
        }
        out.push(' ');
        if let Some(prefix) = &attr.prefix {
            out.push_str(prefix.as_ref());
            out.push(':');
        }
        out.push_str(local);
        out.push_str("=\"");
        crate::html::escape::escape_into(&attr.value, true, out);
        out.push('"');
    }
    // SVG elements are not HTML-void: always emit an explicit end tag.
    out.push('>');
    for child in node.children() {
        match child.data() {
            NodeData::Element(_) => write_svg_element(&child, false, out),
            NodeData::Text(text) => crate::html::escape::escape_into(&text.borrow(), false, out),
            _ => {}
        }
    }
    out.push_str("</");
    out.push_str(name);
    out.push('>');
}

/// usvg options wired to render deterministically with only the bundled font.
pub(crate) fn options() -> usvg::Options<'static> {
    let mut opt = usvg::Options {
        font_family: DEJAVU_FAMILY.to_string(),
        ..usvg::Options::default()
    };
    opt.fontdb = font_database();
    opt
}

/// The shared, one-time-built font database: only DejaVu Sans, mapped onto every
/// generic family so any `font-family` in an SVG resolves to it.
fn font_database() -> Arc<usvg::fontdb::Database> {
    static FONT_DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONT_DB
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_font_data(DEJAVU_SANS.to_vec());
            db.set_serif_family(DEJAVU_FAMILY);
            db.set_sans_serif_family(DEJAVU_FAMILY);
            db.set_monospace_family(DEJAVU_FAMILY);
            db.set_cursive_family(DEJAVU_FAMILY);
            db.set_fantasy_family(DEJAVU_FAMILY);
            Arc::new(db)
        })
        .clone()
}

/// Decode standard-alphabet base64 (`A-Za-z0-9+/`, `=` padding), skipping ASCII
/// whitespace. Returns `None` on any other byte. Small enough not to warrant a
/// dependency, and the only base64 we ever see is `<image>` data URIs.
fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::png::PngEncoder;
    use image::{ExtendedColorType, ImageEncoder, RgbImage};

    /// Standard base64 encoder, for building `data:` fixtures in tests.
    fn encode_base64(data: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            out.push(ALPHABET[(n >> 18) as usize & 63] as char);
            out.push(ALPHABET[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[(n >> 6) as usize & 63] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[n as usize & 63] as char
            } else {
                '='
            });
        }
        out
    }

    fn tiny_png() -> Vec<u8> {
        let img = RgbImage::from_pixel(4, 4, image::Rgb([10, 20, 30]));
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(img.as_raw(), 4, 4, ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    #[test]
    fn base64_round_trips() {
        for sample in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            let encoded = encode_base64(sample);
            assert_eq!(decode_base64(&encoded).unwrap(), sample, "{encoded}");
        }
        // A known vector, and whitespace tolerance.
        assert_eq!(decode_base64("SGVsbG8=").unwrap(), b"Hello");
        assert_eq!(decode_base64("SGVs\nbG8=").unwrap(), b"Hello");
        assert_eq!(decode_base64("not base64!"), None);
    }

    #[test]
    fn href_wrapper_is_detected() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 600 800"><title>Cover</title><image width="600" height="800" xlink:href="cover.jpg"/></svg>"#;
        assert_eq!(
            as_image_wrapper(svg),
            Some(WrapperTarget::Href("cover.jpg".to_string()))
        );
    }

    #[test]
    fn plain_href_without_xlink_is_detected() {
        let svg =
            r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="../images/pic.png"/></svg>"#;
        assert_eq!(
            as_image_wrapper(svg),
            Some(WrapperTarget::Href("../images/pic.png".to_string()))
        );
    }

    #[test]
    fn data_uri_wrapper_decodes_payload() {
        let png = tiny_png();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="data:image/png;base64,{}"/></svg>"#,
            encode_base64(&png)
        );
        let Some(WrapperTarget::DataUri(bytes)) = as_image_wrapper(&svg) else {
            panic!("expected a decoded data-uri payload");
        };
        assert_eq!(bytes, png, "payload must round-trip");
        assert!(
            bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]),
            "decoded to PNG"
        );
    }

    #[test]
    fn real_vector_is_not_a_wrapper() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="50" height="50"/><circle cx="70" cy="70" r="20"/></svg>"#;
        assert_eq!(as_image_wrapper(svg), None);
    }

    #[test]
    fn two_images_is_not_a_simple_wrapper() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="a.png"/><image href="b.png"/></svg>"#;
        assert_eq!(as_image_wrapper(svg), None);
    }

    #[test]
    fn g_wrapped_href_is_detected() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><g transform="translate(0,0)"><image xlink:href="photo.jpg"/></g></svg>"#;
        assert_eq!(
            as_image_wrapper(svg),
            Some(WrapperTarget::Href("photo.jpg".to_string()))
        );
    }

    #[test]
    fn g_wrapped_data_uri_decodes_payload() {
        let png = tiny_png();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><g><image xlink:href="data:image/png;base64,{}"/></g></svg>"#,
            encode_base64(&png)
        );
        let Some(WrapperTarget::DataUri(bytes)) = as_image_wrapper(&svg) else {
            panic!("expected a decoded data-uri payload");
        };
        assert_eq!(bytes, png, "payload must round-trip");
    }

    #[test]
    fn two_images_in_a_group_is_not_a_wrapper() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><g><image href="a.png"/><image href="b.png"/></g></svg>"#;
        assert_eq!(as_image_wrapper(svg), None);
    }

    #[test]
    fn group_plus_root_level_image_sibling_is_not_a_wrapper() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><g><image href="a.png"/></g><image href="b.png"/></svg>"#;
        assert_eq!(as_image_wrapper(svg), None);
    }

    #[test]
    fn three_deep_nested_group_chain_is_detected() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><g><g><g><image href="deep.png"/></g></g></g></svg>"#;
        assert_eq!(
            as_image_wrapper(svg),
            Some(WrapperTarget::Href("deep.png".to_string()))
        );
    }

    #[test]
    fn four_deep_nested_group_chain_is_detected() {
        // Exactly at MAX_WRAPPER_DEPTH: the innermost <g> is entered at depth 4
        // (the last `depth < MAX_WRAPPER_DEPTH` recursion), where it finds the
        // image. Pins the depth-cap boundary between the 3-deep and 5-deep cases.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><g><g><g><g><image href="deep.png"/></g></g></g></g></svg>"#;
        assert_eq!(
            as_image_wrapper(svg),
            Some(WrapperTarget::Href("deep.png".to_string()))
        );
    }

    #[test]
    fn five_deep_nested_group_chain_exceeds_the_cap() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><g><g><g><g><g><image href="deep.png"/></g></g></g></g></g></svg>"#;
        assert_eq!(as_image_wrapper(svg), None);
    }

    #[test]
    fn ignorables_inside_the_group_are_skipped() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><g><title>t</title><image href="a.png"/></g></svg>"#;
        assert_eq!(
            as_image_wrapper(svg),
            Some(WrapperTarget::Href("a.png".to_string()))
        );
    }

    #[test]
    fn render_hint_is_line_art_for_rect_only() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="10" height="10"/></svg>"#;
        assert_eq!(svg_render_hint(svg), RenderHint::LineArt);
    }

    #[test]
    fn render_hint_is_line_art_for_circle_and_path() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><circle cx="5" cy="5" r="5"/><path d="M0 0 L10 10"/></svg>"#;
        assert_eq!(svg_render_hint(svg), RenderHint::LineArt);
    }

    #[test]
    fn render_hint_is_line_art_for_text_only() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="0" y="10">hi</text></svg>"#;
        assert_eq!(svg_render_hint(svg), RenderHint::LineArt);
    }

    #[test]
    fn render_hint_is_auto_for_each_continuous_tone_marker() {
        for marker in CONTINUOUS_TONE_MARKERS {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="10" height="10"/><{marker} id="m"/></svg>"#
            );
            assert_eq!(
                svg_render_hint(&svg),
                RenderHint::Auto,
                "a top-level <{marker}> must select Auto"
            );
        }
    }

    #[test]
    fn render_hint_is_auto_for_a_marker_nested_in_defs() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="g"/></defs><rect width="10" height="10" fill="url(#g)"/></svg>"#;
        assert_eq!(svg_render_hint(svg), RenderHint::Auto);
    }

    #[test]
    fn render_hint_is_auto_for_malformed_input() {
        assert_eq!(svg_render_hint("<svg><this is not xml"), RenderHint::Auto);
    }

    #[test]
    fn malformed_svg_yields_none_and_a_warning() {
        let mut warnings = Vec::new();
        let out = rasterize_svg(
            "<svg><this is not xml",
            &DeviceCaps::x4(),
            &mut warnings,
            "images/broken.svg",
        );
        assert!(out.is_none());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("broken.svg"));
    }

    #[test]
    fn real_vector_rasterizes_non_blank_on_white() {
        // Rectangles, a circle and text: must actually render (some dark pixels)
        // and keep white corners.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 600">
            <rect x="40" y="40" width="300" height="200" fill="black"/>
            <circle cx="600" cy="300" r="120" fill="rgb(30,30,30)"/>
            <text x="60" y="500" font-size="80" fill="black">Hello</text>
        </svg>"#;
        let mut warnings = Vec::new();
        let img = rasterize_svg(svg, &DeviceCaps::x4(), &mut warnings, "images/diagram.svg")
            .expect("a real vector should rasterize");
        assert!(
            warnings.is_empty(),
            "no warning for a good SVG: {warnings:?}"
        );
        // 800x600 fits inside 480x730 -> width binds -> 480 wide.
        assert!(img.width() <= 480, "fit within inline width");
        assert_eq!(img.width(), 480);
        assert_eq!(img.height(), 360);

        let gray = img.to_luma8();
        assert!(
            gray.pixels().any(|p| p.0[0] < 200),
            "the shapes/text must have rendered as dark pixels"
        );
        // Corners are background: white.
        let (w, h) = gray.dimensions();
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
            assert!(
                gray.get_pixel(x, y).0[0] >= 250,
                "corner ({x},{y}) should be white background"
            );
        }
    }

    #[test]
    fn tiny_icon_is_allowed_up_to_2x_not_blown_up() {
        // 32x32 intrinsic: fits far below inline_max, but the tiny-icon rule caps
        // growth at 2x -> 64x64 (never the full 480).
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 32 32"><rect x="4" y="4" width="24" height="24" fill="black"/></svg>"#;
        let mut warnings = Vec::new();
        let img = rasterize_svg(svg, &DeviceCaps::x4(), &mut warnings, "icon.svg")
            .expect("tiny icon rasterizes");
        assert_eq!((img.width(), img.height()), (64, 64));
    }

    #[test]
    fn target_size_downscales_large_and_never_upscales_medium() {
        let inline_max = DeviceCaps::x4().inline_max;
        assert_eq!(target_size(800.0, 600.0, inline_max), (480, 360));
        // 200x150 is >= 100 wide and smaller than the box: stays put.
        assert_eq!(target_size(200.0, 150.0, inline_max), (200, 150));
        // 32x32 tiny icon: 2x.
        assert_eq!(target_size(32.0, 32.0, inline_max), (64, 64));
    }

    #[test]
    fn target_size_fills_the_cover_box_when_given_one() {
        // 600x1000 has the cover box's 3:5 aspect ratio: fitting into the x4
        // cover box (480x800) must land exactly on it, not on the inline box.
        assert_eq!(target_size(600.0, 1000.0, (480, 800)), (480, 800));
    }

    #[test]
    fn inline_svg_subtree_serializes_with_namespaces() {
        let doc = crate::html::testutil::doc_from_body(
            r#"<p><svg viewBox="0 0 10 10" width="10" height="10"><rect x="1" y="1" width="8" height="8"/><image xlink:href="c.jpg"/></svg></p>"#,
        );
        let svg = crate::html::dom::collect_by_name(&doc, "svg")
            .into_iter()
            .next()
            .expect("an inline svg");
        let serialized = serialize_svg_subtree(&svg);
        assert!(
            serialized.contains(r#"xmlns="http://www.w3.org/2000/svg""#),
            "svg namespace injected: {serialized}"
        );
        assert!(serialized.contains("viewBox=\"0 0 10 10\""), "{serialized}");
        assert!(serialized.contains("xlink:href=\"c.jpg\""), "{serialized}");
        // It round-trips into a usable wrapper / parseable SVG.
        assert!(roxmltree::Document::parse(&serialized).is_ok());
    }
}
