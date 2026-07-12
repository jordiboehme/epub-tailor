//! Encoding an image the device can decode: a baseline JPEG (photos) or an
//! 8-bit PNG (line art), plus the byte-budget loop that steps quality and then
//! dimensions down until an image fits its budget.
//!
//! Both formats are emitted in the canvas's own color space - luma for a
//! grayscale panel, RGB for a color one - so a color device never has its color
//! silently encoded away. The device only decodes baseline JPEG (progressive
//! JPEG degrades to a 1/8-resolution blur) and 8-bit PNG, so JPEGs go through
//! `jpeg-encoder`, which only ever emits baseline (SOF0) data.

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::imageops::FilterType as ResizeFilter;
use image::{ExtendedColorType, ImageEncoder};
use jpeg_encoder::{ColorType, Encoder};

use super::canvas::Canvas;

/// Lowest JPEG quality the budget loop will drop to.
const QUALITY_FLOOR: u8 = 40;
/// Step by which JPEG quality is reduced each budget iteration.
const QUALITY_STEP: u8 = 6;

/// Encode a canvas as a baseline JPEG at `quality` (1-100), with one luma
/// component for a gray canvas and three for an RGB one. The output always uses
/// the baseline (SOF0) process.
pub(super) fn encode_jpeg(canvas: &Canvas, quality: u8) -> Vec<u8> {
    let (w, h) = canvas.dimensions();
    let (raw, color_type) = match canvas {
        Canvas::Gray(img) => (img.as_raw().as_slice(), ColorType::Luma),
        Canvas::Rgb(img) => (img.as_raw().as_slice(), ColorType::Rgb),
    };
    let mut out = Vec::new();
    // Encoding a fixed-size in-memory buffer whose dimensions fit the device
    // screen (well within u16) cannot fail.
    Encoder::new(&mut out, quality)
        .encode(raw, w as u16, h as u16, color_type)
        .expect("baseline JPEG encode of an in-memory buffer cannot fail");
    out
}

/// Encode a canvas as an 8-bit PNG (luma or RGB) at the best (smallest) DEFLATE
/// setting, for line art the device renders crisply.
pub(super) fn encode_png(canvas: &Canvas) -> Vec<u8> {
    let (w, h) = canvas.dimensions();
    let (raw, color_type) = match canvas {
        Canvas::Gray(img) => (img.as_raw().as_slice(), ExtendedColorType::L8),
        Canvas::Rgb(img) => (img.as_raw().as_slice(), ExtendedColorType::Rgb8),
    };
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive)
        .write_image(raw, w, h, color_type)
        .expect("PNG encode of an in-memory buffer cannot fail");
    out
}

/// The outcome of the byte-budget loop: the chosen JPEG bytes, the dimensions
/// they were encoded at, the final quality, and whether the budget was still
/// exceeded (in which case this is the smallest attempt).
pub(super) struct BudgetFit {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub quality: u8,
    pub over_budget: bool,
}

/// Encode `canvas` as a baseline JPEG within `budget` bytes, in its own color
/// space.
///
/// Steps quality down by [`QUALITY_STEP`] to a floor of [`QUALITY_FLOOR`],
/// then, still at the floor, steps the dimensions down 10% at a time to a floor
/// of 50% of the input size. Returns the highest-quality attempt that fits; if
/// none fit, returns the smallest attempt with `over_budget` set.
pub(super) fn fit_to_budget(canvas: Canvas, start_quality: u8, budget: usize) -> BudgetFit {
    let (w0, h0) = canvas.dimensions();
    let mut smallest: Option<BudgetFit> = None;

    // Phase 1: drop quality at the full (fitted) size.
    let mut quality = start_quality.max(QUALITY_FLOOR);
    loop {
        let data = encode_jpeg(&canvas, quality);
        if data.len() <= budget {
            return BudgetFit {
                data,
                width: w0,
                height: h0,
                quality,
                over_budget: false,
            };
        }
        keep_smallest(&mut smallest, data, w0, h0, quality);
        if quality <= QUALITY_FLOOR {
            break;
        }
        quality = quality.saturating_sub(QUALITY_STEP).max(QUALITY_FLOOR);
    }

    // Phase 2: drop dimensions at the quality floor, down to 50% of the input.
    let min_w = (w0 / 2).max(1);
    let min_h = (h0 / 2).max(1);
    let mut percent = 90u32;
    while percent >= 50 {
        let nw = (w0 * percent / 100).max(min_w);
        let nh = (h0 * percent / 100).max(min_h);
        let small = canvas.resize(nw, nh, ResizeFilter::Lanczos3);
        let data = encode_jpeg(&small, QUALITY_FLOOR);
        if data.len() <= budget {
            return BudgetFit {
                data,
                width: nw,
                height: nh,
                quality: QUALITY_FLOOR,
                over_budget: false,
            };
        }
        keep_smallest(&mut smallest, data, nw, nh, QUALITY_FLOOR);
        percent -= 10;
    }

    // Nothing fit: keep the smallest attempt and flag it.
    let mut fit = smallest.expect("the first attempt is always recorded");
    fit.over_budget = true;
    fit
}

/// Replace `slot` with a new attempt when the new one is strictly smaller (or
/// `slot` is empty), so the fallback always holds the smallest attempt seen.
fn keep_smallest(
    slot: &mut Option<BudgetFit>,
    data: Vec<u8>,
    width: u32,
    height: u32,
    quality: u8,
) {
    let replace = slot.as_ref().is_none_or(|b| data.len() < b.data.len());
    if replace {
        *slot = Some(BudgetFit {
            data,
            width,
            height,
            quality,
            over_budget: true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma, Rgb, RgbImage};

    /// A deterministic high-entropy grayscale image, so JPEG cannot compress it
    /// to nothing (useful for exercising the budget loop).
    fn noise(w: u32, h: u32) -> Canvas {
        let mut img = GrayImage::new(w, h);
        let mut state = 0x1234_5678u32;
        for px in img.pixels_mut() {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            px.0[0] = (state >> 24) as u8;
        }
        Canvas::Gray(img)
    }

    fn gradient(w: u32, h: u32) -> Canvas {
        let mut img = GrayImage::new(w, h);
        for (x, _y, px) in img.enumerate_pixels_mut() {
            *px = Luma([((x * 255) / w.max(1)) as u8]);
        }
        Canvas::Gray(img)
    }

    /// A deterministic RGB image with a different value in every channel, so a
    /// grayscale encode would be detectable.
    fn color_noise(w: u32, h: u32) -> Canvas {
        let mut img = RgbImage::new(w, h);
        let mut state = 0x9E37_79B9u32;
        for px in img.pixels_mut() {
            for channel in &mut px.0 {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *channel = (state >> 24) as u8;
            }
        }
        Canvas::Rgb(img)
    }

    #[test]
    fn jpeg_is_baseline_grayscale() {
        let jpeg = encode_jpeg(&gradient(32, 24), 82);
        // SOF0 (baseline) present, SOF2 (progressive) absent.
        assert!(
            jpeg.windows(2).any(|w| w == [0xFF, 0xC0]),
            "baseline SOF0 marker must be present"
        );
        assert!(
            !jpeg.windows(2).any(|w| w == [0xFF, 0xC2]),
            "progressive SOF2 marker must be absent"
        );
        // The SOF0 component count byte (10 bytes past the marker) must be 1.
        let sof0 = jpeg
            .windows(2)
            .position(|w| w == [0xFF, 0xC0])
            .expect("SOF0 present");
        assert_eq!(
            jpeg[sof0 + 9],
            1,
            "grayscale JPEG has exactly one component"
        );
    }

    #[test]
    fn a_color_canvas_encodes_a_three_component_jpeg() {
        let jpeg = encode_jpeg(&color_noise(32, 24), 82);
        assert!(
            jpeg.windows(2).any(|w| w == [0xFF, 0xC0]),
            "baseline SOF0 marker must be present"
        );
        let sof0 = jpeg
            .windows(2)
            .position(|w| w == [0xFF, 0xC0])
            .expect("SOF0 present");
        assert_eq!(
            jpeg[sof0 + 9],
            3,
            "a color panel must get a three-component JPEG, not a grayscaled one"
        );
    }

    #[test]
    fn png_round_trips_grayscale_losslessly() {
        let src = gradient(20, 10);
        let png = encode_png(&src);
        let decoded = image::load_from_memory(&png)
            .expect("valid PNG")
            .into_luma8();
        assert_eq!(decoded.dimensions(), (20, 10));
        let Canvas::Gray(raw) = &src else {
            panic!("gradient builds a gray canvas")
        };
        assert_eq!(decoded.as_raw(), raw.as_raw(), "PNG must be lossless");
    }

    #[test]
    fn png_round_trips_color_losslessly() {
        let src = color_noise(20, 10);
        let png = encode_png(&src);
        let decoded = image::load_from_memory(&png).expect("valid PNG");
        assert!(decoded.color().has_color(), "color PNG must stay color");
        let Canvas::Rgb(raw) = &src else {
            panic!("color_noise builds an rgb canvas")
        };
        assert_eq!(
            decoded.into_rgb8().as_raw(),
            raw.as_raw(),
            "PNG must be lossless"
        );
    }

    #[test]
    fn a_gray_canvas_encodes_a_single_channel_png() {
        let mut g = GrayImage::new(4, 4);
        for (i, px) in g.pixels_mut().enumerate() {
            px.0[0] = (i * 16) as u8;
        }
        let png = encode_png(&Canvas::Gray(g));
        let color = image::load_from_memory(&png).expect("valid PNG").color();
        assert!(!color.has_color(), "PNG must be grayscale");
        let _ = Rgb([0u8, 0, 0]);
    }

    #[test]
    fn budget_loop_fits_by_stepping_quality() {
        // A modest noise image that fits once quality drops a little.
        let img = noise(200, 200);
        let full = encode_jpeg(&img, 82).len();
        let budget = full * 3 / 4; // require some quality reduction
        let fit = fit_to_budget(img, 82, budget);
        assert!(!fit.over_budget, "should fit within budget");
        assert!(fit.data.len() <= budget);
        assert!(fit.quality <= 82);
        assert_eq!((fit.width, fit.height), (200, 200), "dims untouched");
    }

    #[test]
    fn budget_loop_steps_dimensions_when_quality_floor_is_not_enough() {
        let img = noise(400, 400);
        // A tiny budget forces dimension reduction after the quality floor.
        let budget = 2_000;
        let fit = fit_to_budget(img, 82, budget);
        // Either it fit by shrinking, or it kept the smallest attempt and flagged it.
        if fit.over_budget {
            assert!(fit.data.len() > budget);
        } else {
            assert!(fit.data.len() <= budget);
            assert!(
                fit.width < 400 || fit.height < 400,
                "dimensions were reduced"
            );
            assert!(fit.width >= 200 && fit.height >= 200, "not below 50%");
        }
    }

    #[test]
    fn budget_loop_flags_when_nothing_fits() {
        let img = noise(400, 400);
        let fit = fit_to_budget(img, 82, 10); // impossibly small
        assert!(fit.over_budget, "cannot fit a 400x400 image in 10 bytes");
        // Smallest attempt is at the quality floor and the 50% size floor.
        assert_eq!(fit.quality, QUALITY_FLOOR);
        assert!(fit.width <= 200 && fit.height <= 200);
    }

    #[test]
    fn a_color_image_stays_color_all_the_way_through_the_budget_loop() {
        // Squeeze hard enough to drive both phases (quality, then dimensions):
        // the output must still be a color JPEG, never a grayscaled one.
        let fit = fit_to_budget(color_noise(400, 400), 82, 2_000);
        let decoded = image::load_from_memory(&fit.data).expect("valid JPEG");
        assert!(
            decoded.color().has_color(),
            "the budget loop must not grayscale a color panel's image"
        );
    }
}
