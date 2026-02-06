//! Transport and beat-phase helpers for Pump timing.

/// Transport snapshot used by Pump's gain envelope engine.
#[derive(Debug, Copy, Clone)]
pub struct TransportState {
    /// Host tempo in beats per minute.
    pub tempo_bpm: f32,
    /// Whether host transport is currently playing.
    pub is_playing: bool,
    /// Host song position in quarter-note beats, when available.
    pub song_pos_beats: Option<f64>,
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            tempo_bpm: 120.0,
            is_playing: false,
            song_pos_beats: None,
        }
    }
}

/// Per-sample clock frame from the running transport clock.
#[derive(Debug, Copy, Clone)]
pub struct ClockFrame {
    /// Beat position in quarter-note units.
    pub beat_position: f64,
}

impl ClockFrame {
    /// Compute normalized cycle phase for a beat division and offset.
    pub fn phase_for_cycle(self, beats_per_cycle: f32, phase_offset: f32) -> f32 {
        phase_from_beats(self.beat_position, beats_per_cycle, phase_offset)
    }
}

/// Running transport clock with fallback when hosts omit beat timeline.
pub struct TransportClock {
    sample_rate: f32,
    fallback_beat_position: f64,
}

impl TransportClock {
    /// Create a new transport clock for the current sample rate.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: sample_rate.max(1.0),
            fallback_beat_position: 0.0,
        }
    }

    /// Advance one sample and return the current clock frame.
    pub fn tick(&mut self, transport: TransportState) -> ClockFrame {
        let tempo_bpm = transport.tempo_bpm.clamp(20.0, 320.0);
        let beat_increment = tempo_bpm as f64 / (self.sample_rate as f64 * 60.0);

        let beat_position = transport
            .song_pos_beats
            .unwrap_or(self.fallback_beat_position);

        if transport.is_playing {
            self.fallback_beat_position = beat_position + beat_increment;
        } else {
            self.fallback_beat_position = beat_position;
        }

        ClockFrame { beat_position }
    }
}

/// Convert beat position into `[0, 1)` cycle phase.
pub fn phase_from_beats(beat_position: f64, beats_per_cycle: f32, phase_offset: f32) -> f32 {
    let cycle = beats_per_cycle.max(1.0e-4) as f64;
    let base = (beat_position / cycle).fract() as f32;
    (base + phase_offset).rem_euclid(1.0)
}

#[cfg(test)]
mod tests {
    use super::{phase_from_beats, TransportClock, TransportState};

    #[test]
    fn phase_wraps_to_unit_interval() {
        let phase = phase_from_beats(9.0, 1.0, 0.75);
        assert!((0.0..1.0).contains(&phase));
    }

    #[test]
    fn clock_advances_when_playing_without_song_position() {
        let mut clock = TransportClock::new(48_000.0);
        let a = clock.tick(TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: None,
        });
        let b = clock.tick(TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: None,
        });
        assert!(b.beat_position > a.beat_position);
    }

    #[test]
    fn clock_stops_advancing_when_not_playing() {
        let mut clock = TransportClock::new(48_000.0);
        let a = clock.tick(TransportState {
            tempo_bpm: 120.0,
            is_playing: false,
            song_pos_beats: None,
        });
        let b = clock.tick(TransportState {
            tempo_bpm: 120.0,
            is_playing: false,
            song_pos_beats: None,
        });
        assert_eq!(a.beat_position, b.beat_position);
    }
}
