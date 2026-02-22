use super::*;
/// Maximum sync division index as floating-point parameter range max.
pub const MAX_SYNC_DIVISION: f32 = (SYNC_DIVISIONS.len() - 1) as f32;

const AUTO: u32 = ParamInfoFlags::IS_AUTOMATABLE.bits();
const AUTO_ENUM: u32 = AUTO | ParamInfoFlags::IS_STEPPED.bits() | ParamInfoFlags::IS_ENUM.bits();

#[derive(Copy, Clone)]
struct ParamDef {
    id: ClapId,
    name: &'static [u8],
    module: &'static [u8],
    min_value: f64,
    max_value: f64,
    default_value: f64,
    flags: u32,
}

impl ParamDef {
    fn to_spec(self) -> ParamSpec<'static> {
        let flags = ParamInfoFlags::from_bits_truncate(self.flags);
        let mut builder = ParamBuilder::new(self.id, self.name, self.module)
            .range(self.min_value, self.max_value)
            .default(self.default_value);

        if flags.contains(ParamInfoFlags::IS_AUTOMATABLE) {
            builder = builder.automatable();
        }
        if flags.contains(ParamInfoFlags::IS_STEPPED) {
            builder = builder.stepped();
        }
        if flags.contains(ParamInfoFlags::IS_ENUM) {
            builder = builder.enumerated();
        }

        builder.build()
    }
}

const PARAM_DEFS: [ParamDef; 5] = [
    ParamDef {
        id: PARAM_MIX_ID,
        name: b"Mix",
        module: b"Pump",
        min_value: MIN_MIX as f64,
        max_value: MAX_MIX as f64,
        default_value: DEFAULT_MIX as f64,
        flags: AUTO,
    },
    ParamDef {
        id: PARAM_DEPTH_ID,
        name: b"Depth",
        module: b"Pump",
        min_value: MIN_DEPTH as f64,
        max_value: MAX_DEPTH as f64,
        default_value: DEFAULT_DEPTH as f64,
        flags: AUTO,
    },
    ParamDef {
        id: PARAM_PHASE_OFFSET_ID,
        name: b"Phase Offset",
        module: b"Pump",
        min_value: MIN_PHASE_OFFSET as f64,
        max_value: MAX_PHASE_OFFSET as f64,
        default_value: DEFAULT_PHASE_OFFSET as f64,
        flags: AUTO,
    },
    ParamDef {
        id: PARAM_OUTPUT_GAIN_ID,
        name: b"Output",
        module: b"Pump",
        min_value: MIN_OUTPUT_GAIN_DB as f64,
        max_value: MAX_OUTPUT_GAIN_DB as f64,
        default_value: DEFAULT_OUTPUT_GAIN_DB as f64,
        flags: AUTO,
    },
    ParamDef {
        id: PARAM_SYNC_DIVISION_ID,
        name: b"Division",
        module: b"Pump",
        min_value: 0.0,
        max_value: MAX_SYNC_DIVISION as f64,
        default_value: DEFAULT_SYNC_DIVISION_INDEX as f64,
        flags: AUTO_ENUM,
    },
];

#[cfg(feature = "vst3")]
fn param_def_for_id(param_id: ClapId) -> Option<ParamDef> {
    PARAM_DEFS.iter().copied().find(|def| def.id == param_id)
}

#[cfg(feature = "vst3")]
fn plain_to_normalized(plain: f64, min: f64, max: f64) -> f64 {
    let span = max - min;
    if span.abs() <= f64::EPSILON {
        return 0.0;
    }
    ((plain - min) / span).clamp(0.0, 1.0)
}

#[cfg(feature = "vst3")]
fn normalized_to_plain(normalized: f64, min: f64, max: f64) -> f64 {
    min + normalized.clamp(0.0, 1.0) * (max - min)
}

/// Return one parameter's default plain value.
#[cfg(feature = "vst3")]
pub fn default_plain_value(param_id: ClapId) -> Option<f64> {
    param_def_for_id(param_id).map(|def| def.default_value)
}

/// Convert a parameter plain value to normalized host value.
#[cfg(feature = "vst3")]
pub fn normalized_from_plain_value(param_id: ClapId, plain: f64) -> Option<f64> {
    let def = param_def_for_id(param_id)?;
    Some(plain_to_normalized(plain, def.min_value, def.max_value))
}

/// Convert a parameter normalized host value to plain value.
#[cfg(feature = "vst3")]
pub fn plain_from_normalized_value(param_id: ClapId, normalized: f64) -> Option<f64> {
    let def = param_def_for_id(param_id)?;
    let plain = normalized_to_plain(normalized, def.min_value, def.max_value);
    if param_id == PARAM_SYNC_DIVISION_ID {
        return Some(plain.round());
    }
    Some(plain)
}

/// Return a parameter default as normalized host value.
#[cfg(feature = "vst3")]
pub fn default_normalized_value(param_id: ClapId) -> Option<f64> {
    let plain = default_plain_value(param_id)?;
    normalized_from_plain_value(param_id, plain)
}

/// Return the number of host-visible scalar parameters.
pub fn param_count() -> u32 {
    PARAM_DEFS.len() as u32
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
        PARAM_DEPTH_ID => Some(params.depth() as f64),
        PARAM_PHASE_OFFSET_ID => Some(params.phase_offset() as f64),
        PARAM_OUTPUT_GAIN_ID => Some(params.output_gain_db() as f64),
        PARAM_SYNC_DIVISION_ID => Some(params.sync_division() as f64),
        _ => None,
    }
}

/// Apply one plain parameter value into shared parameter state.
///
/// Returns `true` when `param_id` is recognized.
fn apply_plain_param_value(params: &PumpParams, param_id: ClapId, value: f64) -> bool {
    match param_id {
        PARAM_MIX_ID => params.set_mix(value as f32),
        PARAM_DEPTH_ID => params.set_depth(value as f32),
        PARAM_PHASE_OFFSET_ID => params.set_phase_offset(value as f32),
        PARAM_OUTPUT_GAIN_ID => params.set_output_gain_db(value as f32),
        PARAM_SYNC_DIVISION_ID => params.set_sync_division(value as f32),
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

/// Format a host-visible parameter value for display.
pub fn value_to_text(
    params: &PumpParams,
    param_id: ClapId,
    value: f64,
    writer: &mut ParamDisplayWriter,
) -> std::fmt::Result {
    let _ = params;
    match param_id {
        PARAM_MIX_ID | PARAM_DEPTH_ID => {
            write!(writer, "{:.0}%", (value * 100.0).clamp(0.0, 100.0))
        }
        PARAM_PHASE_OFFSET_ID => write!(writer, "{:.0}%", (value * 100.0).rem_euclid(100.0)),
        PARAM_OUTPUT_GAIN_ID => write!(writer, "{value:+.1} dB"),
        PARAM_SYNC_DIVISION_ID => {
            let index = clamp_sync_division(value as f32);
            write!(writer, "{}", sync_division_label(index))
        }
        _ => Err(std::fmt::Error),
    }
}

/// Parse user-entered text into a host-visible parameter value.
pub fn text_to_value(param_id: ClapId, text: &CStr) -> Option<f64> {
    let raw = text.to_str().ok()?.trim();
    match param_id {
        PARAM_MIX_ID | PARAM_DEPTH_ID => {
            let stripped = raw.trim_end_matches('%').trim();
            let value: f64 = stripped.parse().ok()?;
            Some((value / 100.0).clamp(0.0, 1.0))
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
        _ => None,
    }
}

/// Apply one host automation event value into shared parameter state.
pub fn apply_param_event(params: &PumpParams, param_id: ClapId, value: f32) {
    let _applied = apply_plain_param_value(params, param_id, value as f64);
}
