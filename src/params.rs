//! Parameter definitions and shared atomic state for Pump.

use std::ffi::CStr;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU32, Ordering};

use toybox::clack_extensions::params::{ParamDisplayWriter, ParamInfoFlags, ParamInfoWriter};
use toybox::clack_plugin::prelude::ClapId;
use toybox::clap::params::{ParamBuilder, ParamSpec};

use crate::curve::{default_sidechain_curve, CURVE_TABLE_LEN};

/// Parameter id for dry/wet blend.
pub const PARAM_MIX_ID: ClapId = ClapId::new(1);
/// Parameter id for duck amount.
pub const PARAM_DEPTH_ID: ClapId = ClapId::new(2);
/// Parameter id for cycle phase offset.
pub const PARAM_PHASE_OFFSET_ID: ClapId = ClapId::new(3);
/// Parameter id for output trim in decibels.
pub const PARAM_OUTPUT_GAIN_ID: ClapId = ClapId::new(4);
/// Parameter id for beat-sync cycle division.
pub const PARAM_SYNC_DIVISION_ID: ClapId = ClapId::new(5);

/// Default dry/wet blend.
pub const DEFAULT_MIX: f32 = 1.0;
/// Default duck depth.
pub const DEFAULT_DEPTH: f32 = 0.7;
/// Default cycle phase offset.
pub const DEFAULT_PHASE_OFFSET: f32 = 0.0;
/// Default output gain.
pub const DEFAULT_OUTPUT_GAIN_DB: f32 = 0.0;
/// Default sync division index (`1/4`).
pub const DEFAULT_SYNC_DIVISION_INDEX: usize = 4;

/// Minimum mix value.
pub const MIN_MIX: f32 = 0.0;
/// Maximum mix value.
pub const MAX_MIX: f32 = 1.0;
/// Minimum depth value.
pub const MIN_DEPTH: f32 = 0.0;
/// Maximum depth value.
pub const MAX_DEPTH: f32 = 1.0;
/// Minimum phase offset value.
pub const MIN_PHASE_OFFSET: f32 = 0.0;
/// Maximum phase offset value.
pub const MAX_PHASE_OFFSET: f32 = 1.0;
/// Minimum output gain in decibels.
pub const MIN_OUTPUT_GAIN_DB: f32 = -24.0;
/// Maximum output gain in decibels.
pub const MAX_OUTPUT_GAIN_DB: f32 = 12.0;

/// One named beat division option.
#[derive(Debug, Copy, Clone)]
pub struct SyncDivision {
    /// Human-readable label used in GUI and host display.
    pub label: &'static str,
    /// Length in quarter-note beats.
    pub beats: f32,
}

/// Supported beat-sync cycle divisions.
pub const SYNC_DIVISIONS: [SyncDivision; 8] = [
    SyncDivision {
        label: "1/16",
        beats: 0.25,
    },
    SyncDivision {
        label: "1/8T",
        beats: 1.0 / 3.0,
    },
    SyncDivision {
        label: "1/8",
        beats: 0.5,
    },
    SyncDivision {
        label: "1/4T",
        beats: 2.0 / 3.0,
    },
    SyncDivision {
        label: "1/4",
        beats: 1.0,
    },
    SyncDivision {
        label: "1/2",
        beats: 2.0,
    },
    SyncDivision {
        label: "1 Bar",
        beats: 4.0,
    },
    SyncDivision {
        label: "2 Bars",
        beats: 8.0,
    },
];

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
    match param_id {
        PARAM_MIX_ID => params.set_mix(value),
        PARAM_DEPTH_ID => params.set_depth(value),
        PARAM_PHASE_OFFSET_ID => params.set_phase_offset(value),
        PARAM_OUTPUT_GAIN_ID => params.set_output_gain_db(value),
        PARAM_SYNC_DIVISION_ID => params.set_sync_division(value),
        _ => {}
    }
}

/// Return sync division label for a given index.
pub fn sync_division_label(index: usize) -> &'static str {
    SYNC_DIVISIONS
        .get(index)
        .map(|division| division.label)
        .unwrap_or(SYNC_DIVISIONS[DEFAULT_SYNC_DIVISION_INDEX].label)
}

/// Return cycle length in beats for a given sync division index.
pub fn sync_division_beats(index: usize) -> f32 {
    SYNC_DIVISIONS
        .get(index)
        .map(|division| division.beats)
        .unwrap_or(SYNC_DIVISIONS[DEFAULT_SYNC_DIVISION_INDEX].beats)
}

/// Convert a sync division index into a host-normalized string list.
pub fn sync_division_labels() -> Vec<String> {
    SYNC_DIVISIONS
        .iter()
        .map(|division| division.label.to_string())
        .collect()
}

/// Clamp and round sync division value into a valid index.
pub fn clamp_sync_division(value: f32) -> usize {
    value.round().clamp(0.0, MAX_SYNC_DIVISION) as usize
}

/// Parse a sync division from host/user text.
pub fn sync_division_index_from_text(text: &str) -> Option<usize> {
    let normalized = text.trim().to_ascii_lowercase();
    for (index, division) in SYNC_DIVISIONS.iter().enumerate() {
        let label = division.label.to_ascii_lowercase();
        if normalized == label {
            return Some(index);
        }
    }

    normalized
        .replace("bars", "bar")
        .parse::<f32>()
        .ok()
        .map(clamp_sync_division)
}

/// Shared atomic parameter/state storage across threads.
pub struct PumpParams {
    mix: AtomicF32,
    depth: AtomicF32,
    phase_offset: AtomicF32,
    output_gain_db: AtomicF32,
    sync_division: AtomicU32,
    curve: [AtomicF32; CURVE_TABLE_LEN],
    curve_revision: AtomicU32,
}

impl PumpParams {
    /// Create params with production defaults and default curve.
    pub fn new() -> Self {
        let default_curve = default_sidechain_curve();
        Self {
            mix: AtomicF32::new(DEFAULT_MIX),
            depth: AtomicF32::new(DEFAULT_DEPTH),
            phase_offset: AtomicF32::new(DEFAULT_PHASE_OFFSET),
            output_gain_db: AtomicF32::new(DEFAULT_OUTPUT_GAIN_DB),
            sync_division: AtomicU32::new(DEFAULT_SYNC_DIVISION_INDEX as u32),
            curve: std::array::from_fn(|index| AtomicF32::new(default_curve[index])),
            curve_revision: AtomicU32::new(1),
        }
    }

    /// Get dry/wet mix amount.
    pub fn mix(&self) -> f32 {
        self.mix.load(Ordering::Relaxed)
    }

    /// Get duck depth amount.
    pub fn depth(&self) -> f32 {
        self.depth.load(Ordering::Relaxed)
    }

    /// Get cycle phase offset.
    pub fn phase_offset(&self) -> f32 {
        self.phase_offset.load(Ordering::Relaxed)
    }

    /// Get output gain in decibels.
    pub fn output_gain_db(&self) -> f32 {
        self.output_gain_db.load(Ordering::Relaxed)
    }

    /// Get sync division index.
    pub fn sync_division(&self) -> usize {
        clamp_sync_division(self.sync_division.load(Ordering::Relaxed) as f32)
    }

    /// Get sync division in beats per cycle.
    pub fn sync_beats_per_cycle(&self) -> f32 {
        sync_division_beats(self.sync_division())
    }

    /// Set dry/wet mix amount.
    pub fn set_mix(&self, value: f32) {
        self.mix
            .store(value.clamp(MIN_MIX, MAX_MIX), Ordering::Relaxed);
    }

    /// Set duck depth amount.
    pub fn set_depth(&self, value: f32) {
        self.depth
            .store(value.clamp(MIN_DEPTH, MAX_DEPTH), Ordering::Relaxed);
    }

    /// Set cycle phase offset.
    pub fn set_phase_offset(&self, value: f32) {
        self.phase_offset.store(
            value.clamp(MIN_PHASE_OFFSET, MAX_PHASE_OFFSET),
            Ordering::Relaxed,
        );
    }

    /// Set output gain in decibels.
    pub fn set_output_gain_db(&self, value: f32) {
        self.output_gain_db.store(
            value.clamp(MIN_OUTPUT_GAIN_DB, MAX_OUTPUT_GAIN_DB),
            Ordering::Relaxed,
        );
    }

    /// Set sync division index from scalar host value.
    pub fn set_sync_division(&self, value: f32) {
        let index = clamp_sync_division(value);
        self.sync_division.store(index as u32, Ordering::Relaxed);
    }

    /// Read the current curve revision counter.
    pub fn curve_revision(&self) -> u32 {
        self.curve_revision.load(Ordering::Acquire)
    }

    /// Read one point from the curve table.
    pub fn curve_value(&self, index: usize) -> f32 {
        self.curve
            .get(index)
            .map(|sample| sample.load(Ordering::Acquire).clamp(0.0, 1.0))
            .unwrap_or(1.0)
    }

    /// Snapshot the whole curve table in one array.
    pub fn curve_snapshot(&self) -> [f32; CURVE_TABLE_LEN] {
        let mut values = [1.0_f32; CURVE_TABLE_LEN];
        for (index, value) in values.iter_mut().enumerate() {
            *value = self.curve[index].load(Ordering::Acquire).clamp(0.0, 1.0);
        }
        values
    }

    /// Replace the whole curve table and advance revision.
    pub fn set_curve(&self, values: &[f32; CURVE_TABLE_LEN]) {
        for (index, sample) in values.iter().copied().enumerate() {
            self.curve[index].store(sample.clamp(0.0, 1.0), Ordering::Release);
        }
        self.curve_revision.fetch_add(1, Ordering::AcqRel);
    }

    /// Restore default curve shape and advance revision.
    pub fn reset_curve_to_default(&self) {
        self.set_curve(&default_sidechain_curve());
    }
}

impl Default for PumpParams {
    fn default() -> Self {
        Self::new()
    }
}

/// Encode all parameter state including curve table into bytes.
pub fn encode_state_payload(params: &PumpParams) -> Vec<u8> {
    let mut payload = Vec::with_capacity(4 * (5 + CURVE_TABLE_LEN));
    payload.extend_from_slice(&params.mix().to_le_bytes());
    payload.extend_from_slice(&params.depth().to_le_bytes());
    payload.extend_from_slice(&params.phase_offset().to_le_bytes());
    payload.extend_from_slice(&params.output_gain_db().to_le_bytes());
    payload.extend_from_slice(&(params.sync_division() as f32).to_le_bytes());

    let curve = params.curve_snapshot();
    for sample in curve {
        payload.extend_from_slice(&sample.to_le_bytes());
    }

    payload
}

/// Decode parameter state payload and apply it to shared params.
pub fn decode_state_payload(params: &PumpParams, payload: &[u8]) -> Result<(), &'static str> {
    let expected_len = 4 * (5 + CURVE_TABLE_LEN);
    if payload.len() != expected_len {
        return Err("invalid pump state payload length");
    }

    let mut offset = 0usize;
    let read_f32 = |bytes: &[u8], offset: &mut usize| -> Option<f32> {
        let end = *offset + 4;
        if end > bytes.len() {
            return None;
        }
        let value = f32::from_le_bytes(bytes[*offset..end].try_into().ok()?);
        *offset = end;
        Some(value)
    };

    let Some(mix) = read_f32(payload, &mut offset) else {
        return Err("invalid mix field");
    };
    let Some(depth) = read_f32(payload, &mut offset) else {
        return Err("invalid depth field");
    };
    let Some(phase_offset) = read_f32(payload, &mut offset) else {
        return Err("invalid phase offset field");
    };
    let Some(output_gain_db) = read_f32(payload, &mut offset) else {
        return Err("invalid output gain field");
    };
    let Some(sync_division) = read_f32(payload, &mut offset) else {
        return Err("invalid sync division field");
    };

    let mut curve = [1.0_f32; CURVE_TABLE_LEN];
    for sample in &mut curve {
        let Some(value) = read_f32(payload, &mut offset) else {
            return Err("invalid curve sample");
        };
        *sample = value;
    }

    params.set_mix(mix);
    params.set_depth(depth);
    params.set_phase_offset(phase_offset);
    params.set_output_gain_db(output_gain_db);
    params.set_sync_division(sync_division);
    params.set_curve(&curve);

    Ok(())
}

/// Atomic `f32` utility backed by `AtomicU32`.
struct AtomicF32 {
    value: AtomicU32,
}

impl AtomicF32 {
    fn new(value: f32) -> Self {
        Self {
            value: AtomicU32::new(f32_to_bits(value)),
        }
    }

    fn load(&self, ordering: Ordering) -> f32 {
        bits_to_f32(self.value.load(ordering))
    }

    fn store(&self, value: f32, ordering: Ordering) {
        self.value.store(f32_to_bits(value), ordering);
    }
}

fn f32_to_bits(value: f32) -> u32 {
    u32::from_ne_bytes(value.to_ne_bytes())
}

fn bits_to_f32(value: u32) -> f32 {
    f32::from_ne_bytes(value.to_ne_bytes())
}

#[cfg(test)]
mod tests {
    use super::{
        clamp_sync_division, decode_state_payload, encode_state_payload,
        sync_division_index_from_text, PumpParams, MAX_SYNC_DIVISION,
    };

    #[test]
    fn sync_division_parser_accepts_labels() {
        assert_eq!(sync_division_index_from_text("1/4"), Some(4));
        assert_eq!(sync_division_index_from_text("2 bars"), Some(7));
        assert_eq!(sync_division_index_from_text("bogus"), None);
    }

    #[test]
    fn sync_division_clamping_is_bounded() {
        assert_eq!(clamp_sync_division(-2.0), 0);
        assert_eq!(clamp_sync_division(999.0), MAX_SYNC_DIVISION as usize);
    }

    #[test]
    fn state_roundtrip_preserves_values() {
        let params = PumpParams::new();
        params.set_mix(0.23);
        params.set_depth(0.81);
        params.set_phase_offset(0.42);
        params.set_output_gain_db(-3.0);
        params.set_sync_division(6.0);

        let payload = encode_state_payload(&params);

        let restored = PumpParams::new();
        decode_state_payload(&restored, &payload).expect("state should decode");

        assert!((restored.mix() - 0.23).abs() < 1.0e-6);
        assert!((restored.depth() - 0.81).abs() < 1.0e-6);
        assert!((restored.phase_offset() - 0.42).abs() < 1.0e-6);
        assert!((restored.output_gain_db() + 3.0).abs() < 1.0e-6);
        assert_eq!(restored.sync_division(), 6);
    }
}
