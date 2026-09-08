//! Pure curve and waveform projection used by the native GPUI renderer.
//!
//! These helpers deliberately contain no GPUI types.  Keeping the DSP preview
//! mapping here makes the native paint code a projection of the same phase,
//! smoothing, and gain rules that the legacy editor exposed to hosts.

use crate::curve::{sample_editable_curve, EditableCurve};

/// Sample the authored curve at a viewport phase after applying the displayed
/// phase offset.
pub(crate) fn sample_display_curve(
    curve: &EditableCurve,
    viewport_phase: f32,
    phase_offset: f32,
) -> f32 {
    let authored = crate::dsp::authored_curve_phase(viewport_phase, phase_offset);
    sample_editable_curve(curve, authored)
}

/// Return the legacy-compatible preview radius for a Smooth amount.
pub(crate) fn smooth_preview_radius(amount: f32) -> i32 {
    let amount = if amount.is_finite() {
        amount.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if amount <= crate::dsp::SMOOTH_COMPATIBILITY_KNEE {
        return (amount * 8.0).round() as i32;
    }

    let t = (amount - crate::dsp::SMOOTH_COMPATIBILITY_KNEE)
        / (1.0 - crate::dsp::SMOOTH_COMPATIBILITY_KNEE);
    let smoothstep = t * t * (3.0 - 2.0 * t);
    (amount * 8.0 + (20.0 - 8.0) * smoothstep).round() as i32
}

/// Sample the moving average used for the secondary smoothed curve preview.
pub(crate) fn sample_smoothed_curve(
    curve: &EditableCurve,
    authored_phase: f32,
    smooth: f32,
) -> f32 {
    if smooth <= f32::EPSILON {
        return sample_editable_curve(curve, authored_phase);
    }
    let radius = smooth_preview_radius(smooth);
    let sample_step = 1.0 / 96.0;
    let mut total = 0.0;
    let mut count = 0.0;
    for offset in -radius..=radius {
        total += sample_editable_curve(curve, authored_phase + offset as f32 * sample_step);
        count += 1.0;
    }
    (total / count).clamp(0.0, 1.0)
}

/// Project the incoming waveform through the curve and current gain mapping.
///
/// The returned values are finite, clamped amplitudes.  In particular, an
/// unsettled host phase offset comes from the DSP snapshot and remains visible
/// in the preview until the host has applied it.
pub(crate) fn processed_waveform(
    curve: &EditableCurve,
    incoming: &[f32],
    applied_phase_offset: f32,
    depth_db: f32,
    floor_db: f32,
) -> Vec<f32> {
    if incoming.is_empty() {
        return Vec::new();
    }
    let denominator = incoming.len().saturating_sub(1).max(1) as f32;
    incoming
        .iter()
        .copied()
        .enumerate()
        .map(|(index, input)| {
            let viewport_phase = index as f32 / denominator;
            let curve_value = sample_display_curve(curve, viewport_phase, applied_phase_offset);
            let gain = crate::dsp::curve_value_to_gain(curve_value, depth_db, floor_db);
            let input = if input.is_finite() { input } else { 0.0 };
            (input * gain).clamp(0.0, 1.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{CurveNode, CurveSegment};

    fn curve() -> EditableCurve {
        EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.0 }, CurveNode { x: 1.0, y: 1.0 }],
            segments: vec![CurveSegment { tension: 0.0 }],
            ..EditableCurve::default()
        }
    }

    #[test]
    fn smooth_radius_keeps_legacy_knee_and_extended_tail() {
        assert_eq!(smooth_preview_radius(0.0), 0);
        assert_eq!(smooth_preview_radius(0.5), 4);
        assert_eq!(smooth_preview_radius(0.75), 6);
        assert_eq!(smooth_preview_radius(1.0), 20);
    }

    #[test]
    fn display_sampling_applies_authored_phase_offset() {
        let curve = curve();
        let value = sample_display_curve(&curve, 0.25, 0.25);
        assert!((value - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn processed_waveform_is_finite_clamped_and_phase_mapped() {
        let curve = curve();
        let waveform = processed_waveform(&curve, &[f32::NAN, 0.8, 2.0, -1.0], 0.25, 120.0, -60.0);
        assert_eq!(waveform.len(), 4);
        assert!(waveform
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value)));
        assert_eq!(waveform[0], 0.0);
        assert_eq!(waveform[2], 1.0);
        assert_eq!(waveform[3], 0.0);
    }

    #[test]
    fn finite_floor_preserves_floor_gain_in_preview() {
        let curve = curve();
        let waveform = processed_waveform(&curve, &[0.0, 1.0], 0.0, 120.0, -12.0);
        assert!(waveform[1] > 0.2);
        assert!(waveform[1] < 0.3);
    }
}
