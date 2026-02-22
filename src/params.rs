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

const STATE_MAGIC: &[u8; 4] = b"PMP2";
const STATE_VERSION: u32 = 4;

/// Host-visible numeric parameter id for dry/wet blend.
pub const PARAM_MIX_NUM: u32 = 1;
/// Host-visible numeric parameter id for duck amount.
pub const PARAM_DEPTH_NUM: u32 = 2;
/// Host-visible numeric parameter id for cycle phase offset.
pub const PARAM_PHASE_OFFSET_NUM: u32 = 3;
/// Host-visible numeric parameter id for output trim in decibels.
pub const PARAM_OUTPUT_GAIN_NUM: u32 = 4;
/// Host-visible numeric parameter id for beat-sync cycle division.
pub const PARAM_SYNC_DIVISION_NUM: u32 = 5;

/// Parameter id for dry/wet blend.
pub const PARAM_MIX_ID: ClapId = ClapId::new(PARAM_MIX_NUM);
/// Parameter id for duck amount.
pub const PARAM_DEPTH_ID: ClapId = ClapId::new(PARAM_DEPTH_NUM);
/// Parameter id for cycle phase offset.
pub const PARAM_PHASE_OFFSET_ID: ClapId = ClapId::new(PARAM_PHASE_OFFSET_NUM);
/// Parameter id for output trim in decibels.
pub const PARAM_OUTPUT_GAIN_ID: ClapId = ClapId::new(PARAM_OUTPUT_GAIN_NUM);
/// Parameter id for beat-sync cycle division.
pub const PARAM_SYNC_DIVISION_ID: ClapId = ClapId::new(PARAM_SYNC_DIVISION_NUM);

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
    /// Reserved for state-payload compatibility with older releases.
    ///
    /// Pump now treats all presets, including `Init`, as writable.
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
            }],
        }
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

mod host_api;
mod runtime_impl;
mod state_codec;

#[cfg(feature = "vst3")]
pub use host_api::{
    apply_normalized_param_value, clap_id_from_vst3_param_id, format_plain_value_text,
    normalized_from_plain_value, parse_plain_value_text, plain_from_normalized_value,
    vst3_param_info_for_index,
};
pub use host_api::{
    apply_param_event, get_param_value, param_count, text_to_value, value_to_text,
    write_param_info, MAX_SYNC_DIVISION,
};
pub use state_codec::{decode_state_payload, encode_state_payload};

#[cfg(test)]
mod state_decode_tests;
#[cfg(test)]
mod tests;
