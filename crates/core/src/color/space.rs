//! Perceptual color math for the gray-tone remap: sRGB to CIE Lab/LCh (D65)
//! and the Helmholtz-Kohlrausch "apparent lightness" correction.
//!
//! Plain luminance is not what a color *looks* like: a saturated color appears
//! noticeably brighter than a gray of equal luminance (the Helmholtz-Kohlrausch
//! effect), which is why naive grayscale renders red text too dark. The
//! correction here is Fairchild & Pirrotta's chromatic lightness L** (1991), a
//! closed-form model of the effect over CIELAB - the same practical route the
//! "Apparent Greyscale" conversion (Smith et al., Eurographics 2008) takes via
//! Nayatani's VAC model. A gray has zero chroma and therefore zero lift, which
//! makes remapping an already-remapped book a fixed point.
//!
//! Everything is hand-rolled on purpose: the solver only ever needs the 1D
//! lightness axis, far too small a slice of a color library to justify a new
//! dependency.

/// An opaque sRGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Rgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb8 {
    pub(crate) fn new(r: u8, g: u8, b: u8) -> Self {
        Rgb8 { r, g, b }
    }

    /// The `#rrggbb` form, the shape every rewritten value is emitted in.
    pub(crate) fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// A color in CIE LCh(ab), D65: lightness `l` in [0,100], chroma `c` >= 0,
/// hue angle `h_deg` in [0,360).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Lch {
    pub l: f32,
    pub c: f32,
    pub h_deg: f32,
}

/// D65 reference white in XYZ (Y normalized to 1).
const WHITE_X: f32 = 0.950_47;
const WHITE_Z: f32 = 1.088_83;

/// CIE Lab constants: `EPSILON` = (6/29)^3, `KAPPA` = (29/3)^3.
const EPSILON: f32 = 216.0 / 24389.0;
const KAPPA: f32 = 24389.0 / 27.0;

/// IEC 61966-2-1 sRGB decoding: one 8-bit channel to linear light in [0,1].
fn srgb_to_linear(channel: u8) -> f32 {
    let u = f32::from(channel) / 255.0;
    if u <= 0.04045 {
        u / 12.92
    } else {
        ((u + 0.055) / 1.055).powf(2.4)
    }
}

/// IEC 61966-2-1 sRGB encoding: linear light in [0,1] to one 8-bit channel.
/// Production code only ever emits panel level values, so this inverse (and
/// [`gray_for_lightness`] on top of it) exists to pin the math in tests.
#[cfg(test)]
fn linear_to_srgb(linear: f32) -> u8 {
    let clamped = linear.clamp(0.0, 1.0);
    let encoded = if clamped <= 0.003_130_8 {
        12.92 * clamped
    } else {
        1.055 * clamped.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

/// The Lab companding function f(t).
fn lab_f(t: f32) -> f32 {
    if t > EPSILON {
        t.cbrt()
    } else {
        (KAPPA * t + 16.0) / 116.0
    }
}

/// Convert an opaque sRGB color to CIE LCh (D65).
pub(crate) fn rgb_to_lch(rgb: Rgb8) -> Lch {
    let (r, g, b) = (
        srgb_to_linear(rgb.r),
        srgb_to_linear(rgb.g),
        srgb_to_linear(rgb.b),
    );
    // sRGB primaries to XYZ, D65.
    let x = 0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b;
    let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175 * b;
    let z = 0.019_333_9 * r + 0.119_192 * g + 0.950_304_1 * b;

    let (fx, fy, fz) = (lab_f(x / WHITE_X), lab_f(y), lab_f(z / WHITE_Z));
    let l = 116.0 * fy - 16.0;
    let a = 500.0 * (fx - fy);
    let b = 200.0 * (fy - fz);

    let c = (a * a + b * b).sqrt();
    // Sub-noise chroma IS an exact neutral: the sRGB matrix rows do not sum
    // to exactly 1.0 in f32, so a pure gray picks up ~1e-6 of chroma whose
    // survival depends on the platform's rounding (x86 keeps it, ARM happens
    // to cancel it). The nearest real color on the u8 grid is ~0.3 C* away,
    // so anything below 1e-4 is float noise - snap it (and the then-undefined
    // hue) to zero so neutrals are exact fixed points everywhere.
    if c < 1e-4 {
        return Lch {
            l,
            c: 0.0,
            h_deg: 0.0,
        };
    }
    let h_deg = b.atan2(a).to_degrees().rem_euclid(360.0);
    Lch { l, c, h_deg }
}

/// Helmholtz-Kohlrausch-corrected "apparent lightness" in [0,100]: Fairchild &
/// Pirrotta's L** = L* + (2.5 - 0.025 L*) (0.116 |sin((h - 90deg)/2)| + 0.085) C*.
///
/// The hue factor bottoms out at yellow (h = 90deg, whose luminance already
/// tracks its appearance) and peaks toward blue/violet, where the effect is
/// strongest. A neutral (C* = 0) is returned unchanged.
pub(crate) fn apparent_lightness(lch: Lch) -> f32 {
    let hue_factor = 0.116 * ((lch.h_deg - 90.0).to_radians() / 2.0).sin().abs() + 0.085;
    let lightness_factor = 2.5 - 0.025 * lch.l;
    (lch.l + lightness_factor * hue_factor * lch.c).clamp(0.0, 100.0)
}

/// The gray with Lab lightness `l_star`, as one 8-bit sRGB channel value
/// (r = g = b). A gray has zero chroma, so its apparent lightness IS its L*;
/// this is the exact inverse of [`lightness_of_gray`] and exists to pin that
/// in tests (production only emits panel level values).
#[cfg(test)]
pub(crate) fn gray_for_lightness(l_star: f32) -> u8 {
    let l = l_star.clamp(0.0, 100.0);
    // Inverse Lab companding for a neutral: L* -> Y; for a gray the linear
    // channel value equals Y (the luminance weights sum to 1).
    let y = if l > 8.0 {
        ((l + 16.0) / 116.0).powi(3)
    } else {
        l / KAPPA
    };
    linear_to_srgb(y)
}

/// The Lab lightness L* of the gray `value` (r = g = b), in [0,100].
pub(crate) fn lightness_of_gray(value: u8) -> f32 {
    let y = srgb_to_linear(value);
    116.0 * lab_f(y) - 16.0
}

/// Composite a translucent sRGB color over white, the way a renderer paints it
/// on an unstyled page. Compositing happens on the encoded channels, matching
/// CSS `rgba()` rendering. `alpha` is in [0,1].
pub(crate) fn composite_over_white(r: u8, g: u8, b: u8, alpha: f32) -> Rgb8 {
    let a = alpha.clamp(0.0, 1.0);
    let mix = |c: u8| -> u8 { (a * f32::from(c) + (1.0 - a) * 255.0).round() as u8 };
    Rgb8::new(mix(r), mix(g), mix(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_and_white_hit_the_lightness_endpoints() {
        assert!(rgb_to_lch(Rgb8::new(0, 0, 0)).l.abs() < 1e-3);
        assert!((rgb_to_lch(Rgb8::new(255, 255, 255)).l - 100.0).abs() < 1e-2);
    }

    #[test]
    fn mid_gray_has_the_textbook_lightness() {
        let l = rgb_to_lch(Rgb8::new(128, 128, 128)).l;
        assert!((l - 53.59).abs() < 0.1, "L* of #808080 was {l}");
    }

    #[test]
    fn grays_are_neutral() {
        for v in [0u8, 31, 128, 200, 255] {
            let lch = rgb_to_lch(Rgb8::new(v, v, v));
            assert!(lch.c < 1e-3, "gray {v} has chroma {}", lch.c);
            assert_eq!(lch.h_deg, 0.0, "neutral hue is pinned");
        }
    }

    #[test]
    fn red_gets_a_helmholtz_kohlrausch_lift() {
        let lch = rgb_to_lch(Rgb8::new(255, 0, 0));
        assert!((lch.l - 53.24).abs() < 0.3, "L* of red was {}", lch.l);
        let apparent = apparent_lightness(lch);
        assert!(
            (60.0..=80.0).contains(&apparent),
            "red should appear brighter than its luminance, got {apparent}"
        );
    }

    #[test]
    fn saturated_blue_gets_the_strongest_lift() {
        // The hue factor peaks toward blue/violet: the H-K effect is strongest
        // there, which is exactly why luminance-only grayscale reads blue as
        // far too dark.
        let blue = rgb_to_lch(Rgb8::new(0, 0, 255));
        let lift = apparent_lightness(blue) - blue.l;
        let red = rgb_to_lch(Rgb8::new(255, 0, 0));
        let red_lift = apparent_lightness(red) - red.l;
        assert!(lift > red_lift, "blue lift {lift} vs red lift {red_lift}");
    }

    #[test]
    fn yellow_gets_almost_no_lift() {
        // h = 90deg zeroes the sine term; only the small constant remains
        // relative to yellow's already-high lightness.
        let yellow = rgb_to_lch(Rgb8::new(255, 255, 0));
        let apparent = apparent_lightness(yellow);
        assert!(
            apparent - yellow.l < 10.0,
            "yellow lift was {}",
            apparent - yellow.l
        );
    }

    #[test]
    fn a_neutral_has_no_lift_at_all() {
        for v in [0u8, 77, 128, 255] {
            let lch = rgb_to_lch(Rgb8::new(v, v, v));
            assert_eq!(apparent_lightness(lch), lch.l);
        }
    }

    #[test]
    fn gray_for_lightness_inverts_lightness_of_gray_exactly() {
        for v in 0..=255u8 {
            assert_eq!(
                gray_for_lightness(lightness_of_gray(v)),
                v,
                "round-trip broke at {v}"
            );
        }
    }

    #[test]
    fn gray_for_lightness_is_monotonic() {
        let mut prev = gray_for_lightness(0.0);
        for step in 1..=200 {
            let next = gray_for_lightness(step as f32 * 0.5);
            assert!(next >= prev, "not monotonic at L*={}", step as f32 * 0.5);
            prev = next;
        }
    }

    #[test]
    fn gray_for_lightness_clamps_out_of_range_input() {
        assert_eq!(gray_for_lightness(-5.0), 0);
        assert_eq!(gray_for_lightness(120.0), 255);
    }

    #[test]
    fn compositing_over_white_matches_css_rendering() {
        assert_eq!(composite_over_white(0, 0, 0, 0.0), Rgb8::new(255, 255, 255));
        assert_eq!(composite_over_white(10, 20, 30, 1.0), Rgb8::new(10, 20, 30));
        let half_black = composite_over_white(0, 0, 0, 0.5);
        assert!(
            half_black.r >= 127 && half_black.r <= 128,
            "got {half_black:?}"
        );
        assert_eq!(half_black.r, half_black.g);
        assert_eq!(half_black.g, half_black.b);
    }

    #[test]
    fn hex_form_is_lowercase_rrggbb() {
        assert_eq!(Rgb8::new(74, 74, 74).to_hex(), "#4a4a4a");
        assert_eq!(Rgb8::new(0, 255, 15).to_hex(), "#00ff0f");
    }
}
