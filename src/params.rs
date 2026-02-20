//! Parameter definitions and shared atomic state for Pump.

use std::ffi::CStr;
use std::fmt::Write as _;
use std::io::Cursor;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::RwLock;

use toybox::clack_extensions::params::{ParamDisplayWriter, ParamInfoFlags, ParamInfoWriter};
use toybox::clack_plugin::prelude::ClapId;
use toybox::clap::params::{ParamBuilder, ParamSpec};
use toybox::dsp::AtomicF32;

use crate::curve::{
    curve_table_to_editable, default_editable_curve, editable_curve_to_table, CurveNode,
    CurveSegment, EditableCurve, CURVE_TABLE_LEN, MAX_EDITABLE_NODES,
};

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
/// Maximum number of stored user presets.
pub const MAX_PRESETS: usize = 16;
/// Maximum preset-name length in characters.
pub const MAX_PRESET_NAME_CHARS: usize = 24;
/// Default preset name for the initialized plugin state.
pub const DEFAULT_PRESET_NAME: &str = "Init";

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

const STATE_MAGIC: &[u8; 4] = b"PMP2";
const STATE_VERSION: u32 = 4;

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

/// Serializable snapshot of one Pump preset.
#[derive(Clone, Debug, PartialEq)]
pub struct PumpPreset {
    /// User-facing preset name.
    pub name: String,
    /// True when this preset is immutable (for built-in `Init`).
    pub is_read_only: bool,
    /// Dry/wet mix amount.
    pub mix: f32,
    /// Duck depth amount.
    pub depth: f32,
    /// Cycle phase offset.
    pub phase_offset: f32,
    /// Output trim in decibels.
    pub output_gain_db: f32,
    /// Sync-division index.
    pub sync_division: usize,
    /// Editable curve shape.
    pub editable_curve: EditableCurve,
}

/// Ordered preset collection with a selected index.
#[derive(Clone, Debug, PartialEq)]
pub struct PumpPresetBank {
    /// Currently selected preset index.
    pub selected: usize,
    /// Preset entries.
    pub presets: Vec<PumpPreset>,
}

/// Outcome from saving current state into the preset bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavePresetOutcome {
    /// Existing non-read-only preset was overwritten.
    Overwritten {
        /// Index of the overwritten preset.
        index: usize,
    },
    /// New preset was created.
    Created {
        /// Index of the created preset.
        index: usize,
    },
    /// Save was blocked because target preset is immutable.
    BlockedReadOnly,
    /// Save was blocked because the preset bank is at capacity.
    BlockedFull,
    /// Save was blocked due to an invalid name.
    InvalidName,
}

impl PumpPresetBank {
    /// Build the default single-entry preset bank from plugin defaults.
    pub fn default_init() -> Self {
        Self {
            selected: 0,
            presets: vec![PumpPreset {
                name: DEFAULT_PRESET_NAME.to_string(),
                is_read_only: true,
                mix: DEFAULT_MIX,
                depth: DEFAULT_DEPTH,
                phase_offset: DEFAULT_PHASE_OFFSET,
                output_gain_db: DEFAULT_OUTPUT_GAIN_DB,
                sync_division: DEFAULT_SYNC_DIVISION_INDEX,
                editable_curve: default_editable_curve(),
            }],
        }
    }
}

/// Build the canonical read-only `Init` preset entry.
fn default_init_preset() -> PumpPreset {
    PumpPreset {
        name: DEFAULT_PRESET_NAME.to_string(),
        is_read_only: true,
        mix: DEFAULT_MIX,
        depth: DEFAULT_DEPTH,
        phase_offset: DEFAULT_PHASE_OFFSET,
        output_gain_db: DEFAULT_OUTPUT_GAIN_DB,
        sync_division: DEFAULT_SYNC_DIVISION_INDEX,
        editable_curve: default_editable_curve(),
    }
}

fn sanitize_preset_name(raw: &str, fallback_index: usize) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return format!("Preset {}", fallback_index.saturating_add(1));
    }
    trimmed.chars().take(MAX_PRESET_NAME_CHARS).collect()
}

fn normalized_preset_name(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

fn float_near_eq(left: f32, right: f32) -> bool {
    (left - right).abs() <= 1.0e-4
}

fn curve_near_eq(left: &EditableCurve, right: &EditableCurve) -> bool {
    if left.nodes.len() != right.nodes.len() || left.segments.len() != right.segments.len() {
        return false;
    }
    left.nodes
        .iter()
        .zip(right.nodes.iter())
        .all(|(lhs, rhs)| float_near_eq(lhs.x, rhs.x) && float_near_eq(lhs.y, rhs.y))
        && left
            .segments
            .iter()
            .zip(right.segments.iter())
            .all(|(lhs, rhs)| float_near_eq(lhs.tension, rhs.tension))
}

/// Shared atomic parameter/state storage across threads.
pub struct PumpParams {
    mix: AtomicF32,
    depth: AtomicF32,
    phase_offset: AtomicF32,
    output_gain_db: AtomicF32,
    sync_division: AtomicU32,
    editable_curve: RwLock<EditableCurve>,
    curve: [AtomicF32; CURVE_TABLE_LEN],
    curve_revision: AtomicU32,
    preset_bank: RwLock<PumpPresetBank>,
}

impl PumpParams {
    /// Create params with production defaults and default curve.
    pub fn new() -> Self {
        let editable_curve = default_editable_curve();
        let default_curve = editable_curve_to_table(&editable_curve);
        Self {
            mix: AtomicF32::new(DEFAULT_MIX),
            depth: AtomicF32::new(DEFAULT_DEPTH),
            phase_offset: AtomicF32::new(DEFAULT_PHASE_OFFSET),
            output_gain_db: AtomicF32::new(DEFAULT_OUTPUT_GAIN_DB),
            sync_division: AtomicU32::new(DEFAULT_SYNC_DIVISION_INDEX as u32),
            editable_curve: RwLock::new(editable_curve),
            curve: std::array::from_fn(|index| AtomicF32::new(default_curve[index])),
            curve_revision: AtomicU32::new(1),
            preset_bank: RwLock::new(PumpPresetBank::default_init()),
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
            .map(|sample: &AtomicF32| sample.load(Ordering::Acquire).clamp(0.0, 1.0))
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

    /// Snapshot the editable spline curve.
    pub fn editable_curve_snapshot(&self) -> EditableCurve {
        self.editable_curve
            .read()
            .map(|curve| curve.clone())
            .unwrap_or_else(|_| default_editable_curve())
    }

    /// Replace the editable spline curve, regenerate the table, and advance revision.
    pub fn set_editable_curve(&self, editable_curve: &EditableCurve) {
        let normalized = editable_curve.clone().normalized();
        let curve_table = editable_curve_to_table(&normalized);
        if let Ok(mut guard) = self.editable_curve.write() {
            *guard = normalized;
        }
        self.store_curve_table(&curve_table);
        self.curve_revision.fetch_add(1, Ordering::AcqRel);
    }

    /// Replace the whole curve table and advance revision.
    pub fn set_curve(&self, values: &[f32; CURVE_TABLE_LEN]) {
        if let Ok(mut guard) = self.editable_curve.write() {
            *guard = curve_table_to_editable(values);
        }
        self.store_curve_table(values);
        self.curve_revision.fetch_add(1, Ordering::AcqRel);
    }

    /// Restore default curve shape and advance revision.
    pub fn reset_curve_to_default(&self) {
        self.set_editable_curve(&default_editable_curve());
    }

    fn current_preset_snapshot_with_name(&self, name: String) -> PumpPreset {
        PumpPreset {
            name,
            is_read_only: false,
            mix: self.mix(),
            depth: self.depth(),
            phase_offset: self.phase_offset(),
            output_gain_db: self.output_gain_db(),
            sync_division: self.sync_division(),
            editable_curve: self.editable_curve_snapshot(),
        }
    }

    fn apply_preset_snapshot(&self, preset: &PumpPreset) {
        self.set_mix(preset.mix);
        self.set_depth(preset.depth);
        self.set_phase_offset(preset.phase_offset);
        self.set_output_gain_db(preset.output_gain_db);
        self.set_sync_division(preset.sync_division as f32);
        self.set_editable_curve(&preset.editable_curve);
    }

    /// Snapshot the stored preset bank.
    pub fn preset_bank_snapshot(&self) -> PumpPresetBank {
        self.preset_bank
            .read()
            .map(|bank| bank.clone())
            .unwrap_or_else(|_| PumpPresetBank::default_init())
    }

    /// Replace the full preset bank, clamping to supported limits.
    pub fn set_preset_bank(&self, bank: PumpPresetBank) {
        let mut normalized = bank;
        if normalized.presets.is_empty() {
            normalized = PumpPresetBank::default_init();
        }
        if normalized.presets.len() > MAX_PRESETS {
            normalized.presets.truncate(MAX_PRESETS);
        }
        let init_name = normalized_preset_name(DEFAULT_PRESET_NAME);
        let mut first_read_only: Option<usize> = None;
        for (index, preset) in normalized.presets.iter_mut().enumerate() {
            preset.name = sanitize_preset_name(&preset.name, index);
            if preset.is_read_only {
                if first_read_only.is_some() {
                    preset.is_read_only = false;
                } else {
                    first_read_only = Some(index);
                }
            }
            preset.sync_division = preset.sync_division.min(MAX_SYNC_DIVISION as usize);
            preset.editable_curve = preset.editable_curve.clone().normalized();
            if !preset.is_read_only && normalized_preset_name(&preset.name) == init_name {
                preset.name = sanitize_preset_name("", index);
            }
        }
        if first_read_only.is_none() {
            // Preserve user-authored presets exactly as loaded and prepend a
            // canonical read-only Init entry instead of overwriting index 0.
            normalized.presets.insert(0, default_init_preset());
            normalized.selected = normalized.selected.saturating_add(1);
            if normalized.presets.len() > MAX_PRESETS {
                normalized.presets.truncate(MAX_PRESETS);
            }
        } else if let Some(read_only_index) = first_read_only {
            if read_only_index != 0 {
                let init_preset = normalized.presets.remove(read_only_index);
                normalized.presets.insert(0, init_preset);
                normalized.selected = if normalized.selected == read_only_index {
                    0
                } else if normalized.selected < read_only_index {
                    normalized.selected + 1
                } else {
                    normalized.selected
                };
            }
            normalized.presets[0].name = DEFAULT_PRESET_NAME.to_string();
            normalized.presets[0].is_read_only = true;
        }
        normalized.selected = normalized
            .selected
            .min(normalized.presets.len().saturating_sub(1));
        if let Ok(mut guard) = self.preset_bank.write() {
            *guard = normalized;
        }
    }

    /// Return true when the preset at `index` is read-only.
    pub fn is_preset_read_only(&self, index: usize) -> bool {
        self.preset_bank
            .read()
            .ok()
            .and_then(|bank| bank.presets.get(index).map(|preset| preset.is_read_only))
            .unwrap_or(false)
    }

    /// Load one preset by index into the active parameter state.
    pub fn load_preset(&self, index: usize) -> Option<usize> {
        let preset = self
            .preset_bank
            .read()
            .ok()
            .and_then(|bank| bank.presets.get(index).cloned())?;
        self.apply_preset_snapshot(&preset);
        if let Ok(mut guard) = self.preset_bank.write() {
            guard.selected = index.min(guard.presets.len().saturating_sub(1));
            return Some(guard.selected);
        }
        Some(index)
    }

    /// Insert a new preset cloned from current state and select it.
    pub fn add_preset_from_current_state(&self) -> Option<usize> {
        let snapshot = self.current_preset_snapshot_with_name(String::new());
        let Ok(mut guard) = self.preset_bank.write() else {
            return None;
        };
        if guard.presets.len() >= MAX_PRESETS {
            return None;
        }
        let insert_at = guard.selected.saturating_add(1).min(guard.presets.len());
        let fallback_index = guard.presets.len();
        let mut inserted = snapshot;
        inserted.name = sanitize_preset_name("", fallback_index);
        inserted.is_read_only = false;
        guard.presets.insert(insert_at, inserted);
        guard.selected = insert_at;
        Some(insert_at)
    }

    /// Rename one preset entry.
    pub fn rename_preset(&self, index: usize, new_name: &str) -> bool {
        let Ok(mut guard) = self.preset_bank.write() else {
            return false;
        };
        let Some(preset) = guard.presets.get_mut(index) else {
            return false;
        };
        if preset.is_read_only {
            return false;
        }
        let candidate = sanitize_preset_name(new_name, index);
        if normalized_preset_name(&candidate) == normalized_preset_name(DEFAULT_PRESET_NAME) {
            return false;
        }
        preset.name = candidate;
        true
    }

    /// Save current state by preset name using overwrite-or-create semantics.
    pub fn save_current_state_by_name(&self, name: &str) -> SavePresetOutcome {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return SavePresetOutcome::InvalidName;
        }
        let candidate_name: String = trimmed.chars().take(MAX_PRESET_NAME_CHARS).collect();
        if candidate_name.is_empty() {
            return SavePresetOutcome::InvalidName;
        }
        let normalized_candidate = normalized_preset_name(&candidate_name);
        let init_name = normalized_preset_name(DEFAULT_PRESET_NAME);

        let snapshot = self.current_preset_snapshot_with_name(candidate_name.clone());
        let Ok(mut guard) = self.preset_bank.write() else {
            return SavePresetOutcome::InvalidName;
        };

        let matching_index = guard
            .presets
            .iter()
            .position(|preset| normalized_preset_name(&preset.name) == normalized_candidate);
        if let Some(index) = matching_index {
            if guard
                .presets
                .get(index)
                .map(|preset| preset.is_read_only)
                .unwrap_or(false)
            {
                return SavePresetOutcome::BlockedReadOnly;
            }
            if let Some(existing) = guard.presets.get_mut(index) {
                existing.mix = snapshot.mix;
                existing.depth = snapshot.depth;
                existing.phase_offset = snapshot.phase_offset;
                existing.output_gain_db = snapshot.output_gain_db;
                existing.sync_division = snapshot.sync_division;
                existing.editable_curve = snapshot.editable_curve;
            }
            guard.selected = index;
            return SavePresetOutcome::Overwritten { index };
        }

        if normalized_candidate == init_name {
            return SavePresetOutcome::BlockedReadOnly;
        }
        if guard.presets.len() >= MAX_PRESETS {
            return SavePresetOutcome::BlockedFull;
        }
        let created_index = guard.presets.len();
        guard.presets.push(snapshot);
        guard.selected = created_index;
        SavePresetOutcome::Created {
            index: created_index,
        }
    }

    /// Return true when current parameters/curve differ from selected preset.
    pub fn current_state_differs_from_selected_preset(&self) -> bool {
        let bank = self.preset_bank_snapshot();
        let Some(selected) = bank.presets.get(bank.selected) else {
            return false;
        };
        let current = self.current_preset_snapshot_with_name(String::new());
        !float_near_eq(current.mix, selected.mix)
            || !float_near_eq(current.depth, selected.depth)
            || !float_near_eq(current.phase_offset, selected.phase_offset)
            || !float_near_eq(current.output_gain_db, selected.output_gain_db)
            || current.sync_division != selected.sync_division
            || !curve_near_eq(&current.editable_curve, &selected.editable_curve)
    }

    fn store_curve_table(&self, values: &[f32; CURVE_TABLE_LEN]) {
        for (index, sample) in values.iter().copied().enumerate() {
            self.curve[index].store(sample.clamp(0.0, 1.0), Ordering::Release);
        }
    }
}

impl Default for PumpParams {
    fn default() -> Self {
        Self::new()
    }
}

/// Encode all parameter state including curve table into bytes.
pub fn encode_state_payload(params: &PumpParams) -> Vec<u8> {
    let editable = params.editable_curve_snapshot();
    let node_count = editable.nodes.len().min(MAX_EDITABLE_NODES);
    let segment_count = node_count.saturating_sub(1);
    let bank = params.preset_bank_snapshot();
    let mut payload = Vec::with_capacity(64 + node_count * 8 + segment_count * 4);

    payload.extend_from_slice(STATE_MAGIC);
    payload.extend_from_slice(&STATE_VERSION.to_le_bytes());
    payload.extend_from_slice(&params.mix().to_le_bytes());
    payload.extend_from_slice(&params.depth().to_le_bytes());
    payload.extend_from_slice(&params.phase_offset().to_le_bytes());
    payload.extend_from_slice(&params.output_gain_db().to_le_bytes());
    payload.extend_from_slice(&(params.sync_division() as f32).to_le_bytes());
    payload.extend_from_slice(&(node_count as u32).to_le_bytes());

    for node in editable.nodes.iter().take(node_count) {
        payload.extend_from_slice(&node.x.to_le_bytes());
        payload.extend_from_slice(&node.y.to_le_bytes());
    }
    for segment in editable.segments.iter().take(segment_count) {
        payload.extend_from_slice(&segment.tension.to_le_bytes());
    }
    payload.extend_from_slice(&(bank.selected as u32).to_le_bytes());
    payload.extend_from_slice(&(bank.presets.len() as u32).to_le_bytes());
    for (index, preset) in bank.presets.iter().enumerate() {
        encode_preset(&mut payload, preset, index);
    }

    payload
}

/// Decode parameter state payload and apply it to shared params.
pub fn decode_state_payload(params: &PumpParams, payload: &[u8]) -> Result<(), &'static str> {
    if payload.len() == legacy_payload_len() {
        return decode_legacy_state_payload(params, payload);
    }

    let mut cursor = Cursor::new(payload);
    let Some(magic) = read_u32(&mut cursor) else {
        return Err("invalid state header");
    };
    if magic != u32::from_le_bytes(*STATE_MAGIC) {
        return Err("unknown state payload format");
    }
    let Some(version) = read_u32(&mut cursor) else {
        return Err("invalid state version");
    };
    if !(2..=STATE_VERSION).contains(&version) {
        return Err("unsupported state payload version");
    }

    let Some(mix) = read_f32(&mut cursor) else {
        return Err("invalid mix field");
    };
    let Some(depth) = read_f32(&mut cursor) else {
        return Err("invalid depth field");
    };
    let Some(phase_offset) = read_f32(&mut cursor) else {
        return Err("invalid phase offset field");
    };
    let Some(output_gain_db) = read_f32(&mut cursor) else {
        return Err("invalid output gain field");
    };
    let Some(sync_division) = read_f32(&mut cursor) else {
        return Err("invalid sync division field");
    };
    let Some(node_count) = read_u32(&mut cursor).map(|count| count as usize) else {
        return Err("invalid node count");
    };
    let editable_curve = decode_curve(&mut cursor, node_count)?;

    let preset_bank = if version >= 3 {
        decode_preset_bank(&mut cursor, version)?
    } else {
        PumpPresetBank {
            selected: 0,
            presets: vec![PumpPreset {
                name: DEFAULT_PRESET_NAME.to_string(),
                is_read_only: true,
                mix,
                depth,
                phase_offset,
                output_gain_db,
                sync_division: clamp_sync_division(sync_division),
                editable_curve: editable_curve.clone(),
            }],
        }
    };

    if cursor.position() != payload.len() as u64 {
        return Err("unexpected trailing state bytes");
    }

    params.set_mix(mix);
    params.set_depth(depth);
    params.set_phase_offset(phase_offset);
    params.set_output_gain_db(output_gain_db);
    params.set_sync_division(sync_division);
    params.set_editable_curve(&editable_curve);
    params.set_preset_bank(preset_bank);

    Ok(())
}

fn encode_preset(payload: &mut Vec<u8>, preset: &PumpPreset, index: usize) {
    let name = sanitize_preset_name(&preset.name, index);
    payload.extend_from_slice(&(name.len() as u32).to_le_bytes());
    payload.extend_from_slice(name.as_bytes());
    payload.extend_from_slice(&preset.mix.to_le_bytes());
    payload.extend_from_slice(&preset.depth.to_le_bytes());
    payload.extend_from_slice(&preset.phase_offset.to_le_bytes());
    payload.extend_from_slice(&preset.output_gain_db.to_le_bytes());
    payload.extend_from_slice(&(preset.sync_division as u32).to_le_bytes());
    payload.push(u8::from(preset.is_read_only));

    let normalized_curve = preset.editable_curve.clone().normalized();
    let node_count = normalized_curve.nodes.len().min(MAX_EDITABLE_NODES);
    payload.extend_from_slice(&(node_count as u32).to_le_bytes());
    for node in normalized_curve.nodes.iter().take(node_count) {
        payload.extend_from_slice(&node.x.to_le_bytes());
        payload.extend_from_slice(&node.y.to_le_bytes());
    }
    for segment in normalized_curve
        .segments
        .iter()
        .take(node_count.saturating_sub(1))
    {
        payload.extend_from_slice(&segment.tension.to_le_bytes());
    }
}

fn decode_curve(
    cursor: &mut Cursor<&[u8]>,
    node_count: usize,
) -> Result<EditableCurve, &'static str> {
    if !(2..=MAX_EDITABLE_NODES).contains(&node_count) {
        return Err("invalid node count bounds");
    }
    let mut nodes = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let Some(x) = read_f32(cursor) else {
            return Err("invalid curve node x");
        };
        let Some(y) = read_f32(cursor) else {
            return Err("invalid curve node y");
        };
        nodes.push(CurveNode { x, y });
    }
    let mut segments = Vec::with_capacity(node_count.saturating_sub(1));
    for _ in 0..node_count.saturating_sub(1) {
        let Some(tension) = read_f32(cursor) else {
            return Err("invalid curve segment");
        };
        segments.push(CurveSegment { tension });
    }
    Ok(EditableCurve { nodes, segments })
}

fn decode_preset_bank(
    cursor: &mut Cursor<&[u8]>,
    version: u32,
) -> Result<PumpPresetBank, &'static str> {
    let Some(selected) = read_u32(cursor).map(|value| value as usize) else {
        return Err("invalid preset selected index");
    };
    let Some(count) = read_u32(cursor).map(|value| value as usize) else {
        return Err("invalid preset count");
    };
    if count == 0 || count > MAX_PRESETS {
        return Err("invalid preset count bounds");
    }
    let mut presets = Vec::with_capacity(count);
    for index in 0..count {
        let Some(name_len) = read_u32(cursor).map(|value| value as usize) else {
            return Err("invalid preset name length");
        };
        if name_len == 0 || name_len > 256 {
            return Err("invalid preset name length bounds");
        }
        let mut name_bytes = vec![0_u8; name_len];
        std::io::Read::read_exact(cursor, &mut name_bytes).map_err(|_| "invalid preset name")?;
        let raw_name = std::str::from_utf8(&name_bytes).map_err(|_| "invalid preset name utf8")?;
        let Some(mix) = read_f32(cursor) else {
            return Err("invalid preset mix");
        };
        let Some(depth) = read_f32(cursor) else {
            return Err("invalid preset depth");
        };
        let Some(phase_offset) = read_f32(cursor) else {
            return Err("invalid preset phase offset");
        };
        let Some(output_gain_db) = read_f32(cursor) else {
            return Err("invalid preset output gain");
        };
        let Some(sync_division) = read_u32(cursor).map(|value| value as usize) else {
            return Err("invalid preset sync division");
        };
        let is_read_only = if version >= 4 {
            let Some(flag) = read_u8(cursor) else {
                return Err("invalid preset read-only flag");
            };
            flag != 0
        } else {
            index == 0
                && normalized_preset_name(raw_name) == normalized_preset_name(DEFAULT_PRESET_NAME)
        };
        let Some(node_count) = read_u32(cursor).map(|value| value as usize) else {
            return Err("invalid preset node count");
        };
        let editable_curve = decode_curve(cursor, node_count)?;
        presets.push(PumpPreset {
            name: sanitize_preset_name(raw_name, index),
            is_read_only,
            mix,
            depth,
            phase_offset,
            output_gain_db,
            sync_division: sync_division.min(MAX_SYNC_DIVISION as usize),
            editable_curve: editable_curve.normalized(),
        });
    }
    Ok(PumpPresetBank {
        selected: selected.min(count.saturating_sub(1)),
        presets,
    })
}

fn legacy_payload_len() -> usize {
    4 * (5 + CURVE_TABLE_LEN)
}

fn decode_legacy_state_payload(params: &PumpParams, payload: &[u8]) -> Result<(), &'static str> {
    if payload.len() != legacy_payload_len() {
        return Err("invalid pump state payload length");
    }

    let mut cursor = Cursor::new(payload);
    let Some(mix) = read_f32(&mut cursor) else {
        return Err("invalid mix field");
    };
    let Some(depth) = read_f32(&mut cursor) else {
        return Err("invalid depth field");
    };
    let Some(phase_offset) = read_f32(&mut cursor) else {
        return Err("invalid phase offset field");
    };
    let Some(output_gain_db) = read_f32(&mut cursor) else {
        return Err("invalid output gain field");
    };
    let Some(sync_division) = read_f32(&mut cursor) else {
        return Err("invalid sync division field");
    };

    let mut curve = [1.0_f32; CURVE_TABLE_LEN];
    for sample in &mut curve {
        let Some(value) = read_f32(&mut cursor) else {
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
    params.set_preset_bank(PumpPresetBank {
        selected: 0,
        presets: vec![PumpPreset {
            name: DEFAULT_PRESET_NAME.to_string(),
            is_read_only: true,
            mix: params.mix(),
            depth: params.depth(),
            phase_offset: params.phase_offset(),
            output_gain_db: params.output_gain_db(),
            sync_division: params.sync_division(),
            editable_curve: params.editable_curve_snapshot(),
        }],
    });

    Ok(())
}

fn read_f32(cursor: &mut Cursor<&[u8]>) -> Option<f32> {
    let mut bytes = [0_u8; 4];
    std::io::Read::read_exact(cursor, &mut bytes).ok()?;
    Some(f32::from_le_bytes(bytes))
}

fn read_u32(cursor: &mut Cursor<&[u8]>) -> Option<u32> {
    let mut bytes = [0_u8; 4];
    std::io::Read::read_exact(cursor, &mut bytes).ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn read_u8(cursor: &mut Cursor<&[u8]>) -> Option<u8> {
    let mut bytes = [0_u8; 1];
    std::io::Read::read_exact(cursor, &mut bytes).ok()?;
    Some(bytes[0])
}

#[cfg(test)]
mod tests {
    use super::{
        clamp_sync_division, decode_state_payload, encode_state_payload,
        sync_division_index_from_text, PumpParams, PumpPreset, PumpPresetBank, SavePresetOutcome,
        MAX_PRESET_NAME_CHARS, MAX_SYNC_DIVISION,
    };
    use crate::curve::{CurveNode, CurveSegment, EditableCurve, CURVE_TABLE_LEN};

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
        params.set_editable_curve(&EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.2, y: 0.1 },
                CurveNode { x: 1.0, y: 0.9 },
            ],
            segments: vec![
                CurveSegment { tension: -0.5 },
                CurveSegment { tension: 0.4 },
            ],
        });

        let payload = encode_state_payload(&params);

        let restored = PumpParams::new();
        decode_state_payload(&restored, &payload).expect("state should decode");

        assert!((restored.mix() - 0.23).abs() < 1.0e-6);
        assert!((restored.depth() - 0.81).abs() < 1.0e-6);
        assert!((restored.phase_offset() - 0.42).abs() < 1.0e-6);
        assert!((restored.output_gain_db() + 3.0).abs() < 1.0e-6);
        assert_eq!(restored.sync_division(), 6);
        let editable = restored.editable_curve_snapshot();
        assert_eq!(editable.nodes.len(), 3);
        assert_eq!(editable.segments.len(), 2);
    }

    #[test]
    fn legacy_payload_still_decodes() {
        let mut legacy = Vec::with_capacity(4 * (5 + CURVE_TABLE_LEN));
        legacy.extend_from_slice(&0.4_f32.to_le_bytes());
        legacy.extend_from_slice(&0.6_f32.to_le_bytes());
        legacy.extend_from_slice(&0.2_f32.to_le_bytes());
        legacy.extend_from_slice(&(-1.5_f32).to_le_bytes());
        legacy.extend_from_slice(&4.0_f32.to_le_bytes());
        for index in 0..CURVE_TABLE_LEN {
            let phase = index as f32 / (CURVE_TABLE_LEN - 1) as f32;
            legacy.extend_from_slice(&phase.to_le_bytes());
        }

        let restored = PumpParams::new();
        decode_state_payload(&restored, &legacy).expect("legacy state should decode");
        assert!((restored.mix() - 0.4).abs() < 1.0e-6);
        assert!((restored.depth() - 0.6).abs() < 1.0e-6);
        assert_eq!(restored.sync_division(), 4);
        let editable = restored.editable_curve_snapshot();
        assert!(editable.nodes.len() >= 2);
        assert_eq!(editable.segments.len(), editable.nodes.len() - 1);
    }

    #[test]
    fn default_curve_is_simple_after_reset() {
        let params = PumpParams::new();
        let curve = params.editable_curve_snapshot();
        assert_eq!(curve.nodes.len(), 4);
        assert_eq!(curve.segments.len(), 3);

        params.set_editable_curve(&EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 1.0 }, CurveNode { x: 1.0, y: 0.0 }],
            segments: vec![CurveSegment { tension: 0.0 }],
        });
        params.reset_curve_to_default();
        let reset_curve = params.editable_curve_snapshot();
        assert_eq!(reset_curve.nodes.len(), 4);
        assert_eq!(reset_curve.segments.len(), 3);
    }

    #[test]
    fn sync_division_change_does_not_mutate_curve_or_revision() {
        let params = PumpParams::new();
        let custom_curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.3, y: 0.2 },
                CurveNode { x: 1.0, y: 0.8 },
            ],
            segments: vec![
                CurveSegment { tension: -0.3 },
                CurveSegment { tension: 0.2 },
            ],
        }
        .normalized();
        params.set_editable_curve(&custom_curve);
        let curve_before = params.editable_curve_snapshot();
        let revision_before = params.curve_revision();

        params.set_sync_division(2.0);

        assert_eq!(params.sync_division(), 2);
        assert_eq!(
            params.editable_curve_snapshot(),
            curve_before,
            "sync division changes must preserve editable curve"
        );
        assert_eq!(
            params.curve_revision(),
            revision_before,
            "sync division changes must not bump curve revision"
        );
    }

    #[test]
    fn preset_add_rename_and_load_roundtrip_current_state() {
        let params = PumpParams::new();
        params.set_mix(0.31);
        params.set_depth(0.91);
        params.set_phase_offset(0.27);
        params.set_output_gain_db(-6.0);
        params.set_sync_division(6.0);
        params.set_editable_curve(&EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.9 },
                CurveNode { x: 0.4, y: 0.2 },
                CurveNode { x: 1.0, y: 0.9 },
            ],
            segments: vec![
                CurveSegment { tension: -0.2 },
                CurveSegment { tension: 0.3 },
            ],
        });

        let inserted = params
            .add_preset_from_current_state()
            .expect("preset insertion should succeed");
        assert_eq!(inserted, 1);
        assert!(params.rename_preset(inserted, "My Wide Preset Name That Should Clamp"));

        let bank = params.preset_bank_snapshot();
        assert_eq!(bank.selected, 1);
        assert_eq!(bank.presets.len(), 2);
        assert!(bank.presets[1].name.chars().count() <= MAX_PRESET_NAME_CHARS);

        params.set_mix(0.05);
        params.set_depth(0.1);
        params.set_sync_division(1.0);
        params.load_preset(1).expect("preset load should succeed");
        assert!((params.mix() - bank.presets[1].mix).abs() < 1.0e-6);
        assert!((params.depth() - bank.presets[1].depth).abs() < 1.0e-6);
        assert_eq!(params.sync_division(), bank.presets[1].sync_division);
    }

    #[test]
    fn preset_bank_roundtrips_through_state_payload() {
        let params = PumpParams::new();
        params
            .add_preset_from_current_state()
            .expect("preset insertion should succeed");
        assert!(params.rename_preset(1, "Verse"));
        params.load_preset(1).expect("preset load should succeed");
        let payload = encode_state_payload(&params);

        let restored = PumpParams::new();
        decode_state_payload(&restored, &payload).expect("state should decode");
        let bank = restored.preset_bank_snapshot();
        assert_eq!(bank.presets.len(), 2);
        assert_eq!(bank.selected, 1);
        assert_eq!(bank.presets[1].name, "Verse");
        assert!(bank.presets[0].is_read_only);
    }

    #[test]
    fn set_preset_bank_inserts_init_without_overwriting_user_presets() {
        let params = PumpParams::new();
        params.set_preset_bank(PumpPresetBank {
            selected: 1,
            presets: vec![
                PumpPreset {
                    name: "Live A".to_string(),
                    is_read_only: false,
                    mix: 0.11,
                    depth: 0.22,
                    phase_offset: 0.33,
                    output_gain_db: -1.0,
                    sync_division: 2,
                    editable_curve: params.editable_curve_snapshot(),
                },
                PumpPreset {
                    name: "Live B".to_string(),
                    is_read_only: false,
                    mix: 0.77,
                    depth: 0.66,
                    phase_offset: 0.55,
                    output_gain_db: -2.0,
                    sync_division: 4,
                    editable_curve: params.editable_curve_snapshot(),
                },
            ],
        });

        let bank = params.preset_bank_snapshot();
        assert_eq!(bank.presets.len(), 3);
        assert_eq!(
            bank.selected, 2,
            "selected index should shift with Init prepend"
        );
        assert_eq!(bank.presets[0].name, "Init");
        assert!(bank.presets[0].is_read_only);
        assert_eq!(bank.presets[1].name, "Live A");
        assert!((bank.presets[1].mix - 0.11).abs() < 1.0e-6);
        assert_eq!(bank.presets[2].name, "Live B");
        assert!((bank.presets[2].mix - 0.77).abs() < 1.0e-6);
    }

    #[test]
    fn save_by_name_overwrites_case_insensitive_match() {
        let params = PumpParams::new();
        params
            .add_preset_from_current_state()
            .expect("preset insertion should succeed");
        assert!(params.rename_preset(1, "Verse"));
        params.set_mix(0.12);
        params.set_depth(0.33);

        let outcome = params.save_current_state_by_name(" verse ");
        assert_eq!(outcome, SavePresetOutcome::Overwritten { index: 1 });
        let bank = params.preset_bank_snapshot();
        assert_eq!(bank.selected, 1);
        assert_eq!(bank.presets[1].name, "Verse");
        assert!((bank.presets[1].mix - 0.12).abs() < 1.0e-6);
        assert!((bank.presets[1].depth - 0.33).abs() < 1.0e-6);
    }

    #[test]
    fn save_by_name_creates_when_no_match_exists() {
        let params = PumpParams::new();
        params.set_mix(0.77);
        let outcome = params.save_current_state_by_name("Hook");
        assert_eq!(outcome, SavePresetOutcome::Created { index: 1 });
        let bank = params.preset_bank_snapshot();
        assert_eq!(bank.presets.len(), 2);
        assert_eq!(bank.selected, 1);
        assert_eq!(bank.presets[1].name, "Hook");
        assert!((bank.presets[1].mix - 0.77).abs() < 1.0e-6);
    }

    #[test]
    fn init_preset_is_read_only_for_rename_and_save() {
        let params = PumpParams::new();
        assert!(params.is_preset_read_only(0));
        assert!(!params.rename_preset(0, "Init2"));
        assert_eq!(
            params.save_current_state_by_name("Init"),
            SavePresetOutcome::BlockedReadOnly
        );
        let bank = params.preset_bank_snapshot();
        assert_eq!(bank.presets[0].name, "Init");
    }

    #[test]
    fn save_by_name_blocks_when_preset_bank_is_full() {
        let params = PumpParams::new();
        for _ in 1..16 {
            params
                .add_preset_from_current_state()
                .expect("preset insertion should succeed");
        }
        assert_eq!(params.preset_bank_snapshot().presets.len(), 16);
        assert_eq!(
            params.save_current_state_by_name("BrandNew"),
            SavePresetOutcome::BlockedFull
        );
    }
}
