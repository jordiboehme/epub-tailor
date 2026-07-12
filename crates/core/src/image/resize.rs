//! Downscale-only fit-to-box.
//!
//! The device downscales with nearest-neighbor and never upscales, so we
//! pre-fit every image to the reading area with a good filter (Lanczos3),
//! preserving aspect ratio, and leave images that already fit untouched.

use image::imageops::FilterType;

use super::canvas::Canvas;

/// Fit `canvas` inside `max_w` x `max_h`, preserving aspect ratio and color
/// space, downscaling with Lanczos3. Returns a copy unchanged when the image
/// already fits (never upscales).
pub(super) fn fit(canvas: &Canvas, max_w: u32, max_h: u32) -> Canvas {
    let (w, h) = canvas.dimensions();
    if w == 0 || h == 0 || (w <= max_w && h <= max_h) {
        return canvas.clone();
    }
    let scale = f64::min(max_w as f64 / w as f64, max_h as f64 / h as f64);
    let nw = ((w as f64 * scale).round() as u32).clamp(1, max_w);
    let nh = ((h as f64 * scale).round() as u32).clamp(1, max_h);
    canvas.resize(nw, nh, FilterType::Lanczos3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, RgbImage};

    fn blank(w: u32, h: u32) -> Canvas {
        Canvas::Gray(GrayImage::new(w, h))
    }

    #[test]
    fn downscales_tall_image_into_inline_box() {
        // 1600x2400 into 480x730: width binds (scale 0.3) -> 480x720, fits box.
        let fitted = fit(&blank(1600, 2400), 480, 730);
        assert_eq!(fitted.dimensions(), (480, 720));
        let (w, h) = fitted.dimensions();
        assert!(w <= 480 && h <= 730);
    }

    #[test]
    fn never_upscales_a_small_image() {
        let fitted = fit(&blank(200, 300), 480, 730);
        assert_eq!(fitted.dimensions(), (200, 300), "must not upscale");
    }

    #[test]
    fn wide_image_binds_on_height() {
        // 2000x500 into 480x730: width binds -> 480x120.
        let fitted = fit(&blank(2000, 500), 480, 730);
        assert_eq!(fitted.dimensions(), (480, 120));
    }

    #[test]
    fn exact_fit_is_left_unchanged() {
        let fitted = fit(&blank(480, 730), 480, 730);
        assert_eq!(fitted.dimensions(), (480, 730));
    }

    #[test]
    fn fitting_a_color_image_keeps_it_color() {
        // 1600x2400 into the vision color's 1264x1680: height binds -> 1120x1680.
        let fitted = fit(&Canvas::Rgb(RgbImage::new(1600, 2400)), 1264, 1680);
        assert_eq!(fitted.dimensions(), (1120, 1680));
        assert!(fitted.is_color(), "a color panel must not get gray images");
    }
}
