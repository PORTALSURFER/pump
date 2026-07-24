//! Declarative curve-editor GUI for Pump.

use std::sync::{Arc, Mutex, OnceLock};

use toybox::clack_extensions::gui::{GuiSize, Window};
use toybox::clack_plugin::plugin::PluginError;
use toybox::clack_plugin::utils::ClapId;
use toybox::clap::automation::{AutomationConfig, AutomationQueue};
use toybox::clap::gui::{
    GuiHostWindow, GuiOpenRequest, HostParamRequester, InputState, ShortcutBinding,
    ShortcutModifiers,
};
use toybox::gui::declarative::{
    button, column, column_slots, curve_editor, dropdown, grid, indicator, knob, panel,
    root_frame_sized, row_slots, spacer, stack, surface, textbox, toggle, weighted_slot,
    weighted_slot_lengths, CurveEditorModifier, CurveEditorStyle, CurveGridConfig,
    CurveHighlightMode, CurveInteractionOptions, CurveModel, CurvePoint,
    CurveSegment as CurveEditorSegment, CurveSegmentMoveOptions, CurveSnapConfig, EndpointMode,
    GridTemplate, LayoutBox, Node, OverflowPolicy, RegionInteractionKind, RootScaleMode,
    ScrollViewSpec, Slot, SlotAlign, SlotCrossSize, SlotParams, SurfaceCommand, ThemeTokens,
    TrackSize, UiAction, UiSpec,
};
use toybox::gui::{Color, MainPalette, Point, Rect, Size};
use toybox::raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use crate::curve::{
    sample_editable_curve, CurveNode, CurveSegment, EditableCurve, MAX_EDITABLE_NODES,
    MAX_SEGMENT_TENSION, MIN_SEGMENT_TENSION,
};
use crate::params::{
    sync_division_label, PresetMutationError, PumpParams, PumpPresetBank, SavePresetOutcome,
    DEFAULT_DEPTH_DB, DEFAULT_FLOOR_DB, DEFAULT_MIX, DEFAULT_OUTPUT_GAIN_DB, DEFAULT_PHASE_OFFSET,
    DEFAULT_PRESET_NAME, GLOBAL_CURVE_SLOT_COUNT, MAX_DEPTH_DB, MAX_FLOOR_DB, MAX_MIX,
    MAX_OUTPUT_GAIN_DB, MAX_PHASE_OFFSET, MAX_PRESET_NAME_CHARS, MAX_SYNC_DIVISION, MIN_DEPTH_DB,
    MIN_FLOOR_DB, MIN_MIX, MIN_OUTPUT_GAIN_DB, MIN_PHASE_OFFSET, PARAM_DEPTH_ID, PARAM_FLOOR_ID,
    PARAM_MIX_ID, PARAM_OUTPUT_GAIN_ID, PARAM_PHASE_OFFSET_ID, PARAM_SYNC_DIVISION_ID,
};
use crate::GuiStatus;

mod curve_math;
mod layout_support;
#[cfg(any(feature = "vst3", test))]
mod radiant_editor;
mod state_impl;
mod window_host;

use curve_math::*;
use layout_support::*;
#[cfg(test)]
pub(crate) use radiant_editor::radiant_editor_frame_for_params;
#[cfg(feature = "vst3")]
pub(crate) use radiant_editor::RadiantPumpEditor;
pub(crate) use window_host::preferred_window_size;
pub use window_host::PumpGui;

/// Default logical width for the Pump design canvas.
///
/// Patchbay owns runtime scaling and resize policy; Pump only publishes this
/// baseline logical size.
pub const WINDOW_WIDTH: u32 = 420;
/// Default logical height for the Pump design canvas.
///
/// Patchbay owns runtime scaling and resize policy; Pump only publishes this
/// baseline logical size.
pub const WINDOW_HEIGHT: u32 = 282;
const DESIGN_ASPECT_RATIO: f32 = WINDOW_WIDTH as f32 / WINDOW_HEIGHT as f32;

const ROOT_KEY: &str = "pump-root";
const CURVE_KEY: &str = "curve";
const MIX_KEY: &str = "mix";
const PHASE_KEY: &str = "phase";
const OUTPUT_KEY: &str = "output";
const DIVISION_KEY: &str = "division";
const SNAP_KEY: &str = "snap";
const INCOMING_WAVEFORM_KEY: &str = "incoming-waveform";
const GRID_OVERRIDE_KEY: &str = "grid-override";
const PRESET_DROPDOWN_KEY: &str = "preset-dropdown";
const PRESET_ADD_KEY: &str = "preset-add";
const PRESET_SAVE_KEY: &str = "preset-save";
const PRESET_RENAME_BUTTON_KEY: &str = "preset-rename-button";
const PRESET_RENAME_KEY: &str = "preset-rename";
const UNDO_KEY: &str = "undo";
const REDO_KEY: &str = "redo";
const QUICK_SLOT_KEY_PREFIX: &str = "quick-slot-";
const QUICK_SLOT_NAV_KEY: &str = "quick-slot-nav";
const QUICK_SLOT_PREVIOUS_KEY: &str = "quick-slot-previous";
const QUICK_SLOT_NEXT_KEY: &str = "quick-slot-next";
const QUICK_SLOT_NAV_WIDTH: u32 = 20;
const QUICK_SLOT_GAP: u32 = 4;
const QUICK_SLOT_VISIBLE_COUNT: usize = 6;
const SHORTCUT_KEY_RENAME: char = 'r';
const SHORTCUT_KEY_SAVE: char = 's';
pub(crate) const SHORTCUT_KEY_SNAP_INVERT: char = 's';
const SHORTCUT_KEY_ADD: char = '+';
const SHORTCUT_KEY_ADD_ALT: char = '=';
const SHORTCUT_KEY_UNDO: char = 'z';
const SHORTCUT_KEY_REDO: char = 'y';
const SHORTCUT_KEY_QUICK_SLOT_PREVIOUS: char = '[';
const SHORTCUT_KEY_QUICK_SLOT_NEXT: char = ']';

const HEADER_SECTION_WEIGHT: u16 = 7;
const CURVE_SECTION_WEIGHT: u16 = 58;
const QUICK_SHAPES_SECTION_WEIGHT: u16 = 9;
const CONTROLS_SECTION_WEIGHT: u16 = 26;
const ROOT_SECTION_WEIGHT_SUM: u32 = HEADER_SECTION_WEIGHT as u32
    + CURVE_SECTION_WEIGHT as u32
    + QUICK_SHAPES_SECTION_WEIGHT as u32
    + CONTROLS_SECTION_WEIGHT as u32;
const KNOBS_SECTION_WEIGHT: u16 = 70;
const DROPDOWN_SECTION_WEIGHT: u16 = 30;
const HEADER_EMPTY_SECTION_PERCENT: u8 = 80;
const HEADER_INDICATOR_SECTION_PERCENT: u8 = 20;
const HEADER_STORAGE_WARNING_SECTION_PERCENT: u8 = 40;
const HEADER_VERSION_LABEL_HEIGHT: u32 = 8;
#[cfg(test)]
const CURVE_W: u32 = WINDOW_WIDTH;
const CURVE_VERTICAL_MARGIN: u32 = 10;
#[cfg(test)]
const CURVE_H: u32 = resolve_curve_editor_height(resolve_vertical_slot_heights(WINDOW_HEIGHT).1);
const CURVE_EDITOR_SECTION_WEIGHT: u16 = 92;
const METER_SECTION_WEIGHT: u16 = 8;
const CURVE_REFERENCE_GUTTER_WIDTH: u32 = 76;
const METER_WIDTH: i32 = 8;
const METER_STROKE: i32 = 1;
const BASE_KNOB_DIAMETER: u32 = 92;
const BASE_TEXT_SCALE: u32 = 2;
const KNOBS_PER_ROW: usize = 5;
const BASE_CONTROL_LINE_UNIT: u32 = 8;
const BASE_DROPDOWN_CONTROL_H: u32 = 24;
const TRANSPORT_INDICATOR_SIZE: u32 = 10;
const PRESET_WARNING_FRAMES: u8 = 45;
const PRESET_WARNING_BLINK_HALF_PERIOD_FRAMES: u8 = 6;
const PRESET_WARNING_MAX: &str = "MAX";
const PRESET_WARNING_NAME: &str = "NAME";
const PRESET_WARNING_STORAGE: &str = "NOT SAVED - CHECK PRESET FOLDER";
const HISTORY_STEP_LIMIT: usize = 128;
const NODE_DRAW_RADIUS: i32 = 4;
const NODE_HIT_RADIUS: i32 = 8;
const PLAYHEAD_DOT_CORE_RADIUS: i32 = 4;
const PLAYHEAD_DOT_GLOW_RADIUS: i32 = 10;
const QUICK_SLOT_PREVIEW_MARGIN: i32 = 3;
const QUICK_SLOT_PREVIEW_STEPS: usize = 24;
const SEGMENT_NEAR_HIT_RADIUS: i32 = 16;
const SEGMENT_DIRECT_HIT_RADIUS: i32 = 6;
const NODE_INSERT_GUARD_RADIUS: i32 = 12;
const CURVE_DRAG_START_THRESHOLD_PX: i32 = 2;
const CURVE_TENSION_PIXEL_SCALE: f32 = 120.0;
const NODE_PUSH_THROUGH_PX: i32 = 10;
const NODE_X_MIN_SPACING: f32 = 1.0e-3;

#[derive(Clone, Copy, Debug, PartialEq)]
struct CurveGainReference {
    gain: f32,
    label: &'static str,
    bitmap_label: &'static str,
    gain_db: Option<f32>,
}

/// Gain references shared by both Pump curve-editor renderers.
///
/// Curve Y values are linear amplitude gains, so finite dB references must be
/// converted into that same domain before either renderer projects them.
#[cfg(test)]
fn curve_gain_references() -> [CurveGainReference; 4] {
    [
        CurveGainReference {
            gain: crate::dsp::db_to_linear(0.0),
            label: "0 dB",
            bitmap_label: "0 dB",
            gain_db: Some(0.0),
        },
        CurveGainReference {
            gain: crate::dsp::db_to_linear(-6.0),
            label: "−6 dB",
            bitmap_label: "-6 dB",
            gain_db: Some(-6.0),
        },
        CurveGainReference {
            gain: crate::dsp::db_to_linear(-12.0),
            label: "−12 dB",
            bitmap_label: "-12 dB",
            gain_db: Some(-12.0),
        },
        CurveGainReference {
            gain: 0.0,
            label: "−∞",
            bitmap_label: "-INF",
            gain_db: None,
        },
    ]
}

/// Return dB guides at stable normalized curve positions using the live DSP
/// mapping. The default wrapper above remains for legacy renderer tests.
fn curve_gain_references_for_mapping(depth_db: f32, floor_db: f32) -> [CurveGainReference; 4] {
    [
        1.0_f32,
        crate::dsp::db_to_linear(-6.0),
        crate::dsp::db_to_linear(-12.0),
        0.0,
    ]
    .map(|curve_value| {
        let gain = crate::dsp::curve_value_to_gain(curve_value, depth_db, floor_db);
        CurveGainReference {
            gain,
            label: "",
            bitmap_label: "",
            gain_db: crate::dsp::gain_to_db(gain),
        }
    })
}

fn curve_gain_reference_text(reference: CurveGainReference, bitmap: bool) -> String {
    match reference.gain_db {
        Some(db) if bitmap => format!("{db:.0} dB"),
        Some(db) => format!("{db:.0} dB").replace('-', "−"),
        None if bitmap => "-INF".to_string(),
        None => "−∞".to_string(),
    }
}

pub(crate) fn build_version_label() -> String {
    format!(
        "{}+{}",
        env!("CARGO_PKG_VERSION"),
        option_env!("PUMP_BUILD_GIT_SHA_SHORT").unwrap_or("unknown")
    )
}

#[cfg(all(test, feature = "screenshot-test", not(target_os = "windows")))]
mod screenshot_tests {
    // Policy anchor: screenshot_renders_initial_ui
    include!("gui/screenshot_non_windows_tests.rs");
}

struct GuiState {
    params: Arc<PumpParams>,
    status: Arc<GuiStatus>,
    automation_queue: Arc<AutomationQueue>,
    automation_config: AutomationConfig,
    param_requester: Option<HostParamRequester>,
    runtime: Mutex<GuiRuntime>,
}

struct GuiRuntime {
    selected_node: Option<usize>,
    selected_nodes: Vec<usize>,
    drag_mode: Option<CurveDragMode>,
    marquee_selection: Option<CurveMarqueeSelection>,
    curve_hovered: bool,
    curve_local_pointer: Point,
    curve_size: Size,
    snap_enabled: bool,
    snap_hovered: bool,
    grid_override: Option<usize>,
    shortcut_snap_invert_held: bool,
    preset_rename_active: bool,
    preset_rename_target: usize,
    preset_name_draft: String,
    preset_warning_frames: u8,
    preset_warning_text: Option<&'static str>,
    quick_slot_hovered: Option<usize>,
    quick_slot_pressed: Option<usize>,
    quick_slot_nav_hovered: bool,
    quick_slot_nav_pressed: bool,
    quick_slot_carousel_focused: bool,
    quick_slot_scroll_offset: i32,
    quick_slot_wheel_consumed: bool,
    loaded_global_curve_slot: Option<usize>,
    pointer_primary_down: bool,
    pointer_secondary_down: bool,
    active_knob_gesture_param: Option<ClapId>,
    undo_history: Vec<UiHistorySnapshot>,
    redo_history: Vec<UiHistorySnapshot>,
    knob_history_anchor: Option<UiHistorySnapshot>,
    curve_history_anchor: Option<UiHistorySnapshot>,
}

#[derive(Clone, Debug)]
enum CurveDragMode {
    MoveNode {
        origin_index: usize,
        origin_curve: EditableCurve,
        start_pointer: Point,
        dragging: bool,
    },
    MoveNodeGroup {
        origin_indices: Vec<usize>,
        origin_curve: EditableCurve,
        start_pointer: Point,
        dragging: bool,
    },
    MoveSegment {
        index: usize,
        start_pointer: Point,
        start_left_x: f32,
        start_right_x: f32,
        start_left_y: f32,
        start_right_y: f32,
        dragging: bool,
    },
    AdjustSegmentCurve {
        index: usize,
        start_pointer: Point,
        start_tension: f32,
        dragging: bool,
    },
}

/// Runtime marquee-selection rectangle in curve-local coordinates.
#[derive(Clone, Copy, Debug)]
struct CurveMarqueeSelection {
    start_pointer: Point,
    current_pointer: Point,
}

/// Snapshot of host-automation control values used to build one UI frame.
#[derive(Clone, Copy, Debug)]
struct ControlSnapshot {
    mix: f32,
    depth_db: f32,
    floor_db: f32,
    phase_offset: f32,
    output_gain_db: f32,
    division: usize,
    incoming_waveform_enabled: bool,
    snap_enabled: bool,
    snap_hovered: bool,
    grid_override: Option<usize>,
    shortcut_snap_invert_held: bool,
}

impl ControlSnapshot {
    fn effective_snap_enabled(self) -> bool {
        self.snap_enabled ^ self.shortcut_snap_invert_held
    }
}

/// Snapshot of mutable GUI/parameter state used for undo/redo history.
#[derive(Clone, Debug, PartialEq)]
struct UiHistorySnapshot {
    mix: f32,
    depth_db: f32,
    floor_db: f32,
    phase_offset: f32,
    output_gain_db: f32,
    sync_division: usize,
    editable_curve: EditableCurve,
    preset_bank: PumpPresetBank,
}

/// Snapshot of preset-bank state needed for header rendering.
#[derive(Clone, Debug)]
struct PresetSnapshot {
    names: Vec<String>,
    selected: usize,
    dirty: bool,
    rename_active: bool,
    rename_draft: String,
    persistence_warning: bool,
    warning_blink_visible: bool,
}

/// Snapshot of curve-editor hover/selection state used for drawing.
#[derive(Clone, Debug)]
struct CurveRenderState {
    selected_node: Option<usize>,
    selected_nodes: Vec<usize>,
    hovered_node: Option<usize>,
    hovered_segment: Option<usize>,
    preview_node: Option<CurveNode>,
}

#[derive(Clone, Copy, Debug, Default)]
struct QuickSlotVisualState {
    hovered: bool,
    pressed: bool,
    active: bool,
    store_hovered: bool,
    deviated: bool,
}

#[cfg(test)]
mod tests {
    // Policy anchor: fn emitted_ui_spec_passes_strict_slot_validation
    include!("gui/tests.rs");
    include!("gui/interaction_and_automation_tests.rs");
}

#[cfg(all(test, feature = "screenshot-test", target_os = "windows"))]
mod screenshot_tests {
    // Policy anchor: screenshot_renders_initial_ui
    include!("gui/screenshot_windows_tests.rs");
}
