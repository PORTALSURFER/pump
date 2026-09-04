//! Core parameter model and shared state types for Pump.
//!
//! This module holds the stable data model (parameter ids/ranges, sync
//! divisions, presets, and the shared `PumpParams` storage) while host wiring
//! and state codec logic live in sibling modules.

use super::*;

pub(crate) const STATE_MAGIC: &[u8; 4] = b"PMP2";
pub(crate) const STATE_VERSION: u32 = 17;

/// The two independently editable Pump sound sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundSide {
    /// The A sound (the legacy/default side).
    A,
    /// The B sound.
    B,
}

impl SoundSide {
    /// Return the storage index used by the A/B snapshot bank.
    pub const fn index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
        }
    }

    /// Return the other sound side.
    pub const fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }

    /// Return the stable host/UI label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }
}

/// Complete editable sound state used by one A/B side.
///
/// This intentionally mirrors the audio-affecting fields in [`PumpPreset`].
/// Quick slots are included so changing a quick shape while one side is active
/// cannot silently alter the other side's editing context.
#[derive(Clone, Debug, PartialEq)]
pub struct PumpSoundState {
    pub mix: f32,
    pub depth_db: f32,
    pub floor_db: f32,
    pub phase_offset: f32,
    pub output_gain_db: f32,
    pub sync_division: usize,
    pub trigger_mode: usize,
    pub smooth: f32,
    pub mode: usize,
    pub swing: f32,
    /// Timing source: synchronized divisions or a free-running rate.
    pub timing_mode: usize,
    /// Canonical free-running timing rate in hertz.
    pub free_rate_hz: f32,
    /// Number of quarter-note beats to hold at the cycle start in Sync mode.
    pub delay_beats: usize,
    pub editable_curve: EditableCurve,
    pub quick_slots: Vec<QuickShapeSlot>,
}

impl PumpSoundState {
    pub(crate) fn default_init() -> Self {
        Self {
            mix: DEFAULT_MIX,
            depth_db: DEFAULT_DEPTH_DB,
            floor_db: DEFAULT_FLOOR_DB,
            phase_offset: DEFAULT_PHASE_OFFSET,
            output_gain_db: DEFAULT_OUTPUT_GAIN_DB,
            sync_division: DEFAULT_SYNC_DIVISION_INDEX,
            trigger_mode: DEFAULT_TRIGGER_MODE,
            smooth: DEFAULT_SMOOTH,
            mode: PROCESSING_MODE_CLASSIC,
            swing: DEFAULT_SWING,
            timing_mode: DEFAULT_TIMING_MODE,
            free_rate_hz: DEFAULT_FREE_RATE_HZ,
            delay_beats: DEFAULT_DELAY_BEATS,
            editable_curve: default_editable_curve(),
            quick_slots: seeded_quick_shape_slots(),
        }
    }
}

/// Host-visible numeric parameter id for dry/wet blend.
pub const PARAM_MIX_NUM: u32 = 1;
/// Host-visible numeric parameter id for curve attenuation depth.
pub const PARAM_DEPTH_NUM: u32 = 2;
/// Host-visible numeric parameter id for cycle phase offset.
pub const PARAM_PHASE_OFFSET_NUM: u32 = 3;
/// Host-visible numeric parameter id for output trim in decibels.
pub const PARAM_OUTPUT_GAIN_NUM: u32 = 4;
/// Host-visible numeric parameter id for beat-sync cycle division.
pub const PARAM_SYNC_DIVISION_NUM: u32 = 5;
/// Permanent VST3-only id for the extended beat-sync cycle division.
#[cfg(feature = "vst3")]
pub const PARAM_SYNC_DIVISION_VST3_V2_NUM: u32 = 15;
/// Permanent VST3-only id for the synchronized cycle-start delay.
#[cfg(feature = "vst3")]
pub const PARAM_DELAY_VST3_NUM: u32 = 16;
/// Host-visible numeric parameter id for the minimum wet gain floor.
pub const PARAM_FLOOR_NUM: u32 = 6;
/// Host-visible numeric parameter id for the curve trigger source.
#[allow(dead_code)]
pub const PARAM_TRIGGER_MODE_NUM: u32 = 7;
/// Host-visible numeric parameter id for evaluated gain smoothing.
pub const PARAM_SMOOTH_NUM: u32 = 8;
/// Host-visible numeric parameter id for the processing mode.
#[allow(dead_code)]
pub const PARAM_MODE_NUM: u32 = 9;
/// Host-visible numeric parameter id for click-safe host bypass.
pub const PARAM_BYPASS_NUM: u32 = 10;
/// Host-visible numeric parameter id for alternating-subdivision swing.
pub const PARAM_SWING_NUM: u32 = 11;
/// Host-visible numeric parameter id for the active A/B sound side.
pub const PARAM_SOUND_NUM: u32 = 12;
/// Host-visible numeric parameter id for the timing source.
pub const PARAM_TIMING_MODE_NUM: u32 = 13;
/// Host-visible numeric parameter id for the canonical free timing rate.
pub const PARAM_FREE_RATE_NUM: u32 = 14;
/// Host-visible numeric parameter id for the synchronized cycle-start delay.
pub const PARAM_DELAY_NUM: u32 = 15;

/// Parameter id for dry/wet blend.
pub const PARAM_MIX_ID: ClapId = ClapId::new(PARAM_MIX_NUM);
/// Parameter id for curve attenuation depth.
pub const PARAM_DEPTH_ID: ClapId = ClapId::new(PARAM_DEPTH_NUM);
/// Parameter id for cycle phase offset.
pub const PARAM_PHASE_OFFSET_ID: ClapId = ClapId::new(PARAM_PHASE_OFFSET_NUM);
/// Parameter id for output trim in decibels.
pub const PARAM_OUTPUT_GAIN_ID: ClapId = ClapId::new(PARAM_OUTPUT_GAIN_NUM);
/// Parameter id for beat-sync cycle division.
pub const PARAM_SYNC_DIVISION_ID: ClapId = ClapId::new(PARAM_SYNC_DIVISION_NUM);
/// Parameter id for the minimum wet gain floor.
pub const PARAM_FLOOR_ID: ClapId = ClapId::new(PARAM_FLOOR_NUM);
/// Parameter id for the curve trigger source.
#[allow(dead_code)]
pub const PARAM_TRIGGER_MODE_ID: ClapId = ClapId::new(PARAM_TRIGGER_MODE_NUM);
/// Parameter id for evaluated gain smoothing.
pub const PARAM_SMOOTH_ID: ClapId = ClapId::new(PARAM_SMOOTH_NUM);
/// Parameter id for the processing mode.
#[allow(dead_code)]
pub const PARAM_MODE_ID: ClapId = ClapId::new(PARAM_MODE_NUM);
/// Parameter id for click-safe host bypass.
pub const PARAM_BYPASS_ID: ClapId = ClapId::new(PARAM_BYPASS_NUM);
/// Parameter id for alternating-subdivision swing.
pub const PARAM_SWING_ID: ClapId = ClapId::new(PARAM_SWING_NUM);
/// Parameter id for the active A/B sound side.
pub const PARAM_SOUND_ID: ClapId = ClapId::new(PARAM_SOUND_NUM);
/// Parameter id for the timing source.
pub const PARAM_TIMING_MODE_ID: ClapId = ClapId::new(PARAM_TIMING_MODE_NUM);
/// Parameter id for the canonical free timing rate in hertz.
pub const PARAM_FREE_RATE_ID: ClapId = ClapId::new(PARAM_FREE_RATE_NUM);
/// Parameter id for the synchronized cycle-start delay.
pub const PARAM_DELAY_ID: ClapId = ClapId::new(PARAM_DELAY_NUM);

/// Plain host value for active processing.
pub const BYPASS_ACTIVE_VALUE: f32 = 0.0;
/// Plain host value for bypassed processing.
pub const BYPASS_BYPASSED_VALUE: f32 = 1.0;
/// Human-readable bypass labels.
pub const BYPASS_LABELS: [&str; 2] = ["ACTIVE", "BYPASSED"];
/// Duration of the GUI's recent host-automation cue.
pub const BYPASS_AUTOMATION_CUE_MICROS: u64 = 250_000;

/// Host-synchronised curve triggering.
#[allow(dead_code)]
pub const TRIGGER_MODE_HOST: usize = 0;
/// Historical external-trigger value retained only for state compatibility.
#[allow(dead_code)]
pub const TRIGGER_MODE_SIDECHAIN: usize = 1;
/// Human-readable trigger-source labels retained for historical state values.
#[allow(dead_code)]
pub const TRIGGER_MODE_LABELS: [&str; 2] = ["Host", "Host"];

/// Historical processing-mode values. Pump now always uses Classic.
pub const PROCESSING_MODE_CLASSIC: usize = 0;
#[allow(dead_code)]
pub const PROCESSING_MODE_PUNCH: usize = 1;
#[allow(dead_code)]
pub const PROCESSING_MODE_LABELS: [&str; 2] = ["Classic", "Classic"];

/// Map historical processing-mode values to the sole supported mode.
pub fn clamp_processing_mode(value: f32) -> usize {
    let _ = value;
    PROCESSING_MODE_CLASSIC
}

/// Map historical trigger-source values to the sole supported host-clock mode.
pub fn clamp_trigger_mode(value: f32) -> usize {
    let _ = value;
    TRIGGER_MODE_HOST
}

/// Default dry/wet blend.
pub const DEFAULT_MIX: f32 = 1.0;
/// Default curve attenuation depth in decibels.
pub const DEFAULT_DEPTH_DB: f32 = 120.0;
/// Default depth retained as a normalized legacy compatibility value.
pub const DEFAULT_DEPTH: f32 = 1.0;
/// Sentinel plain value used by hosts to represent an unbounded (−∞) floor.
pub const FLOOR_NEG_INFINITY_DB: f32 = -60.0;
/// Default minimum wet gain floor (−∞).
pub const DEFAULT_FLOOR_DB: f32 = FLOOR_NEG_INFINITY_DB;
/// Default cycle phase offset.
pub const DEFAULT_PHASE_OFFSET: f32 = 0.0;
/// Default output gain.
pub const DEFAULT_OUTPUT_GAIN_DB: f32 = 0.0;
/// Default curve trigger source.
pub const DEFAULT_TRIGGER_MODE: usize = TRIGGER_MODE_HOST;
/// Default evaluated gain smoothing amount.
pub const DEFAULT_SMOOTH: f32 = 0.0;
/// Default swing amount (straight timing).
pub const DEFAULT_SWING: f32 = 0.0;
/// Timing source values.
pub const TIMING_MODE_SYNC: usize = 0;
pub const TIMING_MODE_FREE: usize = 1;
pub const TIMING_MODE_LABELS: [&str; 2] = ["Sync", "Free"];
/// Default timing source.
pub const DEFAULT_TIMING_MODE: usize = TIMING_MODE_SYNC;
/// Minimum free-running rate in hertz.
pub const MIN_FREE_RATE_HZ: f32 = 0.05;
/// Maximum free-running rate in hertz.
pub const MAX_FREE_RATE_HZ: f32 = 20_000.0;
/// Default free-running rate in hertz.
pub const DEFAULT_FREE_RATE_HZ: f32 = 2.0;
/// Minimum synchronized cycle-start delay in quarter-note beats.
pub const MIN_DELAY_BEATS: usize = 0;
/// Maximum synchronized cycle-start delay in quarter-note beats.
pub const MAX_DELAY_BEATS: usize = 32;
/// Default synchronized cycle-start delay in quarter-note beats.
pub const DEFAULT_DELAY_BEATS: usize = 0;
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
/// Minimum curve attenuation depth in decibels.
pub const MIN_DEPTH_DB: f32 = 0.0;
/// Maximum curve attenuation depth in decibels.
pub const MAX_DEPTH_DB: f32 = 120.0;
/// Minimum finite floor in decibels. This host value is also the −∞ sentinel.
pub const MIN_FLOOR_DB: f32 = FLOOR_NEG_INFINITY_DB;
/// Maximum floor in decibels (unity wet gain).
pub const MAX_FLOOR_DB: f32 = 0.0;
/// Minimum phase offset value.
pub const MIN_PHASE_OFFSET: f32 = 0.0;
/// Maximum phase offset value.
pub const MAX_PHASE_OFFSET: f32 = 1.0;
/// Minimum output gain in decibels.
pub const MIN_OUTPUT_GAIN_DB: f32 = -24.0;
/// Maximum output gain in decibels.
pub const MAX_OUTPUT_GAIN_DB: f32 = 12.0;
/// Maximum evaluated gain smoothing amount.
pub const MAX_SMOOTH: f32 = 1.0;
/// Minimum evaluated gain smoothing amount.
pub const MIN_SMOOTH: f32 = 0.0;
/// Minimum swing amount.
pub const MIN_SWING: f32 = 0.0;
/// Maximum swing amount. At 100%, a cycle midpoint lands at 2/3 of the cycle.
pub const MAX_SWING: f32 = 1.0;

/// Clamp a timing source value into the supported enum range.
pub fn clamp_timing_mode(value: f32) -> usize {
    value
        .round()
        .clamp(TIMING_MODE_SYNC as f32, TIMING_MODE_FREE as f32) as usize
}

/// Clamp a free-running rate into the canonical hertz range.
pub fn clamp_free_rate_hz(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(MIN_FREE_RATE_HZ, MAX_FREE_RATE_HZ)
    } else {
        DEFAULT_FREE_RATE_HZ
    }
}

/// Clamp a synchronized cycle-start delay to its supported integer range.
pub fn clamp_delay_beats(value: f32) -> usize {
    if value.is_finite() {
        value
            .round()
            .clamp(MIN_DELAY_BEATS as f32, MAX_DELAY_BEATS as f32) as usize
    } else {
        DEFAULT_DELAY_BEATS
    }
}

/// One named beat division option.
#[derive(Debug, Copy, Clone)]
pub struct SyncDivision {
    /// Human-readable label used in GUI and host display.
    pub label: &'static str,
    /// Length in quarter-note beats.
    pub beats: f32,
}

/// Supported beat-sync cycle divisions.
pub const SYNC_DIVISIONS: [SyncDivision; 10] = [
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
    SyncDivision {
        label: "4 Bars",
        beats: 16.0,
    },
    SyncDivision {
        label: "8 Bars",
        beats: 32.0,
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
    /// Whether the preset is marked as a favorite in the preset browser.
    pub is_favorite: bool,
    /// Dry/wet mix amount.
    pub mix: f32,
    /// Legacy depth field preserved for backward-compatible state payloads.
    pub depth: f32,
    /// Curve attenuation depth in decibels.
    pub depth_db: f32,
    /// Minimum permitted wet gain in decibels, or [`FLOOR_NEG_INFINITY_DB`].
    pub floor_db: f32,
    /// Cycle phase offset.
    pub phase_offset: f32,
    /// Output trim in decibels.
    pub output_gain_db: f32,
    /// Sync-division index.
    pub sync_division: usize,
    /// Curve trigger source.
    pub trigger_mode: usize,
    /// Evaluated wet-gain smoothing amount.
    pub smooth: f32,
    /// Processing mode.
    pub mode: usize,
    /// Alternating-subdivision swing amount in `[0, 1]`.
    pub swing: f32,
    /// Timing source: synchronized divisions or a free-running rate.
    pub timing_mode: usize,
    /// Canonical free-running timing rate in hertz.
    pub free_rate_hz: f32,
    /// Number of quarter-note beats to hold at the cycle start in Sync mode.
    pub delay_beats: usize,
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

/// Successful outcome from saving current state into the preset bank.
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
}

/// Error returned when a durable preset-bank mutation cannot be completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresetMutationError {
    /// The requested preset or quick-slot index does not exist.
    InvalidIndex,
    /// The supplied preset name is empty after normalization.
    InvalidName,
    /// The preset bank has reached its supported capacity.
    CapacityReached,
    /// The in-memory preset bank could not be accessed.
    StateUnavailable,
    /// The staged bank could not be written durably and was rolled back.
    PersistenceFailed {
        /// Diagnostic detail logged for support and tests.
        message: String,
    },
}

impl PumpPresetBank {
    /// Build the default single-entry preset bank from plugin defaults.
    pub fn default_init() -> Self {
        Self {
            selected: 0,
            presets: vec![PumpPreset {
                name: DEFAULT_PRESET_NAME.to_string(),
                is_read_only: false,
                is_favorite: false,
                mix: DEFAULT_MIX,
                depth: DEFAULT_DEPTH,
                depth_db: DEFAULT_DEPTH_DB,
                floor_db: DEFAULT_FLOOR_DB,
                phase_offset: DEFAULT_PHASE_OFFSET,
                output_gain_db: DEFAULT_OUTPUT_GAIN_DB,
                sync_division: DEFAULT_SYNC_DIVISION_INDEX,
                trigger_mode: DEFAULT_TRIGGER_MODE,
                smooth: DEFAULT_SMOOTH,
                mode: PROCESSING_MODE_CLASSIC,
                swing: DEFAULT_SWING,
                timing_mode: DEFAULT_TIMING_MODE,
                free_rate_hz: DEFAULT_FREE_RATE_HZ,
                delay_beats: DEFAULT_DELAY_BEATS,
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
    let phase_source_equal = match (&left.phase_source, &right.phase_source) {
        (None, None) => true,
        (Some(left), Some(right)) => curve_near_eq(left, right),
        _ => false,
    };
    left.nodes
        .iter()
        .zip(right.nodes.iter())
        .all(|(lhs, rhs)| float_near_eq(lhs.x, rhs.x) && float_near_eq(lhs.y, rhs.y))
        && left
            .segments
            .iter()
            .zip(right.segments.iter())
            .all(|(lhs, rhs)| float_near_eq(lhs.tension, rhs.tension))
        && phase_source_equal
        && float_near_eq(left.phase_offset, right.phase_offset)
}

/// Shared atomic parameter/state storage across threads.
pub struct PumpParams {
    pub(super) mix: AtomicF32,
    pub(super) depth_db: AtomicF32,
    pub(super) floor_db: AtomicF32,
    pub(super) phase_offset: AtomicF32,
    pub(super) output_gain_db: AtomicF32,
    pub(super) sync_division: AtomicU32,
    pub(super) trigger_mode: AtomicU32,
    pub(super) smooth: AtomicF32,
    pub(super) mode: AtomicU32,
    pub(super) swing: AtomicF32,
    pub(super) timing_mode: AtomicU32,
    pub(super) free_rate_hz: AtomicF32,
    pub(super) delay_beats: AtomicU32,
    pub(super) bypass: AtomicBool,
    pub(super) bypass_revision: AtomicU32,
    pub(super) bypass_last_automation_micros: AtomicU64,
    pub(super) editable_curve: RwLock<EditableCurve>,
    pub(super) curve: [AtomicF32; CURVE_TABLE_LEN],
    pub(super) curve_revision: AtomicU32,
    pub(super) preset_bank: RwLock<PumpPresetBank>,
    pub(super) preset_persistence_warning: RwLock<Option<String>>,
    pub(super) active_sound: AtomicU32,
    pub(super) pending_active_sound: AtomicU32,
    pub(super) active_sound_dirty: AtomicBool,
    pub(super) realtime_mix: [AtomicF32; 2],
    pub(super) realtime_depth_db: [AtomicF32; 2],
    pub(super) realtime_floor_db: [AtomicF32; 2],
    pub(super) realtime_phase_offset: [AtomicF32; 2],
    pub(super) realtime_output_gain_db: [AtomicF32; 2],
    pub(super) realtime_sync_division: [AtomicU32; 2],
    pub(super) realtime_trigger_mode: [AtomicU32; 2],
    pub(super) realtime_smooth: [AtomicF32; 2],
    pub(super) realtime_mode: [AtomicU32; 2],
    pub(super) realtime_swing: [AtomicF32; 2],
    pub(super) realtime_timing_mode: [AtomicU32; 2],
    pub(super) realtime_free_rate_hz: [AtomicF32; 2],
    pub(super) realtime_delay_beats: [AtomicU32; 2],
    pub(super) realtime_curve: [[AtomicF32; CURVE_TABLE_LEN]; 2],
    pub(super) sound_states: RwLock<[PumpSoundState; 2]>,
    /// Durable per-side reference states used for A/B dirty indicators.
    pub(super) stored_sound_states: RwLock<[PumpSoundState; 2]>,
    pub(super) sound_state_dirty: [AtomicBool; 2],
}
