//! Encoding a grayscale image the device can decode: a baseline JPEG (photos)
//! or an 8-bit grayscale PNG (line art), plus the byte-budget loop that steps
//! quality and then dimensions down until an image fits its budget.
//!
//! The device only decodes baseline JPEG (progressive JPEG degrades to a
//! 1/8-resolution blur) and 8-bit PNG, so JPEGs go through `jpeg-encoder`,
//! which only ever emits baseline (SOF0) data.

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::imageops::FilterType as ResizeFilter;
use image::{ExtendedColorType, GrayImage, ImageEncoder};
use jpeg_encoder::{ColorType, Encoder};

/// Lowest JPEG quality the budget loop will drop to.
const QUALITY_FLOOR: u8 = 40;
/// Step by which JPEG quality is reduced each budget iteration.
const QUALITY_STEP: u8 = 6;

/// Encode a grayscale image as a baseline JPEG with a single luma component at
/// `quality` (1-100). The output always uses the baseline (SOF0) process.
pub(super) fn encode_jpeg(gray: &GrayImage, quality: u8) -> Vec<u8> {
    let (w, h) = gray.dimensions();
    let mut out = Vec::new();
    // Encoding a fixed-size in-memory grayscale buffer whose dimensions fit the
    // device screen (well within u16) cannot fail.
    Encoder::new(&mut out, quality)
        .encode(gray.as_raw(), w as u16, h as u16, ColorType::Luma)
        .expect("baseline grayscale JPEG encode of an in-memory buffer cannot fail");
    out
}

/// Encode a grayscale image as an 8-bit grayscale PNG at the best (smallest)
/// DEFLATE setting, for line art the device renders crisply.
pub(super) fn encode_png(gray: &GrayImage) -> Vec<u8> {
    let (w, h) = gray.dimensions();
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive)
        .write_image(gray.as_raw(), w, h, ExtendedColorType::L8)
        .expect("grayscale PNG encode of an in-memory buffer cannot fail");
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

/// Encode `gray` as a baseline grayscale JPEG within `budget` bytes.
///
/// Steps quality down by [`QUALITY_STEP`] to a floor of [`QUALITY_FLOOR`],
/// then, still at the floor, steps the dimensions down 10% at a time to a floor
/// of 50% of the input size. Returns the highest-quality attempt that fits; if
/// none fit, returns the smallest attempt with `over_budget` set.
pub(super) fn fit_to_budget(gray: GrayImage, start_quality: u8, budget: usize) -> BudgetFit {
    let (w0, h0) = gray.dimensions();
    let mut smallest: Option<BudgetFit> = None;

    // Phase 1: drop quality at the full (fitted) size.
    let mut quality = start_quality.max(QUALITY_FLOOR);
    loop {
        let data = encode_jpeg(&gray, quality);
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
        let small = image::imageops::resize(&gray, nw, nh, ResizeFilter::Lanczos3);
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
    use image::{Luma, Rgb};

    /// A deterministic high-entropy grayscale image, so JPEG cannot compress it
    /// to nothing (useful for exercising the budget loop).
    fn noise(w: u32, h: u32) -> GrayImage {
        let mut img = GrayImage::new(w, h);
        let mut state = 0x1234_5678u32;
        for px in img.pixels_mut() {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            px.0[0] = (state >> 24) as u8;
        }
        img
    }

    fn gradient(w: u32, h: u32) -> GrayImage {
        let mut img = GrayImage::new(w, h);
        for (x, _y, px) in img.enumerate_pixels_mut() {
            *px = Luma([((x * 255) / w.max(1)) as u8]);
        }
        img
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
    fn png_round_trips_grayscale_losslessly() {
        let src = gradient(20, 10);
        let png = encode_png(&src);
        let decoded = image::load_from_memory(&png)
            .expect("valid PNG")
            .into_luma8();
        assert_eq!(decoded.dimensions(), (20, 10));
        assert_eq!(decoded.as_raw(), src.as_raw(), "PNG must be lossless");
    }

    #[test]
    fn png_of_rgb_input_would_not_be_grayscale_but_luma_encoder_forces_l8() {
        // Sanity: ExtendedColorType::L8 output decodes as single-channel luma.
        let mut g = GrayImage::new(4, 4);
        for (i, px) in g.pixels_mut().enumerate() {
            px.0[0] = (i * 16) as u8;
        }
        let png = encode_png(&g);
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
}
