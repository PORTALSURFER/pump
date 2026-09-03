//! Shared transport-phase helpers used by both CLAP and VST3 paths.

use crate::dsp::{swing_warp_phase, DspSettings};
use crate::GuiTransportTelemetry;
use toybox::dsp::{phase_from_beats, TransportState};

/// Compensate a host beat timeline for input presentation latency.
///
/// The host timeline is expressed at the block's presentation point, while
/// audio entering the plugin may represent an earlier point by the input
/// latency. Keep invalid or unavailable timing inputs unchanged so callers
/// retain the existing fallback behavior.
pub(crate) fn compensate_input_presentation_latency(
    transport: TransportState,
    latency_samples: Option<u32>,
    sample_rate: f64,
) -> TransportState {
    let Some(latency_samples) = latency_samples.filter(|&samples| samples != 0) else {
        return transport;
    };
    let Some(song_pos_beats) = transport.song_pos_beats else {
        return transport;
    };
    let tempo_bpm = f64::from(transport.tempo_bpm);
    if !song_pos_beats.is_finite()
        || !tempo_bpm.is_finite()
        || tempo_bpm <= 0.0
        || !sample_rate.is_finite()
        || sample_rate <= 0.0
    {
        return transport;
    }

    let latency_beats = f64::from(latency_samples) * tempo_bpm / (60.0 * sample_rate);
    let compensated_song_pos_beats = song_pos_beats - latency_beats;
    if !latency_beats.is_finite() || !compensated_song_pos_beats.is_finite() {
        return transport;
    }

    TransportState {
        song_pos_beats: Some(compensated_song_pos_beats),
        ..transport
    }
}

/// Resolve GUI phase from the latest host transport snapshot.
///
/// Uses host beat timeline when available so the curve marker remains aligned to
/// arranger position even when no audio frames are processed for this callback.
pub(crate) fn gui_phase_from_transport(
    transport: TransportState,
    settings: DspSettings,
    fallback_phase: f32,
) -> f32 {
    if settings.timing_mode == crate::params::TIMING_MODE_FREE {
        return fallback_phase.rem_euclid(1.0);
    }
    transport
        .song_pos_beats
        .map(|beats| {
            if settings.swing <= 0.0 {
                phase_from_beats(beats, settings.beats_per_cycle, 0.0)
            } else {
                let raw_phase = phase_from_beats(beats, settings.beats_per_cycle, 0.0);
                swing_warp_phase(raw_phase, settings.swing)
            }
        })
        .unwrap_or_else(|| fallback_phase.rem_euclid(1.0))
}

/// Resolve normalized host beat phase from transport snapshot.
pub(crate) fn host_beat_phase(transport: TransportState) -> Option<f32> {
    transport
        .song_pos_beats
        .map(|beats| beats.rem_euclid(1.0) as f32)
}

/// Resolve whether GUI phase extrapolation should run this block.
///
/// When host beat timeline is unavailable, phase still advances while audio is
/// flowing so visual feedback remains responsive.
pub(crate) fn phase_running_from_transport(transport: TransportState) -> bool {
    transport.is_playing || transport.song_pos_beats.is_none()
}

/// Build GUI transport telemetry from a host transport snapshot.
pub(crate) fn gui_transport_telemetry(
    transport: TransportState,
    settings: DspSettings,
    fallback_beat_phase: f32,
) -> GuiTransportTelemetry {
    GuiTransportTelemetry {
        is_playing: if settings.timing_mode == crate::params::TIMING_MODE_FREE {
            true
        } else {
            phase_running_from_transport(transport)
        },
        transport_is_playing: transport.is_playing,
        has_host_beats_timeline: transport.song_pos_beats.is_some(),
        beat_phase: host_beat_phase(transport).unwrap_or(fallback_beat_phase),
        tempo_bpm: transport.tempo_bpm,
        beats_per_cycle: settings.beats_per_cycle,
        timing_mode: settings.timing_mode,
        effective_cycle_rate_hz: if settings.timing_mode == crate::params::TIMING_MODE_FREE {
            crate::params::clamp_free_rate_hz(settings.free_rate_hz)
        } else {
            0.0
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::dsp::{swing_warp_phase, DspSettings};
    use toybox::dsp::{phase_from_beats, TransportState};

    use super::{
        compensate_input_presentation_latency, gui_phase_from_transport, gui_transport_telemetry,
        host_beat_phase, phase_running_from_transport,
    };

    fn test_settings(beats_per_cycle: f32) -> DspSettings {
        DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle,
            smooth: 0.0,
            swing: 0.0,
            timing_mode: crate::params::TIMING_MODE_SYNC,
            free_rate_hz: crate::params::DEFAULT_FREE_RATE_HZ,
            bypassed: false,
        }
    }

    #[test]
    fn input_presentation_latency_compensation_uses_samples_and_tempo_units() {
        let transport = TransportState {
            tempo_bpm: 120.0,
            song_pos_beats: Some(4.0),
            ..TransportState::default()
        };

        let compensated = compensate_input_presentation_latency(transport, Some(24_000), 48_000.0);

        assert!((compensated.song_pos_beats.unwrap_or_default() - 3.0).abs() < 1.0e-6);
    }

    #[test]
    fn input_presentation_latency_compensation_preserves_zero_missing_and_invalid_inputs() {
        let transport = TransportState {
            tempo_bpm: 120.0,
            song_pos_beats: Some(4.0),
            ..TransportState::default()
        };

        for (latency_samples, sample_rate) in [
            (None, 48_000.0),
            (Some(0), 48_000.0),
            (Some(24), 0.0),
            (Some(24), f64::NAN),
        ] {
            assert_eq!(
                compensate_input_presentation_latency(transport, latency_samples, sample_rate),
                transport
            );
        }

        assert_eq!(
            compensate_input_presentation_latency(
                TransportState {
                    tempo_bpm: 0.0,
                    ..transport
                },
                Some(24),
                48_000.0,
            ),
            TransportState {
                tempo_bpm: 0.0,
                ..transport
            }
        );

        assert_eq!(
            compensate_input_presentation_latency(
                TransportState {
                    song_pos_beats: None,
                    ..transport
                },
                Some(24),
                48_000.0,
            ),
            TransportState {
                song_pos_beats: None,
                ..transport
            }
        );
    }

    #[test]
    fn input_presentation_latency_compensation_allows_negative_beats() {
        let transport = TransportState {
            tempo_bpm: 60.0,
            song_pos_beats: Some(-0.25),
            ..TransportState::default()
        };

        let compensated = compensate_input_presentation_latency(transport, Some(24_000), 48_000.0);

        assert!((compensated.song_pos_beats.unwrap_or_default() + 0.75).abs() < 1.0e-6);
    }

    #[test]
    fn gui_phase_from_transport_prefers_host_song_position() {
        let settings = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.2,
            output_gain_db: 0.0,
            beats_per_cycle: 4.0,
            smooth: 0.0,
            swing: 0.0,
            timing_mode: crate::params::DEFAULT_TIMING_MODE,
            free_rate_hz: crate::params::DEFAULT_FREE_RATE_HZ,
            bypassed: false,
        };
        let transport = TransportState {
            tempo_bpm: 128.0,
            is_playing: true,
            song_pos_beats: Some(9.5),
        };
        let expected = phase_from_beats(9.5, settings.beats_per_cycle, 0.0);
        let resolved = gui_phase_from_transport(transport, settings, 0.75);
        assert!((resolved - expected).abs() < 1.0e-6);
    }

    #[test]
    fn gui_phase_from_transport_warps_raw_host_phase_before_offset() {
        let settings = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.2,
            output_gain_db: 0.0,
            beats_per_cycle: 4.0,
            smooth: 0.0,
            swing: 1.0,
            timing_mode: crate::params::DEFAULT_TIMING_MODE,
            free_rate_hz: crate::params::DEFAULT_FREE_RATE_HZ,
            bypassed: false,
        };
        let transport = TransportState {
            tempo_bpm: 128.0,
            is_playing: true,
            song_pos_beats: Some(2.0),
        };
        let raw_phase = phase_from_beats(2.0, settings.beats_per_cycle, 0.0);
        let expected = swing_warp_phase(raw_phase, settings.swing);
        let resolved = gui_phase_from_transport(transport, settings, 0.75);
        assert!((resolved - expected).abs() < 1.0e-6);
        assert!((resolved - 0.375).abs() < 1.0e-6);
    }

    #[test]
    fn gui_sync_phase_matches_raw_transport_boundaries_and_offset_wrap() {
        let settings = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            smooth: 0.0,
            swing: 0.0,
            timing_mode: crate::params::TIMING_MODE_SYNC,
            free_rate_hz: crate::params::DEFAULT_FREE_RATE_HZ,
            bypassed: false,
        };
        for (host_beats, expected) in [(0.0, 0.0), (0.25, 0.25), (0.5, 0.5), (0.75, 0.75)] {
            let phase = gui_phase_from_transport(
                TransportState {
                    song_pos_beats: Some(host_beats),
                    ..TransportState::default()
                },
                settings,
                0.0,
            );
            assert!((phase - expected).abs() < 1.0e-6);
        }

        let wrapped = gui_phase_from_transport(
            TransportState {
                song_pos_beats: Some(0.875),
                ..TransportState::default()
            },
            DspSettings {
                phase_offset: 0.2,
                ..settings
            },
            0.0,
        );
        assert!((wrapped - 0.875).abs() < 1.0e-6);
    }

    #[test]
    fn gui_free_phase_keeps_raw_offset_origin() {
        let settings = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.2,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            smooth: 0.0,
            swing: 0.0,
            timing_mode: crate::params::TIMING_MODE_FREE,
            free_rate_hz: crate::params::DEFAULT_FREE_RATE_HZ,
            bypassed: false,
        };
        let resolved = gui_phase_from_transport(
            TransportState {
                song_pos_beats: Some(0.0),
                ..TransportState::default()
            },
            settings,
            0.0,
        );
        assert!(resolved.abs() < 1.0e-6);

        let telemetry = gui_transport_telemetry(
            TransportState {
                tempo_bpm: 240.0,
                is_playing: false,
                song_pos_beats: Some(17.0),
            },
            DspSettings {
                timing_mode: crate::params::TIMING_MODE_FREE,
                free_rate_hz: 3.5,
                ..settings
            },
            0.2,
        );
        assert!(telemetry.is_playing);
        assert_eq!(telemetry.effective_cycle_rate_hz, 3.5);
        assert!(!telemetry.transport_is_playing);
    }

    #[test]
    fn gui_phase_from_transport_uses_fallback_without_song_position() {
        let settings = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            smooth: 0.0,
            swing: 0.0,
            timing_mode: crate::params::DEFAULT_TIMING_MODE,
            free_rate_hz: crate::params::DEFAULT_FREE_RATE_HZ,
            bypassed: false,
        };
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: false,
            song_pos_beats: None,
        };
        let resolved = gui_phase_from_transport(transport, settings, 1.25);
        assert!((resolved - 0.25).abs() < 1.0e-6);
    }

    #[test]
    fn host_beat_phase_wraps_negative_and_positive_positions() {
        let positive = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(4.75),
        };
        let negative = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(-0.2),
        };
        assert!((host_beat_phase(positive).unwrap_or_default() - 0.75).abs() < 1.0e-6);
        assert!((host_beat_phase(negative).unwrap_or_default() - 0.8).abs() < 1.0e-6);
    }

    #[test]
    fn phase_running_falls_back_when_song_position_missing() {
        let stopped_without_timeline = TransportState {
            tempo_bpm: 120.0,
            is_playing: false,
            song_pos_beats: None,
        };
        let stopped_with_timeline = TransportState {
            tempo_bpm: 120.0,
            is_playing: false,
            song_pos_beats: Some(2.0),
        };

        assert!(phase_running_from_transport(stopped_without_timeline));
        assert!(!phase_running_from_transport(stopped_with_timeline));
    }

    #[test]
    fn gui_transport_telemetry_uses_fallback_beat_phase_without_timeline() {
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: None,
        };
        let telemetry = gui_transport_telemetry(transport, test_settings(4.0), 0.37);
        assert!(telemetry.is_playing);
        assert!(telemetry.transport_is_playing);
        assert!(!telemetry.has_host_beats_timeline);
        assert!((telemetry.beat_phase - 0.37).abs() < 1.0e-6);
        assert!((telemetry.beats_per_cycle - 4.0).abs() < 1.0e-6);
    }

    #[test]
    fn gui_transport_telemetry_keeps_raw_stopped_state_during_phase_fallback() {
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: false,
            song_pos_beats: None,
        };
        let telemetry = gui_transport_telemetry(transport, test_settings(4.0), 0.37);

        assert!(telemetry.is_playing, "phase fallback should keep advancing");
        assert!(
            !telemetry.transport_is_playing,
            "meter activity must retain the raw stopped state"
        );
    }
}
