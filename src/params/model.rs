//! Core parameter model and shared state types for Pump.
//!
//! This module holds the stable data model (parameter ids/ranges, sync
//! divisions, presets, and the shared `PumpParams` storage) while host wiring
//! and state codec logic live in sibling modules.

use super::*;

pub(crate) const STATE_MAGIC: &[u8; 4] = b"PMP2";
pub(crate) const STATE_VERSION: u32 = 5;

/// Host-visible numeric parameter id for dry/wet blend.
pub const PARAM_MIX_NUM: u32 = 1;
/// Reserved numeric parameter id kept for backward compatibility.
///
/// Depth is no longer exposed as a runtime control.
#[allow(dead_code)]
pub const PARAM_DEPTH_NUM: u32 = 2;
/// Host-visible numeric parameter id for cycle phase offset.
pub const PARAM_PHASE_OFFSET_NUM: u32 = 3;
/// Host-visible numeric parameter id for output trim in decibels.
pub const PARAM_OUTPUT_GAIN_NUM: u32 = 4;
/// Host-visible numeric parameter id for beat-sync cycle division.
pub const PARAM_SYNC_DIVISION_NUM: u32 = 5;

/// Parameter id for dry/wet blend.
pub const PARAM_MIX_ID: ClapId = ClapId::new(PARAM_MIX_NUM);
/// Reserved parameter id kept for backward compatibility.
///
/// Depth is no longer exposed as a runtime control.
#[allow(dead_code)]
pub const PARAM_DEPTH_ID: ClapId = ClapId::new(PARAM_DEPTH_NUM);
/// Parameter id for cycle phase offset.
pub const PARAM_PHASE_OFFSET_ID: ClapId = ClapId::new(PARAM_PHASE_OFFSET_NUM);
/// Parameter id for output trim in decibels.
pub const PARAM_OUTPUT_GAIN_ID: ClapId = ClapId::new(PARAM_OUTPUT_GAIN_NUM);
/// Parameter id for beat-sync cycle division.
pub const PARAM_SYNC_DIVISION_ID: ClapId = ClapId::new(PARAM_SYNC_DIVISION_NUM);

/// Default dry/wet blend.
pub const DEFAULT_MIX: f32 = 1.0;
/// Fixed depth value used for compatibility payloads/presets.
pub const DEFAULT_DEPTH: f32 = 1.0;
/// Default cycle phase offset.
pub const DEFAULT_PHASE_OFFSET: f32 = 0.0;
/// Default output gain.
pub const DEFAULT_OUTPUT_GAIN_DB: f32 = 0.0;
/// Default sync division index (`1/4`).
pub const DEFAULT_SYNC_DIVISION_INDEX: usize = 4;
/// Maximum number of stored user presets.
pub const MAX_PRESETS: usize = 16;
/// Fixed number of overwriteable quick slots stored inside each preset.
pub const QUICK_SLOT_COUNT: usize = 8;
/// Fixed number of globally persisted curve slots.
pub const GLOBAL_CURVE_SLOT_COUNT: usize = QUICK_SLOT_COUNT;
/// Maximum preset-name length in characters.
pub const MAX_PRESET_NAME_CHARS: usize = 24;
/// Default preset name for the initialized plugin state.
pub const DEFAULT_PRESET_NAME: &str = "Init";

/// Minimum mix value.
pub const MIN_MIX: f32 = 0.0;
/// Maximum mix value.
pub const MAX_MIX: f32 = 1.0;
/// Minimum depth value kept for compatibility helpers.
#[allow(dead_code)]
pub const MIN_DEPTH: f32 = 0.0;
/// Maximum depth value kept for compatibility helpers.
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

/// Serializable snapshot of one overwriteable quick slot.
#[derive(Clone, Debug, PartialEq)]
pub struct QuickShapeSlot {
    /// Stored quick-slot curve shown in the per-preset micro-preview tile.
    pub curve: EditableCurve,
}

/// Serializable snapshot of one globally persisted curve slot.
#[derive(Clone, Debug, PartialEq)]
pub struct GlobalCurveSlot {
    /// Stored reusable curve, or `None` when the slot is empty.
    pub curve: Option<EditableCurve>,
}

/// Serializable snapshot of one Pump preset.
#[derive(Clone, Debug, PartialEq)]
pub struct PumpPreset {
    /// User-facing preset name.
    pub name: String,
    /// Reserved for state-payload compatibility with older releases.
    ///
    /// Pump now treats all presets, including `Init`, as writable.
    pub is_read_only: bool,
    /// Dry/wet mix amount.
    pub mix: f32,
    /// Legacy depth field preserved for backward-compatible state payloads.
    pub depth: f32,
    /// Cycle phase offset.
    pub phase_offset: f32,
    /// Output trim in decibels.
    pub output_gain_db: f32,
    /// Sync-division index.
    pub sync_division: usize,
    /// Editable curve shape.
    pub editable_curve: EditableCurve,
    /// Overwriteable quick-slot curves shown below the editor for this preset.
    pub quick_slots: Vec<QuickShapeSlot>,
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
    /// Existing preset was overwritten.
    Overwritten {
        /// Index of the overwritten preset.
        index: usize,
    },
    /// New preset was created.
    Created {
        /// Index of the created preset.
        index: usize,
    },
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
                is_read_only: false,
                mix: DEFAULT_MIX,
                depth: DEFAULT_DEPTH,
                phase_offset: DEFAULT_PHASE_OFFSET,
                output_gain_db: DEFAULT_OUTPUT_GAIN_DB,
                sync_division: DEFAULT_SYNC_DIVISION_INDEX,
                editable_curve: default_editable_curve(),
                quick_slots: seeded_quick_shape_slots(),
            }],
        }
    }
}

/// Return the canonical seeded quick-slot contents for one preset.
pub(crate) fn seeded_quick_shape_slots() -> Vec<QuickShapeSlot> {
    crate::curve_presets::quick_slot_seeds()
        .iter()
        .map(|seed| QuickShapeSlot {
            curve: seed.curve.clone(),
        })
        .collect()
}

pub(crate) fn sanitize_preset_name(raw: &str, fallback_index: usize) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return format!("Preset {}", fallback_index.saturating_add(1));
    }
    trimmed.chars().take(MAX_PRESET_NAME_CHARS).collect()
}

pub(crate) fn normalized_preset_name(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

pub(crate) fn float_near_eq(left: f32, right: f32) -> bool {
    (left - right).abs() <= 1.0e-4
}

pub(crate) fn curve_near_eq(left: &EditableCurve, right: &EditableCurve) -> bool {
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
    pub(super) mix: AtomicF32,
    pub(super) phase_offset: AtomicF32,
    pub(super) output_gain_db: AtomicF32,
    pub(super) sync_division: AtomicU32,
    pub(super) editable_curve: RwLock<EditableCurve>,
    pub(super) curve: [AtomicF32; CURVE_TABLE_LEN],
    pub(super) curve_revision: AtomicU32,
    pub(super) preset_bank: RwLock<PumpPresetBank>,
}
