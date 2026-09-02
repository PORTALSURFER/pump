use super::*;
/// Maximum sync division index as floating-point parameter range max.
pub const MAX_SYNC_DIVISION: f32 = (SYNC_DIVISIONS.len() - 1) as f32;

const AUTO: u32 = ParamInfoFlags::IS_AUTOMATABLE.bits();
const AUTO_ENUM: u32 = AUTO | ParamInfoFlags::IS_STEPPED.bits() | ParamInfoFlags::IS_ENUM.bits();
const AUTO_BYPASS: u32 =
    AUTO | ParamInfoFlags::IS_STEPPED.bits() | ParamInfoFlags::IS_BYPASS.bits();

#[derive(Copy, Clone)]
struct ParamDef {
    #[cfg(feature = "vst3")]
    vst3_id: u32,
    id: ClapId,
    name: &'static str,
    #[cfg(feature = "vst3")]
    short_name: &'static str,
    #[cfg(feature = "vst3")]
    units: &'static str,
    module: &'static str,
    min_value: f64,
    max_value: f64,
    default_value: f64,
    flags: u32,
}

impl ParamDef {
    fn to_spec(self) -> ParamSpec<'static> {
        let flags = ParamInfoFlags::from_bits_truncate(self.flags);
        let (min_value, max_value, default_value) = if self.id == PARAM_FREE_RATE_ID {
            (
                0.0,
                1.0,
                normalized_from_plain_value(self.id, self.default_value).unwrap_or(0.0),
            )
        } else {
            (self.min_value, self.max_value, self.default_value)
        };
        let mut builder = ParamBuilder::new(self.id, self.name.as_bytes(), self.module.as_bytes())
            .range(min_value, max_value)
            .default(default_value);

        if flags.contains(ParamInfoFlags::IS_AUTOMATABLE) {
            builder = builder.automatable();
        }
        if flags.contains(ParamInfoFlags::IS_STEPPED) {
            builder = builder.stepped();
        }
        if flags.contains(ParamInfoFlags::IS_ENUM) {
            builder = builder.enumerated();
        }

        let mut spec = builder.build();
        if flags.contains(ParamInfoFlags::IS_BYPASS) {
            spec.flags |= ParamInfoFlags::IS_BYPASS;
        }
        spec
    }
}

#[cfg(feature = "vst3")]
/// Parameter metadata used by the VST3 controller.
pub struct Vst3ParamInfo {
    /// Stable VST3 numeric parameter id.
    pub id: u32,
    /// Long host-facing parameter title.
    pub title: &'static str,
    /// Short host-facing parameter title.
    pub short_title: &'static str,
    /// Host-facing parameter unit label.
    pub units: &'static str,
    /// VST3 step count (`0` for continuous values).
    pub step_count: i32,
    /// Default value in normalized `[0.0, 1.0]` space.
    pub default_normalized: f64,
    /// Whether the VST3 host should treat this parameter as its bypass control.
    pub is_bypass: bool,
}

const PARAM_DEFS: [ParamDef; 12] = [
    ParamDef {
        #[cfg(feature = "vst3")]
        vst3_id: PARAM_MIX_NUM,
        id: PARAM_MIX_ID,
        name: "Mix",
        #[cfg(feature = "vst3")]
        short_name: "Mix",
        #[cfg(feature = "vst3")]
        units: "%",
        module: "Pump",
        min_value: MIN_MIX as f64,
        max_value: MAX_MIX as f64,
        default_value: DEFAULT_MIX as f64,
        flags: AUTO,
    },
    ParamDef {
        #[cfg(feature = "vst3")]
        vst3_id: PARAM_PHASE_OFFSET_NUM,
        id: PARAM_PHASE_OFFSET_ID,
        name: "Phase Offset",
        #[cfg(feature = "vst3")]
        short_name: "Phase",
        #[cfg(feature = "vst3")]
        units: "%",
        module: "Pump",
        min_value: MIN_PHASE_OFFSET as f64,
        max_value: MAX_PHASE_OFFSET as f64,
        default_value: DEFAULT_PHASE_OFFSET as f64,
        flags: AUTO,
    },
    ParamDef {
        #[cfg(feature = "vst3")]
        vst3_id: PARAM_OUTPUT_GAIN_NUM,
        id: PARAM_OUTPUT_GAIN_ID,
        name: "Output",
        #[cfg(feature = "vst3")]
        short_name: "Output",
        #[cfg(feature = "vst3")]
        units: "dB",
        module: "Pump",
        min_value: MIN_OUTPUT_GAIN_DB as f64,
        max_value: MAX_OUTPUT_GAIN_DB as f64,
        default_value: DEFAULT_OUTPUT_GAIN_DB as f64,
        flags: AUTO,
    },
    ParamDef {
        #[cfg(feature = "vst3")]
        vst3_id: PARAM_SYNC_DIVISION_NUM,
        id: PARAM_SYNC_DIVISION_ID,
        name: "Division",
        #[cfg(feature = "vst3")]
        short_name: "Division",
        #[cfg(feature = "vst3")]
        units: "",
        module: "Pump",
        min_value: 0.0,
        max_value: MAX_SYNC_DIVISION as f64,
        default_value: DEFAULT_SYNC_DIVISION_INDEX as f64,
        flags: AUTO_ENUM,
    },
    ParamDef {
        #[cfg(feature = "vst3")]
        vst3_id: PARAM_DEPTH_NUM,
        id: PARAM_DEPTH_ID,
        name: "Depth",
        #[cfg(feature = "vst3")]
        short_name: "Depth",
        #[cfg(feature = "vst3")]
        units: "dB",
        module: "Pump",
        min_value: MIN_DEPTH_DB as f64,
        max_value: MAX_DEPTH_DB as f64,
        default_value: DEFAULT_DEPTH_DB as f64,
        flags: AUTO,
    },
    ParamDef {
        #[cfg(feature = "vst3")]
        vst3_id: PARAM_FLOOR_NUM,
        id: PARAM_FLOOR_ID,
        name: "Floor",
        #[cfg(feature = "vst3")]
        short_name: "Floor",
        #[cfg(feature = "vst3")]
        units: "dB",
        module: "Pump",
        min_value: MIN_FLOOR_DB as f64,
        max_value: MAX_FLOOR_DB as f64,
        default_value: DEFAULT_FLOOR_DB as f64,
        flags: AUTO,
    },
    ParamDef {
        #[cfg(feature = "vst3")]
        vst3_id: PARAM_SMOOTH_NUM,
        id: PARAM_SMOOTH_ID,
        name: "Smooth",
        #[cfg(feature = "vst3")]
        short_name: "Smooth",
        #[cfg(feature = "vst3")]
        units: "%",
        module: "Pump",
        min_value: MIN_SMOOTH as f64,
        max_value: MAX_SMOOTH as f64,
        default_value: DEFAULT_SMOOTH as f64,
        flags: AUTO,
    },
    ParamDef {
        #[cfg(feature = "vst3")]
        vst3_id: PARAM_BYPASS_NUM,
        id: PARAM_BYPASS_ID,
        name: "Bypass",
        #[cfg(feature = "vst3")]
        short_name: "Bypass",
        #[cfg(feature = "vst3")]
        units: "",
        module: "Pump",
        min_value: BYPASS_ACTIVE_VALUE as f64,
        max_value: BYPASS_BYPASSED_VALUE as f64,
        default_value: BYPASS_ACTIVE_VALUE as f64,
        flags: AUTO_BYPASS,
    },
    ParamDef {
        #[cfg(feature = "vst3")]
        vst3_id: PARAM_SWING_NUM,
        id: PARAM_SWING_ID,
        name: "Swing",
        #[cfg(feature = "vst3")]
        short_name: "Swing",
        #[cfg(feature = "vst3")]
        units: "%",
        module: "Pump",
        min_value: MIN_SWING as f64,
        max_value: MAX_SWING as f64,
        default_value: DEFAULT_SWING as f64,
        flags: AUTO,
    },
    ParamDef {
        #[cfg(feature = "vst3")]
        vst3_id: PARAM_SOUND_NUM,
        id: PARAM_SOUND_ID,
        name: "Sound",
        #[cfg(feature = "vst3")]
        short_name: "Sound",
        #[cfg(feature = "vst3")]
        units: "",
        module: "Pump",
        min_value: SoundSide::A.index() as f64,
        max_value: SoundSide::B.index() as f64,
        default_value: SoundSide::A.index() as f64,
        flags: AUTO_ENUM,
    },
    ParamDef {
        #[cfg(feature = "vst3")]
        vst3_id: PARAM_TIMING_MODE_NUM,
        id: PARAM_TIMING_MODE_ID,
        name: "Timing Mode",
        #[cfg(feature = "vst3")]
        short_name: "Timing",
        #[cfg(feature = "vst3")]
        units: "",
        module: "Pump",
        min_value: TIMING_MODE_SYNC as f64,
        max_value: TIMING_MODE_FREE as f64,
        default_value: DEFAULT_TIMING_MODE as f64,
        flags: AUTO_ENUM,
    },
    ParamDef {
        #[cfg(feature = "vst3")]
        vst3_id: PARAM_FREE_RATE_NUM,
        id: PARAM_FREE_RATE_ID,
        name: "Free Rate",
        #[cfg(feature = "vst3")]
        short_name: "Rate",
        #[cfg(feature = "vst3")]
        units: "Hz",
        module: "Pump",
        min_value: MIN_FREE_RATE_HZ as f64,
        max_value: MAX_FREE_RATE_HZ as f64,
        default_value: DEFAULT_FREE_RATE_HZ as f64,
        flags: AUTO,
    },
];

fn param_def_for_id(param_id: ClapId) -> Option<ParamDef> {
    PARAM_DEFS.iter().copied().find(|def| def.id == param_id)
}

#[cfg(feature = "vst3")]
fn param_def_for_vst3_id(param_id: u32) -> Option<ParamDef> {
    PARAM_DEFS
        .iter()
        .copied()
        .find(|def| def.vst3_id == param_id)
}

#[cfg(feature = "vst3")]
fn vst3_step_count(def: ParamDef) -> i32 {
    if ParamInfoFlags::from_bits_truncate(def.flags).contains(ParamInfoFlags::IS_STEPPED) {
        (def.max_value - def.min_value).round() as i32
    } else {
        0
    }
}

fn plain_to_normalized(plain: f64, min: f64, max: f64) -> f64 {
    let span = max - min;
    if span.abs() <= f64::EPSILON {
        return 0.0;
    }
    ((plain - min) / span).clamp(0.0, 1.0)
}

fn free_rate_to_normalized(plain: f64) -> f64 {
    let min = (MIN_FREE_RATE_HZ as f64).ln();
    let max = (MAX_FREE_RATE_HZ as f64).ln();
    ((plain
        .clamp(MIN_FREE_RATE_HZ as f64, MAX_FREE_RATE_HZ as f64)
        .ln()
        - min)
        / (max - min))
        .clamp(0.0, 1.0)
}

fn normalized_to_free_rate(normalized: f64) -> f64 {
    let min = (MIN_FREE_RATE_HZ as f64).ln();
    let max = (MAX_FREE_RATE_HZ as f64).ln();
    (min + normalized.clamp(0.0, 1.0) * (max - min)).exp()
}

fn normalized_to_plain(normalized: f64, min: f64, max: f64) -> f64 {
    min + normalized.clamp(0.0, 1.0) * (max - min)
}

/// Convert a parameter plain value to normalized host value.
pub fn normalized_from_plain_value(param_id: ClapId, plain: f64) -> Option<f64> {
    let def = param_def_for_id(param_id)?;
    if param_id == PARAM_FREE_RATE_ID {
        return Some(free_rate_to_normalized(plain));
    }
    Some(plain_to_normalized(plain, def.min_value, def.max_value))
}

/// Convert a parameter normalized host value to plain value.
pub fn plain_from_normalized_value(param_id: ClapId, normalized: f64) -> Option<f64> {
    let def = param_def_for_id(param_id)?;
    let plain = if param_id == PARAM_FREE_RATE_ID {
        normalized_to_free_rate(normalized)
    } else {
        normalized_to_plain(normalized, def.min_value, def.max_value)
    };
    if matches!(
        param_id,
        PARAM_SYNC_DIVISION_ID | PARAM_BYPASS_ID | PARAM_SOUND_ID | PARAM_TIMING_MODE_ID
    ) {
        return Some(plain.round());
    }
    Some(plain)
}

/// Resolve a VST3 parameter id to the shared CLAP parameter id when recognized.
#[cfg(feature = "vst3")]
pub fn clap_id_from_vst3_param_id(param_id: u32) -> Option<ClapId> {
    param_def_for_vst3_id(param_id).map(|def| def.id)
}

/// Return VST3 metadata for one parameter index.
#[cfg(feature = "vst3")]
pub fn vst3_param_info_for_index(index: i32) -> Option<Vst3ParamInfo> {
    let index = usize::try_from(index).ok()?;
    let def = PARAM_DEFS.get(index).copied()?;
    Some(Vst3ParamInfo {
        id: def.vst3_id,
        title: def.name,
        short_title: def.short_name,
        units: def.units,
        step_count: vst3_step_count(def),
        default_normalized: normalized_from_plain_value(def.id, def.default_value)?,
        is_bypass: ParamInfoFlags::from_bits_truncate(def.flags)
            .contains(ParamInfoFlags::IS_BYPASS),
    })
}

/// Return the number of host-visible scalar parameters.
pub fn param_count() -> u32 {
    PARAM_DEFS.len() as u32
}

#[cfg(test)]
pub(crate) fn param_flags_for_index(index: usize) -> Option<ParamInfoFlags> {
    PARAM_DEFS
        .get(index)
        .map(|def| ParamInfoFlags::from_bits_truncate(def.flags))
}

/// Write parameter metadata for a host parameter index.
pub fn write_param_info(index: u32, info: &mut ParamInfoWriter) {
    let Some(def) = PARAM_DEFS.get(index as usize).copied() else {
        return;
    };
    def.to_spec().write(info);
}

/// Return a parameter's current value when it is host-visible.
pub fn get_param_value(params: &PumpParams, param_id: ClapId) -> Option<f64> {
    match param_id {
        PARAM_MIX_ID => Some(params.mix() as f64),
        PARAM_DEPTH_ID => Some(params.depth_db() as f64),
        PARAM_FLOOR_ID => Some(params.floor_db() as f64),
        PARAM_PHASE_OFFSET_ID => Some(params.phase_offset() as f64),
        PARAM_OUTPUT_GAIN_ID => Some(params.output_gain_db() as f64),
        PARAM_SYNC_DIVISION_ID => Some(params.sync_division() as f64),
        PARAM_SMOOTH_ID => Some(params.smooth() as f64),
        PARAM_BYPASS_ID => Some(params.bypass_value() as f64),
        PARAM_SWING_ID => Some(params.swing() as f64),
        PARAM_SOUND_ID => Some(params.active_sound().index() as f64),
        PARAM_TIMING_MODE_ID => Some(params.timing_mode() as f64),
        PARAM_FREE_RATE_ID => Some(params.free_rate_hz() as f64),
        _ => None,
    }
}

/// Apply one plain parameter value into shared parameter state.
///
/// Returns `true` when `param_id` is recognized.
fn apply_plain_param_value(params: &PumpParams, param_id: ClapId, value: f64) -> bool {
    match param_id {
        PARAM_MIX_ID => params.set_mix(value as f32),
        PARAM_DEPTH_ID => params.set_depth_db(value as f32),
        PARAM_FLOOR_ID => params.set_floor_db(value as f32),
        PARAM_PHASE_OFFSET_ID => params.set_phase_offset(value as f32),
        PARAM_OUTPUT_GAIN_ID => params.set_output_gain_db(value as f32),
        PARAM_SYNC_DIVISION_ID => params.set_sync_division(value as f32),
        PARAM_SMOOTH_ID => params.set_smooth(value as f32),
        PARAM_BYPASS_ID => params.set_bypass_from_host(value as f32),
        PARAM_SWING_ID => params.set_swing(value as f32),
        PARAM_SOUND_ID => {
            let side = if value.round() >= SoundSide::B.index() as f64 {
                SoundSide::B
            } else {
                SoundSide::A
            };
            // Parameter events are delivered from the realtime callback. Queue
            // the side change; the editor projection consumes it on its own
            // thread so A/B snapshots never take an audio-thread lock.
            params.request_active_sound_from_host(side);
        }
        PARAM_TIMING_MODE_ID => params.set_timing_mode(value as f32),
        PARAM_FREE_RATE_ID => params.set_free_rate_hz(value as f32),
        _ => return false,
    }
    true
}

/// Apply one normalized host parameter value into shared parameter state.
///
/// Returns `true` when `param_id` is recognized and converted.
#[cfg(feature = "vst3")]
pub fn apply_normalized_param_value(
    params: &PumpParams,
    param_id: ClapId,
    normalized: f64,
) -> bool {
    let Some(plain) = plain_from_normalized_value(param_id, normalized) else {
        return false;
    };
    apply_plain_param_value(params, param_id, plain)
}

/// Convert a plain value into the normalized value used at the CLAP boundary.
pub fn clap_value_from_plain_value(param_id: ClapId, plain: f64) -> f64 {
    if param_id == PARAM_FREE_RATE_ID {
        normalized_from_plain_value(param_id, plain).unwrap_or(0.0)
    } else {
        plain
    }
}

/// Convert a normalized CLAP value into the plain value used internally.
pub fn plain_value_from_clap_value(param_id: ClapId, value: f64) -> f64 {
    if param_id == PARAM_FREE_RATE_ID {
        // CLAP hosts should honor the encoded [0, 1] metadata range. Keep
        // accepting an out-of-range plain Hz value for compatibility with
        // hosts that persisted the original raw Free Rate representation.
        if value.is_finite() && !(0.0..=1.0).contains(&value) {
            return clamp_free_rate_hz(value as f32) as f64;
        }
        plain_from_normalized_value(param_id, value).unwrap_or(MIN_FREE_RATE_HZ as f64)
    } else {
        value
    }
}

/// Apply one normalized CLAP parameter event into shared parameter state.
pub fn apply_clap_param_event(params: &PumpParams, param_id: ClapId, value: f32) {
    let plain = plain_value_from_clap_value(param_id, value as f64);
    let _applied = apply_plain_param_value(params, param_id, plain);
}

/// Format a host-visible parameter value for display.
pub fn value_to_text(
    params: &PumpParams,
    param_id: ClapId,
    value: f64,
    writer: &mut ParamDisplayWriter,
) -> std::fmt::Result {
    let _ = params;
    let Some(display) = format_plain_value_text(param_id, value) else {
        return Err(std::fmt::Error);
    };
    write!(writer, "{display}")
}

/// Parse user-entered text into a host-visible parameter value.
pub fn text_to_value(param_id: ClapId, text: &CStr) -> Option<f64> {
    let raw = text.to_str().ok()?.trim();
    parse_plain_value_text(param_id, raw)
}

/// Format one plain parameter value into host-facing display text.
pub fn format_plain_value_text(param_id: ClapId, value: f64) -> Option<String> {
    format_plain_value_text_impl(param_id, value)
}

/// Parse host-facing parameter text into one plain parameter value.
pub fn parse_plain_value_text(param_id: ClapId, raw: &str) -> Option<f64> {
    parse_plain_value_text_impl(param_id, raw)
}

fn format_plain_value_text_impl(param_id: ClapId, value: f64) -> Option<String> {
    match param_id {
        PARAM_MIX_ID => Some(format!("{:.0}%", (value * 100.0).clamp(0.0, 100.0))),
        PARAM_DEPTH_ID => Some(format!(
            "{:.0} dB",
            value.clamp(MIN_DEPTH_DB as f64, MAX_DEPTH_DB as f64)
        )),
        PARAM_FLOOR_ID => {
            if value <= MIN_FLOOR_DB as f64 {
                Some("−∞".to_string())
            } else {
                let display_value = (value * 10.0).round() / 10.0;
                let display_value = display_value.max(MIN_FLOOR_DB as f64 + 0.1);
                Some(format!("{display_value:.1} dB"))
            }
        }
        PARAM_PHASE_OFFSET_ID => Some(format!("{:.0}%", (value * 100.0).rem_euclid(100.0))),
        PARAM_OUTPUT_GAIN_ID => Some(format!("{value:+.1} dB")),
        PARAM_SYNC_DIVISION_ID => {
            Some(sync_division_label(clamp_sync_division(value as f32)).to_string())
        }
        PARAM_SMOOTH_ID => Some(format!("{:.0}%", (value * 100.0).clamp(0.0, 100.0))),
        PARAM_SWING_ID => Some(format!("{:.0}%", (value * 100.0).clamp(0.0, 100.0))),
        PARAM_BYPASS_ID => BYPASS_LABELS
            .get(value.round().clamp(0.0, 1.0) as usize)
            .map(|label| (*label).to_string()),
        PARAM_SOUND_ID => Some(
            if value.round() >= SoundSide::B.index() as f64 {
                "B"
            } else {
                "A"
            }
            .to_string(),
        ),
        PARAM_TIMING_MODE_ID => TIMING_MODE_LABELS
            .get(clamp_timing_mode(value as f32))
            .map(|label| (*label).to_string()),
        PARAM_FREE_RATE_ID => Some(format_free_rate(value as f32)),
        _ => None,
    }
}

fn parse_plain_value_text_impl(param_id: ClapId, raw: &str) -> Option<f64> {
    match param_id {
        PARAM_MIX_ID => {
            let stripped = raw.trim_end_matches('%').trim();
            let value: f64 = stripped.parse().ok()?;
            Some((value / 100.0).clamp(0.0, 1.0))
        }
        PARAM_DEPTH_ID => {
            let stripped = raw.trim_end_matches("dB").trim();
            let value: f64 = stripped.parse().ok()?;
            Some(value.clamp(MIN_DEPTH_DB as f64, MAX_DEPTH_DB as f64))
        }
        PARAM_FLOOR_ID => {
            let normalized = raw.trim().to_ascii_lowercase();
            if matches!(
                normalized.as_str(),
                "−∞" | "-∞" | "∞" | "-inf" | "-infinity"
            ) {
                return Some(MIN_FLOOR_DB as f64);
            }
            let stripped = raw.trim_end_matches("dB").trim();
            let value: f64 = stripped.parse().ok()?;
            Some(value.clamp(MIN_FLOOR_DB as f64, MAX_FLOOR_DB as f64))
        }
        PARAM_PHASE_OFFSET_ID => {
            let stripped = raw.trim_end_matches('%').trim();
            let value: f64 = stripped.parse().ok()?;
            Some((value / 100.0).rem_euclid(1.0))
        }
        PARAM_OUTPUT_GAIN_ID => {
            let stripped = raw.trim_end_matches("dB").trim();
            let value: f64 = stripped.parse().ok()?;
            Some(value.clamp(MIN_OUTPUT_GAIN_DB as f64, MAX_OUTPUT_GAIN_DB as f64))
        }
        PARAM_SYNC_DIVISION_ID => sync_division_index_from_text(raw).map(|index| index as f64),
        PARAM_SMOOTH_ID => {
            let stripped = raw.trim_end_matches('%').trim();
            let value: f64 = stripped.parse().ok()?;
            Some((value / 100.0).clamp(MIN_SMOOTH as f64, MAX_SMOOTH as f64))
        }
        PARAM_SWING_ID => {
            let stripped = raw.trim_end_matches('%').trim();
            let value: f64 = stripped.parse().ok()?;
            Some((value / 100.0).clamp(MIN_SWING as f64, MAX_SWING as f64))
        }
        PARAM_BYPASS_ID => {
            let normalized = raw.trim().to_ascii_lowercase();
            BYPASS_LABELS
                .iter()
                .position(|label| label.to_ascii_lowercase() == normalized)
                .map(|index| index as f64)
                .or_else(|| {
                    raw.parse::<f64>()
                        .ok()
                        .map(|value| value.round().clamp(0.0, 1.0))
                })
        }
        PARAM_SOUND_ID => {
            let normalized = raw.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "a" | "0" => Some(0.0),
                "b" | "1" => Some(1.0),
                _ => None,
            }
        }
        PARAM_TIMING_MODE_ID => TIMING_MODE_LABELS
            .iter()
            .position(|label| label.eq_ignore_ascii_case(raw.trim()))
            .map(|index| index as f64)
            .or_else(|| {
                raw.parse::<f64>()
                    .ok()
                    .map(|value| clamp_timing_mode(value as f32) as f64)
            }),
        PARAM_FREE_RATE_ID => parse_free_rate(raw).map(|value| value as f64),
        _ => None,
    }
}

fn format_free_rate(value: f32) -> String {
    let hz = clamp_free_rate_hz(value);
    if hz >= 1_000.0 {
        format!("{} kHz", hz / 1_000.0)
    } else if hz >= 1.0 {
        format!("{} Hz", hz)
    } else {
        let seconds = 1.0 / hz;
        if seconds >= 1.0 {
            format!("{} s", seconds)
        } else {
            format!("{} ms", seconds * 1_000.0)
        }
    }
}

fn parse_free_rate(raw: &str) -> Option<f32> {
    let normalized = raw.trim().to_ascii_lowercase();
    let (number, multiplier, reciprocal) = if let Some(value) = normalized.strip_suffix("khz") {
        (value.trim(), 1_000.0, false)
    } else if let Some(value) = normalized.strip_suffix("hz") {
        (value.trim(), 1.0, false)
    } else if let Some(value) = normalized.strip_suffix("ms") {
        (value.trim(), 1_000.0, true)
    } else if let Some(value) = normalized.strip_suffix('s') {
        (value.trim(), 1.0, true)
    } else {
        (normalized.as_str(), 1.0, false)
    };
    let number = number.parse::<f32>().ok()?;
    if reciprocal && number <= 0.0 {
        return None;
    }
    let value = if reciprocal {
        multiplier / number
    } else {
        number * multiplier
    };
    Some(clamp_free_rate_hz(value))
}

/// Apply one host automation event value into shared parameter state.
pub fn apply_param_event(params: &PumpParams, param_id: ClapId, value: f32) {
    let _applied = apply_plain_param_value(params, param_id, value as f64);
}
