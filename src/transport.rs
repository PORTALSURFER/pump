//! Shared transport-phase helpers used by both CLAP and VST3 paths.

use crate::dsp::DspSettings;
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
        .map(|beats| phase_from_beats(beats, settings.beats_per_cycle, settings.phase_offset))
        .unwrap_or_else(|| fallback_phase.rem_euclid(1.0))
}

/// Resolve normalized host beat phase from transport snapshot.
pub(crate) fn host_beat_phase(transport: TransportState) -> Option<f32> {
    transport
        .song_pos_beats
        .map(|beats| beats.rem_euclid(1.0) as f32)
}

#[cfg(test)]
mod tests {
    use crate::dsp::DspSettings;
    use toybox::dsp::{phase_from_beats, TransportState};

    use super::{gui_phase_from_transport, host_beat_phase};

    #[test]
    fn gui_phase_from_transport_prefers_host_song_position() {
        let settings = DspSettings {
            mix: 1.0,
            depth: 1.0,
            phase_offset: 0.2,
            output_gain_db: 0.0,
            beats_per_cycle: 4.0,
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
    fn gui_phase_from_transport_uses_fallback_without_song_position() {
        let settings = DspSettings {
            mix: 1.0,
            depth: 1.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
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
}
