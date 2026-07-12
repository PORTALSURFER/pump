//! Real-time gain-envelope DSP for Pump.

use crate::curve::{sample_curve, CURVE_TABLE_LEN};
use toybox::dsp::{TransportClock, TransportState};

/// Control-rate settings snapshot consumed by the DSP engine.
#[derive(Debug, Copy, Clone)]
pub struct DspSettings {
    /// Dry/wet blend of gain modulation.
    pub mix: f32,
    /// Cycle phase offset.
    pub phase_offset: f32,
    /// Post-gain trim in decibels.
    pub output_gain_db: f32,
    /// Length of one modulation cycle in beats.
    pub beats_per_cycle: f32,
}

/// Information exposed to GUI for metering/visualization.
#[derive(Debug, Copy, Clone)]
pub struct DspTelemetry {
    /// Last processed phase in `[0, 1)`.
    pub phase: f32,
    /// Last processed total linear gain, retained for DSP verification.
    #[cfg_attr(not(test), allow(dead_code))]
    pub gain: f32,
    /// Pump envelope gain before output trim, used only for gain-reduction metering.
    pub reduction_gain: f32,
    /// Whether this block contained at least one non-silent input sample.
    pub input_active: bool,
}

/// Stateful real-time gain engine.
pub struct PumpEngine {
    clock: TransportClock,
    mix: OnePole,
    phase_offset: OnePole,
    output_gain_db: OnePole,
    curve_current: [f32; CURVE_TABLE_LEN],
    curve_pending: [f32; CURVE_TABLE_LEN],
    morph_remaining: usize,
    morph_total: usize,
}

impl PumpEngine {
    /// Create a new engine for the current sample rate and initial curve.
    pub fn new(sample_rate: f32, curve: [f32; CURVE_TABLE_LEN]) -> Self {
        Self {
            clock: TransportClock::new(sample_rate),
            mix: OnePole::new(1.0, sample_rate, 0.01),
            phase_offset: OnePole::new(0.0, sample_rate, 0.01),
            output_gain_db: OnePole::new(0.0, sample_rate, 0.01),
            curve_current: curve,
            curve_pending: curve,
            morph_remaining: 0,
            morph_total: 64,
        }
    }

    /// Request a smooth morph to a new target curve.
    pub fn set_target_curve(&mut self, curve: [f32; CURVE_TABLE_LEN]) {
        self.curve_pending = curve;
        self.morph_remaining = self.morph_total;
    }

    /// Process one sample pair in-place and return telemetry for the last sample.
    pub fn process_sample(
        &mut self,
        left: &mut f32,
        right: &mut f32,
        settings: DspSettings,
        transport: TransportState,
    ) -> DspTelemetry {
        let frame = self.clock.tick(resolve_effective_transport(transport));

        let mix = self.mix.next(settings.mix.clamp(0.0, 1.0));
        let phase_offset = self
            .phase_offset
            .next(settings.phase_offset.rem_euclid(1.0).clamp(0.0, 1.0));
        let output_gain_db = self
            .output_gain_db
            .next(settings.output_gain_db.clamp(-60.0, 24.0));

        let phase = frame.phase_for_cycle(settings.beats_per_cycle, phase_offset);
        let shape = self.sample_active_curve(phase);
        let wet_gain = shape;
        let blend_gain = (mix * wet_gain) + (1.0 - mix);
        let output_gain = db_to_linear(output_gain_db);
        let gain = (blend_gain * output_gain).clamp(0.0, 4.0);

        *left *= gain;
        *right *= gain;

        DspTelemetry {
            phase,
            gain,
            reduction_gain: blend_gain.clamp(0.0, 1.0),
            input_active: false,
        }
    }

    fn sample_active_curve(&mut self, phase: f32) -> f32 {
        let current = sample_curve(&self.curve_current, phase);
        if self.morph_remaining == 0 {
            return current;
        }

        let pending = sample_curve(&self.curve_pending, phase);
        let progress = 1.0 - (self.morph_remaining as f32 / self.morph_total as f32);
        let morphed = lerp(current, pending, progress).clamp(0.0, 1.0);

        self.morph_remaining -= 1;
        if self.morph_remaining == 0 {
            self.curve_current = self.curve_pending;
        }

        morphed
    }
}

fn resolve_effective_transport(transport: TransportState) -> TransportState {
    if transport.song_pos_beats.is_none() {
        TransportState {
            is_playing: true,
            ..transport
        }
    } else {
        transport
    }
}

/// Convert decibels to linear gain.
pub fn db_to_linear(db: f32) -> f32 {
    (10.0_f32).powf(db / 20.0)
}

struct OnePole {
    value: f32,
    coeff: f32,
}

impl OnePole {
    fn new(initial: f32, sample_rate: f32, time_seconds: f32) -> Self {
        let sr = sample_rate.max(1.0);
        let tau = time_seconds.max(1.0e-4);
        let coeff = (-1.0 / (tau * sr)).exp();
        Self {
            value: initial,
            coeff,
        }
    }

    fn next(&mut self, target: f32) -> f32 {
        self.value = target + self.coeff * (self.value - target);
        self.value
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::{db_to_linear, DspSettings, PumpEngine};
    use crate::curve::{default_editable_curve, editable_curve_to_table};
    use toybox::dsp::TransportState;

    #[test]
    fn gain_mapping_stays_finite_for_extremes() {
        let curve = editable_curve_to_table(&default_editable_curve());
        let mut engine = PumpEngine::new(48_000.0, curve);

        let settings = DspSettings {
            mix: 1.0,
            phase_offset: 0.0,
            output_gain_db: 12.0,
            beats_per_cycle: 1.0,
        };

        let mut left = 1.0;
        let mut right = 1.0;
        for _ in 0..1024 {
            let telemetry = engine.process_sample(
                &mut left,
                &mut right,
                settings,
                TransportState {
                    tempo_bpm: 128.0,
                    is_playing: true,
                    song_pos_beats: None,
                },
            );
            assert!(telemetry.gain.is_finite());
            assert!(telemetry.phase.is_finite());
            left = 1.0;
            right = 1.0;
        }
    }

    #[test]
    fn db_to_linear_matches_reference_points() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1.0e-6);
        assert!((db_to_linear(6.0) - 1.9952623).abs() < 1.0e-4);
    }

    #[test]
    fn reduction_telemetry_excludes_output_trim() {
        let mut engine = PumpEngine::new(48_000.0, [0.5; crate::curve::CURVE_TABLE_LEN]);
        let mut left = 1.0;
        let mut right = 1.0;
        let settings = DspSettings {
            mix: 1.0,
            phase_offset: 0.0,
            output_gain_db: 12.0,
            beats_per_cycle: 1.0,
        };
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(0.0),
        };
        let mut telemetry = engine.process_sample(&mut left, &mut right, settings, transport);
        for _ in 0..8_000 {
            left = 1.0;
            right = 1.0;
            telemetry = engine.process_sample(&mut left, &mut right, settings, transport);
        }
        assert!((telemetry.reduction_gain - 0.5).abs() < 1.0e-6);
        assert!(
            telemetry.gain > 1.0,
            "output trim may boost total gain independently"
        );
    }

    #[test]
    fn modulation_advances_without_host_transport_timeline() {
        let curve = editable_curve_to_table(&default_editable_curve());
        let mut engine = PumpEngine::new(48_000.0, curve);
        let settings = DspSettings {
            mix: 1.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
        };

        let mut min_gain = 1.0_f32;
        let mut left = 1.0_f32;
        let mut right = 1.0_f32;
        for _ in 0..4_096 {
            let telemetry = engine.process_sample(
                &mut left,
                &mut right,
                settings,
                TransportState {
                    tempo_bpm: 120.0,
                    is_playing: false,
                    song_pos_beats: None,
                },
            );
            min_gain = min_gain.min(telemetry.gain);
            left = 1.0;
            right = 1.0;
        }

        assert!(min_gain < 0.95);
    }
}
