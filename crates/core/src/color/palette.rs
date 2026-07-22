//! From collected document colors to a palette of panel-quantized gray tones.
//!
//! Each collected `(color, role)` pair gets a target - its H-K-corrected
//! apparent lightness clamped into the role's readable range - and a feasible
//! interval around that target. The solver then places one tone per cluster of
//! targets, and every tone is quantized to the panel's actual gray levels. The
//! role ranges deliberately resolve inverted themes: a dark page background
//! clamps up into the light range and light body text clamps down into the
//! dark range, which is exactly what makes the output readable on paper-white
//! e-ink.

use std::collections::HashMap;

use crate::profile::caps::Panel;

use super::solve::{
    CLUSTER_EPS_L, Cluster, JND_L, MAX_SHIFT_L, ToneInput, cluster_targets, min_gap_for,
    quantize_to_panel, solve_tones,
};
use super::space::{Rgb8, apparent_lightness, lightness_of_gray, rgb_to_lch};

/// Ceiling for text tones: dark enough to stay readable on a paper-white page.
const TEXT_MAX_L: f32 = 60.0;

/// Floor for background tones: light enough that dark text stays readable on
/// them.
const BG_MIN_L: f32 = 80.0;

/// Where a collected color was used. The role picks the readable range its
/// gray tone is confined to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Role {
    Text,
    Background,
    Border,
    SvgFill,
    SvgStroke,
    SvgStop,
}

impl Role {
    /// The feasible L* range for tones of this role.
    fn bounds(self) -> (f32, f32) {
        match self {
            Role::Text => (0.0, TEXT_MAX_L),
            Role::Background => (BG_MIN_L, 100.0),
            Role::Border | Role::SvgFill | Role::SvgStroke | Role::SvgStop => (0.0, 100.0),
        }
    }
}

/// Occurrence-weighted set of colors gathered from one solve scope (the
/// document's CSS, or a single SVG).
#[derive(Debug, Clone, Default)]
pub(crate) struct Collected {
    counts: HashMap<(Rgb8, Role), u32>,
}

impl Collected {
    pub(crate) fn add(&mut self, rgb: Rgb8, role: Role) {
        *self.counts.entry((rgb, role)).or_insert(0) += 1;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// The distinct `(color, role)` pairs collected so far, in no particular
    /// order. A test-side window; production reads go through the solve.
    #[cfg(test)]
    pub(crate) fn entries(&self) -> impl Iterator<Item = (Rgb8, Role)> + '_ {
        self.counts.keys().copied()
    }
}

/// The solved mapping from source color (per role) to its panel gray.
#[derive(Debug, Clone, Default)]
pub(crate) struct Palette {
    map: HashMap<(Rgb8, Role), u8>,
}

impl Palette {
    /// The gray tone assigned to `rgb` in `role`, or `None` for a color that
    /// was never collected (left untouched by the rewriters).
    pub(crate) fn gray_for(&self, rgb: Rgb8, role: Role) -> Option<Rgb8> {
        self.map
            .get(&(rgb, role))
            .map(|&value| Rgb8::new(value, value, value))
    }
}

/// Counters for the report entry a solve produces.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PaletteStats {
    /// Distinct `(color, role)` pairs that went into the solve.
    pub colors_in: usize,
    /// Distinct gray tones that came out.
    pub tones_out: usize,
    /// Colors that ended up sharing a tone with a visibly different color
    /// (targets a JND or more apart) - the panel could not keep them apart.
    pub collapsed: usize,
}

/// One solver item, tied back to the collected entries it stands for.
struct Item {
    rgb: Rgb8,
    role: Role,
    input: ToneInput,
}

/// Solve the collected colors of one scope into panel gray tones.
pub(crate) fn build_palette(collected: &Collected, panel: Panel) -> (Palette, PaletteStats) {
    if collected.is_empty() {
        return (Palette::default(), PaletteStats::default());
    }

    let mut items: Vec<Item> = collected
        .counts
        .iter()
        .map(|(&(rgb, role), &count)| Item {
            rgb,
            role,
            input: tone_input(rgb, role, count),
        })
        .collect();
    // HashMap order is arbitrary; the solve must not be. Targets first, then
    // the color and role as deterministic tie-breaks.
    items.sort_by(|a, b| {
        a.input
            .t
            .partial_cmp(&b.input.t)
            .expect("lightness is finite")
            .then(a.rgb.cmp(&b.rgb))
            .then(a.role.cmp(&b.role))
    });

    let levels = panel.gray_levels();
    let d_min = min_gap_for(levels);
    let inputs: Vec<ToneInput> = items.iter().map(|item| item.input).collect();
    let clusters = cluster_targets(&inputs, d_min, CLUSTER_EPS_L);
    let cluster_inputs: Vec<ToneInput> = clusters
        .iter()
        .map(|c| ToneInput {
            t: c.t,
            weight: c.weight,
            lo: c.lo,
            hi: c.hi,
        })
        .collect();
    let tones = solve_tones(&cluster_inputs, d_min);

    let assigned = assign_levels(&clusters, &tones, levels);

    // Group items by their final gray to count visible collapses: a group
    // whose member targets span a JND or more holds colors the reader was
    // meant to tell apart.
    let mut palette = Palette::default();
    let mut group_span: HashMap<u8, (f32, f32, usize)> = HashMap::new();
    for (cluster, &value) in clusters.iter().zip(&assigned) {
        for &member in &cluster.members {
            let item = &items[member];
            palette.map.insert((item.rgb, item.role), value);
            let entry = group_span
                .entry(value)
                .or_insert((f32::INFINITY, f32::NEG_INFINITY, 0));
            entry.0 = entry.0.min(item.input.t);
            entry.1 = entry.1.max(item.input.t);
            entry.2 += 1;
        }
    }
    let collapsed = group_span
        .values()
        .filter(|&&(min_t, max_t, _)| max_t - min_t >= JND_L)
        .map(|&(_, _, size)| size)
        .sum();

    let stats = PaletteStats {
        colors_in: items.len(),
        tones_out: group_span.len(),
        collapsed,
    };
    (palette, stats)
}

/// Build the solver input for one collected color: apparent lightness clamped
/// into the role range, a max-shift interval around it, and the black/white
/// pins.
fn tone_input(rgb: Rgb8, role: Role, count: u32) -> ToneInput {
    let (role_lo, role_hi) = role.bounds();
    let weight = count as f32;

    let black = rgb == Rgb8::new(0, 0, 0);
    let white = rgb == Rgb8::new(255, 255, 255);
    if black && role_lo == 0.0 {
        return ToneInput {
            t: 0.0,
            weight,
            lo: 0.0,
            hi: 0.0,
        };
    }
    if white && role_hi == 100.0 {
        return ToneInput {
            t: 100.0,
            weight,
            lo: 100.0,
            hi: 100.0,
        };
    }

    let raw = apparent_lightness(rgb_to_lch(rgb));
    let t = raw.clamp(role_lo, role_hi);
    // The shift window centers on the clamped target: clamping into the role
    // range is the deliberate move, the solver may only drift a bounded extra
    // distance from there.
    ToneInput {
        t,
        weight,
        lo: (t - MAX_SHIFT_L).max(role_lo),
        hi: (t + MAX_SHIFT_L).min(role_hi),
    }
}

/// Quantize each cluster's tone to a panel level while keeping distinct tones
/// a minimum number of levels apart (two on a 16-level panel, mirroring
/// [`min_gap_for`]; one on a 4-level panel). A tone that cannot be nudged far
/// enough up within its interval merges onto the previous level instead - the
/// caller's collapse counting picks that up.
fn assign_levels(clusters: &[Cluster], tones: &[f32], levels: u8) -> Vec<u8> {
    let level_values = super::solve::panel_levels(levels);
    let min_sep: usize = if levels <= 4 { 1 } else { 2 };
    let index_of = |value: u8| {
        level_values
            .iter()
            .position(|&v| v == value)
            .expect("quantize_to_panel returns a panel level")
    };

    let mut assigned = Vec::with_capacity(clusters.len());
    let mut prev_index: Option<usize> = None;
    for (cluster, &tone) in clusters.iter().zip(tones) {
        let quantized = quantize_to_panel(tone, cluster.lo, cluster.hi, levels);
        let mut index = index_of(quantized);
        if let Some(prev) = prev_index
            && index < prev + min_sep
        {
            index = (prev + min_sep..level_values.len())
                .find(|&i| lightness_of_gray(level_values[i]) <= cluster.hi + 1e-3)
                .unwrap_or(prev);
        }
        prev_index = Some(index);
        assigned.push(level_values[index]);
    }
    assigned
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(entries: &[(Rgb8, Role)], panel: Panel) -> (Palette, PaletteStats) {
        let mut collected = Collected::default();
        for &(rgb, role) in entries {
            collected.add(rgb, role);
        }
        build_palette(&collected, panel)
    }

    fn lightness(palette: &Palette, rgb: Rgb8, role: Role) -> f32 {
        let gray = palette.gray_for(rgb, role).expect("color was collected");
        lightness_of_gray(gray.r)
    }

    #[test]
    fn an_empty_collection_solves_to_an_empty_palette() {
        let (palette, stats) = build(&[], Panel::Gray16);
        assert!(palette.gray_for(Rgb8::new(1, 2, 3), Role::Text).is_none());
        assert_eq!(stats, PaletteStats::default());
    }

    #[test]
    fn black_text_and_white_background_are_fixed_points() {
        let (palette, _) = build(
            &[
                (Rgb8::new(0, 0, 0), Role::Text),
                (Rgb8::new(255, 255, 255), Role::Background),
            ],
            Panel::Gray16,
        );
        assert_eq!(
            palette.gray_for(Rgb8::new(0, 0, 0), Role::Text),
            Some(Rgb8::new(0, 0, 0))
        );
        assert_eq!(
            palette.gray_for(Rgb8::new(255, 255, 255), Role::Background),
            Some(Rgb8::new(255, 255, 255))
        );
    }

    #[test]
    fn text_tones_never_exceed_the_readability_cap() {
        // Saturated red and blue both carry big H-K lifts (targets near or
        // above 60); as text they must still land at or below the cap.
        let (palette, _) = build(
            &[
                (Rgb8::new(255, 0, 0), Role::Text),
                (Rgb8::new(0, 0, 255), Role::Text),
                (Rgb8::new(255, 255, 0), Role::Text),
            ],
            Panel::Gray16,
        );
        for rgb in [
            Rgb8::new(255, 0, 0),
            Rgb8::new(0, 0, 255),
            Rgb8::new(255, 255, 0),
        ] {
            let l = lightness(&palette, rgb, Role::Text);
            assert!(l <= TEXT_MAX_L + 1.0, "{rgb:?} text tone at L*{l}");
        }
    }

    #[test]
    fn background_tones_never_fall_below_the_floor() {
        let (palette, _) = build(
            &[
                (Rgb8::new(0, 0, 64), Role::Background),
                (Rgb8::new(200, 220, 255), Role::Background),
            ],
            Panel::Gray16,
        );
        for rgb in [Rgb8::new(0, 0, 64), Rgb8::new(200, 220, 255)] {
            let l = lightness(&palette, rgb, Role::Background);
            assert!(l >= BG_MIN_L - 1.0, "{rgb:?} background tone at L*{l}");
        }
    }

    #[test]
    fn an_inverted_theme_comes_out_readable() {
        let (palette, _) = build(
            &[
                (Rgb8::new(34, 34, 34), Role::Background),
                (Rgb8::new(238, 238, 238), Role::Text),
            ],
            Panel::Gray16,
        );
        let text_l = lightness(&palette, Rgb8::new(238, 238, 238), Role::Text);
        let bg_l = lightness(&palette, Rgb8::new(34, 34, 34), Role::Background);
        assert!(
            bg_l - text_l >= 15.0,
            "inverted theme should re-invert: text L*{text_l} on background L*{bg_l}"
        );
    }

    #[test]
    fn the_same_color_may_differ_by_role() {
        let red = Rgb8::new(200, 30, 30);
        let (palette, _) = build(&[(red, Role::Text), (red, Role::Background)], Panel::Gray16);
        let as_text = lightness(&palette, red, Role::Text);
        let as_background = lightness(&palette, red, Role::Background);
        assert!(as_text <= TEXT_MAX_L + 1.0);
        assert!(as_background >= BG_MIN_L - 1.0);
    }

    #[test]
    fn near_black_merges_into_the_black_pin() {
        let (palette, stats) = build(
            &[
                (Rgb8::new(0, 0, 0), Role::Text),
                (Rgb8::new(5, 5, 5), Role::Text),
            ],
            Panel::Gray16,
        );
        assert_eq!(
            palette.gray_for(Rgb8::new(5, 5, 5), Role::Text),
            Some(Rgb8::new(0, 0, 0)),
            "indistinguishable near-black rides the pin"
        );
        assert_eq!(stats.collapsed, 0, "a sub-JND merge is not a collapse");
    }

    #[test]
    fn distinct_svg_fills_come_out_distinct_on_gray16() {
        // Two hues of near-equal luminance - the whole point of the solver.
        let teal = Rgb8::new(0, 150, 136);
        let orange = Rgb8::new(230, 126, 34);
        let (palette, stats) = build(
            &[(teal, Role::SvgFill), (orange, Role::SvgFill)],
            Panel::Gray16,
        );
        let a = lightness(&palette, teal, Role::SvgFill);
        let b = lightness(&palette, orange, Role::SvgFill);
        assert!(
            (a - b).abs() >= JND_L - 1.0,
            "teal at L*{a} and orange at L*{b} must be told apart"
        );
        assert_eq!(stats.collapsed, 0);
        assert_eq!(stats.tones_out, 2);
    }

    #[test]
    fn a_gray4_panel_collapses_a_crowd_and_says_so() {
        // Six tones a JND-plus apart everywhere: four levels cannot hold six
        // visibly distinct tones, so some visibly different pair must share a
        // level and be counted as collapsed.
        let entries: Vec<(Rgb8, Role)> = [40u8, 80, 120, 160, 200, 240]
            .into_iter()
            .map(|v| (Rgb8::new(v, v, v), Role::SvgFill))
            .collect();
        let (palette, stats) = build(&entries, Panel::Gray4);
        assert_eq!(stats.colors_in, 6);
        assert!(
            stats.tones_out <= 4,
            "a 4-level panel holds 4 tones at most"
        );
        assert!(stats.collapsed > 0, "the collapse must be reported");
        for (rgb, role) in entries {
            let gray = palette.gray_for(rgb, role).expect("solved");
            assert!(
                super::super::solve::panel_levels(4).contains(&gray.r),
                "{gray:?} is not a gray4 level"
            );
        }
    }

    #[test]
    fn the_solve_is_deterministic_regardless_of_insertion_order() {
        let entries = [
            (Rgb8::new(180, 40, 40), Role::Text),
            (Rgb8::new(40, 140, 60), Role::Border),
            (Rgb8::new(250, 240, 220), Role::Background),
            (Rgb8::new(0, 0, 0), Role::Text),
        ];
        let (forward, _) = build(&entries, Panel::Gray16);
        let reversed: Vec<_> = entries.iter().rev().copied().collect();
        let (backward, _) = build(&reversed, Panel::Gray16);
        for &(rgb, role) in &entries {
            assert_eq!(forward.gray_for(rgb, role), backward.gray_for(rgb, role));
        }
    }

    #[test]
    fn remapping_a_remapped_palette_is_a_fixed_point() {
        let entries = [
            (Rgb8::new(180, 40, 40), Role::Text),
            (Rgb8::new(40, 140, 60), Role::SvgFill),
            (Rgb8::new(0, 0, 255), Role::SvgStroke),
        ];
        let (first, _) = build(&entries, Panel::Gray16);
        let second_entries: Vec<(Rgb8, Role)> = entries
            .iter()
            .map(|&(rgb, role)| (first.gray_for(rgb, role).expect("solved"), role))
            .collect();
        let (second, _) = build(&second_entries, Panel::Gray16);
        for &(gray, role) in &second_entries {
            assert_eq!(
                second.gray_for(gray, role),
                Some(gray),
                "an already-solved gray must map to itself"
            );
        }
    }
}
