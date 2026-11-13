//! Stable compressor control ranges shared by state, host parameters, and DSP.

pub(crate) const COUNT: usize = 19;
pub(crate) const DEFAULTS: [f32; COUNT] = [
    0.0, -20.0, 10.0, 100.0, 4.0, 60.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1_000.0, 0.707,
    0.0, 0.0, 0.0,
];
pub(crate) const MIN: [f32; COUNT] = [
    0.0, -60.0, 0.01, 10.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -60.0, 0.0, 0.0, 0.0, 20.0, 0.25, 0.0,
    0.0, 0.0,
];
pub(crate) const MAX: [f32; COUNT] = [
    1.0, 0.0, 100.0, 3_000.0, 10.0, 60.0, 20.0, 1.0, 1.0, 1.0, 24.0, 1.0, 1.0, 4.0, 20_000.0, 4.0,
    1.0, 1.0, 1.0,
];

pub(crate) const NAMES: [&str; COUNT] = [
    "Compressor",
    "Threshold",
    "Attack",
    "Release",
    "Ratio",
    "Range",
    "Makeup",
    "Compressor Mix",
    "Soft Clip",
    "External Sidechain",
    "Sidechain Gain",
    "Sidechain Mix",
    "Sidechain EQ",
    "Sidechain Filter",
    "Sidechain Frequency",
    "Sidechain Q",
    "Sidechain Listen",
    "Auto Release",
    "Oversampling",
];

pub(crate) fn stepped(index: usize) -> bool {
    matches!(index, 0 | 8 | 9 | 12 | 13 | 16 | 17 | 18)
}

pub(crate) fn sanitize(index: usize, value: f32) -> f32 {
    let value = if value.is_finite() {
        value.clamp(MIN[index], MAX[index])
    } else {
        DEFAULTS[index]
    };
    if stepped(index) {
        value.round()
    } else {
        value
    }
}
