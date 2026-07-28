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
    /// Evaluated wet-gain smoothing amount in `[0, 1]`.
    pub smooth: f32,
    /// Processing mode (`Classic` or `Punch`).
    pub mode: usize,
    /// Whether complete Pump output should crossfade to original dry unity.
    pub bypassed: bool,
}

/// Host-bypass crossfade duration in seconds.
pub const BYPASS_RAMP_SECONDS: f32 = 0.005;

/// Maximum one-pole time constant at 100% Smooth.
pub const MAX_SMOOTH_TIME_SECONDS: f32 = 0.1;

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

    /// Re-arm the detector after the trigger source becomes discontinuous.
    pub fn reset(&mut self) {
        self.armed = true;
        self.refractory_remaining = 0;
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
    /// Whether the click-safe bypass ramp has reached full dry unity.
    pub bypassed: bool,
}

#[derive(Debug, Clone)]
struct ClickSafeBypass {
    current: f32,
    target: f32,
    step: f32,
    remaining: usize,
    total: usize,
}

impl ClickSafeBypass {
    fn new(sample_rate: f32, bypassed: bool) -> Self {
        let value = f32::from(bypassed);
        Self {
            current: value,
            target: value,
            step: 0.0,
            remaining: 0,
            total: (sample_rate.max(1.0) * BYPASS_RAMP_SECONDS)
                .round()
                .max(1.0) as usize,
        }
    }

    fn reset(&mut self, bypassed: bool) {
        let value = f32::from(bypassed);
        self.current = value;
        self.target = value;
        self.step = 0.0;
        self.remaining = 0;
    }

    fn next(&mut self, bypassed: bool) -> f32 {
        let target = f32::from(bypassed);
        if target != self.target {
            self.target = target;
            self.remaining = self.total;
            self.step = (self.target - self.current) / self.total as f32;
        }
        if self.remaining > 0 {
            self.current += self.step;
            self.remaining -= 1;
            if self.remaining == 0 {
                self.current = self.target;
            }
        }
        self.current
    }

    fn fully_bypassed(&self) -> bool {
        self.remaining == 0 && self.current == 1.0
    }
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
    mode_blend: OnePole,
    wet_gain_smoother: GainSmoother,
    curve_current: [f32; CURVE_TABLE_LEN],
    curve_pending: [f32; CURVE_TABLE_LEN],
    morph_remaining: usize,
    morph_total: usize,
    sidechain_detector: SidechainTriggerDetector,
    sidechain_phase: f32,
    sidechain_running: bool,
    sidechain_mode: bool,
    sidechain_bus_present: bool,
    bypass: ClickSafeBypass,
}

impl PumpEngine {
    /// Create a new engine for the current sample rate and initial curve.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(sample_rate: f32, curve: [f32; CURVE_TABLE_LEN]) -> Self {
        Self::new_with_bypass(sample_rate, curve, false)
    }

    /// Create an engine whose bypass blend is initialized from restored state.
    pub fn new_with_bypass(
        sample_rate: f32,
        curve: [f32; CURVE_TABLE_LEN],
        bypassed: bool,
    ) -> Self {
        let mut engine = Self {
            clock: TransportClock::new(sample_rate),
            sample_rate: sample_rate.max(1.0),
            mix: OnePole::new(1.0, sample_rate, 0.01),
            depth_db: OnePole::new(120.0, sample_rate, 0.01),
            floor_db: OnePole::new(-60.0, sample_rate, 0.01),
            phase_offset: OnePole::new(0.0, sample_rate, 0.01),
            output_gain_db: OnePole::new(0.0, sample_rate, 0.01),
            mode_blend: OnePole::new(0.0, sample_rate, 0.01),
            wet_gain_smoother: GainSmoother::new(1.0),
            curve_current: curve,
            curve_pending: curve,
            morph_remaining: 0,
            morph_total: 64,
            sidechain_detector: SidechainTriggerDetector::new(sample_rate),
            sidechain_phase: 0.0,
            sidechain_running: false,
            sidechain_mode: false,
            sidechain_bus_present: false,
            bypass: ClickSafeBypass::new(sample_rate, bypassed),
        };
        engine.reset_with_bypass(bypassed);
        engine
    }

    /// Request a smooth morph to a new target curve.
    pub fn set_target_curve(&mut self, curve: [f32; CURVE_TABLE_LEN]) {
        self.curve_pending = curve;
        self.morph_remaining = self.morph_total;
    }

    /// Reset stateful DSP history at a processing-session boundary.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn reset(&mut self) {
        let bypassed = self.bypass.fully_bypassed();
        self.reset_with_bypass(bypassed);
    }

    /// Reset DSP history and snap bypass to restored activation state.
    pub fn reset_with_bypass(&mut self, bypassed: bool) {
        self.wet_gain_smoother.reset(1.0);
        self.mode_blend.reset(0.0);
        self.sidechain_detector.reset();
        self.sidechain_phase = 0.0;
        self.sidechain_running = false;
        self.sidechain_mode = false;
        self.sidechain_bus_present = false;
        self.bypass.reset(bypassed);
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
        let dry_left = *left;
        let dry_right = *right;
        let frame = self.clock.tick(resolve_effective_transport(transport));

        let sidechain_mode = settings.trigger_mode == crate::params::TRIGGER_MODE_SIDECHAIN;
        let sidechain_bus_present = sidechain.is_some();
        if sidechain_mode != self.sidechain_mode
            || (sidechain_mode && sidechain_bus_present != self.sidechain_bus_present)
        {
            // A mode or bus transition is a discontinuity in the trigger
            // source. Do not resume an old event after a host reroute or an
            // automation change; wait for the next deterministic trigger.
            self.sidechain_detector.reset();
            self.sidechain_phase = 0.0;
            self.sidechain_running = false;
        }
        self.sidechain_mode = sidechain_mode;
        if sidechain_mode {
            self.sidechain_bus_present = sidechain_bus_present;
        }
        let sidechain_active = sidechain_mode && sidechain_bus_present;
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
        let mode_target = if settings.mode == crate::params::PROCESSING_MODE_PUNCH {
            1.0
        } else {
            0.0
        };
        let mode_mix = self.mode_blend.next(mode_target);
        let classic_wet_gain = curve_value_to_gain(shape, depth_db, floor_db);
        let punch_wet_gain = curve_value_to_gain(shape, depth_db * 0.5, floor_db);
        let wet_gain = lerp(classic_wet_gain, punch_wet_gain, mode_mix);
        let smooth = if settings.smooth.is_finite() {
            settings.smooth.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let wet_gain = self
            .wet_gain_smoother
            .next(wet_gain, smooth, self.sample_rate);
        let blend_gain = (mix * wet_gain) + (1.0 - mix);
        let output_gain = db_to_linear(output_gain_db);
        let gain = (blend_gain * output_gain).clamp(0.0, 4.0);

        let pumped_left = dry_left * gain;
        let pumped_right = dry_right * gain;
        let bypass_blend = self.bypass.next(settings.bypassed);
        if bypass_blend == 1.0 {
            *left = dry_left;
            *right = dry_right;
        } else if bypass_blend == 0.0 {
            *left = pumped_left;
            *right = pumped_right;
        } else {
            *left = lerp(pumped_left, dry_left, bypass_blend);
            *right = lerp(pumped_right, dry_right, bypass_blend);
        }

        DspTelemetry {
            phase,
            gain,
            reduction_gain: blend_gain.clamp(0.0, 1.0),
            input_active: false,
            bypassed: self.bypass.fully_bypassed(),
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

/// Zero-latency sample-rate-stable smoother for the evaluated wet gain.
struct GainSmoother {
    value: f32,
}

impl GainSmoother {
    fn new(initial: f32) -> Self {
        Self { value: initial }
    }

    fn reset(&mut self, value: f32) {
        self.value = finite_gain(value);
    }

    fn next(&mut self, target: f32, amount: f32, sample_rate: f32) -> f32 {
        let target = finite_gain(target);
        let amount = if amount.is_finite() {
            amount.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if amount <= 0.0 {
            self.value = target;
            return target;
        }

        let tau = amount * MAX_SMOOTH_TIME_SECONDS;
        let coefficient = if tau > 0.0 {
            (-1.0 / (tau * sample_rate.max(1.0))).exp()
        } else {
            0.0
        };
        let next = target + coefficient * (self.value - target);
        self.value = if next.is_finite() {
            // Flush the vanishing tail so the audio thread never carries
            // denormal-sized state between samples.
            if (next - target).abs() < 1.0e-20 {
                target
            } else {
                next.clamp(0.0, 1.0)
            }
        } else {
            target
        };
        self.value
    }
}

fn finite_gain(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        1.0
    }
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

    fn reset(&mut self, value: f32) {
        self.value = value;
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
            smooth: 0.0,
            mode: crate::params::PROCESSING_MODE_CLASSIC,
            bypassed: false,
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
    fn classic_mode_matches_the_existing_curve_gain_and_punch_is_distinct() {
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(0.0),
        };
        let classic = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            trigger_mode: crate::params::TRIGGER_MODE_HOST,
            smooth: 0.0,
            mode: crate::params::PROCESSING_MODE_CLASSIC,
            bypassed: false,
        };
        let punch = DspSettings {
            mode: crate::params::PROCESSING_MODE_PUNCH,
            ..classic
        };
        let mut engine = PumpEngine::new(48_000.0, [0.25; crate::curve::CURVE_TABLE_LEN]);
        let mut left = 1.0;
        let mut right = 1.0;
        engine.process_sample(&mut left, &mut right, classic, transport, None);
        assert!((left - curve_value_to_gain(0.25, 120.0, -60.0)).abs() < 1.0e-6);

        let mut first_punch = 1.0;
        for sample in 0..4_000 {
            left = 1.0;
            right = 1.0;
            engine.process_sample(&mut left, &mut right, punch, transport, None);
            if sample == 0 {
                first_punch = left;
            }
        }
        assert!(
            first_punch < 0.3,
            "mode changes must ramp from Classic safely"
        );
        assert!(
            (left - 0.5).abs() < 0.01,
            "Punch should halve attenuation depth"
        );
    }

    #[test]
    fn db_to_linear_matches_reference_points() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1.0e-6);
        assert!((db_to_linear(6.0) - 1.9952623).abs() < 1.0e-4);
    }

    fn bypass_test_settings(bypassed: bool) -> DspSettings {
        DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            trigger_mode: crate::params::TRIGGER_MODE_HOST,
            smooth: 0.0,
            mode: crate::params::PROCESSING_MODE_CLASSIC,
            bypassed,
        }
    }

    fn bypass_test_transport() -> TransportState {
        TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: None,
        }
    }

    #[test]
    fn bypass_crossfade_has_exact_five_millisecond_endpoints_at_multiple_rates() {
        for sample_rate in [1_000.0_f32, 44_100.0, 48_000.0, 96_000.0] {
            let ramp_samples = (sample_rate * 0.005).round().max(1.0) as usize;
            let mut engine = PumpEngine::new(sample_rate, [0.0; crate::curve::CURVE_TABLE_LEN]);
            let mut penultimate = 0.0;
            for sample in 0..ramp_samples {
                let dry = f32::from_bits(0x3f12_3456);
                let mut left = dry;
                let mut right = -dry;
                let telemetry = engine.process_sample(
                    &mut left,
                    &mut right,
                    bypass_test_settings(true),
                    bypass_test_transport(),
                    None,
                );
                if sample + 1 < ramp_samples {
                    penultimate = left;
                    assert!(!telemetry.bypassed);
                } else {
                    assert_eq!(left.to_bits(), dry.to_bits());
                    assert_eq!(right.to_bits(), (-dry).to_bits());
                    assert!(telemetry.bypassed);
                }
            }
            if ramp_samples > 1 {
                assert!(penultimate < f32::from_bits(0x3f12_3456));
            }

            let dry = f32::from_bits(0x3e91_2345);
            let mut left = dry;
            let mut right = -dry;
            engine.process_sample(
                &mut left,
                &mut right,
                bypass_test_settings(true),
                bypass_test_transport(),
                None,
            );
            assert_eq!(left.to_bits(), dry.to_bits());
            assert_eq!(right.to_bits(), (-dry).to_bits());
        }
    }

    #[test]
    fn bypass_retargets_from_the_current_blend_without_a_jump() {
        let mut engine = PumpEngine::new(1_000.0, [0.0; crate::curve::CURVE_TABLE_LEN]);
        let mut render = |bypassed| {
            let mut left = 1.0;
            let mut right = 1.0;
            engine.process_sample(
                &mut left,
                &mut right,
                bypass_test_settings(bypassed),
                bypass_test_transport(),
                None,
            );
            left
        };

        assert!((render(true) - 0.2).abs() < 1.0e-6);
        assert!((render(true) - 0.4).abs() < 1.0e-6);
        assert!((render(false) - 0.32).abs() < 1.0e-6);
        assert!((render(true) - 0.456).abs() < 1.0e-6);
    }

    #[test]
    fn restored_activation_starts_bypassed_while_dsp_timing_keeps_advancing() {
        let curve = [0.0; crate::curve::CURVE_TABLE_LEN];
        let mut active = PumpEngine::new(1_000.0, curve);
        let mut bypassed = PumpEngine::new_with_bypass(1_000.0, curve, true);
        for _ in 0..16 {
            let dry = f32::from_bits(0x3f01_2345);
            let mut active_left = dry;
            let mut active_right = dry;
            let mut bypass_left = dry;
            let mut bypass_right = dry;
            let active_telemetry = active.process_sample(
                &mut active_left,
                &mut active_right,
                bypass_test_settings(false),
                bypass_test_transport(),
                None,
            );
            let bypass_telemetry = bypassed.process_sample(
                &mut bypass_left,
                &mut bypass_right,
                bypass_test_settings(true),
                bypass_test_transport(),
                None,
            );
            assert_eq!(bypass_left.to_bits(), dry.to_bits());
            assert!(bypass_telemetry.bypassed);
            assert_eq!(bypass_telemetry.phase, active_telemetry.phase);
        }
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
            smooth: 0.0,
            mode: crate::params::PROCESSING_MODE_CLASSIC,
            bypassed: false,
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
            smooth: 0.0,
            mode: crate::params::PROCESSING_MODE_CLASSIC,
            bypassed: false,
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

    fn smoothing_settings(amount: f32) -> DspSettings {
        DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            trigger_mode: crate::params::TRIGGER_MODE_HOST,
            smooth: amount,
            mode: crate::params::PROCESSING_MODE_CLASSIC,
            bypassed: false,
        }
    }

    fn smoothing_transport(tempo_bpm: f32) -> TransportState {
        TransportState {
            tempo_bpm,
            is_playing: true,
            song_pos_beats: Some(0.0),
        }
    }

    #[test]
    fn zero_smooth_is_an_exact_identity_for_evaluated_gain() {
        let mut engine = PumpEngine::new(48_000.0, [0.0; crate::curve::CURVE_TABLE_LEN]);
        let settings = smoothing_settings(0.0);
        let mut left = 1.0;
        let mut right = 1.0;
        let telemetry = engine.process_sample(
            &mut left,
            &mut right,
            settings,
            smoothing_transport(120.0),
            None,
        );

        assert_eq!(left, 0.0);
        assert_eq!(right, 0.0);
        assert_eq!(telemetry.gain, 0.0);
    }

    #[test]
    fn maximum_smooth_softens_an_impulse_without_overshoot_or_nonfinite_state() {
        let mut engine = PumpEngine::new(48_000.0, [0.0; crate::curve::CURVE_TABLE_LEN]);
        let settings = smoothing_settings(1.0);
        let transport = smoothing_transport(120.0);
        let mut values = Vec::new();
        for _ in 0..8 {
            let mut left = 1.0;
            let mut right = 1.0;
            engine.process_sample(&mut left, &mut right, settings, transport, None);
            values.push(left);
        }

        assert!(values[0] < 1.0);
        assert!(values[0] > values[1]);
        assert!(values.windows(2).all(|pair| pair[0] >= pair[1]));
        assert!(values
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value)));

        for _ in 0..48_000 {
            let mut left = 1.0;
            let mut right = 1.0;
            engine.process_sample(&mut left, &mut right, settings, transport, None);
            values.push(left);
        }
        assert!(values.last().copied().unwrap_or(1.0) < 0.001);
    }

    #[test]
    fn smooth_remains_continuous_below_one_sample_and_across_seek() {
        let ramp =
            std::array::from_fn(|index| index as f32 / (crate::curve::CURVE_TABLE_LEN - 1) as f32);
        let mut engine = PumpEngine::new(48_000.0, ramp);
        let previous = engine.process_sample(
            &mut 1.0,
            &mut 1.0,
            smoothing_settings(0.0001),
            smoothing_transport(120.0),
            None,
        );
        let near_zero = engine.process_sample(
            &mut 1.0,
            &mut 1.0,
            smoothing_settings(0.00001),
            smoothing_transport(120.0),
            None,
        );
        assert!(near_zero.gain < previous.gain);
        assert!(near_zero.gain.is_finite());

        let before_seek = engine.process_sample(
            &mut 1.0,
            &mut 1.0,
            smoothing_settings(1.0),
            smoothing_transport(60.0),
            None,
        );
        let seek_transport = TransportState {
            tempo_bpm: 240.0,
            is_playing: true,
            song_pos_beats: Some(0.37),
        };
        let after_seek = engine.process_sample(
            &mut 1.0,
            &mut 1.0,
            smoothing_settings(1.0),
            seek_transport,
            None,
        );
        let mut unsmoothed_reference = PumpEngine::new(48_000.0, ramp);
        let unsmoothed_target = unsmoothed_reference
            .process_sample(
                &mut 1.0,
                &mut 1.0,
                smoothing_settings(0.0),
                seek_transport,
                None,
            )
            .gain;
        assert!(after_seek.gain > before_seek.gain);
        assert!(after_seek.gain < unsmoothed_target);
        assert!(after_seek.gain.is_finite());

        let mut converged = after_seek.gain;
        for _ in 0..48_000 {
            let telemetry = engine.process_sample(
                &mut 1.0,
                &mut 1.0,
                smoothing_settings(1.0),
                seek_transport,
                None,
            );
            converged = telemetry.gain;
        }
        assert!((converged - unsmoothed_target).abs() < 0.001);
    }

    #[test]
    fn smooth_time_constant_is_sample_rate_and_tempo_independent() {
        let mut at_44k = PumpEngine::new(44_100.0, [0.0; crate::curve::CURVE_TABLE_LEN]);
        let mut at_48k = PumpEngine::new(48_000.0, [0.0; crate::curve::CURVE_TABLE_LEN]);
        let settings = smoothing_settings(1.0);
        let mut output_44k = 1.0;
        let mut output_48k = 1.0;
        for _ in 0..4_410 {
            let mut input = 1.0;
            let mut right = 1.0;
            let telemetry = at_44k.process_sample(
                &mut input,
                &mut right,
                settings,
                smoothing_transport(60.0),
                None,
            );
            output_44k = telemetry.gain;
        }
        for _ in 0..4_800 {
            let mut input = 1.0;
            let mut right = 1.0;
            let telemetry = at_48k.process_sample(
                &mut input,
                &mut right,
                settings,
                smoothing_transport(240.0),
                None,
            );
            output_48k = telemetry.gain;
        }
        assert!((output_44k - output_48k).abs() < 0.01);
    }

    #[test]
    fn reset_restarts_smoothing_from_unity() {
        let mut engine = PumpEngine::new(48_000.0, [0.0; crate::curve::CURVE_TABLE_LEN]);
        let settings = smoothing_settings(1.0);
        let transport = smoothing_transport(120.0);
        for _ in 0..2_000 {
            let mut left = 1.0;
            let mut right = 1.0;
            engine.process_sample(&mut left, &mut right, settings, transport, None);
        }
        engine.reset();
        let mut left = 1.0;
        let mut right = 1.0;
        engine.process_sample(&mut left, &mut right, settings, transport, None);
        assert!(left > 0.9, "reset must clear the previous smoothed gain");
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
            smooth: 0.0,
            mode: crate::params::PROCESSING_MODE_CLASSIC,
            bypassed: false,
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
            smooth: 0.0,
            mode: crate::params::PROCESSING_MODE_CLASSIC,
            bypassed: false,
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

    #[test]
    fn sidechain_mode_restarts_after_bus_reconnect_instead_of_resuming_old_phase() {
        let mut engine = PumpEngine::new(100.0, [1.0; crate::curve::CURVE_TABLE_LEN]);
        let settings = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            trigger_mode: crate::params::TRIGGER_MODE_SIDECHAIN,
            smooth: 0.0,
            mode: crate::params::PROCESSING_MODE_CLASSIC,
            bypassed: false,
        };
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: false,
            song_pos_beats: Some(0.75),
        };
        let mut left = 1.0;
        let mut right = 1.0;

        let trigger =
            engine.process_sample(&mut left, &mut right, settings, transport, Some((0.5, 0.0)));
        assert_eq!(trigger.phase, 0.0);

        let running =
            engine.process_sample(&mut left, &mut right, settings, transport, Some((0.5, 0.0)));
        assert!((running.phase - 0.02).abs() < 1.0e-6);

        let fallback = engine.process_sample(&mut left, &mut right, settings, transport, None);
        assert!((fallback.phase - 0.75).abs() < 1.0e-6);

        let reconnected =
            engine.process_sample(&mut left, &mut right, settings, transport, Some((0.5, 0.0)));
        assert_eq!(reconnected.phase, 0.0);
    }

    #[test]
    fn silent_sidechain_does_not_trigger() {
        let mut detector = SidechainTriggerDetector::new(48_000.0);
        for _ in 0..128 {
            assert!(!detector.process(0.0, 0.0));
        }
        assert!(!detector.process(f32::NAN, f32::NAN));
    }
}
