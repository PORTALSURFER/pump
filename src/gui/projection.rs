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
    use crate::curve::{editable_curve_to_table, CurveNode, CurveSegment};
    use crate::dsp::{DspSettings, PumpEngine};
    use crate::params::{DEFAULT_FREE_RATE_HZ, TIMING_MODE_SYNC};
    use toybox::dsp::TransportState;

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

    #[test]
    fn runtime_gain_matches_processed_waveform_at_effective_phase_bin() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.1 },
                CurveNode { x: 0.2, y: 0.9 },
                CurveNode { x: 0.63, y: 0.2 },
                CurveNode { x: 1.0, y: 0.7 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 3],
            ..EditableCurve::default()
        }
        .normalized();
        let phase_offset = 0.25;
        let bin = 48;
        let displayed_phase = bin as f32 / 95.0;
        let mut waveform = [0.0; 96];
        waveform[bin] = 1.0;
        let processed = processed_waveform(&curve, &waveform, phase_offset, 120.0, -60.0);

        let settings = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            delay_beats: 0,
            smooth: 0.0,
            swing: 0.0,
            timing_mode: TIMING_MODE_SYNC,
            free_rate_hz: DEFAULT_FREE_RATE_HZ,
            bypassed: false,
            filter_enabled: false,
            filter_hp_freq_hz: crate::params::DEFAULT_FILTER_HP_FREQ_HZ,
            filter_hp_q: crate::params::DEFAULT_FILTER_HP_Q,
            filter_lp_freq_hz: crate::params::DEFAULT_FILTER_LP_FREQ_HZ,
            filter_lp_q: crate::params::DEFAULT_FILTER_LP_Q,
            filter_hp_slope: 0,
            filter_lp_slope: 0,
        };
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(displayed_phase as f64),
        };
        let mut engine = PumpEngine::new(1_000.0, editable_curve_to_table(&curve));
        let mut left = 1.0;
        let mut right = 1.0;
        let mut telemetry = engine.process_sample(&mut left, &mut right, settings, transport);
        for _ in 0..512 {
            left = 1.0;
            right = 1.0;
            telemetry = engine.process_sample(&mut left, &mut right, settings, transport);
        }

        assert!((telemetry.phase - displayed_phase).abs() < 1.0e-5);
        assert!((telemetry.gain - processed[bin]).abs() < 1.0e-3);
    }

    #[test]
    fn processed_waveform_tracks_unsettled_applied_offset_from_dsp_snapshot() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.05 },
                CurveNode { x: 0.23, y: 0.95 },
                CurveNode { x: 0.61, y: 0.12 },
                CurveNode { x: 1.0, y: 0.82 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 3],
            ..EditableCurve::default()
        }
        .normalized();
        let target_phase_offset = 0.7;
        let bin = 24;
        let effective_phase = bin as f32 / 95.0;
        let mut waveform = [0.0; 96];
        waveform[bin] = 1.0;
        let settings = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            delay_beats: 0,
            smooth: 0.0,
            swing: 0.0,
            timing_mode: TIMING_MODE_SYNC,
            free_rate_hz: DEFAULT_FREE_RATE_HZ,
            bypassed: false,
            filter_enabled: false,
            filter_hp_freq_hz: crate::params::DEFAULT_FILTER_HP_FREQ_HZ,
            filter_hp_q: crate::params::DEFAULT_FILTER_HP_Q,
            filter_lp_freq_hz: crate::params::DEFAULT_FILTER_LP_FREQ_HZ,
            filter_lp_q: crate::params::DEFAULT_FILTER_LP_Q,
            filter_hp_slope: 0,
            filter_lp_slope: 0,
        };
        let transition_settings = DspSettings {
            phase_offset: target_phase_offset,
            ..settings
        };
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(effective_phase as f64),
        };
        let mut engine = PumpEngine::new(1_000.0, editable_curve_to_table(&curve));
        let mut left = 1.0;
        let mut right = 1.0;
        engine.process_sample(&mut left, &mut right, settings, transport);
        let telemetry =
            engine.process_sample(&mut left, &mut right, transition_settings, transport);
        assert!(telemetry.applied_phase_offset > 0.0);
        assert!(telemetry.applied_phase_offset < target_phase_offset);

        let status = crate::GuiStatus::default();
        status.publish_dsp_telemetry(telemetry);
        let applied = status
            .dsp_snapshot()
            .expect("DSP telemetry should publish a phase pair")
            .applied_phase_offset;
        let processed = processed_waveform(&curve, &waveform, applied, 120.0, -60.0);
        let target_processed =
            processed_waveform(&curve, &waveform, target_phase_offset, 120.0, -60.0);

        assert!((telemetry.gain - processed[bin]).abs() < 1.0e-3);
        assert!((processed[bin] - target_processed[bin]).abs() > 0.05);
    }
}
