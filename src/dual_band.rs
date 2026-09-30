//! Allocation-free minimum-phase dual-band crossover.
//! Controls retain IDs 24..30; removed IDs 31..35 are never reused.
pub const COUNT: usize = 7;
pub const DEFAULTS: [f32; COUNT] = [0., 200., 1., 1., 1., 0., 0.];
pub const MIN: [f32; COUNT] = [0., 20., 0., 0., 0., 0., 0.];
pub const MAX: [f32; COUNT] = [1., 20000., 1., 1., 1., 1., 1.];
pub const NAMES: [&str; COUNT] = [
    "Dual Band",
    "Crossover",
    "Crossover Slope",
    "Low Mix",
    "High Mix",
    "Low Solo",
    "High Solo",
];
pub fn stepped(i: usize) -> bool {
    matches!(i, 0 | 2 | 5 | 6)
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

pub struct DualBand {
    sr: f32,
    controls: [f32; COUNT],
    split: [Split; 2],
}
impl DualBand {
    pub fn new(sr: f32) -> Self {
        Self {
            sr: if sr.is_finite() { sr.max(100.) } else { 48000. },
            controls: DEFAULTS,
            split: std::array::from_fn(|_| Split::default()),
        }
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
    #[cfg(test)]
    fn tick(
        &mut self,
        input: [f32; 2],
        target: [f32; COUNT],
        envelope: f32,
        _legacy: [f32; 2],
        trim: f32,
    ) -> [f32; 2] {
        self.prepare(target);
        self.split_mix(input, envelope, input.map(|x| x * envelope * trim), trim)
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
    fn tone_power(hz: f32, p: [f32; COUNT]) -> f32 {
        let mut dsp = DualBand::new(48000.);
        let mut sum = 0.;
        for n in 0..48000 {
            let x = (std::f32::consts::TAU * hz * n as f32 / 48000.).sin();
            let y = dsp.tick([x, 0.], p, 0., [0., 0.], 1.);
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
        assert!(tone_power(50., p) < 0.005);
        assert!(tone_power(5000., p) > 0.45);
        p[3] = 0.;
        p[5] = 1.;
        assert!(tone_power(50., p) > 0.45);
        assert!(tone_power(5000., p) < 0.005);
        p[5] = 0.;
        p[6] = 1.;
        assert!(tone_power(5000., p) > 0.45);
        assert!(tone_power(50., p) < 0.005);
    }
    #[test]
    fn extreme_automation_is_finite_and_allocation_free() {
        for sr in [22050., 44100., 48000., 96000., 192000.] {
            let mut dsp = DualBand::new(sr);
            crate::test_alloc::assert_no_alloc(|| {
                for n in 0..4096 {
                    let p = if n % 256 < 128 { MIN } else { MAX };
                    let x = (n as f32 * 0.4).sin();
                    let y = dsp.tick([x, -x], p, 0.5, [x, -x], 1.);
                    assert!(y.into_iter().all(|v| v.is_finite() && v.abs() < 10.));
                }
            });
        }
    }
}
