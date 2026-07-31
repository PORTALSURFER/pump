//! Shared transport-phase helpers used by both CLAP and VST3 paths.

use crate::dsp::{swing_warp_phase, DspSettings};
use crate::GuiTransportTelemetry;
use toybox::dsp::{phase_from_beats, TransportState};

/// Resolve GUI phase from the latest host transport snapshot.
///
/// Uses host beat timeline when available so the curve marker remains aligned to
/// arranger position even when no audio frames are processed for this callback.
pub(crate) fn gui_phase_from_transport(
    transport: TransportState,
    settings: DspSettings,
    fallback_phase: f32,
) -> f32 {
    transport
        .song_pos_beats
        .map(|beats| {
            if settings.swing <= 0.0 {
                // Preserve the legacy host-timeline phase calculation exactly.
                phase_from_beats(beats, settings.beats_per_cycle, settings.phase_offset)
            } else {
                let raw_phase = phase_from_beats(beats, settings.beats_per_cycle, 0.0);
                (swing_warp_phase(raw_phase, settings.swing) + settings.phase_offset)
                    .rem_euclid(1.0)
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
    beats_per_cycle: f32,
    fallback_beat_phase: f32,
) -> GuiTransportTelemetry {
    GuiTransportTelemetry {
        is_playing: phase_running_from_transport(transport),
        transport_is_playing: transport.is_playing,
        has_host_beats_timeline: transport.song_pos_beats.is_some(),
        beat_phase: host_beat_phase(transport).unwrap_or(fallback_beat_phase),
        tempo_bpm: transport.tempo_bpm,
        beats_per_cycle,
    }
}

#[cfg(test)]
mod tests {
    use crate::dsp::{swing_warp_phase, DspSettings};
    use toybox::dsp::{phase_from_beats, TransportState};

    use super::{
        gui_phase_from_transport, gui_transport_telemetry, host_beat_phase,
        phase_running_from_transport,
    };

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
        let expected = phase_from_beats(9.5, settings.beats_per_cycle, settings.phase_offset);
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
        let expected =
            (swing_warp_phase(raw_phase, settings.swing) + settings.phase_offset).rem_euclid(1.0);
        let resolved = gui_phase_from_transport(transport, settings, 0.75);
        assert!((resolved - expected).abs() < 1.0e-6);
        assert!((resolved - 0.575).abs() < 1.0e-6);
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
        assert!((wrapped - 0.075).abs() < 1.0e-6);
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
        assert!((resolved - settings.phase_offset).abs() < 1.0e-6);
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
        let telemetry = gui_transport_telemetry(transport, 4.0, 0.37);
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
        let telemetry = gui_transport_telemetry(transport, 4.0, 0.37);

        assert!(telemetry.is_playing, "phase fallback should keep advancing");
        assert!(
            !telemetry.transport_is_playing,
            "meter activity must retain the raw stopped state"
        );
    }
}
