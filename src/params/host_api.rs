use super::*;
/// Maximum sync division index as floating-point parameter range max.
pub const MAX_SYNC_DIVISION: f32 = (SYNC_DIVISIONS.len() - 1) as f32;

const AUTO: u32 = ParamInfoFlags::IS_AUTOMATABLE.bits();
const AUTO_ENUM: u32 = AUTO | ParamInfoFlags::IS_STEPPED.bits() | ParamInfoFlags::IS_ENUM.bits();

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
        let mut builder = ParamBuilder::new(self.id, self.name.as_bytes(), self.module.as_bytes())
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
}

const PARAM_DEFS: [ParamDef; 4] = [
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
];

#[cfg(feature = "vst3")]
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
    })
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
        PARAM_PHASE_OFFSET_ID => Some(format!("{:.0}%", (value * 100.0).rem_euclid(100.0))),
        PARAM_OUTPUT_GAIN_ID => Some(format!("{value:+.1} dB")),
        PARAM_SYNC_DIVISION_ID => {
            Some(sync_division_label(clamp_sync_division(value as f32)).to_string())
        }
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
