//! Built-in quick-shape curve presets for Pump.
//!
//! These shapes provide fast starting points for common pumping envelopes
//! without becoming part of the persistent preset-bank model.

use std::sync::OnceLock;

use crate::curve::{CurveNode, CurveSegment, EditableCurve};
use crate::params::{MAX_SYNC_DIVISION, SYNC_DIVISIONS};

/// One built-in quick-shape preset shown in the Pump GUI.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct QuickShapePreset {
    /// Stable button/action key used by the declarative UI.
    pub key: &'static str,
    /// Compact label rendered on the quick-shape button.
    pub label: &'static str,
    /// Sync-division index applied together with the curve.
    pub sync_division: usize,
    /// Editable curve shape applied when the button is pressed.
    pub curve: EditableCurve,
}

/// Return the stable ordered list of built-in quick-shape presets.
pub(crate) fn quick_shape_presets() -> &'static [QuickShapePreset] {
    static PRESETS: OnceLock<Vec<QuickShapePreset>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        vec![
            quick_shape_preset("shape-sine", "Sine", 4, sine_curve()),
            quick_shape_preset("shape-soft", "Soft", 4, soft_curve()),
            quick_shape_preset("shape-tght", "Tght", 2, tight_curve()),
            quick_shape_preset("shape-long", "Long", 5, long_curve()),
            quick_shape_preset("shape-cut", "Cut", 0, cut_curve()),
            quick_shape_preset("shape-gate", "Gate", 2, gate_curve()),
            quick_shape_preset("shape-ramp", "Ramp", 3, ramp_curve()),
            quick_shape_preset("shape-trip", "Trip", 1, trip_curve()),
        ]
    })
}

/// Resolve one quick-shape preset by its stable action key.
pub(crate) fn quick_shape_preset_by_key(key: &str) -> Option<&'static QuickShapePreset> {
    quick_shape_presets()
        .iter()
        .find(|preset| preset.key == key)
}

fn quick_shape_preset(
    key: &'static str,
    label: &'static str,
    sync_division: usize,
    curve: EditableCurve,
) -> QuickShapePreset {
    debug_assert!(sync_division <= MAX_SYNC_DIVISION as usize);
    debug_assert!(sync_division < SYNC_DIVISIONS.len());
    QuickShapePreset {
        key,
        label,
        sync_division: sync_division.min(MAX_SYNC_DIVISION as usize),
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
    }
    .normalized()
}

fn sine_curve() -> EditableCurve {
    curve(
        &[
            (0.0, 1.0),
            (0.18, 0.34),
            (0.5, 0.0),
            (0.82, 0.34),
            (1.0, 1.0),
        ],
        &[-0.45, 0.05, -0.05, 0.45],
    )
}

fn soft_curve() -> EditableCurve {
    curve(
        &[(0.0, 1.0), (0.08, 0.08), (0.34, 0.58), (1.0, 1.0)],
        &[-0.35, 0.42, -0.08],
    )
}

fn tight_curve() -> EditableCurve {
    curve(
        &[(0.0, 1.0), (0.045, 0.02), (0.18, 0.78), (1.0, 1.0)],
        &[-0.55, 0.7, -0.05],
    )
}

fn long_curve() -> EditableCurve {
    curve(
        &[(0.0, 1.0), (0.055, 0.03), (0.48, 0.24), (1.0, 1.0)],
        &[-0.5, 0.55, -0.02],
    )
}

fn cut_curve() -> EditableCurve {
    curve(
        &[
            (0.0, 1.0),
            (0.015, 0.15),
            (0.04, 0.0),
            (0.085, 0.96),
            (1.0, 1.0),
        ],
        &[-0.1, 0.0, 0.1, 0.0],
    )
}

fn gate_curve() -> EditableCurve {
    curve(
        &[
            (0.0, 1.0),
            (0.02, 0.0),
            (0.45, 0.0),
            (0.49, 1.0),
            (1.0, 1.0),
        ],
        &[0.0, 0.0, 0.0, 0.0],
    )
}

fn ramp_curve() -> EditableCurve {
    curve(&[(0.0, 1.0), (0.05, 0.0), (1.0, 1.0)], &[-0.1, 0.0])
}

fn trip_curve() -> EditableCurve {
    curve(
        &[
            (0.0, 1.0),
            (0.055, 0.05),
            (0.24, 0.82),
            (0.41, 0.3),
            (0.63, 0.94),
            (1.0, 1.0),
        ],
        &[-0.45, 0.28, -0.3, 0.25, -0.04],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{editable_curve_to_table, MAX_SEGMENT_TENSION, MIN_SEGMENT_TENSION};
    use crate::params::DEFAULT_SYNC_DIVISION_INDEX;

    #[test]
    fn quick_shape_presets_have_stable_order() {
        let labels: Vec<_> = quick_shape_presets()
            .iter()
            .map(|preset| preset.label)
            .collect();
        assert_eq!(
            labels,
            vec!["Sine", "Soft", "Tght", "Long", "Cut", "Gate", "Ramp", "Trip"]
        );
    }

    #[test]
    fn quick_shape_presets_stay_bounded_and_normalized() {
        for preset in quick_shape_presets() {
            assert!(
                preset.sync_division <= MAX_SYNC_DIVISION as usize,
                "sync division should stay within supported range for {}",
                preset.label
            );
            assert!(
                preset.curve.nodes.len() >= 2,
                "preset {} should provide at least two nodes",
                preset.label
            );
            assert_eq!(preset.curve.nodes[0].x, 0.0);
            assert_eq!(preset.curve.nodes[preset.curve.nodes.len() - 1].x, 1.0);
            assert_eq!(
                preset.curve.segments.len(),
                preset.curve.nodes.len() - 1,
                "preset {} should provide one segment per node span",
                preset.label
            );
            assert!(
                preset
                    .curve
                    .nodes
                    .iter()
                    .all(|node| (0.0..=1.0).contains(&node.y)),
                "preset {} nodes should stay within the normalized gain range",
                preset.label
            );
            assert!(
                preset.curve.segments.iter().all(|segment| {
                    (MIN_SEGMENT_TENSION..=MAX_SEGMENT_TENSION).contains(&segment.tension)
                }),
                "preset {} segment tensions should stay within the supported range",
                preset.label
            );
            let last_index = preset.curve.nodes.len() - 1;
            assert!(
                (preset.curve.nodes[0].y - preset.curve.nodes[last_index].y).abs() <= f32::EPSILON,
                "preset {} endpoints should remain coupled",
                preset.label
            );

            let table = editable_curve_to_table(&preset.curve);
            assert!(
                table.iter().all(|value| (0.0..=1.0).contains(value)),
                "preset {} table should remain bounded",
                preset.label
            );
        }
    }

    #[test]
    fn quick_shape_presets_apply_expected_sync_divisions() {
        let divisions: Vec<_> = quick_shape_presets()
            .iter()
            .map(|preset| preset.sync_division)
            .collect();
        assert_eq!(
            divisions,
            vec![
                DEFAULT_SYNC_DIVISION_INDEX,
                DEFAULT_SYNC_DIVISION_INDEX,
                2,
                5,
                0,
                2,
                3,
                1
            ]
        );
    }

    #[test]
    fn quick_shape_preset_lookup_matches_action_key() {
        let preset = quick_shape_preset_by_key("shape-cut").expect("shape should exist");
        assert_eq!(preset.label, "Cut");
        assert_eq!(preset.sync_division, 0);
    }
}
