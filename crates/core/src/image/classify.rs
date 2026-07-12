//! Line-art vs photo classification from a grayscale luminance histogram.
//!
//! Line art (diagrams, scanned text, screenshots) survives best as a lossless
//! grayscale PNG and must skip the autocontrast stretch that would fringe its
//! edges; photographs survive best as a JPEG after a contrast stretch. We tell
//! them apart with two cheap histogram signals: very few distinct tones, or two
//! tones that dominate the whole image.

use image::GrayImage;

/// What a grayscale image is treated as, for encode-format and autocontrast
/// decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// Few tones or two dominant tones: kept crisp (PNG, no autocontrast).
    LineArt,
    /// Continuous tone: autocontrast + JPEG.
    Photo,
}

/// The maximum number of distinct tones an image may have to count as line art
/// on the distinct-tone signal alone.
const MAX_DISTINCT_TONES: usize = 16;

/// The share (in percent) of pixels the two most common tones must cover for an
/// image to count as line art on the dominant-tone signal.
const DOMINANT_TWO_PERCENT: u64 = 85;

/// Classify `gray` as line art or photo from its 256-bin luminance histogram.
///
/// Line art iff the image has at most [`MAX_DISTINCT_TONES`] distinct tones, or
/// its two most common tones together cover at least [`DOMINANT_TWO_PERCENT`]%
/// of its pixels.
pub(super) fn classify(gray: &GrayImage) -> Kind {
    let total = gray.width() as u64 * gray.height() as u64;
    if total == 0 {
        return Kind::LineArt;
    }
    let hist = histogram(gray);

    let distinct = hist.iter().filter(|&&c| c > 0).count();
    if distinct <= MAX_DISTINCT_TONES {
        return Kind::LineArt;
    }

    let mut top1 = 0u32;
    let mut top2 = 0u32;
    for &count in &hist {
        if count >= top1 {
            top2 = top1;
            top1 = count;
        } else if count > top2 {
            top2 = count;
        }
    }
    if (top1 as u64 + top2 as u64) * 100 >= total * DOMINANT_TWO_PERCENT {
        return Kind::LineArt;
    }

    Kind::Photo
}

/// The 256-bin luminance histogram of a grayscale image. A single bin cannot
/// overflow `u32`: the device's source-pixel cap (2048x1536) is far below
/// `u32::MAX`.
fn histogram(gray: &GrayImage) -> [u32; 256] {
    let mut hist = [0u32; 256];
    for px in gray.pixels() {
        hist[px.0[0] as usize] += 1;
    }
    hist
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    fn checkerboard(size: u32) -> GrayImage {
        let mut img = GrayImage::new(size, size);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = Luma([if (x + y) % 2 == 0 { 0 } else { 255 }]);
        }
        img
    }

    fn smooth_gradient(w: u32, h: u32) -> GrayImage {
        let mut img = GrayImage::new(w, h);
        for (x, _y, px) in img.enumerate_pixels_mut() {
            *px = Luma([((x * 255) / w.max(1)) as u8]);
        }
        img
    }

    #[test]
    fn checkerboard_is_line_art() {
        assert_eq!(classify(&checkerboard(32)), Kind::LineArt);
    }

    #[test]
    fn two_level_diagram_is_line_art() {
        // A black rectangle on a white field: exactly two tones.
        let mut img = GrayImage::from_pixel(50, 40, Luma([255]));
        for (x, y, px) in img.enumerate_pixels_mut() {
            if (10..30).contains(&x) && (5..25).contains(&y) {
                *px = Luma([0]);
            }
        }
        assert_eq!(classify(&img), Kind::LineArt);
    }

    #[test]
    fn smooth_gradient_is_photo() {
        // 256 distinct tones spread evenly: neither signal fires.
        assert_eq!(classify(&smooth_gradient(256, 64)), Kind::Photo);
    }

    #[test]
    fn mostly_flat_with_noise_is_line_art_via_dominant_tones() {
        // A near-solid image (two dominant tones) with a sprinkling of other
        // tones: more than 16 distinct tones, but the top two cover >= 85%.
        let mut img = GrayImage::from_pixel(100, 100, Luma([255]));
        // Black border rows -> second dominant tone.
        for x in 0..100 {
            img.put_pixel(x, 0, Luma([0]));
            img.put_pixel(x, 1, Luma([0]));
        }
        // A few dozen scattered mid-tones (still a small fraction of pixels).
        for i in 0..40u32 {
            img.put_pixel(i, 50, Luma([(20 + i) as u8]));
        }
        assert_eq!(classify(&img), Kind::LineArt);
    }
}
