//! Optional tall-image splitting (opt-in via `--split-tall-images`).
//!
//! Comics and full-page scans arrive as one very tall strip. Fit to the screen
//! width and it becomes taller than the device can show at a usable size, so we
//! slice it into page-height tiles with a 10% overlap between consecutive tiles
//! (so a line split across a boundary is readable on both pages). Each tile is
//! encoded as its own baseline JPEG, in the panel's own color space.

use image::imageops::FilterType;

use super::{Prepared, encode};
use crate::profile::DeviceCaps;
use crate::report::Warning;

/// One page tile of a split image: its encoded baseline-JPEG bytes, in the
/// panel's own color space.
pub(crate) struct Tile {
    pub data: Vec<u8>,
}

/// The result of attempting a split.
pub(super) enum Outcome {
    /// The image was tall enough and was sliced into these ordered tiles.
    Tiles(Vec<Tile>),
    /// The image was not tall enough to split; the prepared image is handed
    /// back so the caller can process it as a normal inline image.
    NotTall(Prepared),
}

/// Fraction (denominator) of the tile height that consecutive tiles overlap by:
/// `tile_h / OVERLAP_DIVISOR` = 10% overlap.
const OVERLAP_DIVISOR: u32 = 10;

/// Split `prepared` into page tiles when, after fitting to the screen width, it
/// is more than twice the screen height. Otherwise returns it unchanged.
pub(super) fn run(
    prepared: Prepared,
    profile: &DeviceCaps,
    quality: u8,
    warnings: &mut Vec<Warning>,
    path: &str,
) -> Outcome {
    let (w0, h0) = prepared.canvas.dimensions();
    let max_w = profile.inline_max.0;
    let tile_h = profile.screen_h;

    // Predict the fit-to-width height without resizing yet.
    let fitted_h = if w0 > max_w {
        ((h0 as f64 * max_w as f64 / w0 as f64).round() as u32).max(1)
    } else {
        h0
    };
    if fitted_h <= tile_h.saturating_mul(2) {
        return Outcome::NotTall(prepared);
    }

    // Now actually fit to width (downscale only).
    let fitted = if w0 > max_w {
        prepared
            .canvas
            .resize(max_w, fitted_h, FilterType::Lanczos3)
    } else {
        prepared.canvas
    };
    let (fw, fh) = fitted.dimensions();

    let overlap = tile_h / OVERLAP_DIVISOR;
    let step = tile_h - overlap;
    let budget = profile.inline_budget_bytes;

    let mut tiles = Vec::new();
    let mut top = 0u32;
    loop {
        let bottom = (top + tile_h).min(fh);
        let tile = fitted.crop(0, top, fw, bottom - top);
        let fit = encode::fit_to_budget(tile, quality, budget);
        if fit.over_budget {
            warnings.push(Warning {
                message: format!(
                    "could not fit a tile of {path} within its {budget}-byte budget; kept the smallest version ({}x{})",
                    fit.width, fit.height
                ),
                file: Some(path.to_string()),
            });
        }
        tiles.push(Tile { data: fit.data });
        if bottom >= fh {
            break;
        }
        top += step;
    }

    Outcome::Tiles(tiles)
}

#[cfg(test)]
mod tests {
    use super::super::canvas::Canvas;
    use super::*;
    use image::GrayImage;

    fn tall_prepared(w: u32, h: u32) -> Prepared {
        // A vertical gradient photo (many tones), so it is not line art.
        let mut gray = GrayImage::new(w, h);
        for (_x, y, px) in gray.enumerate_pixels_mut() {
            px.0[0] = ((y * 255) / h.max(1)) as u8;
        }
        prepared_from(Canvas::Gray(gray), w, h)
    }

    fn prepared_from(canvas: Canvas, w: u32, h: u32) -> Prepared {
        Prepared {
            canvas,
            line_art: false,
            in_w: w,
            in_h: h,
            in_fmt: super::super::InFormat::Png,
        }
    }

    #[test]
    fn tall_image_splits_into_overlapping_tiles() {
        let profile = DeviceCaps::x4(); // screen_h 800, inline width 480
        let mut warnings = Vec::new();
        let outcome = run(
            tall_prepared(480, 3000),
            &profile,
            82,
            &mut warnings,
            "t.png",
        );
        let Outcome::Tiles(tiles) = outcome else {
            panic!("a 480x3000 image should split");
        };
        assert!(tiles.len() >= 2, "expected 2+ tiles, got {}", tiles.len());
        // Tiles are at most a screen tall, all the same width.
        for tile in &tiles {
            let decoded = image::load_from_memory(&tile.data).unwrap();
            assert_eq!(decoded.width(), 480);
            assert!(decoded.height() <= 800);
        }
    }

    #[test]
    fn short_image_is_not_split() {
        let profile = DeviceCaps::x4();
        let mut warnings = Vec::new();
        // 480x1000 fits-to-width to 480x1000, which is <= 2*800, so no split.
        let outcome = run(
            tall_prepared(480, 1000),
            &profile,
            82,
            &mut warnings,
            "s.png",
        );
        assert!(matches!(outcome, Outcome::NotTall(_)));
    }

    #[test]
    fn wide_tall_image_fits_width_before_deciding() {
        let profile = DeviceCaps::x4();
        let mut warnings = Vec::new();
        // 960x3400 -> fit width 480 -> height 1700 > 1600 -> splits.
        let outcome = run(
            tall_prepared(960, 3400),
            &profile,
            82,
            &mut warnings,
            "w.png",
        );
        let Outcome::Tiles(tiles) = outcome else {
            panic!("should split after fitting width");
        };
        assert!(tiles.len() >= 2);
        assert!(
            tiles
                .iter()
                .all(|t| image::load_from_memory(&t.data).unwrap().width() == 480)
        );
    }

    #[test]
    fn tiles_of_a_color_image_stay_color() {
        // A color panel splitting a tall comic strip must not get gray tiles.
        let mut rgb = image::RgbImage::new(600, 4000);
        for (_x, y, px) in rgb.enumerate_pixels_mut() {
            *px = image::Rgb([200, ((y * 255) / 4000) as u8, 40]);
        }
        let profile = DeviceCaps {
            panel: crate::profile::Panel::Color,
            ..DeviceCaps::x4()
        };
        let mut warnings = Vec::new();
        let outcome = run(
            prepared_from(Canvas::Rgb(rgb), 600, 4000),
            &profile,
            82,
            &mut warnings,
            "c.png",
        );
        let Outcome::Tiles(tiles) = outcome else {
            panic!("a 600x4000 image should split");
        };
        assert!(!tiles.is_empty());
        for tile in &tiles {
            let decoded = image::load_from_memory(&tile.data).expect("valid JPEG");
            assert!(decoded.color().has_color(), "tiles must stay color");
        }
    }
}
