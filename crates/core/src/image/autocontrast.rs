//! Percentile-clip autocontrast for photos.
//!
//! On the device's 4-level grayscale, flat mid-tone photos turn to mush, so we
//! stretch the tonal range before quantization: clip the darkest and lightest
//! 1% of pixels (to ignore stray specular highlights and deep shadows), then
//! linearly rescale what remains to the full 0-255 range. Line art skips this -
//! stretching its handful of tones would fringe the edges.

use image::GrayImage;

/// Percent of pixels clipped from each tail before the linear stretch.
const CLIP_PERCENT: u64 = 1;

/// Stretch `gray` in place so its 1st and 99th luminance percentiles map to 0
/// and 255. A no-op when the image is empty, flat, or already effectively spans
/// the clipped range.
pub(super) fn autocontrast(gray: &mut GrayImage) {
    let total = gray.width() as u64 * gray.height() as u64;
    if total == 0 {
        return;
    }

    let mut hist = [0u64; 256];
    for px in gray.pixels() {
        hist[px.0[0] as usize] += 1;
    }

    let clip = total * CLIP_PERCENT / 100;
    let low = low_percentile(&hist, clip);
    let high = high_percentile(&hist, clip);
    if high <= low {
        return;
    }

    let span = (high - low) as u32;
    let mut lut = [0u8; 256];
    for (value, out) in lut.iter_mut().enumerate() {
        let shifted = (value as i32 - low as i32).clamp(0, span as i32) as u32;
        *out = (shifted * 255 / span) as u8;
    }
    for px in gray.pixels_mut() {
        px.0[0] = lut[px.0[0] as usize];
    }
}

/// Lowest tone whose cumulative count (from black) exceeds `clip`.
fn low_percentile(hist: &[u64; 256], clip: u64) -> u8 {
    let mut cum = 0u64;
    for (value, &count) in hist.iter().enumerate() {
        cum += count;
        if cum > clip {
            return value as u8;
        }
    }
    0
}

/// Highest tone whose cumulative count (from white) exceeds `clip`.
fn high_percentile(hist: &[u64; 256], clip: u64) -> u8 {
    let mut cum = 0u64;
    for value in (0..256).rev() {
        cum += hist[value];
        if cum > clip {
            return value as u8;
        }
    }
    255
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    /// A gradient spanning only `[lo, hi]`, wide enough that the 1% clip barely
    /// bites.
    fn narrow_gradient(w: u32, lo: u8, hi: u8) -> GrayImage {
        let mut img = GrayImage::new(w, 4);
        let span = (hi - lo) as u32;
        for (x, _y, px) in img.enumerate_pixels_mut() {
            *px = Luma([(lo as u32 + (x * span) / w.max(1)) as u8]);
        }
        img
    }

    #[test]
    fn stretches_a_narrow_range_to_full() {
        let mut img = narrow_gradient(400, 64, 192);
        autocontrast(&mut img);
        let min = img.pixels().map(|p| p.0[0]).min().unwrap();
        let max = img.pixels().map(|p| p.0[0]).max().unwrap();
        assert!(min < 12, "dark end should stretch toward 0, got {min}");
        assert!(max > 243, "light end should stretch toward 255, got {max}");
    }

    #[test]
    fn flat_image_is_left_alone() {
        let mut img = GrayImage::from_pixel(10, 10, Luma([128]));
        autocontrast(&mut img);
        assert!(img.pixels().all(|p| p.0[0] == 128), "flat image unchanged");
    }

    #[test]
    fn empty_image_does_not_panic() {
        let mut img = GrayImage::new(0, 0);
        autocontrast(&mut img);
        assert_eq!(img.dimensions(), (0, 0));
    }
}
