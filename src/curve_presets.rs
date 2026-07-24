//! Seed quick-slot curves for Pump presets.
//!
//! These shapes are the factory contents for each preset's 8 overwriteable
//! quick slots. They are not applied directly by name at runtime; instead,
//! each selected preset carries its own mutable copy of these curves.

use std::sync::OnceLock;

use crate::curve::{CurveNode, CurveSegment, EditableCurve};

/// One factory quick-slot seed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct QuickSlotSeed {
    /// Stable internal name used by tests and debug output.
    pub name: &'static str,
    /// Seeded editable curve for this slot.
    pub curve: EditableCurve,
}

/// Return the stable ordered factory quick-slot seeds.
pub(crate) fn quick_slot_seeds() -> &'static [QuickSlotSeed] {
    static SEEDS: OnceLock<Vec<QuickSlotSeed>> = OnceLock::new();
    SEEDS.get_or_init(|| {
        vec![
            quick_slot_seed("Sine", sine_curve()),
            quick_slot_seed("Soft", soft_curve()),
            quick_slot_seed("Tight", tight_curve()),
            quick_slot_seed("Long", long_curve()),
            quick_slot_seed("Snap", snap_curve()),
            quick_slot_seed("Punch", punch_curve()),
            quick_slot_seed("Thump", thump_curve()),
            quick_slot_seed("Double", double_curve()),
        ]
    })
}

fn quick_slot_seed(name: &'static str, curve: EditableCurve) -> QuickSlotSeed {
    QuickSlotSeed {
        name,
        curve: curve.normalized(),
    }
}

fn curve(nodes: &[(f32, f32)], tensions: &[f32]) -> EditableCurve {
    EditableCurve {
        nodes: nodes
            .iter()
            .copied()
            .map(|(x, y)| CurveNode { x, y })
            .collect(),
        segments: tensions
            .iter()
            .copied()
            .map(|tension| CurveSegment { tension })
            .collect(),
        ..EditableCurve::default()
    }
    .normalized()
}

fn sine_curve() -> EditableCurve {
    curve(
        &[
            (0.0, 1.0),
            (0.16, 0.82),
            (0.34, 0.36),
            (0.5, 0.0),
            (0.66, 0.36),
            (0.84, 0.82),
            (1.0, 1.0),
        ],
        &[-0.55, -0.1, 0.25, -0.25, 0.1, 0.55],
    )
}

fn soft_curve() -> EditableCurve {
    curve(
        &[(0.0, 1.0), (0.08, 0.12), (0.34, 0.58), (1.0, 1.0)],
        &[-0.35, 0.42, -0.08],
    )
}

fn tight_curve() -> EditableCurve {
    curve(
        &[(0.0, 1.0), (0.035, 0.02), (0.15, 0.8), (1.0, 1.0)],
        &[-0.58, 0.72, -0.04],
    )
}

fn long_curve() -> EditableCurve {
    curve(
        &[(0.0, 1.0), (0.05, 0.04), (0.46, 0.22), (1.0, 1.0)],
        &[-0.48, 0.58, -0.02],
    )
}

fn snap_curve() -> EditableCurve {
    curve(
        &[
            (0.0, 1.0),
            (0.012, 0.22),
            (0.03, 0.0),
            (0.075, 0.94),
            (1.0, 1.0),
        ],
        &[-0.08, 0.02, 0.12, 0.0],
    )
}

fn punch_curve() -> EditableCurve {
    curve(
        &[
            (0.0, 1.0),
            (0.02, 0.06),
            (0.12, 0.44),
            (0.28, 0.18),
            (0.58, 0.9),
            (1.0, 1.0),
        ],
        &[-0.52, 0.3, -0.24, 0.22, -0.04],
    )
}

fn thump_curve() -> EditableCurve {
    curve(
        &[
            (0.0, 1.0),
            (0.03, 0.08),
            (0.18, 0.12),
            (0.42, 0.48),
            (0.72, 0.9),
            (1.0, 1.0),
        ],
        &[-0.35, 0.05, 0.12, 0.08, -0.02],
    )
}

fn double_curve() -> EditableCurve {
    curve(
        &[
            (0.0, 1.0),
            (0.04, 0.05),
            (0.18, 0.82),
            (0.34, 0.2),
            (0.56, 0.9),
            (1.0, 1.0),
        ],
        &[-0.48, 0.18, -0.28, 0.2, -0.05],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{editable_curve_to_table, MAX_SEGMENT_TENSION, MIN_SEGMENT_TENSION};

    #[test]
    fn quick_slot_seeds_have_stable_order() {
        let names: Vec<_> = quick_slot_seeds().iter().map(|seed| seed.name).collect();
        assert_eq!(
            names,
            vec!["Sine", "Soft", "Tight", "Long", "Snap", "Punch", "Thump", "Double"]
        );
    }

    #[test]
    fn quick_slot_seeds_stay_bounded_and_normalized() {
        for seed in quick_slot_seeds() {
            assert!(
                seed.curve.nodes.len() >= 2,
                "seed {} should provide at least two nodes",
                seed.name
            );
            assert_eq!(seed.curve.nodes[0].x, 0.0);
            assert_eq!(seed.curve.nodes[seed.curve.nodes.len() - 1].x, 1.0);
            assert_eq!(seed.curve.segments.len(), seed.curve.nodes.len() - 1);
            assert!(
                seed.curve
                    .nodes
                    .iter()
                    .all(|node| (0.0..=1.0).contains(&node.y)),
                "seed {} nodes should stay within the normalized gain range",
                seed.name
            );
            assert!(
                seed.curve.segments.iter().all(|segment| {
                    (MIN_SEGMENT_TENSION..=MAX_SEGMENT_TENSION).contains(&segment.tension)
                }),
                "seed {} segment tensions should stay within the supported range",
                seed.name
            );
            let last_index = seed.curve.nodes.len() - 1;
            assert!(
                (seed.curve.nodes[0].y - seed.curve.nodes[last_index].y).abs() <= f32::EPSILON,
                "seed {} endpoints should remain coupled",
                seed.name
            );
            let table = editable_curve_to_table(&seed.curve);
            assert!(
                table.iter().all(|value| (0.0..=1.0).contains(value)),
                "seed {} table should remain bounded",
                seed.name
            );
        }
    }
}
