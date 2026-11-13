//! Stereo-linked gain-reduction detector for sidechain compression.
//!
//! This detector produces gain only; routing, filtering and output processing
//! belong to the caller. The time constants describe exponential settling of
//! gain reduction in decibels, not a model of any particular compressor.

/// Compressor detector controls, in physical units.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CompressorSettings {
    pub(crate) threshold_db: f32,
    pub(crate) ratio: f32,
    pub(crate) attack_ms: f32,
    pub(crate) release_ms: f32,
    pub(crate) range_db: f32,
    pub(crate) auto_release: bool,
}

/// Stateful stereo-linked peak detector. The audio thread owns this value.
#[derive(Debug)]
pub(crate) struct CompressorDetector {
    sample_rate: f32,
    reduction_db: f32,
    slow_reduction_db: f32,
    slow_coefficient: f32,
    cached_times: (f32, f32),
    coefficients: (f32, f32),
}

impl CompressorDetector {
    pub(crate) fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: if sample_rate.is_finite() && sample_rate > 0.0 {
                sample_rate
            } else {
                44_100.0
            },
            reduction_db: 0.0,
            slow_reduction_db: 0.0,
            slow_coefficient: time_coefficient(
                1_000.0,
                if sample_rate.is_finite() && sample_rate > 0.0 {
                    sample_rate
                } else {
                    44_100.0
                },
            ),
            cached_times: (f32::NAN, f32::NAN),
            coefficients: (0.0, 0.0),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.reduction_db = 0.0;
        self.slow_reduction_db = 0.0;
    }

    /// Transfer the detector history without changing this instance's sample rate.
    pub(crate) fn synchronize_from(&mut self, other: &Self) {
        self.reduction_db = other.reduction_db;
        self.slow_reduction_db = other.slow_reduction_db;
    }

    /// Advance by one stereo detector frame and return linear gain in [0, 1].
    pub(crate) fn process(&mut self, left: f32, right: f32, settings: CompressorSettings) -> f32 {
        let magnitude = finite_magnitude(left).max(finite_magnitude(right));
        let input_db = 20.0 * magnitude.max(1.0e-12).log10();
        let threshold_db = finite_or(settings.threshold_db, 0.0).clamp(-120.0, 24.0);
        let ratio = finite_or(settings.ratio, 1.0).clamp(1.0, 100.0);
        let range_db = finite_or(settings.range_db, 0.0).clamp(0.0, 120.0);
        let target = ((input_db - threshold_db).max(0.0) * (1.0 - 1.0 / ratio)).min(range_db);
        let times = (
            finite_or(settings.attack_ms, 0.0).clamp(0.0, 60_000.0),
            if settings.auto_release {
                100.0
            } else {
                finite_or(settings.release_ms, 0.0).clamp(0.0, 60_000.0)
            },
        );
        if times != self.cached_times {
            self.cached_times = times;
            self.coefficients = (
                time_coefficient(times.0, self.sample_rate),
                time_coefficient(times.1, self.sample_rate),
            );
        }
        let coefficient = if target > self.reduction_db {
            self.coefficients.0
        } else {
            self.coefficients.1
        };
        self.reduction_db = target + coefficient * (self.reduction_db - target);
        self.slow_reduction_db = target + self.slow_coefficient * (self.slow_reduction_db - target);
        let reduction_db = if settings.auto_release {
            self.reduction_db.max(self.slow_reduction_db)
        } else {
            self.reduction_db
        };
        10.0_f32.powf(-reduction_db * 0.05)
    }
}

fn time_coefficient(time_ms: f32, sample_rate: f32) -> f32 {
    if time_ms == 0.0 {
        0.0
    } else {
        (-1.0 / (time_ms * 0.001 * sample_rate)).exp()
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

fn finite_magnitude(value: f32) -> f32 {
    if value.is_finite() {
        value.abs()
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Default)]
struct FilterState {
    z1: f32,
    z2: f32,
}

impl FilterState {
    fn tick(&mut self, input: f32, c: [f32; 5]) -> f32 {
        let input = if input.is_finite() { input } else { 0.0 };
        let output = c[0] * input + self.z1;
        self.z1 = c[1] * input - c[3] * output + self.z2;
        self.z2 = c[2] * input - c[4] * output;
        if !output.is_finite() || !self.z1.is_finite() || !self.z2.is_finite() {
            *self = Self::default();
            return 0.0;
        }
        if self.z1.abs() < 1e-20 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1e-20 {
            self.z2 = 0.0;
        }
        output
    }
}

/// Stereo detector-only EQ; coefficients are rebuilt only when controls change.
pub(crate) struct DetectorFilter {
    sample_rate: f32,
    states: [FilterState; 2],
    last: Option<(u32, u32, u32)>,
    coefficients: [f32; 5],
}

impl DetectorFilter {
    pub(crate) fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: if sample_rate.is_finite() {
                sample_rate.max(100.0)
            } else {
                48_000.0
            },
            states: [FilterState::default(); 2],
            last: None,
            coefficients: [1.0, 0.0, 0.0, 0.0, 0.0],
        }
    }

    pub(crate) fn reset(&mut self) {
        self.states = [FilterState::default(); 2];
        self.last = None;
    }

    pub(crate) fn process(
        &mut self,
        input: [f32; 2],
        enabled: bool,
        mode: usize,
        frequency: f32,
        q: f32,
    ) -> [f32; 2] {
        if !enabled {
            if self.last.is_some() {
                self.reset();
            }
            return input;
        }
        let mode = mode.min(4);
        let frequency = finite_or(frequency, 1000.0).clamp(
            20.0_f32.min(self.sample_rate * 0.45),
            self.sample_rate * 0.45,
        );
        let q = finite_or(q, 0.707).clamp(0.25, 4.0);
        let key = (mode as u32, frequency.to_bits(), q.to_bits());
        if self.last != Some(key) {
            if self.last.is_some_and(|previous| previous.0 != key.0) {
                self.states = [FilterState::default(); 2];
            }
            self.coefficients = detector_coefficients(self.sample_rate, mode, frequency, q);
            self.last = Some(key);
        }
        [
            self.states[0].tick(input[0], self.coefficients),
            self.states[1].tick(input[1], self.coefficients),
        ]
    }
}

fn detector_coefficients(sample_rate: f32, mode: usize, frequency: f32, q: f32) -> [f32; 5] {
    let omega = std::f32::consts::TAU * frequency / sample_rate;
    let cosine = omega.cos();
    let alpha = omega.sin() / (2.0 * q);
    let (b0, b1, b2, a0, a1, a2) = match mode {
        0 => (
            (1.0 - cosine) * 0.5,
            1.0 - cosine,
            (1.0 - cosine) * 0.5,
            1.0 + alpha,
            -2.0 * cosine,
            1.0 - alpha,
        ),
        1 => (
            (1.0 + cosine) * 0.5,
            -(1.0 + cosine),
            (1.0 + cosine) * 0.5,
            1.0 + alpha,
            -2.0 * cosine,
            1.0 - alpha,
        ),
        2 => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cosine, 1.0 - alpha),
        3 => (
            1.0,
            -2.0 * cosine,
            1.0,
            1.0 + alpha,
            -2.0 * cosine,
            1.0 - alpha,
        ),
        _ => {
            let amplitude = 10.0_f32.powf(6.0 / 40.0);
            (
                1.0 + alpha * amplitude,
                -2.0 * cosine,
                1.0 - alpha * amplitude,
                1.0 + alpha / amplitude,
                -2.0 * cosine,
                1.0 - alpha / amplitude,
            )
        }
    };
    [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> CompressorSettings {
        CompressorSettings {
            threshold_db: -20.0,
            ratio: 4.0,
            attack_ms: 0.0,
            release_ms: 0.0,
            range_db: 60.0,
            auto_release: false,
        }
    }

    #[test]
    fn threshold_ratio_range_and_stereo_link_control_gain() {
        let mut detector = CompressorDetector::new(48_000.0);
        assert_eq!(detector.process(0.01, 0.0, settings()), 1.0);
        let expected = 10.0_f32.powf(-15.0 / 20.0);
        assert!((detector.process(0.0, -1.0, settings()) - expected).abs() < 1.0e-6);
        let limited = CompressorSettings {
            range_db: 6.0,
            ..settings()
        };
        assert!((detector.process(1.0, 0.0, limited) - 10.0_f32.powf(-6.0 / 20.0)).abs() < 1.0e-6);
        let unity = CompressorSettings {
            ratio: 1.0,
            ..settings()
        };
        assert_eq!(detector.process(1.0, 1.0, unity), 1.0);
    }

    #[test]
    fn attack_release_and_reset_follow_the_gain_reduction() {
        let mut detector = CompressorDetector::new(1_000.0);
        let controls = CompressorSettings {
            attack_ms: 10.0,
            release_ms: 100.0,
            ..settings()
        };
        for _ in 0..10 {
            detector.process(1.0, 0.0, controls);
        }
        let after_attack = detector.reduction_db;
        assert!((after_attack - 15.0 * (1.0 - (-1.0_f32).exp())).abs() < 1.0e-4);
        for _ in 0..100 {
            detector.process(0.0, 0.0, controls);
        }
        assert!((detector.reduction_db - after_attack * (-1.0_f32).exp()).abs() < 1.0e-4);
        detector.reset();
        assert_eq!(detector.process(0.0, 0.0, controls), 1.0);
    }

    #[test]
    fn detector_filter_selects_frequency_without_affecting_other_channel() {
        let mut filter = DetectorFilter::new(48_000.0);
        let mut low_power = 0.0;
        let mut high_power = 0.0;
        for i in 0..12_000 {
            let low = (std::f32::consts::TAU * 100.0 * i as f32 / 48_000.0).sin();
            let high = (std::f32::consts::TAU * 8_000.0 * i as f32 / 48_000.0).sin();
            let output = filter.process([low, high], true, 0, 1_000.0, 0.707);
            if i > 1000 {
                low_power += output[0] * output[0];
                high_power += output[1] * output[1];
            }
        }
        assert!(low_power > high_power * 100.0);
        assert_eq!(
            filter.process([0.3, -0.2], false, 0, 1000.0, 0.707),
            [0.3, -0.2]
        );
    }

    #[test]
    fn automatic_release_retains_sustained_reduction_longer_than_a_transient() {
        let controls = CompressorSettings {
            attack_ms: 1.0,
            auto_release: true,
            ..settings()
        };
        let mut transient = CompressorDetector::new(1_000.0);
        let mut sustained = CompressorDetector::new(1_000.0);
        for _ in 0..10 {
            transient.process(1.0, 0.0, controls);
        }
        for _ in 0..2_000 {
            sustained.process(1.0, 0.0, controls);
        }
        let mut transient_gain = 0.0;
        let mut sustained_gain = 0.0;
        for _ in 0..300 {
            transient_gain = transient.process(0.0, 0.0, controls);
            sustained_gain = sustained.process(0.0, 0.0, controls);
        }
        assert!(sustained_gain < transient_gain - 0.1);
    }

    #[test]
    fn invalid_inputs_do_not_poison_detector_state() {
        let mut detector = CompressorDetector::new(f32::NAN);
        let bad = CompressorSettings {
            threshold_db: f32::NAN,
            ratio: f32::NAN,
            attack_ms: f32::NAN,
            release_ms: f32::NAN,
            range_db: f32::NAN,
            auto_release: false,
        };
        assert_eq!(detector.process(f32::NAN, f32::INFINITY, bad), 1.0);
        assert!(detector.process(1.0, 0.0, settings()).is_finite());
    }
}
