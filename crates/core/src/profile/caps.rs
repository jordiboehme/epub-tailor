//! Hardware and firmware capability numbers a device profile carries.

use serde::{Deserialize, Serialize};

/// What the panel can actually paint.
///
/// One attribute carries both facts the pipeline needs: whether color survives,
/// and how many gray levels the panel resolves. They are not independent - a
/// Kaleido panel renders 4096 colors *and* 16 grays - so folding them into a
/// single value keeps a profile from claiming a combination no panel has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Panel {
    /// 2-bit grayscale: the Xteink/CrossPoint panels, Bayer-dithered to four levels.
    Gray4,
    /// 4-bit grayscale: E Ink Carta and friends, the usual 16 levels.
    Gray16,
    /// A color panel (E Ink Kaleido 3 on every device we ship a profile for):
    /// 4096 colors over the same 16 gray levels.
    Color,
}

impl Panel {
    /// Whether color survives to the screen. Drives the image pipeline: a
    /// grayscale panel gets 8-bit luma (all it can show, and a third of the
    /// bytes); a color panel keeps RGB through fitting, budgeting and encoding.
    pub fn is_color(self) -> bool {
        matches!(self, Panel::Color)
    }

    /// Distinct gray levels the panel resolves.
    pub fn gray_levels(self) -> u8 {
        match self {
            Panel::Gray4 => 4,
            Panel::Gray16 | Panel::Color => 16,
        }
    }
}

/// Decode and render capability numbers for a target device.
///
/// For the built-in device profiles these numbers are not arbitrary: every
/// field maps to a documented firmware limitation (decode caps, screen
/// geometry, budget bytes for community-tested "reads well on device" image
/// sizes). See `docs/device-constraints.md` for the rationale behind each
/// value. Every consumer of a cap is feature-gated (see
/// [`super::Features`]), so a profile that disables a transform never has the
/// matching cap consulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DeviceCaps {
    pub screen_w: u32,
    pub screen_h: u32,
    pub ppi: u32,
    /// What the panel paints: grayscale (and at what depth) or color.
    pub panel: Panel,
    /// Firmware decode hard cap: images larger than this abort decoding entirely.
    pub max_src_px: (u32, u32),
    /// Target size to fit an inline (in-flow) image to, without upscaling.
    pub inline_max: (u32, u32),
    /// Target size to fit a cover image to, without upscaling.
    pub cover_max: (u32, u32),
    /// Community-tested byte budget for inline images.
    pub inline_budget_bytes: usize,
    /// Community-tested byte budget for cover images.
    pub cover_budget_bytes: usize,
    /// Firmware cap on bytes read from a single CSS file.
    pub css_max_bytes: usize,
    /// Firmware cap on CSS rules parsed for the whole book.
    pub css_max_rules: usize,
}

impl DeviceCaps {
    /// Capabilities of the Xteink X4 (CrossPoint firmware).
    pub fn x4() -> Self {
        DeviceCaps {
            screen_w: 480,
            screen_h: 800,
            ppi: 220,
            panel: Panel::Gray4,
            max_src_px: (2048, 1536),
            inline_max: (480, 730),
            cover_max: (480, 800),
            inline_budget_bytes: 100 * 1024,
            cover_budget_bytes: 127 * 1024,
            css_max_bytes: 128 * 1024,
            css_max_rules: 1500,
        }
    }

    /// Capabilities of the Xteink X3 (CrossPoint firmware).
    pub fn x3() -> Self {
        DeviceCaps {
            screen_w: 528,
            screen_h: 792,
            ppi: 220,
            panel: Panel::Gray4,
            max_src_px: (2048, 1536),
            inline_max: (528, 722),
            cover_max: (528, 792),
            inline_budget_bytes: 100 * 1024,
            cover_budget_bytes: 127 * 1024,
            css_max_bytes: 128 * 1024,
            css_max_rules: 1500,
        }
    }

    /// Effectively unbounded caps for the all-capable `epub` profile.
    ///
    /// That profile switches every cap-consuming transform off, so these
    /// values are a belt-and-braces guarantee: even a consumer consulted by
    /// mistake could never shrink, re-encode, discolor or split anything.
    /// The panel is [`Panel::Color`] for the same reason: grayscaling is a
    /// downgrade, so the permissive baseline never does it. A grayscale device
    /// profile opts into it with `"panel": "gray4"` or `"gray16"`.
    pub fn permissive() -> Self {
        DeviceCaps {
            screen_w: u32::MAX,
            screen_h: u32::MAX,
            ppi: 0,
            panel: Panel::Color,
            max_src_px: (u32::MAX, u32::MAX),
            inline_max: (u32::MAX, u32::MAX),
            cover_max: (u32::MAX, u32::MAX),
            inline_budget_bytes: usize::MAX,
            cover_budget_bytes: usize::MAX,
            css_max_bytes: usize::MAX,
            css_max_rules: usize::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x4_has_expected_values() {
        let c = DeviceCaps::x4();
        assert_eq!(c.screen_w, 480);
        assert_eq!(c.screen_h, 800);
        assert_eq!(c.ppi, 220);
        assert_eq!(c.panel, Panel::Gray4);
        assert_eq!(c.max_src_px, (2048, 1536));
        assert_eq!(c.inline_max, (480, 730));
        assert_eq!(c.cover_max, (480, 800));
        assert_eq!(c.inline_budget_bytes, 100 * 1024);
        assert_eq!(c.cover_budget_bytes, 127 * 1024);
        assert_eq!(c.css_max_bytes, 128 * 1024);
        assert_eq!(c.css_max_rules, 1500);
    }

    #[test]
    fn x3_has_expected_values() {
        let c = DeviceCaps::x3();
        assert_eq!(c.screen_w, 528);
        assert_eq!(c.screen_h, 792);
        assert_eq!(c.ppi, 220);
        assert_eq!(c.panel, Panel::Gray4);
        assert_eq!(c.max_src_px, (2048, 1536));
        assert_eq!(c.inline_max, (528, 722));
        assert_eq!(c.cover_max, (528, 792));
        assert_eq!(c.inline_budget_bytes, 100 * 1024);
        assert_eq!(c.cover_budget_bytes, 127 * 1024);
        assert_eq!(c.css_max_bytes, 128 * 1024);
        assert_eq!(c.css_max_rules, 1500);
    }

    #[test]
    fn permissive_never_undercuts_a_real_device() {
        let p = DeviceCaps::permissive();
        for real in [DeviceCaps::x4(), DeviceCaps::x3()] {
            assert!(p.max_src_px.0 >= real.max_src_px.0);
            assert!(p.inline_max.1 >= real.inline_max.1);
            assert!(p.inline_budget_bytes >= real.inline_budget_bytes);
            assert!(p.css_max_bytes >= real.css_max_bytes);
            assert!(p.css_max_rules >= real.css_max_rules);
            // Grayscaling is a downgrade: the permissive baseline never does it.
            assert!(p.panel.is_color() || !real.panel.is_color());
        }
    }

    #[test]
    fn a_panel_carries_both_its_color_and_its_gray_depth() {
        assert!(!Panel::Gray4.is_color());
        assert_eq!(Panel::Gray4.gray_levels(), 4);
        assert!(!Panel::Gray16.is_color());
        assert_eq!(Panel::Gray16.gray_levels(), 16);
        // Kaleido renders 4096 colors over the same 16 grays.
        assert!(Panel::Color.is_color());
        assert_eq!(Panel::Color.gray_levels(), 16);
    }

    #[test]
    fn panel_names_round_trip_through_json() {
        for (json, panel) in [
            ("\"gray4\"", Panel::Gray4),
            ("\"gray16\"", Panel::Gray16),
            ("\"color\"", Panel::Color),
        ] {
            assert_eq!(serde_json::from_str::<Panel>(json).expect("parses"), panel);
            assert_eq!(serde_json::to_string(&panel).expect("serializes"), json);
        }
    }
}
