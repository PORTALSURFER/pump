//! Real-time gain-envelope DSP for Pump.

use crate::curve::{sample_curve, CURVE_TABLE_LEN};
use toybox::dsp::{TransportClock, TransportState};

/// Control-rate settings snapshot consumed by the DSP engine.
#[derive(Debug, Copy, Clone)]
pub struct DspSettings {
    /// Dry/wet blend of gain modulation.
    pub mix: f32,
    /// Maximum attenuation applied at a zero curve value, in decibels.
    pub depth_db: f32,
    /// Minimum wet gain in decibels; the minimum value means −∞.
    pub floor_db: f32,
    /// Cycle phase offset.
    pub phase_offset: f32,
    /// Post-gain trim in decibels.
    pub output_gain_db: f32,
    /// Length of one modulation cycle in beats.
    pub beats_per_cycle: f32,
    /// Curve trigger source (`0` host transport, `1` external sidechain).
    pub trigger_mode: usize,
}

/// Sidechain trigger detector policy.
///
/// The detector uses the larger absolute sample of the two sidechain channels.
/// A trigger is a rising crossing of `TRIGGER_ATTACK_THRESHOLD`; the signal
/// must subsequently fall below `TRIGGER_RELEASE_THRESHOLD` before another
/// trigger can be accepted. A 10 ms refractory period rejects chatter while
/// retaining the low-to-high crossing requirement. Missing or silent sidechain
/// input produces no trigger and lets the engine fall back to host timing.
pub const TRIGGER_ATTACK_THRESHOLD: f32 = 0.25;
pub const TRIGGER_RELEASE_THRESHOLD: f32 = 0.125;
pub const TRIGGER_REFRACTORY_MS: f32 = 10.0;

/// Allocation-free, sample-stable sidechain transient detector.
#[derive(Debug, Clone)]
pub struct SidechainTriggerDetector {
    armed: bool,
    refractory_samples: usize,
    refractory_remaining: usize,
}

impl SidechainTriggerDetector {
    /// Create a detector for a sample rate.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            armed: true,
            refractory_samples: ((TRIGGER_REFRACTORY_MS / 1000.0) * sample_rate.max(1.0))
                .round()
                .max(1.0) as usize,
            refractory_remaining: 0,
        }
    }

    /// Process one stereo sidechain sample and report a newly accepted trigger.
    pub fn process(&mut self, left: f32, right: f32) -> bool {
        let level = left.abs().max(right.abs());
        let level = if level.is_finite() { level } else { 0.0 };
        if self.refractory_remaining > 0 {
            self.refractory_remaining -= 1;
        }
        if level <= TRIGGER_RELEASE_THRESHOLD {
            self.armed = true;
        }
        if self.armed && self.refractory_remaining == 0 && level >= TRIGGER_ATTACK_THRESHOLD {
            self.armed = false;
            self.refractory_remaining = self.refractory_samples;
            return true;
        }
        false
    }
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
    sample_rate: f32,
    mix: OnePole,
    depth_db: OnePole,
    floor_db: OnePole,
    phase_offset: OnePole,
    output_gain_db: OnePole,
    curve_current: [f32; CURVE_TABLE_LEN],
    curve_pending: [f32; CURVE_TABLE_LEN],
    morph_remaining: usize,
    morph_total: usize,
    sidechain_detector: SidechainTriggerDetector,
    sidechain_phase: f32,
    sidechain_running: bool,
}

impl PumpEngine {
    /// Create a new engine for the current sample rate and initial curve.
    pub fn new(sample_rate: f32, curve: [f32; CURVE_TABLE_LEN]) -> Self {
        Self {
            clock: TransportClock::new(sample_rate),
            sample_rate: sample_rate.max(1.0),
            mix: OnePole::new(1.0, sample_rate, 0.01),
            depth_db: OnePole::new(120.0, sample_rate, 0.01),
            floor_db: OnePole::new(-60.0, sample_rate, 0.01),
            phase_offset: OnePole::new(0.0, sample_rate, 0.01),
            output_gain_db: OnePole::new(0.0, sample_rate, 0.01),
            curve_current: curve,
            curve_pending: curve,
            morph_remaining: 0,
            morph_total: 64,
            sidechain_detector: SidechainTriggerDetector::new(sample_rate),
            sidechain_phase: 0.0,
            sidechain_running: false,
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
        sidechain: Option<(f32, f32)>,
    ) -> DspTelemetry {
        let frame = self.clock.tick(resolve_effective_transport(transport));

        let sidechain_active =
            settings.trigger_mode == crate::params::TRIGGER_MODE_SIDECHAIN && sidechain.is_some();
        let sidechain_trigger = sidechain_active
            && sidechain.is_some_and(|(left, right)| self.sidechain_detector.process(left, right));

        let mix = self.mix.next(settings.mix.clamp(0.0, 1.0));
        let depth_target = if settings.depth_db.is_finite() {
            settings.depth_db.clamp(0.0, 120.0)
        } else {
            120.0
        };
        let floor_target = if settings.floor_db.is_finite() {
            settings.floor_db.clamp(-60.0, 0.0)
        } else {
            -60.0
        };
        let depth_db = self.depth_db.next(depth_target);
        let floor_db = self.floor_db.next(floor_target);
        let phase_offset = self
            .phase_offset
            .next(settings.phase_offset.rem_euclid(1.0).clamp(0.0, 1.0));
        let output_gain_db = self
            .output_gain_db
            .next(settings.output_gain_db.clamp(-60.0, 24.0));

        let phase = if sidechain_active {
            if sidechain_trigger {
                self.sidechain_phase = 0.0;
                self.sidechain_running = true;
            }
            let phase = (self.sidechain_phase + phase_offset).rem_euclid(1.0);
            if self.sidechain_running {
                let beat_increment = transport.tempo_bpm.clamp(20.0, 320.0)
                    / (self.clock_sample_rate() * 60.0 * settings.beats_per_cycle.max(1.0e-4));
                self.sidechain_phase = (self.sidechain_phase + beat_increment).rem_euclid(1.0);
            }
            phase
        } else {
            frame.phase_for_cycle(settings.beats_per_cycle, phase_offset)
        };
        let shape = self.sample_active_curve(phase);
        let wet_gain = curve_value_to_gain(shape, depth_db, floor_db);
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

    fn clock_sample_rate(&self) -> f32 {
        // The clock intentionally keeps its sample-rate field private. The
        // sidechain phase increment is derived from the same sample-rate
        // contract and is stored by the engine for sample-stable operation.
        self.sample_rate
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

/// Map a normalized curve value to wet gain using Depth and Floor.
///
/// A curve value of `1` is unity. The existing normalized curve is interpreted
/// as its legacy linear gain at maximum Depth; lower Depth values scale its
/// attenuation in dB toward unity. Floor clamps only the wet curve gain; Mix
/// and Output are applied afterwards.
pub fn curve_value_to_gain(curve_value: f32, depth_db: f32, floor_db: f32) -> f32 {
    let curve_value = if curve_value.is_finite() {
        curve_value.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let depth_db = if depth_db.is_finite() {
        depth_db.clamp(0.0, 120.0)
    } else {
        120.0
    };
    let floor_db = if floor_db.is_finite() {
        floor_db.clamp(-60.0, 0.0)
    } else {
        -60.0
    };
    let floor_is_neg_infinity = floor_db <= -60.0;
    let requested_gain = if depth_db <= 0.0 {
        1.0
    } else if curve_value <= 0.0 {
        0.0
    } else if depth_db >= 120.0 {
        curve_value
    } else {
        curve_value.powf(depth_db / 120.0)
    };
    if floor_is_neg_infinity {
        requested_gain.clamp(0.0, 1.0)
    } else {
        requested_gain.max(db_to_linear(floor_db)).clamp(0.0, 1.0)
    }
}

/// Return the finite dB value represented by a wet gain, or −∞ for silence.
pub fn gain_to_db(gain: f32) -> Option<f32> {
    if gain <= 0.0 {
        None
    } else {
        Some(20.0 * gain.clamp(f32::MIN_POSITIVE, 1.0).log10())
    }
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
    use super::{
        curve_value_to_gain, db_to_linear, DspSettings, PumpEngine, SidechainTriggerDetector,
        TRIGGER_ATTACK_THRESHOLD, TRIGGER_RELEASE_THRESHOLD,
    };
    use crate::curve::{default_editable_curve, editable_curve_to_table};
    use toybox::dsp::TransportState;

    #[test]
    fn gain_mapping_stays_finite_for_extremes() {
        let curve = editable_curve_to_table(&default_editable_curve());
        let mut engine = PumpEngine::new(48_000.0, curve);

        let settings = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 12.0,
            beats_per_cycle: 1.0,
            trigger_mode: crate::params::TRIGGER_MODE_HOST,
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
                None,
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
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 12.0,
            beats_per_cycle: 1.0,
            trigger_mode: crate::params::TRIGGER_MODE_HOST,
        };
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(0.0),
        };
        let mut telemetry = engine.process_sample(&mut left, &mut right, settings, transport, None);
        for _ in 0..8_000 {
            left = 1.0;
            right = 1.0;
            telemetry = engine.process_sample(&mut left, &mut right, settings, transport, None);
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
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            trigger_mode: crate::params::TRIGGER_MODE_HOST,
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
                None,
            );
            min_gain = min_gain.min(telemetry.gain);
            left = 1.0;
            right = 1.0;
        }

        assert!(min_gain < 0.95);
    }

    #[test]
    fn depth_and_floor_mapping_covers_shallow_deep_and_finite_floor() {
        assert_eq!(curve_value_to_gain(0.25, 120.0, -60.0), 0.25);
        assert!((curve_value_to_gain(0.25, 60.0, -60.0) - 0.5).abs() < 1.0e-6);
        assert_eq!(curve_value_to_gain(0.0, 120.0, -60.0), 0.0);
        assert!((curve_value_to_gain(0.0, 120.0, -18.0) - db_to_linear(-18.0)).abs() < 1.0e-6);
        assert_eq!(curve_value_to_gain(0.4, 0.0, -60.0), 1.0);
        assert_eq!(curve_value_to_gain(0.0, 0.0, -60.0), 1.0);
        assert!(curve_value_to_gain(f32::NAN, f32::NAN, f32::NAN).is_finite());
    }

    #[test]
    fn sidechain_detector_uses_stereo_hysteresis_and_refractory() {
        let mut detector = SidechainTriggerDetector::new(1_000.0);

        assert!(!detector.process(TRIGGER_ATTACK_THRESHOLD - 0.01, 0.0));
        assert!(detector.process(0.0, TRIGGER_ATTACK_THRESHOLD));

        // A sustained high signal cannot retrigger after the refractory window:
        // it must first cross the release threshold.
        for _ in 0..32 {
            assert!(!detector.process(0.8, 0.1));
        }
        assert!(!detector.process(TRIGGER_RELEASE_THRESHOLD, 0.0));
        assert!(detector.process(0.0, TRIGGER_ATTACK_THRESHOLD + 0.01));
        assert!(!detector.process(f32::NAN, f32::INFINITY));
    }

    #[test]
    fn sidechain_trigger_restarts_at_a_sample_stable_phase() {
        let mut engine = PumpEngine::new(100.0, [1.0; crate::curve::CURVE_TABLE_LEN]);
        let settings = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            trigger_mode: crate::params::TRIGGER_MODE_SIDECHAIN,
        };
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(0.5),
        };
        let mut left = 1.0;
        let mut right = 1.0;

        let before_trigger =
            engine.process_sample(&mut left, &mut right, settings, transport, Some((0.0, 0.0)));
        let trigger =
            engine.process_sample(&mut left, &mut right, settings, transport, Some((0.0, 0.5)));
        let following =
            engine.process_sample(&mut left, &mut right, settings, transport, Some((0.0, 0.5)));

        assert_eq!(before_trigger.phase, 0.0);
        assert_eq!(trigger.phase, 0.0);
        assert!((following.phase - 0.02).abs() < 1.0e-6);
    }

    #[test]
    fn sidechain_mode_falls_back_to_host_phase_when_bus_is_missing() {
        let mut engine = PumpEngine::new(48_000.0, [1.0; crate::curve::CURVE_TABLE_LEN]);
        let settings = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            trigger_mode: crate::params::TRIGGER_MODE_SIDECHAIN,
        };
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(0.5),
        };
        let mut left = 1.0;
        let mut right = 1.0;

        let telemetry = engine.process_sample(&mut left, &mut right, settings, transport, None);

        assert!((telemetry.phase - 0.5).abs() < 1.0e-6);
    }
}
