//! The 1D tone solver: place gray tones near their targets while keeping them
//! apart, in order, and inside their per-item bounds.
//!
//! The problem is a tiny quadratic program on a line: minimize
//! `sum w_i (g_i - t_i)^2` subject to `g` non-decreasing with a minimum gap
//! `d_min` between neighbors and `lo_i <= g_i <= hi_i`. Substituting
//! `u_i = g_i - i*d_min` turns the gap constraint into plain monotonicity, and
//! the result is weighted isotonic regression with interval bounds, solved
//! exactly by pool-adjacent-violators with per-block clamping. A pinned tone
//! (pure black, pure white) is expressed as a degenerate box `lo == hi`.
//!
//! When a panel cannot keep every target apart - always possible on a 4-level
//! panel - [`cluster_targets`] first merges indistinguishable neighbors, then
//! keeps merging the closest compatible neighbors until the instance is
//! feasible. The caller turns clusters whose members were a JND or more apart
//! into a user-facing warning.

use super::space::lightness_of_gray;

/// Categorical just-noticeable difference in L*: tones this far apart read as
/// distinct at a glance (a single JND under lab conditions is ~1).
pub(crate) const JND_L: f32 = 10.0;

/// Targets closer than this are indistinguishable in print anyway and are
/// merged before solving rather than pushed apart.
pub(crate) const CLUSTER_EPS_L: f32 = 3.0;

/// Hard cap on how far a tone may move from its apparent lightness, so a
/// deliberately muted color can never be spread into prominence (or back).
pub(crate) const MAX_SHIFT_L: f32 = 30.0;

/// The minimum L* gap between distinct tones on a panel with `levels` gray
/// levels: the categorical JND or two device quantization steps, whichever is
/// larger. On a 4-level panel the steps are so coarse that one of them already
/// dwarfs the JND, and demanding two would leave room for only two tones total.
pub(crate) fn min_gap_for(levels: u8) -> f32 {
    let step = 100.0 / f32::from(levels.max(2) - 1);
    if levels <= 4 {
        step
    } else {
        JND_L.max(2.0 * step)
    }
}

/// One tone to place: target lightness `t` (already clamped into `[lo, hi]` by
/// the caller), its occurrence weight, and its feasible interval. `lo == hi`
/// pins the tone.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ToneInput {
    pub t: f32,
    pub weight: f32,
    pub lo: f32,
    pub hi: f32,
}

/// A group of solver inputs that share one output tone.
#[derive(Debug, Clone)]
pub(crate) struct Cluster {
    /// Weighted mean target of the members.
    pub t: f32,
    pub weight: f32,
    /// Intersection of the members' feasible intervals.
    pub lo: f32,
    pub hi: f32,
    /// Indices into the input slice this cluster covers.
    pub members: Vec<usize>,
    /// Smallest and largest member target: a span of a JND or more means
    /// visibly distinct colors were collapsed (worth a warning).
    pub min_t: f32,
    pub max_t: f32,
}

impl Cluster {
    fn singleton(index: usize, input: &ToneInput) -> Self {
        Cluster {
            t: input.t,
            weight: input.weight.max(f32::EPSILON),
            lo: input.lo,
            hi: input.hi,
            members: vec![index],
            min_t: input.t,
            max_t: input.t,
        }
    }

    fn merge(a: &Cluster, b: &Cluster) -> Cluster {
        let weight = a.weight + b.weight;
        Cluster {
            t: (a.t * a.weight + b.t * b.weight) / weight,
            weight,
            lo: a.lo.max(b.lo),
            hi: a.hi.min(b.hi),
            members: a.members.iter().chain(&b.members).copied().collect(),
            min_t: a.min_t.min(b.min_t),
            max_t: a.max_t.max(b.max_t),
        }
    }
}

/// Whether two feasible intervals overlap (so their members can share a tone).
fn boxes_intersect(a: &Cluster, b: &Cluster) -> bool {
    a.lo.max(b.lo) <= a.hi.min(b.hi)
}

/// Whether tones can be placed for `clusters` (sorted by target) with gap
/// `d_min`: greedy left-to-right reachability against each upper bound.
fn is_feasible(clusters: &[Cluster], d_min: f32) -> bool {
    let mut reach = f32::NEG_INFINITY;
    for cluster in clusters {
        reach = if reach == f32::NEG_INFINITY {
            cluster.lo
        } else {
            cluster.lo.max(reach + d_min)
        };
        if reach > cluster.hi + 1e-4 {
            return false;
        }
    }
    true
}

/// Group `inputs` (sorted ascending by `t`) into the clusters that will each
/// receive one tone: first merge neighbors closer than `eps` (indistinguishable
/// anyway), then keep merging the closest compatible neighbors until a solution
/// with gap `d_min` exists. Neighbors whose intervals do not overlap (e.g. a
/// text tone and a background tone) are never merged; if only incompatible
/// neighbors remain the loop gives up and the solver clamps best-effort.
pub(crate) fn cluster_targets(inputs: &[ToneInput], d_min: f32, eps: f32) -> Vec<Cluster> {
    let mut clusters: Vec<Cluster> = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        let cluster = Cluster::singleton(index, input);
        match clusters.last() {
            Some(prev) if cluster.t - prev.t < eps && boxes_intersect(prev, &cluster) => {
                let merged = Cluster::merge(prev, &cluster);
                *clusters.last_mut().expect("just matched") = merged;
            }
            _ => clusters.push(cluster),
        }
    }

    while !is_feasible(&clusters, d_min) {
        // The closest adjacent pair whose intervals overlap.
        let candidate = (0..clusters.len().saturating_sub(1))
            .filter(|&i| boxes_intersect(&clusters[i], &clusters[i + 1]))
            .min_by(|&a, &b| {
                let gap_a = clusters[a + 1].t - clusters[a].t;
                let gap_b = clusters[b + 1].t - clusters[b].t;
                gap_a.partial_cmp(&gap_b).expect("targets are finite")
            });
        let Some(i) = candidate else {
            break;
        };
        let merged = Cluster::merge(&clusters[i], &clusters[i + 1]);
        clusters[i] = merged;
        clusters.remove(i + 1);
    }
    clusters
}

/// One pooled block of the isotonic regression, in u-space.
struct Block {
    /// Sum of `weight * target` over members.
    weighted_targets: f32,
    weight: f32,
    lo: f32,
    hi: f32,
    len: usize,
}

impl Block {
    /// The block's tone: the weighted mean projected into its bounds. An empty
    /// bound intersection cannot happen on a feasible instance; the midpoint
    /// fallback keeps the solver total on the infeasible leftovers
    /// [`cluster_targets`] can hand over when only incompatible neighbors
    /// remained.
    fn value(&self) -> f32 {
        let mean = self.weighted_targets / self.weight;
        if self.lo > self.hi {
            0.5 * (self.lo + self.hi)
        } else {
            mean.clamp(self.lo, self.hi)
        }
    }
}

/// Place one tone per input (sorted ascending by `t`), minimizing the weighted
/// squared distance to the targets subject to ordering, the minimum gap
/// `d_min`, and each input's bounds. Exact for feasible instances (see the
/// module docs); returns tones in input order.
pub(crate) fn solve_tones(inputs: &[ToneInput], d_min: f32) -> Vec<f32> {
    let mut blocks: Vec<Block> = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        let shift = i as f32 * d_min;
        let weight = input.weight.max(f32::EPSILON);
        let mut block = Block {
            weighted_targets: (input.t - shift) * weight,
            weight,
            lo: input.lo - shift,
            hi: input.hi - shift,
            len: 1,
        };
        while let Some(prev) = blocks.last() {
            if prev.value() <= block.value() + 1e-6 {
                break;
            }
            let prev = blocks.pop().expect("just peeked");
            block = Block {
                weighted_targets: prev.weighted_targets + block.weighted_targets,
                weight: prev.weight + block.weight,
                lo: prev.lo.max(block.lo),
                hi: prev.hi.min(block.hi),
                len: prev.len + block.len,
            };
        }
        blocks.push(block);
    }

    let mut tones = Vec::with_capacity(inputs.len());
    for block in &blocks {
        let value = block.value();
        for _ in 0..block.len {
            let i = tones.len();
            tones.push(value + i as f32 * d_min);
        }
    }
    tones
}

/// The gray channel values a panel with `levels` gray levels can paint,
/// ascending: level i is `round(255 * i / (levels - 1))`.
pub(crate) fn panel_levels(levels: u8) -> Vec<u8> {
    let n = levels.max(2);
    (0..n)
        .map(|i| (255.0 * f32::from(i) / f32::from(n - 1)).round() as u8)
        .collect()
}

/// Quantize a solved lightness to the panel level nearest in L*, preferring
/// levels inside `[lo, hi]` (so a text tone cannot round up into
/// background-light territory). Falls back to the globally nearest level when
/// no level lands inside the interval.
pub(crate) fn quantize_to_panel(l: f32, lo: f32, hi: f32, levels: u8) -> u8 {
    let all = panel_levels(levels);
    let in_box: Vec<u8> = all
        .iter()
        .copied()
        .filter(|&value| {
            let level_l = lightness_of_gray(value);
            level_l >= lo - 1e-3 && level_l <= hi + 1e-3
        })
        .collect();
    let pool = if in_box.is_empty() { all } else { in_box };
    pool.into_iter()
        .min_by(|&a, &b| {
            let da = (lightness_of_gray(a) - l).abs();
            let db = (lightness_of_gray(b) - l).abs();
            da.partial_cmp(&db)
                .expect("lightness is finite")
                .then(a.cmp(&b))
        })
        .expect("a panel has at least two levels")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn free(t: f32) -> ToneInput {
        ToneInput {
            t,
            weight: 1.0,
            lo: 0.0,
            hi: 100.0,
        }
    }

    fn pinned(t: f32) -> ToneInput {
        ToneInput {
            t,
            weight: 1.0,
            lo: t,
            hi: t,
        }
    }

    #[test]
    fn zero_gap_pava_pools_violating_runs_to_the_mean() {
        let tones = solve_tones(&[free(3.0), free(1.0), free(2.0)], 0.0);
        for tone in tones {
            assert!(
                (tone - 2.0).abs() < 1e-4,
                "expected the pooled mean, got {tone}"
            );
        }
    }

    #[test]
    fn pooling_respects_weights() {
        let mut heavy = free(4.0);
        heavy.weight = 3.0;
        let tones = solve_tones(&[heavy, free(0.0)], 0.0);
        for tone in tones {
            assert!((tone - 3.0).abs() < 1e-4, "weighted mean is 3, got {tone}");
        }
    }

    #[test]
    fn identical_targets_are_spread_by_exactly_the_gap() {
        let tones = solve_tones(&[free(50.0), free(50.0), free(50.0)], 10.0);
        assert!((tones[0] - 40.0).abs() < 1e-3, "got {tones:?}");
        assert!((tones[1] - 50.0).abs() < 1e-3, "got {tones:?}");
        assert!((tones[2] - 60.0).abs() < 1e-3, "got {tones:?}");
    }

    #[test]
    fn well_separated_targets_are_left_alone() {
        let tones = solve_tones(&[free(10.0), free(50.0), free(90.0)], 10.0);
        assert!((tones[0] - 10.0).abs() < 1e-3);
        assert!((tones[1] - 50.0).abs() < 1e-3);
        assert!((tones[2] - 90.0).abs() < 1e-3);
    }

    #[test]
    fn bounds_are_respected() {
        let capped = ToneInput {
            t: 70.0,
            weight: 1.0,
            lo: 0.0,
            hi: 60.0,
        };
        let tones = solve_tones(&[capped], 10.0);
        assert!((tones[0] - 60.0).abs() < 1e-3);

        let lifted = ToneInput {
            t: 20.0,
            weight: 1.0,
            lo: 40.0,
            hi: 100.0,
        };
        let tones = solve_tones(&[free(10.0), lifted], 10.0);
        assert!((tones[0] - 10.0).abs() < 1e-3, "got {tones:?}");
        assert!((tones[1] - 40.0).abs() < 1e-3, "got {tones:?}");
    }

    #[test]
    fn pinned_anchors_do_not_move() {
        let tones = solve_tones(&[pinned(0.0), free(30.0), pinned(100.0)], 13.34);
        assert_eq!(tones[0], 0.0);
        assert!((tones[1] - 30.0).abs() < 1e-3);
        assert_eq!(tones[2], 100.0);
    }

    #[test]
    fn a_crowd_against_a_pinned_anchor_stacks_off_it() {
        // Both free tones want to sit at 95, but white is pinned at 100 and
        // everything must stay a gap below it.
        let tones = solve_tones(&[free(95.0), free(95.0), pinned(100.0)], 10.0);
        assert!((tones[2] - 100.0).abs() < 1e-6);
        assert!((tones[1] - 90.0).abs() < 1e-3, "got {tones:?}");
        assert!((tones[0] - 80.0).abs() < 1e-3, "got {tones:?}");
    }

    #[test]
    fn eps_merge_pools_indistinguishable_neighbors() {
        let clusters = cluster_targets(&[free(10.0), free(11.5), free(40.0)], 13.34, 3.0);
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].members, vec![0, 1]);
        assert!((clusters[0].t - 10.75).abs() < 1e-3);
        assert!(clusters[0].max_t - clusters[0].min_t < JND_L);
    }

    #[test]
    fn infeasible_crowds_merge_until_the_panel_can_hold_them() {
        // Six tones capped at 60 with a 33.3 gap: only two fit (0-ish and
        // 33.3-60); everything must merge down to two clusters.
        let inputs: Vec<ToneInput> = [5.0, 15.0, 25.0, 35.0, 45.0, 55.0]
            .iter()
            .map(|&t| ToneInput {
                t,
                weight: 1.0,
                lo: 0.0,
                hi: 60.0,
            })
            .collect();
        let d_min = min_gap_for(4);
        let clusters = cluster_targets(&inputs, d_min, CLUSTER_EPS_L);
        assert!(clusters.len() <= 2, "got {} clusters", clusters.len());
        let all_members: usize = clusters.iter().map(|c| c.members.len()).sum();
        assert_eq!(all_members, 6, "every input stays accounted for");
        assert!(
            clusters.iter().any(|c| c.max_t - c.min_t >= JND_L),
            "a visible collapse must be detectable for the warning"
        );
    }

    #[test]
    fn feasibility_merging_takes_the_closest_pair_first() {
        // eps = 0 isolates the feasibility loop: four tones cannot hold a
        // 33.34 gap in [0, 100], and the closest pair (20, 22) must be the one
        // that merges.
        let inputs = [free(10.0), free(20.0), free(22.0), free(55.0)];
        let clusters = cluster_targets(&inputs, 33.34, 0.0);
        assert_eq!(clusters.len(), 3, "one merge suffices");
        assert_eq!(clusters[1].members, vec![1, 2]);
    }

    #[test]
    fn panel_levels_match_the_hardware() {
        assert_eq!(panel_levels(4), vec![0, 85, 170, 255]);
        let sixteen = panel_levels(16);
        assert_eq!(sixteen.len(), 16);
        assert_eq!(sixteen[0], 0);
        assert_eq!(sixteen[1], 17);
        assert_eq!(sixteen[15], 255);
        for pair in sixteen.windows(2) {
            let step = pair[1] - pair[0];
            assert!(step == 17, "16-level steps are 17, got {step}");
        }
    }

    #[test]
    fn quantize_picks_the_nearest_level_by_lightness() {
        assert_eq!(quantize_to_panel(0.0, 0.0, 100.0, 4), 0);
        assert_eq!(quantize_to_panel(100.0, 0.0, 100.0, 4), 255);
        assert_eq!(quantize_to_panel(38.0, 0.0, 100.0, 4), 85);
        assert_eq!(quantize_to_panel(70.0, 0.0, 100.0, 4), 170);
    }

    #[test]
    fn quantize_prefers_a_level_inside_the_bounds() {
        // 70 is nearest the 170 level (L* ~73), but a text tone is capped at
        // 60 and must round down into its interval instead.
        assert_eq!(quantize_to_panel(70.0, 0.0, 60.0, 4), 85);
    }

    #[test]
    fn min_gap_scales_with_the_panel() {
        assert!(
            (min_gap_for(4) - 100.0 / 3.0).abs() < 0.01,
            "4 levels: one step"
        );
        assert!(
            (min_gap_for(16) - 13.333).abs() < 0.01,
            "16 levels: two steps"
        );
        assert_eq!(
            min_gap_for(200),
            JND_L,
            "a dense panel bottoms out at the JND"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(300))]

        #[test]
        fn solved_tones_satisfy_every_constraint(
            mut targets in prop::collection::vec(0.0f32..100.0, 1..8),
            weights in prop::collection::vec(0.5f32..5.0, 8),
        ) {
            targets.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
            let d_min = min_gap_for(16);
            let inputs: Vec<ToneInput> = targets
                .iter()
                .zip(&weights)
                .map(|(&t, &weight)| ToneInput { t, weight, lo: 0.0, hi: 100.0 })
                .collect();
            // Up to 8 tones with gap 13.34 always fit in [0, 100].
            let tones = solve_tones(&inputs, d_min);
            for (i, &tone) in tones.iter().enumerate() {
                prop_assert!((-1e-3..=100.0 + 1e-3).contains(&tone), "out of range: {tone}");
                if i > 0 {
                    prop_assert!(
                        tone - tones[i - 1] >= d_min - 1e-3,
                        "gap violated: {} then {tone}", tones[i - 1]
                    );
                }
            }
        }

        #[test]
        fn clustering_always_yields_a_feasible_instance(
            mut targets in prop::collection::vec(0.0f32..100.0, 1..24),
        ) {
            targets.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
            let d_min = min_gap_for(4);
            let inputs: Vec<ToneInput> = targets
                .iter()
                .map(|&t| ToneInput { t, weight: 1.0, lo: 0.0, hi: 100.0 })
                .collect();
            let clusters = cluster_targets(&inputs, d_min, CLUSTER_EPS_L);
            prop_assert!(is_feasible(&clusters, d_min));
            let members: usize = clusters.iter().map(|c| c.members.len()).sum();
            prop_assert_eq!(members, inputs.len());
            let solver_inputs: Vec<ToneInput> = clusters
                .iter()
                .map(|c| ToneInput { t: c.t, weight: c.weight, lo: c.lo, hi: c.hi })
                .collect();
            let tones = solve_tones(&solver_inputs, d_min);
            for (i, &tone) in tones.iter().enumerate() {
                prop_assert!((-1e-3..=100.0 + 1e-3).contains(&tone));
                if i > 0 {
                    prop_assert!(tone - tones[i - 1] >= d_min - 1e-3);
                }
            }
        }
    }
}
