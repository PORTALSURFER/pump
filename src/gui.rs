//! Shared Radiant editor contract and layout helpers for Pump.

use toybox::clack_extensions::gui::GuiSize;

use crate::curve::{CurveNode, EditableCurve};

#[cfg(all(
    any(target_os = "macos", target_os = "windows"),
    any(feature = "radiant-gui", feature = "vst3", test)
))]
mod radiant_editor;

mod curve_paint;

pub(crate) mod visual_system;

#[cfg(all(target_os = "macos", feature = "screenshot-test", test))]
mod screenshot_tests;

#[cfg(all(
    any(target_os = "macos", target_os = "windows"),
    feature = "vst3",
    test
))]
pub(crate) use radiant_editor::try_toggle_bypass;
#[cfg(all(
    any(target_os = "macos", target_os = "windows"),
    any(feature = "radiant-gui", feature = "vst3")
))]
#[cfg(feature = "vst3")]
pub(crate) use radiant_editor::HostParamEditSink;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) use radiant_editor::{HostParamFlushRequester, RadiantPumpEditor};

/// Minimum supported logical editor size.
pub const MIN_WINDOW_WIDTH: u32 = 640;
pub const MIN_WINDOW_HEIGHT: u32 = 400;
/// Default logical editor size.
pub const WINDOW_WIDTH: u32 = 640;
pub const WINDOW_HEIGHT: u32 = 400;
/// Maximum supported logical editor size.
pub const MAX_WINDOW_WIDTH: u32 = 1280;
pub const MAX_WINDOW_HEIGHT: u32 = 800;

const PRESET_WARNING_STORAGE: &str = "NOT SAVED - CHECK PRESET FOLDER";

/// Return a stable preferred size before a host has opened the child view.
pub(crate) fn preferred_window_size() -> (u32, u32) {
    (WINDOW_WIDTH, WINDOW_HEIGHT)
}

/// Normalize a host request to the supported 8:5 logical contract.
#[allow(dead_code)]
pub(crate) fn normalize_host_size(size: GuiSize) -> GuiSize {
    let width = size.width.clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_WIDTH);
    let height = size.height.clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_HEIGHT);
    let scale = (width as f64 / WINDOW_WIDTH as f64)
        .min(height as f64 / WINDOW_HEIGHT as f64)
        .clamp(
            MIN_WINDOW_WIDTH as f64 / WINDOW_WIDTH as f64,
            MAX_WINDOW_WIDTH as f64 / WINDOW_WIDTH as f64,
        );
    GuiSize {
        width: (WINDOW_WIDTH as f64 * scale).round() as u32,
        height: (WINDOW_HEIGHT as f64 * scale).round() as u32,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CurveBeatGrid {
    pub(crate) minor: Vec<f32>,
    pub(crate) major: Vec<f32>,
}

pub(crate) fn curve_beat_grid(sync_division: usize, width: f32) -> CurveBeatGrid {
    const SIXTEENTH: f32 = 0.25;
    let Some(division) = crate::params::SYNC_DIVISIONS.get(sync_division) else {
        return CurveBeatGrid::default();
    };
    let cycle = division.beats;
    if !cycle.is_finite() || cycle <= SIXTEENTH || !width.is_finite() || width <= 0.0 {
        return CurveBeatGrid::default();
    }
    let mut major = Vec::new();
    let mut beat = 1.0;
    while beat < cycle - 1.0e-5 {
        major.push(beat / cycle);
        beat += 1.0;
    }
    if major.is_empty() {
        major.push(0.5);
    }
    let mut interval = SIXTEENTH;
    while interval < cycle && width * interval / cycle < 8.0 {
        interval *= 2.0;
    }
    let mut minor = Vec::new();
    let mut position = interval;
    while position < cycle - 1.0e-5 {
        let normalized = position / cycle;
        if !major
            .iter()
            .any(|major| (major - normalized).abs() < 1.0e-5)
        {
            minor.push(normalized);
        }
        position += interval;
    }
    CurveBeatGrid { minor, major }
}

pub(crate) fn snap_curve_time_to_beat_grid(sync_division: usize, width: f32, time: f32) -> f32 {
    let time = time.clamp(0.0, 1.0);
    let grid = curve_beat_grid(sync_division, width);
    grid.minor
        .into_iter()
        .chain(grid.major)
        .chain([0.0, 1.0])
        .min_by(|left, right| {
            (time - *left)
                .abs()
                .total_cmp(&(time - *right).abs())
                .then_with(|| left.total_cmp(right))
        })
        .unwrap_or(time)
}

/// Snap a curve time to the displayed beat grid, applying the same phase warp
/// as the swung transport and grid markers.
///
/// Keep the unswung path delegated to the legacy helper so Swing = 0 remains
/// bit-for-bit identical. Endpoints stay fixed while interior grid candidates
/// are warped into their displayed positions before choosing the nearest one.
pub(crate) fn snap_curve_time_to_beat_grid_with_swing(
    sync_division: usize,
    width: f32,
    time: f32,
    swing: f32,
) -> f32 {
    if swing <= 0.0 {
        return snap_curve_time_to_beat_grid(sync_division, width, time);
    }

    let time = time.clamp(0.0, 1.0);
    let grid = curve_beat_grid(sync_division, width);
    grid.minor
        .into_iter()
        .chain(grid.major)
        .chain([0.0, 1.0])
        .map(|candidate| {
            if candidate <= 0.0 || candidate >= 1.0 {
                candidate
            } else {
                crate::dsp::swing_warp_phase(candidate, swing)
            }
        })
        .min_by(|left, right| {
            (time - *left)
                .abs()
                .total_cmp(&(time - *right).abs())
                .then_with(|| left.total_cmp(right))
        })
        .unwrap_or(time)
}

fn enforce_curve_endpoints(curve: &mut EditableCurve) {
    if curve.nodes.len() < 2 {
        return;
    }
    let y = curve.nodes[0].y.clamp(0.0, 1.0);
    let last = curve.nodes.len() - 1;
    curve.nodes[0] = CurveNode { x: 0.0, y };
    curve.nodes[last] = CurveNode { x: 1.0, y };
}

pub(crate) fn move_segment_translated(
    curve: &mut EditableCurve,
    segment_index: usize,
    start_left: (f32, f32),
    start_right: (f32, f32),
    delta: (f32, f32),
) {
    if curve.nodes.len() < 2 || segment_index + 1 >= curve.nodes.len() {
        return;
    }
    let right = segment_index + 1;
    let (left_x, left_y) = start_left;
    let (right_x, right_y) = start_right;
    let mut dx = delta.0;
    if segment_index == 0 || right == curve.nodes.len() - 1 {
        dx = 0.0;
    } else {
        dx = dx.clamp(
            curve.nodes[segment_index - 1].x + 1.0e-3 - left_x,
            curve.nodes[right + 1].x - 1.0e-3 - right_x,
        );
    }
    let dy = delta
        .1
        .clamp(-left_y.min(right_y), 1.0 - left_y.max(right_y));
    curve.nodes[segment_index].x = left_x + dx;
    curve.nodes[right].x = right_x + dx;
    curve.nodes[segment_index].y = left_y + dy;
    curve.nodes[right].y = right_y + dy;
    enforce_curve_endpoints(curve);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CurveGainReference {
    pub(crate) gain: f32,
    pub(crate) gain_db: Option<f32>,
}

#[allow(dead_code)]
pub(crate) fn curve_gain_references() -> [CurveGainReference; 4] {
    [
        CurveGainReference {
            gain: 1.0,
            gain_db: Some(0.0),
        },
        CurveGainReference {
            gain: crate::dsp::db_to_linear(-6.0),
            gain_db: Some(-6.0),
        },
        CurveGainReference {
            gain: crate::dsp::db_to_linear(-12.0),
            gain_db: Some(-12.0),
        },
        CurveGainReference {
            gain: 0.0,
            gain_db: None,
        },
    ]
}

pub(crate) fn curve_gain_references_for_mapping(
    depth_db: f32,
    floor_db: f32,
) -> [CurveGainReference; 4] {
    [
        1.0,
        crate::dsp::db_to_linear(-6.0),
        crate::dsp::db_to_linear(-12.0),
        0.0,
    ]
    .map(|value| {
        let gain = crate::dsp::curve_value_to_gain(value, depth_db, floor_db);
        CurveGainReference {
            gain,
            gain_db: crate::dsp::gain_to_db(gain),
        }
    })
}

pub(crate) fn curve_gain_reference_text(reference: CurveGainReference, bitmap: bool) -> String {
    match reference.gain_db {
        Some(db) if bitmap => format!("{db:.0} dB"),
        Some(db) => format!("{db:.0} dB").replace('-', "−"),
        None if bitmap => "-INF".to_string(),
        None => "−∞".to_string(),
    }
}

pub(crate) fn build_version_label() -> String {
    format!(
        "{}+{}",
        env!("CARGO_PKG_VERSION"),
        option_env!("PUMP_BUILD_GIT_SHA_SHORT").unwrap_or("unknown")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_version_label_includes_current_git_sha() {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--short=7", "HEAD"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("git should be available in repository tests");
        if !output.status.success() {
            return;
        }

        let expected_sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert!(!expected_sha.is_empty());
        assert_eq!(
            option_env!("PUMP_BUILD_GIT_SHA_SHORT"),
            Some(expected_sha.as_str()),
            "build.rs must export the current short Git SHA"
        );
        assert_eq!(
            build_version_label(),
            format!("{}+{expected_sha}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn normalized_host_size_preserves_contract() {
        assert_eq!(
            normalize_host_size(GuiSize {
                width: 1,
                height: 1
            }),
            GuiSize {
                width: 640,
                height: 400
            }
        );
        assert_eq!(
            normalize_host_size(GuiSize {
                width: 5000,
                height: 5000
            }),
            GuiSize {
                width: 1280,
                height: 800
            }
        );
    }

    #[test]
    fn swung_snap_preserves_zero_identity_and_warps_midpoint() {
        let width = 396.0;
        for time in [0.0, 0.34, 0.5, 0.9, 1.0] {
            assert_eq!(
                snap_curve_time_to_beat_grid_with_swing(6, width, time, 0.0),
                snap_curve_time_to_beat_grid(6, width, time)
            );
        }
        assert_eq!(
            snap_curve_time_to_beat_grid_with_swing(6, width, 0.375, 1.0),
            0.375
        );
        assert_eq!(
            snap_curve_time_to_beat_grid_with_swing(6, width, 0.0, 1.0),
            0.0
        );
        assert_eq!(
            snap_curve_time_to_beat_grid_with_swing(6, width, 1.0, 1.0),
            1.0
        );
    }
}
