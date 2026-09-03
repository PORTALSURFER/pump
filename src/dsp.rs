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
    /// Evaluated wet-gain smoothing amount in `[0, 1]`.
    pub smooth: f32,
    /// Alternating-subdivision swing amount in `[0, 1]`.
    pub swing: f32,
    /// Timing source: synchronized divisions or a free-running rate.
    pub timing_mode: usize,
    /// Canonical free-running timing rate in hertz.
    pub free_rate_hz: f32,
    /// Whether complete Pump output should crossfade to original dry unity.
    pub bypassed: bool,
}

/// Host-bypass crossfade duration in seconds.
pub const BYPASS_RAMP_SECONDS: f32 = 0.005;

/// Smooth amount at the legacy one-pole time-constant boundary.
pub const SMOOTH_COMPATIBILITY_KNEE: f32 = 0.75;

/// Maximum one-pole time constant at 100% Smooth.
pub const MAX_SMOOTH_TIME_SECONDS: f32 = 0.25;

/// Return the one-pole time constant selected by a Smooth amount.
///
/// The mapping through 75% is intentionally byte-for-byte compatible with the
/// legacy `amount * 100 ms` mapping. The upper tail uses a smoothstep from the
/// legacy 75 ms endpoint to the new 250 ms maximum.
pub(crate) fn smooth_time_constant_seconds(amount: f32) -> f32 {
    let amount = if amount.is_finite() {
        amount.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if amount <= SMOOTH_COMPATIBILITY_KNEE {
        return amount * 0.1;
    }

    let t = (amount - SMOOTH_COMPATIBILITY_KNEE) / (1.0 - SMOOTH_COMPATIBILITY_KNEE);
    let smoothstep = t * t * (3.0 - 2.0 * t);
    let legacy_seconds = amount * 0.1;
    legacy_seconds + (MAX_SMOOTH_TIME_SECONDS - 0.1) * smoothstep
}

/// Information exposed to GUI for metering/visualization.
#[derive(Debug, Copy, Clone)]
pub struct DspTelemetry {
    /// Last processed phase in `[0, 1)`.
    pub phase: f32,
    /// Smoothed phase offset applied to the authored curve for this sample.
    pub applied_phase_offset: f32,
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

/// Map an effective/display phase back to the authored curve phase.
pub(crate) fn authored_curve_phase(effective_phase: f32, phase_offset: f32) -> f32 {
    (effective_phase - phase_offset).rem_euclid(1.0)
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
    swing: OnePole,
    output_gain_db: OnePole,
    wet_gain_smoother: GainSmoother,
    curve_current: [f32; CURVE_TABLE_LEN],
    curve_pending: [f32; CURVE_TABLE_LEN],
    morph_remaining: usize,
    morph_total: usize,
    bypass: ClickSafeBypass,
    free_phase: f64,
    free_phase_active: bool,
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
            swing: OnePole::new(0.0, sample_rate, 0.01),
            output_gain_db: OnePole::new(0.0, sample_rate, 0.01),
            wet_gain_smoother: GainSmoother::new(1.0),
            curve_current: curve,
            curve_pending: curve,
            morph_remaining: 0,
            morph_total: 64,
            bypass: ClickSafeBypass::new(sample_rate, bypassed),
            free_phase: 0.0,
            free_phase_active: false,
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
        self.bypass.reset(bypassed);
        self.free_phase = 0.0;
        self.free_phase_active = false;
    }

    /// Process one sample pair in-place and return telemetry for the last sample.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn process_sample(
        &mut self,
        left: &mut f32,
        right: &mut f32,
        settings: DspSettings,
        transport: TransportState,
    ) -> DspTelemetry {
        self.process_sample_with_raw_cycle_phase(left, right, settings, transport)
            .0
    }

    /// Process one sample and return the private raw cycle-phase witness alongside telemetry.
    pub(crate) fn process_sample_with_raw_cycle_phase(
        &mut self,
        left: &mut f32,
        right: &mut f32,
        settings: DspSettings,
        transport: TransportState,
    ) -> (DspTelemetry, f32) {
        let dry_left = *left;
        let dry_right = *right;
        let frame = self.clock.tick(resolve_effective_transport(transport));

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
        let swing = self.swing.next(if settings.swing.is_finite() {
            settings.swing.clamp(0.0, 1.0)
        } else {
            0.0
        });
        let output_gain_db = self
            .output_gain_db
            .next(settings.output_gain_db.clamp(-60.0, 24.0));

        // Keep the zero-swing path byte-for-byte compatible with the legacy
        // phase calculation. Non-zero swing warps the cyclic phase first and
        // then applies phase offset as a pure cyclic translation.
        let raw_cycle_phase = if settings.timing_mode == crate::params::TIMING_MODE_FREE {
            if !self.free_phase_active {
                self.free_phase = 0.0;
                self.free_phase_active = true;
            }
            let raw_cycle_phase = self.free_phase as f32;
            let rate = crate::params::clamp_free_rate_hz(settings.free_rate_hz) as f64;
            self.free_phase = (self.free_phase + rate / self.sample_rate as f64).fract();
            raw_cycle_phase
        } else {
            self.free_phase_active = false;
            frame.phase_for_cycle(settings.beats_per_cycle, 0.0)
        };
        let phase = if swing <= 0.0 {
            raw_cycle_phase
        } else {
            swing_warp_phase(raw_cycle_phase, swing)
        };
        let shape = self.sample_active_curve(authored_curve_phase(phase, phase_offset));
        let wet_gain = curve_value_to_gain(shape, depth_db, floor_db);
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

        (
            DspTelemetry {
                phase,
                applied_phase_offset: phase_offset,
                gain,
                reduction_gain: blend_gain.clamp(0.0, 1.0),
                input_active: false,
                bypassed: self.bypass.fully_bypassed(),
            },
            raw_cycle_phase,
        )
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

/// Warp a normalized cycle phase for alternating-subdivision swing.
///
/// A value of zero is an exact identity. At one, the logical cycle midpoint
/// occurs at 2/3 of the elapsed cycle (a 2:1 triplet feel); the second half is
/// compressed so the cycle still ends at phase one.
pub fn swing_warp_phase(phase: f32, swing: f32) -> f32 {
    if !phase.is_finite() || !swing.is_finite() || swing <= 0.0 {
        return phase;
    }
    let phase = phase.rem_euclid(1.0);
    let swing = swing.clamp(0.0, 1.0);
    let midpoint = 0.5 + swing / 6.0;
    if phase <= midpoint {
        phase * 0.5 / midpoint
    } else {
        0.5 + (phase - midpoint) * 0.5 / (1.0 - midpoint)
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

        let tau = smooth_time_constant_seconds(amount);
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
        curve_value_to_gain, db_to_linear, smooth_time_constant_seconds, swing_warp_phase,
        DspSettings, PumpEngine, MAX_SMOOTH_TIME_SECONDS, SMOOTH_COMPATIBILITY_KNEE,
    };
    use crate::curve::{default_editable_curve, editable_curve_to_table, sample_curve};
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
            smooth: 0.0,
            swing: 0.0,
            timing_mode: crate::params::DEFAULT_TIMING_MODE,
            free_rate_hz: crate::params::DEFAULT_FREE_RATE_HZ,
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
            );
            assert!(telemetry.gain.is_finite());
            assert!(telemetry.phase.is_finite());
            left = 1.0;
            right = 1.0;
        }
    }

    #[test]
    fn classic_curve_gain_mapping_is_preserved() {
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(0.0),
        };
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
        let mut engine = PumpEngine::new(48_000.0, [0.25; crate::curve::CURVE_TABLE_LEN]);
        let mut left = 1.0;
        let mut right = 1.0;
        engine.process_sample(&mut left, &mut right, settings, transport);
        assert!((left - curve_value_to_gain(0.25, 120.0, -60.0)).abs() < 1.0e-6);
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
            smooth: 0.0,
            swing: 0.0,
            timing_mode: crate::params::DEFAULT_TIMING_MODE,
            free_rate_hz: crate::params::DEFAULT_FREE_RATE_HZ,
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
            );
            let bypass_telemetry = bypassed.process_sample(
                &mut bypass_left,
                &mut bypass_right,
                bypass_test_settings(true),
                bypass_test_transport(),
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
            smooth: 0.0,
            swing: 0.0,
            timing_mode: crate::params::DEFAULT_TIMING_MODE,
            free_rate_hz: crate::params::DEFAULT_FREE_RATE_HZ,
            bypassed: false,
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

    #[test]
    fn free_phase_uses_sample_rate_not_host_tempo_or_song_position() {
        let settings = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            smooth: 0.0,
            swing: 0.0,
            timing_mode: crate::params::TIMING_MODE_FREE,
            free_rate_hz: 10.0,
            bypassed: false,
        };
        let mut first = PumpEngine::new(1_000.0, [0.5; crate::curve::CURVE_TABLE_LEN]);
        let mut second = PumpEngine::new(1_000.0, [0.5; crate::curve::CURVE_TABLE_LEN]);
        for index in 0..8 {
            let mut first_left = 1.0;
            let mut first_right = 1.0;
            let mut second_left = 1.0;
            let mut second_right = 1.0;
            let (first_telemetry, first_raw_cycle_phase) = first
                .process_sample_with_raw_cycle_phase(
                    &mut first_left,
                    &mut first_right,
                    settings,
                    TransportState {
                        tempo_bpm: 40.0,
                        is_playing: true,
                        song_pos_beats: Some(12.0 + index as f64),
                    },
                );
            let (second_telemetry, second_raw_cycle_phase) = second
                .process_sample_with_raw_cycle_phase(
                    &mut second_left,
                    &mut second_right,
                    settings,
                    TransportState {
                        tempo_bpm: 300.0,
                        is_playing: false,
                        song_pos_beats: Some(-44.0),
                    },
                );
            let first_phase = first_telemetry.phase;
            let second_phase = second_telemetry.phase;
            assert!((first_phase - second_phase).abs() < 1.0e-7);
            assert!((first_phase - index as f32 * 0.01).abs() < 1.0e-6);
            assert!((first_raw_cycle_phase - index as f32 * 0.01).abs() < 1.0e-6);
            assert!((second_raw_cycle_phase - index as f32 * 0.01).abs() < 1.0e-6);
        }
    }

    #[test]
    fn free_phase_keeps_raw_origin_after_smoothing() {
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
            free_rate_hz: 10.0,
            bypassed: false,
        };
        let mut engine = PumpEngine::new(1_000.0, [0.5; crate::curve::CURVE_TABLE_LEN]);
        for _ in 0..1_000 {
            engine.process_sample(&mut 1.0, &mut 1.0, settings, TransportState::default());
        }

        let telemetry =
            engine.process_sample(&mut 1.0, &mut 1.0, settings, TransportState::default());
        assert!(telemetry.phase.abs() < 1.0e-4);
    }

    #[test]
    fn switching_out_of_free_timing_resets_phase_for_next_free_run() {
        let free = DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            smooth: 0.0,
            swing: 0.0,
            timing_mode: crate::params::TIMING_MODE_FREE,
            free_rate_hz: 10.0,
            bypassed: false,
        };
        let sync = DspSettings {
            timing_mode: crate::params::TIMING_MODE_SYNC,
            ..free
        };
        let mut engine = PumpEngine::new(1_000.0, [0.5; crate::curve::CURVE_TABLE_LEN]);
        let mut left = 1.0;
        let mut right = 1.0;
        engine.process_sample(&mut left, &mut right, free, TransportState::default());
        engine.process_sample(&mut left, &mut right, free, TransportState::default());
        engine.process_sample(
            &mut left,
            &mut right,
            sync,
            TransportState {
                tempo_bpm: 120.0,
                is_playing: true,
                song_pos_beats: Some(0.5),
            },
        );
        let telemetry =
            engine.process_sample(&mut left, &mut right, free, TransportState::default());
        assert!(telemetry.phase.abs() < 1.0e-7);
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
            smooth: amount,
            swing: 0.0,
            timing_mode: crate::params::DEFAULT_TIMING_MODE,
            free_rate_hz: crate::params::DEFAULT_FREE_RATE_HZ,
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
    fn smooth_time_constant_preserves_legacy_mapping_through_knee() {
        for amount in [0.0, 0.125, 0.5, SMOOTH_COMPATIBILITY_KNEE] {
            assert_eq!(
                smooth_time_constant_seconds(amount),
                amount * 0.1,
                "legacy tau changed at Smooth amount {amount}"
            );
        }
    }

    #[test]
    fn smooth_time_constant_tail_is_monotonic_and_anchored() {
        let knee = smooth_time_constant_seconds(SMOOTH_COMPATIBILITY_KNEE);
        assert_eq!(knee, 0.075);
        assert_eq!(smooth_time_constant_seconds(1.0), MAX_SMOOTH_TIME_SECONDS);

        let mut previous = knee;
        for step in 1..=100 {
            let amount =
                SMOOTH_COMPATIBILITY_KNEE + (1.0 - SMOOTH_COMPATIBILITY_KNEE) * step as f32 / 100.0;
            let current = smooth_time_constant_seconds(amount);
            assert!(current >= previous, "tau must be monotonic at {amount}");
            previous = current;
        }
        assert!(smooth_time_constant_seconds(0.875) > 0.075);
        assert!(smooth_time_constant_seconds(0.875) < MAX_SMOOTH_TIME_SECONDS);
    }

    #[test]
    fn zero_smooth_is_an_exact_identity_for_evaluated_gain() {
        let mut engine = PumpEngine::new(48_000.0, [0.0; crate::curve::CURVE_TABLE_LEN]);
        let settings = smoothing_settings(0.0);
        let mut left = 1.0;
        let mut right = 1.0;
        let telemetry =
            engine.process_sample(&mut left, &mut right, settings, smoothing_transport(120.0));

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
            engine.process_sample(&mut left, &mut right, settings, transport);
            values.push(left);
        }

        assert!(values[0] < 1.0);
        assert!(values[0] > values[1]);
        assert!(values.windows(2).all(|pair| pair[0] >= pair[1]));
        assert!(values
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value)));

        for _ in 0..120_000 {
            let mut left = 1.0;
            let mut right = 1.0;
            engine.process_sample(&mut left, &mut right, settings, transport);
            values.push(left);
        }
        assert!(values.last().copied().unwrap_or(1.0) < 0.001);
    }

    #[test]
    fn full_smooth_step_response_reaches_one_tau_at_250_ms() {
        let mut engine = PumpEngine::new(48_000.0, [0.0; crate::curve::CURVE_TABLE_LEN]);
        let settings = smoothing_settings(1.0);
        let transport = smoothing_transport(120.0);
        let mut response = 1.0;
        for _ in 0..12_000 {
            let mut left = 1.0;
            let mut right = 1.0;
            response = engine
                .process_sample(&mut left, &mut right, settings, transport)
                .gain;
        }

        assert!(
            (response - (-1.0_f32).exp()).abs() < 0.01,
            "250 ms step response should be one tau, got {response}"
        );
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
        );
        let near_zero = engine.process_sample(
            &mut 1.0,
            &mut 1.0,
            smoothing_settings(0.00001),
            smoothing_transport(120.0),
        );
        assert!(near_zero.gain < previous.gain);
        assert!(near_zero.gain.is_finite());

        let before_seek = engine.process_sample(
            &mut 1.0,
            &mut 1.0,
            smoothing_settings(1.0),
            smoothing_transport(60.0),
        );
        let seek_transport = TransportState {
            tempo_bpm: 240.0,
            is_playing: true,
            song_pos_beats: Some(0.37),
        };
        let after_seek =
            engine.process_sample(&mut 1.0, &mut 1.0, smoothing_settings(1.0), seek_transport);
        let mut unsmoothed_reference = PumpEngine::new(48_000.0, ramp);
        let unsmoothed_target = unsmoothed_reference
            .process_sample(&mut 1.0, &mut 1.0, smoothing_settings(0.0), seek_transport)
            .gain;
        assert!(after_seek.gain > before_seek.gain);
        assert!(after_seek.gain < unsmoothed_target);
        assert!(after_seek.gain.is_finite());

        let mut converged = after_seek.gain;
        for _ in 0..240_000 {
            let telemetry =
                engine.process_sample(&mut 1.0, &mut 1.0, smoothing_settings(1.0), seek_transport);
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
            let telemetry =
                at_44k.process_sample(&mut input, &mut right, settings, smoothing_transport(60.0));
            output_44k = telemetry.gain;
        }
        for _ in 0..4_800 {
            let mut input = 1.0;
            let mut right = 1.0;
            let telemetry =
                at_48k.process_sample(&mut input, &mut right, settings, smoothing_transport(240.0));
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
            engine.process_sample(&mut left, &mut right, settings, transport);
        }
        engine.reset();
        let mut left = 1.0;
        let mut right = 1.0;
        engine.process_sample(&mut left, &mut right, settings, transport);
        assert!(left > 0.9, "reset must clear the previous smoothed gain");
    }

    #[test]
    fn swing_warp_is_identity_at_zero_and_triplet_at_full() {
        assert_eq!(super::swing_warp_phase(0.5, 0.0), 0.5);
        assert!((super::swing_warp_phase(0.5, 1.0) - 0.375).abs() < 1.0e-6);
        assert!((super::swing_warp_phase(2.0 / 3.0, 1.0) - 0.5).abs() < 1.0e-6);
        assert!((super::swing_warp_phase(1.0, 1.0) - 0.0).abs() < 1.0e-6);
    }

    fn sync_test_settings(phase_offset: f32, swing: f32) -> DspSettings {
        DspSettings {
            mix: 1.0,
            depth_db: 120.0,
            floor_db: -60.0,
            phase_offset,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
            smooth: 0.0,
            swing,
            timing_mode: crate::params::TIMING_MODE_SYNC,
            free_rate_hz: crate::params::DEFAULT_FREE_RATE_HZ,
            bypassed: false,
        }
    }

    fn settled_sync_telemetry(
        host_beats: f64,
        phase_offset: f32,
        swing: f32,
    ) -> super::DspTelemetry {
        let settings = sync_test_settings(phase_offset, swing);
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(host_beats),
        };
        let mut engine = PumpEngine::new(1_000.0, [0.5; crate::curve::CURVE_TABLE_LEN]);
        engine.process_sample(&mut 1.0, &mut 1.0, settings, transport);
        for _ in 0..256 {
            engine.process_sample(&mut 1.0, &mut 1.0, settings, transport);
        }
        engine.process_sample(&mut 1.0, &mut 1.0, settings, transport)
    }

    #[test]
    fn sync_raw_zero_follows_host_phase_and_samples_authored_curve() {
        let mut curve = [0.5; crate::curve::CURVE_TABLE_LEN];
        curve[0] = 0.13;
        curve[255] = 0.27;
        curve[256] = 0.41;
        curve[511] = 0.59;
        curve[512] = 0.73;
        curve[767] = 0.86;
        curve[768] = 0.32;
        curve[1022] = 0.91;
        curve[1023] = 0.18;
        let settings = sync_test_settings(0.0, 0.0);

        for host_beats in [0.0, 0.25, 0.5, 0.75, 1022.0 / 1023.0] {
            let expected_phase = toybox::dsp::phase_from_beats(host_beats, 1.0, 0.0);
            let transport = TransportState {
                tempo_bpm: 120.0,
                is_playing: true,
                song_pos_beats: Some(host_beats),
            };
            let mut engine = PumpEngine::new(48_000.0, curve);
            let telemetry = engine.process_sample(&mut 1.0, &mut 1.0, settings, transport);
            assert!((telemetry.phase - expected_phase).abs() < 1.0e-6);
            assert!((telemetry.gain - sample_curve(&curve, expected_phase)).abs() < 1.0e-6);
        }
    }

    #[test]
    fn sync_nonzero_offsets_keep_effective_phase_but_change_authored_audio_position() {
        let base = settled_sync_telemetry(0.875, 0.0, 0.0).phase;
        let positive = settled_sync_telemetry(0.875, 0.2, 0.0).phase;
        let wrapped = settled_sync_telemetry(0.875, 0.8, 0.0).phase;

        assert!((base - 0.875).abs() < 1.0e-6);
        assert!((positive - base).abs() < 1.0e-6);
        assert!((wrapped - base).abs() < 1.0e-6);
    }

    #[test]
    fn phase_offset_moves_authored_audio_features_right() {
        let mut curve = [0.0; crate::curve::CURVE_TABLE_LEN];
        let curve_len = curve.len();
        for (index, value) in curve.iter_mut().enumerate() {
            *value = index as f32 / (curve_len - 1) as f32;
        }
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(0.5),
        };
        let mut base_engine = PumpEngine::new(1_000.0, curve);
        let mut offset_engine = PumpEngine::new(1_000.0, curve);
        let base_settings = sync_test_settings(0.0, 0.0);
        let offset_settings = sync_test_settings(0.25, 0.0);
        let mut base_gain = 0.0;
        let mut offset_gain = 0.0;
        for _ in 0..256 {
            base_gain = base_engine
                .process_sample(&mut 1.0, &mut 1.0, base_settings, transport)
                .gain;
            offset_gain = offset_engine
                .process_sample(&mut 1.0, &mut 1.0, offset_settings, transport)
                .gain;
        }
        assert!(offset_gain < base_gain);
    }

    #[test]
    fn sync_swing_warps_host_phase_without_adding_raw_offset() {
        let raw_phase = 0.5_f32;
        let raw_offset = 0.2;
        let actual = settled_sync_telemetry(raw_phase as f64, raw_offset, 1.0).phase;
        let expected = swing_warp_phase(raw_phase, 1.0);
        let offset_before_warp = swing_warp_phase(raw_phase + raw_offset, 1.0);

        assert!((actual - expected).abs() < 1.0e-6);
        assert!((actual - offset_before_warp).abs() > 1.0e-3);
    }

    #[test]
    fn raw_cycle_phase_excludes_swing_and_offset() {
        let settings = sync_test_settings(0.2, 1.0);
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(0.5),
        };
        let mut engine = PumpEngine::new(1_000.0, [0.5; crate::curve::CURVE_TABLE_LEN]);
        engine.process_sample(&mut 1.0, &mut 1.0, settings, transport);
        for _ in 0..256 {
            engine.process_sample(&mut 1.0, &mut 1.0, settings, transport);
        }
        let (telemetry, raw_cycle_phase) =
            engine.process_sample_with_raw_cycle_phase(&mut 1.0, &mut 1.0, settings, transport);

        assert!((raw_cycle_phase - 0.5).abs() < 1.0e-6);
        assert!((telemetry.phase - swing_warp_phase(0.5, 1.0)).abs() < 1.0e-6);
    }
}
