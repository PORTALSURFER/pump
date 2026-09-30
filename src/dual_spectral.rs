//! Allocation-free minimum-phase crossover and sidechain-driven dynamic EQ.
//! New controls occupy IDs 24..35; legacy filter IDs retain their meaning.
pub const COUNT: usize = 12;
pub const DEFAULTS: [f32; COUNT] = [0., 200., 1., 1., 1., 0., 0., 0., 12., 10., 150., 0.];
pub const MIN: [f32; COUNT] = [0., 20., 0., 0., 0., 0., 0., 0., 0., 0.1, 5., 0.];
pub const MAX: [f32; COUNT] = [1., 20000., 1., 1., 1., 1., 1., 1., 36., 500., 2000., 1.];
pub const NAMES: [&str; COUNT] = [
    "Dual Band",
    "Crossover",
    "Crossover Slope",
    "Low Mix",
    "High Mix",
    "Low Solo",
    "High Solo",
    "Spectral Duck",
    "Spectral Depth",
    "Spectral Attack",
    "Spectral Release",
    "Spectral Envelope",
];
pub fn stepped(i: usize) -> bool {
    matches!(i, 0 | 2 | 5 | 6 | 7 | 11)
}
pub fn sanitize(i: usize, value: f32) -> f32 {
    let value = if value.is_finite() {
        value.clamp(MIN[i], MAX[i])
    } else {
        DEFAULTS[i]
    };
    if stepped(i) {
        value.round()
    } else {
        value
    }
}

/// Steady-state LR band magnitudes for the editor's frequency plot.
/// Bilinear frequency warping matches the digital crossover sections.
pub(crate) fn crossover_magnitudes(hz: f32, sr: f32, crossover: f32, slope: usize) -> [f32; 2] {
    let sr = if sr.is_finite() { sr.max(100.) } else { 48000. };
    let frequency = f64::from(hz.clamp(1., sr * 0.45));
    let cutoff = f64::from(crossover.clamp(1., sr * 0.45));
    let rate = f64::from(sr);
    let ratio = (std::f64::consts::PI * frequency / rate).tan()
        / (std::f64::consts::PI * cutoff / rate).tan();
    let power = ratio.powi(if slope == 0 { 2 } else { 4 });
    [(1. / (1. + power)) as f32, (power / (1. + power)) as f32]
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    z1: f64,
    z2: f64,
}
#[derive(Clone, Copy)]
struct Coeff {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}
impl Coeff {
    fn filter(sr: f32, hz: f32, q: f64, high: bool) -> Self {
        let w = 2. * std::f64::consts::PI * f64::from(hz.clamp(1., sr * 0.45)) / f64::from(sr);
        let c = w.cos();
        let a = w.sin() / (2. * q);
        let d = 1. + a;
        let (b0, b1) = if high {
            ((1. + c) / 2., -(1. + c))
        } else {
            ((1. - c) / 2., 1. - c)
        };
        Self {
            b0: b0 / d,
            b1: b1 / d,
            b2: b0 / d,
            a1: -2. * c / d,
            a2: (1. - a) / d,
        }
    }
    fn peak(sr: f32, hz: f32, db: f32) -> Self {
        let w = 2. * std::f64::consts::PI * f64::from(hz) / f64::from(sr);
        let a = 10_f64.powf(f64::from(db) / 40.);
        let alpha = w.sin() / (2. * 3.5);
        let d = 1. + alpha / a;
        Self {
            b0: (1. + alpha * a) / d,
            b1: -2. * w.cos() / d,
            b2: (1. - alpha * a) / d,
            a1: -2. * w.cos() / d,
            a2: (1. - alpha / a) / d,
        }
    }
    fn band(sr: f32, hz: f32) -> Self {
        let w = 2. * std::f64::consts::PI * f64::from(hz) / f64::from(sr);
        let a = w.sin() / (2. * 3.5);
        let d = 1. + a;
        Self {
            b0: a / d,
            b1: 0.,
            b2: -a / d,
            a1: -2. * w.cos() / d,
            a2: (1. - a) / d,
        }
    }
}
impl Biquad {
    fn tick(&mut self, x: f32, c: Coeff) -> f32 {
        let y = c.b0 * f64::from(x) + self.z1;
        self.z1 = c.b1 * f64::from(x) - c.a1 * y + self.z2;
        self.z2 = c.b2 * f64::from(x) - c.a2 * y;
        if y.is_finite() {
            y as f32
        } else {
            *self = Self::default();
            0.
        }
    }
}

#[derive(Default)]
struct Split {
    low: [[Biquad; 2]; 2],
    high: [[Biquad; 2]; 2],
}
impl Split {
    fn tick(&mut self, input: [f32; 2], sr: f32, hz: f32, slope: usize) -> ([f32; 2], [f32; 2]) {
        // LR2 = Butterworth first-order sections cascaded (Q=0.5 biquad).
        // LR4 = two Butterworth second-order sections (Q=sqrt(0.5)).
        let q = if slope == 0 {
            0.5
        } else {
            std::f64::consts::FRAC_1_SQRT_2
        };
        let lc = Coeff::filter(sr, hz, q, false);
        let hc = Coeff::filter(sr, hz, q, true);
        let mut low = input;
        let mut high = input;
        for ch in 0..2 {
            low[ch] = self.low[ch][0].tick(low[ch], lc);
            high[ch] = self.high[ch][0].tick(high[ch], hc);
            if slope == 1 {
                low[ch] = self.low[ch][1].tick(low[ch], lc);
                high[ch] = self.high[ch][1].tick(high[ch], hc);
            } else {
                high[ch] = -high[ch];
            }
        }
        (low, high)
    }
}

pub const BANDS: usize = 24;
pub struct DualSpectral {
    sr: f32,
    controls: [f32; COUNT],
    split: [Split; 2],
    detector: [[Biquad; 2]; BANDS],
    eq: [[Biquad; 2]; BANDS],
    levels: [f32; BANDS],
    pub reduction_db: [f32; BANDS],
    frequencies: [f32; BANDS],
    detector_coeff: [Coeff; BANDS],
    eq_coeff: [Coeff; BANDS],
    control_phase: usize,
}
impl DualSpectral {
    pub fn new(sr: f32) -> Self {
        let sr = if sr.is_finite() { sr.max(100.) } else { 48000. };
        let top = 20000_f32.min(sr * 0.45);
        let frequencies =
            std::array::from_fn(|i| 20. * (top / 20.).powf(i as f32 / (BANDS - 1) as f32));
        Self {
            sr,
            controls: DEFAULTS,
            split: std::array::from_fn(|_| Split::default()),
            detector: [[Biquad::default(); 2]; BANDS],
            eq: [[Biquad::default(); 2]; BANDS],
            levels: [0.; BANDS],
            reduction_db: [0.; BANDS],
            detector_coeff: frequencies.map(|f| Coeff::band(sr, f)),
            eq_coeff: frequencies.map(|f| Coeff::peak(sr, f, 0.)),
            frequencies,
            control_phase: 0,
        }
    }
    pub fn spectrum(&self) -> [f32; BANDS] {
        self.levels
    }
    pub fn reset(&mut self) {
        *self = Self::new(self.sr);
    }
    pub fn prepare(&mut self, target: [f32; COUNT]) {
        let alpha = 1. - (-1. / (self.sr * 0.005)).exp();
        for (i, t) in target.iter().enumerate() {
            self.controls[i] += alpha * (sanitize(i, *t) - self.controls[i]);
        }
    }
    pub fn volume_envelope(&self, envelope: f32) -> f32 {
        envelope + self.controls[7] * self.controls[11] * (1. - envelope)
    }
    pub fn split_mix(
        &mut self,
        input: [f32; 2],
        envelope: f32,
        legacy: [f32; 2],
        trim: f32,
    ) -> [f32; 2] {
        let p = self.controls;
        if p[0] == 0. {
            return legacy;
        }
        let mut dual = [0.; 2];
        for slope in 0..2 {
            let (low, high) = self.split[slope].tick(input, self.sr, p[1], slope);
            let low_gain = 1. - p[3] * (1. - envelope);
            let high_gain = 1. - p[4] * (1. - envelope);
            let solo_any = (p[5] + p[6]).min(1.);
            let lw = 1. - solo_any + p[5];
            let hw = 1. - solo_any + p[6];
            let weight = if slope == 0 { 1. - p[2] } else { p[2] };
            for ch in 0..2 {
                dual[ch] += weight * (low[ch] * low_gain * lw + high[ch] * high_gain * hw) * trim;
            }
        }
        std::array::from_fn(|ch| legacy[ch] + p[0] * (dual[ch] - legacy[ch]))
    }
    pub fn spectral(
        &mut self,
        input: [f32; 2],
        sidechain: Option<[f32; 2]>,
        envelope: f32,
    ) -> [f32; 2] {
        let p = self.controls;
        let alpha = 1. - (-1. / (self.sr * 0.005)).exp();
        let mut output = input;
        let sc = sidechain
            .unwrap_or([0.; 2])
            .map(|v| if v.is_finite() { v } else { 0. });
        let attack = (-1. / (self.sr * p[9] * 0.001)).exp();
        let release = (-1. / (self.sr * p[10] * 0.001)).exp();
        // EQ coefficients update at a bounded control rate; detection and smoothing run per sample.
        for i in 0..BANDS {
            let l = self.detector[i][0]
                .tick(sc[0], self.detector_coeff[i])
                .abs();
            let r = self.detector[i][1]
                .tick(sc[1], self.detector_coeff[i])
                .abs();
            let level = l.max(r);
            let a = if level > self.levels[i] {
                attack
            } else {
                release
            };
            self.levels[i] = a * self.levels[i] + (1. - a) * level;
        }
        let strongest = self.levels.iter().copied().fold(0_f32, f32::max);
        for i in 0..BANDS {
            // Suppress detector skirts so one strong tone does not duck the
            // whole spectrum. Absolute strength still tends to zero at silence.
            let contrast = (self.levels[i] / strongest.max(1e-12)).powi(2);
            let activity = contrast * (strongest * 8.).clamp(0., 1.);
            let envelope_depth = 1. - p[11] + p[11] * (1. - envelope);
            let db = -p[7] * p[8] * activity * envelope_depth;
            self.reduction_db[i] += alpha * (db - self.reduction_db[i]);
            if self.control_phase == 0 {
                self.eq_coeff[i] = Coeff::peak(self.sr, self.frequencies[i], self.reduction_db[i]);
            }
            for (ch, value) in output.iter_mut().enumerate() {
                *value = self.eq[i][ch].tick(*value, self.eq_coeff[i]);
            }
        }
        self.control_phase = (self.control_phase + 1) % 16;
        output
    }
    #[cfg(test)]
    fn tick(
        &mut self,
        input: [f32; 2],
        sidechain: Option<[f32; 2]>,
        target: [f32; COUNT],
        envelope: f32,
        _legacy: [f32; 2],
        trim: f32,
    ) -> [f32; 2] {
        self.prepare(target);
        let filtered = self.spectral(input, sidechain, envelope);
        let volume = self.volume_envelope(envelope);
        self.split_mix(filtered, volume, filtered.map(|x| x * volume * trim), trim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lr_recombination_has_flat_magnitude() {
        for sr in [44100., 48000., 96000.] {
            for slope in 0..2 {
                for hz in [50., 200., 1000., 10000.] {
                    let mut split = Split::default();
                    let mut ins = 0.;
                    let mut outs = 0.;
                    let mut low_power = 0.;
                    let mut high_power = 0.;
                    for n in 0..(sr as usize) {
                        let x = (2. * std::f32::consts::PI * hz * n as f32 / sr).sin();
                        let (l, h) = split.tick([x, 0.], sr, 200., slope);
                        if n > sr as usize / 2 {
                            ins += x * x;
                            outs += (l[0] + h[0]).powi(2);
                            low_power += l[0] * l[0];
                            high_power += h[0] * h[0];
                            assert_eq!(l[1] + h[1], 0.);
                        }
                    }
                    assert!(
                        (outs / ins - 1.).abs() < 0.01,
                        "sr {sr} slope {slope} hz {hz}: {}",
                        outs / ins
                    );
                    let response = crossover_magnitudes(hz, sr, 200., slope);
                    assert!((low_power / ins - response[0].powi(2)).abs() < 0.01);
                    assert!((high_power / ins - response[1].powi(2)).abs() < 0.01);
                }
            }
        }
    }
    #[test]
    fn silent_sidechain_is_unity_and_stereo_stays_independent() {
        let mut dsp = DualSpectral::new(48000.);
        let mut p = DEFAULTS;
        p[7] = 1.;
        for n in 0..48000 {
            let x = (n as f32 * 0.1).sin();
            let y = dsp.tick([x, 0.], None, p, 1., [x, 0.], 1.);
            assert!((y[0] - x).abs() < 1e-5);
            assert_eq!(y[1], 0.);
        }
    }
    #[test]
    fn spectral_sidechain_attenuates_competing_frequency_and_releases() {
        let mut dsp = DualSpectral::new(48000.);
        let mut p = DEFAULTS;
        p[7] = 1.;
        p[8] = 18.;
        let mut power = 0.;
        let mut off = 0.;
        for n in 0..96000 {
            let x = (2. * std::f32::consts::PI * 1000. * n as f32 / 48000.).sin();
            let y = dsp.tick([x, 0.], Some([x, 0.]), p, 1., [x, 0.], 1.);
            if n > 48000 {
                power += y[0] * y[0];
            }
            assert!(y[0].is_finite());
        }
        assert!(power / 48000. < 0.15, "{power}");
        for n in 0..240000 {
            let x = (n as f32 * 0.1).sin();
            let y = dsp.tick([x, 0.], None, p, 1., [x, 0.], 1.);
            if n > 192000 {
                off += (y[0] - x).abs();
            }
        }
        assert!(off / 48000. < 0.005, "{off}");
    }
    fn tone_power(hz: f32, side_hz: Option<f32>, mut p: [f32; COUNT]) -> f32 {
        let mut dsp = DualSpectral::new(48000.);
        let mut sum = 0.;
        p[9] = 1.;
        for n in 0..48000 {
            let x = (std::f32::consts::TAU * hz * n as f32 / 48000.).sin();
            let sc = side_hz.map(|f| {
                let v = (std::f32::consts::TAU * f * n as f32 / 48000.).sin();
                [v, v]
            });
            let y = dsp.tick([x, 0.], sc, p, 0., [0., 0.], 1.);
            if n > 24000 {
                sum += y[0] * y[0];
            }
        }
        sum / 24000.
    }
    #[test]
    fn low_band_ducking_preserves_high_and_solos_isolate_bands() {
        let mut p = DEFAULTS;
        p[0] = 1.;
        p[1] = 400.;
        p[3] = 1.;
        p[4] = 0.;
        assert!(tone_power(50., None, p) < 0.005);
        assert!(tone_power(5000., None, p) > 0.45);
        p[3] = 0.;
        p[5] = 1.;
        assert!(tone_power(50., None, p) > 0.45);
        assert!(tone_power(5000., None, p) < 0.005);
        p[5] = 0.;
        p[6] = 1.;
        assert!(tone_power(5000., None, p) > 0.45);
        assert!(tone_power(50., None, p) < 0.005);
    }
    #[test]
    fn spectral_reduces_target_more_than_distant_frequency() {
        let mut p = DEFAULTS;
        p[7] = 1.;
        p[8] = 18.;
        p[11] = 1.;
        let target = tone_power(1000., Some(1000.), p);
        let distant = tone_power(8000., Some(1000.), p);
        assert!(target < 0.15, "{target}");
        assert!(distant > 0.35, "{distant}");
        assert!(distant > target * 3.);
    }
    #[test]
    fn extreme_automation_is_finite_and_allocation_free() {
        for sr in [22050., 44100., 48000., 96000., 192000.] {
            let mut dsp = DualSpectral::new(sr);
            crate::test_alloc::assert_no_alloc(|| {
                for n in 0..4096 {
                    let p = if n % 256 < 128 { MIN } else { MAX };
                    let x = (n as f32 * 0.4).sin();
                    let y = dsp.tick([x, -x], Some([x, 0.]), p, 0.5, [x, -x], 1.);
                    assert!(y.into_iter().all(|v| v.is_finite() && v.abs() < 10.));
                }
            });
        }
    }
}
