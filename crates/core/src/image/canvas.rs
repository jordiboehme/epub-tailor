//! The pixel buffer the image pipeline carries: grayscale or RGB.
//!
//! A grayscale e-ink device can only ever show gray, so its images are reduced
//! to 8-bit luma once, up front - it is what the panel renders and it costs a
//! third of the bytes. A Kaleido-class color panel would have that color thrown
//! away for nothing, so its images stay RGB through fitting, budgeting and
//! encoding. Everything downstream of [`super::to_device_canvas`] is written
//! against this enum so the two paths cannot drift apart.
//!
//! Alpha is composited onto white before either variant is built: no target
//! device renders transparency (Amazon says so outright), so a flattened image
//! is what every one of them would show anyway.

use image::{DynamicImage, GrayImage, RgbImage};

/// A decoded image, in the color space its target device can actually render.
#[derive(Clone)]
pub(crate) enum Canvas {
    /// 8-bit luma, for a grayscale panel.
    Gray(GrayImage),
    /// 8-bit RGB, for a color panel.
    Rgb(RgbImage),
}

impl Canvas {
    /// Build the canvas a device wants from a decoded image whose alpha has
    /// already been flattened: RGB for a color panel, luma for a gray one.
    pub(crate) fn from_flattened(img: DynamicImage, color: bool) -> Self {
        if color {
            Canvas::Rgb(img.into_rgb8())
        } else {
            Canvas::Gray(img.into_luma8())
        }
    }

    pub(crate) fn dimensions(&self) -> (u32, u32) {
        match self {
            Canvas::Gray(img) => img.dimensions(),
            Canvas::Rgb(img) => img.dimensions(),
        }
    }

    /// Whether these pixels carry color.
    #[cfg(test)]
    pub(crate) fn is_color(&self) -> bool {
        matches!(self, Canvas::Rgb(_))
    }

    /// A grayscale view of the pixels, for the classifier and the contrast
    /// stretch, both of which reason about luminance only. Free for a
    /// [`Canvas::Gray`].
    pub(crate) fn to_luma(&self) -> GrayImage {
        match self {
            Canvas::Gray(img) => img.clone(),
            Canvas::Rgb(img) => DynamicImage::ImageRgb8(img.clone()).into_luma8(),
        }
    }

    /// Resample to `w` x `h` with `filter`, keeping the color space.
    pub(crate) fn resize(&self, w: u32, h: u32, filter: image::imageops::FilterType) -> Self {
        match self {
            Canvas::Gray(img) => Canvas::Gray(image::imageops::resize(img, w, h, filter)),
            Canvas::Rgb(img) => Canvas::Rgb(image::imageops::resize(img, w, h, filter)),
        }
    }

    /// Crop a `w` x `h` window with its top-left corner at (`x`, `y`), keeping
    /// the color space. Used to slice a tall image into page tiles.
    pub(crate) fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Self {
        match self {
            Canvas::Gray(img) => {
                Canvas::Gray(image::imageops::crop_imm(img, x, y, w, h).to_image())
            }
            Canvas::Rgb(img) => Canvas::Rgb(image::imageops::crop_imm(img, x, y, w, h).to_image()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn rgba(w: u32, h: u32, px: Rgba<u8>) -> DynamicImage {
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(w, h, px))
    }

    #[test]
    fn a_color_target_keeps_rgb_pixels() {
        let canvas = Canvas::from_flattened(rgba(2, 2, Rgba([200, 30, 40, 255])), true);
        assert!(canvas.is_color());
        match &canvas {
            Canvas::Rgb(img) => assert_eq!(img.get_pixel(0, 0).0, [200, 30, 40]),
            Canvas::Gray(_) => panic!("a color target must not grayscale"),
        }
    }

    #[test]
    fn a_gray_target_reduces_to_luma() {
        let canvas = Canvas::from_flattened(rgba(2, 2, Rgba([200, 30, 40, 255])), false);
        assert!(!canvas.is_color());
        assert!(matches!(canvas, Canvas::Gray(_)));
    }

    #[test]
    fn resize_and_crop_preserve_the_color_space() {
        let canvas = Canvas::from_flattened(rgba(8, 8, Rgba([10, 200, 90, 255])), true);
        let resized = canvas.resize(4, 4, image::imageops::FilterType::Lanczos3);
        assert_eq!(resized.dimensions(), (4, 4));
        assert!(resized.is_color());
        let cropped = canvas.crop(0, 0, 2, 3);
        assert_eq!(cropped.dimensions(), (2, 3));
        assert!(cropped.is_color());
    }
}
