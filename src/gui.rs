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
    root_frame_sized, row_slots, spacer, textbox, weighted_slot, weighted_slot_lengths,
    CurveEditorStyle, CurveHighlightMode, CurveInteractionOptions, CurveModel, CurvePoint,
    CurveSegment as CurveEditorSegment, EndpointMode, GridTemplate, LayoutBox, Node,
    OverflowPolicy, RegionInteractionKind, RootScaleMode, Slot, SlotAlign, SlotCrossSize,
    SlotParams, SurfaceCommand, ThemeTokens, TrackSize, UiAction, UiSpec,
};
use toybox::gui::{Color, MainPalette, Point, Rect, Size};
use toybox::raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use crate::curve::{
    sample_editable_curve, CurveNode, CurveSegment, EditableCurve, MAX_EDITABLE_NODES,
    MAX_SEGMENT_TENSION, MIN_SEGMENT_TENSION,
};
use crate::params::{
    sync_division_label, PumpParams, SavePresetOutcome, DEFAULT_DEPTH, DEFAULT_MIX,
    DEFAULT_OUTPUT_GAIN_DB, DEFAULT_PHASE_OFFSET, DEFAULT_PRESET_NAME, MAX_DEPTH, MAX_MIX,
    MAX_OUTPUT_GAIN_DB, MAX_PHASE_OFFSET, MAX_PRESET_NAME_CHARS, MAX_SYNC_DIVISION, MIN_DEPTH,
    MIN_MIX, MIN_OUTPUT_GAIN_DB, MIN_PHASE_OFFSET, PARAM_DEPTH_ID, PARAM_MIX_ID,
    PARAM_OUTPUT_GAIN_ID, PARAM_PHASE_OFFSET_ID, PARAM_SYNC_DIVISION_ID,
};
use crate::time_utils::monotonic_micros;
use crate::GuiStatus;

mod curve_math;
mod layout_support;
mod state_impl;

use curve_math::*;
use layout_support::*;

/// Default logical width for the Pump design canvas.
///
/// Patchbay owns runtime scaling and resize policy; Pump only publishes this
/// baseline logical size.
pub const WINDOW_WIDTH: u32 = 420;
/// Default logical height for the Pump design canvas.
///
/// Patchbay owns runtime scaling and resize policy; Pump only publishes this
/// baseline logical size.
pub const WINDOW_HEIGHT: u32 = 258;
const DESIGN_ASPECT_RATIO: f32 = WINDOW_WIDTH as f32 / WINDOW_HEIGHT as f32;

const ROOT_KEY: &str = "pump-root";
const CURVE_KEY: &str = "curve";
const MIX_KEY: &str = "mix";
const DEPTH_KEY: &str = "depth";
const PHASE_KEY: &str = "phase";
const OUTPUT_KEY: &str = "output";
const DIVISION_KEY: &str = "division";
const RESET_KEY: &str = "reset";
const PRESET_DROPDOWN_KEY: &str = "preset-dropdown";
const PRESET_ADD_KEY: &str = "preset-add";
const PRESET_SAVE_KEY: &str = "preset-save";
const PRESET_RENAME_BUTTON_KEY: &str = "preset-rename-button";
const PRESET_RENAME_KEY: &str = "preset-rename";
const SHORTCUT_KEY_RENAME: char = 'r';
const SHORTCUT_KEY_SAVE: char = 's';
const SHORTCUT_KEY_ADD: char = '+';
const SHORTCUT_KEY_ADD_ALT: char = '=';

const HEADER_SECTION_WEIGHT: u16 = 7;
const CURVE_SECTION_WEIGHT: u16 = 63;
const CONTROLS_SECTION_WEIGHT: u16 = 30;
const ROOT_SECTION_WEIGHT_SUM: u32 =
    HEADER_SECTION_WEIGHT as u32 + CURVE_SECTION_WEIGHT as u32 + CONTROLS_SECTION_WEIGHT as u32;
const KNOBS_SECTION_WEIGHT: u16 = 70;
const DROPDOWN_SECTION_WEIGHT: u16 = 30;
const HEADER_EMPTY_SECTION_PERCENT: u8 = 80;
const HEADER_INDICATOR_SECTION_PERCENT: u8 = 20;
const CURVE_W: u32 = WINDOW_WIDTH;
const CURVE_H: u32 = resolve_vertical_slot_heights(WINDOW_HEIGHT).1;
const METER_X_OFFSET: i32 = 12;
const METER_Y_OFFSET: i32 = 10;
const METER_WIDTH: i32 = 6;
const METER_STROKE: i32 = 1;
const BASE_KNOB_DIAMETER: u32 = 92;
const BASE_TEXT_SCALE: u32 = 2;
const KNOBS_PER_ROW: usize = 4;
const BASE_CONTROL_LINE_UNIT: u32 = 8;
const BASE_DROPDOWN_CONTROL_H: u32 = 24;
const TRANSPORT_INDICATOR_SIZE: u32 = 10;
const RESET_GUARD_AFTER_DROPDOWN_MICROS: u64 = 120_000;
const PRESET_WARNING_FRAMES: u8 = 45;
const PRESET_WARNING_BLINK_HALF_PERIOD_FRAMES: u8 = 6;
const PRESET_WARNING_MAX: &str = "MAX";
const PRESET_WARNING_NAME: &str = "NAME";
const NODE_DRAW_RADIUS: i32 = 4;
const NODE_HIT_RADIUS: i32 = 8;
const PLAYHEAD_DOT_CORE_RADIUS: i32 = 4;
const PLAYHEAD_DOT_GLOW_RADIUS: i32 = 10;
const SEGMENT_NEAR_HIT_RADIUS: i32 = 16;
const SEGMENT_DIRECT_HIT_RADIUS: i32 = 6;
const NODE_INSERT_GUARD_RADIUS: i32 = 12;
const CURVE_DRAG_START_THRESHOLD_PX: i32 = 2;
const CURVE_TENSION_PIXEL_SCALE: f32 = 120.0;
const NODE_PUSH_THROUGH_PX: i32 = 10;
const NODE_X_MIN_SPACING: f32 = 1.0e-3;

#[cfg(all(test, feature = "screenshot-test", not(target_os = "windows")))]
mod screenshot_tests {
    // Policy anchor: screenshot_renders_initial_ui
    include!("gui/screenshot_non_windows_tests.rs");
}

/// Host-window wrapper for the Pump editor.
#[derive(Default)]
pub struct PumpGui {
    window: GuiHostWindow,
}

impl PumpGui {
    /// Return default focused-window keyboard shortcuts for Pump.
    fn default_shortcuts() -> Vec<ShortcutBinding> {
        vec![
            ShortcutBinding::new(
                PRESET_RENAME_BUTTON_KEY,
                SHORTCUT_KEY_RENAME,
                ShortcutModifiers::default(),
            ),
            ShortcutBinding::new(
                PRESET_SAVE_KEY,
                SHORTCUT_KEY_SAVE,
                ShortcutModifiers::default(),
            ),
            ShortcutBinding::new(
                PRESET_ADD_KEY,
                SHORTCUT_KEY_ADD,
                ShortcutModifiers::new(true, false, false),
            ),
            ShortcutBinding::new(
                PRESET_ADD_KEY,
                SHORTCUT_KEY_ADD_ALT,
                ShortcutModifiers::default(),
            ),
        ]
    }

    /// Attach raw host window handle.
    pub fn set_parent_raw(&mut self, parent: RawWindowHandle) {
        self.window.set_parent(parent);
    }

    /// Attach CLAP host parent window.
    pub fn set_parent(&mut self, window: Window<'_>) {
        self.set_parent_raw(window.raw_window_handle());
    }

    /// Open Pump editor.
    pub fn open(
        &mut self,
        params: &Arc<PumpParams>,
        status: &Arc<GuiStatus>,
        automation_queue: Arc<AutomationQueue>,
        param_requester: Option<HostParamRequester>,
    ) -> Result<(), PluginError> {
        self.window.set_aspect_ratio(Some(DESIGN_ASPECT_RATIO));
        self.window.set_shortcuts(Self::default_shortcuts());
        let state = GuiState::new(
            Arc::clone(params),
            Arc::clone(status),
            automation_queue,
            param_requester,
        );
        let open_size = state.measured_open_size();
        let on_init = Box::new(|_state: &mut GuiState| {});
        let build = Box::new(|input: &InputState, state: &GuiState| state.build_ui(input));
        let reduce = Box::new(|state: &mut GuiState, action: UiAction| state.reduce_action(action));

        self.window
            .open_parented_with(GuiOpenRequest::<GuiState, _, _, _>::new(
                "pump".to_string(),
                open_size,
                state,
                on_init,
                build,
                reduce,
            ))
    }

    /// Request a logical resize from the GUI thread.
    #[cfg(any(feature = "vst3", windows))]
    pub fn request_resize(&self, width: u32, height: u32) {
        self.window.request_resize(width, height);
    }

    /// Inject one character tagged as host-injected key input.
    #[cfg(any(feature = "vst3", windows))]
    pub fn post_injected_text_char(&self, ch: char, modifiers: ShortcutModifiers) -> bool {
        self.window.post_injected_text_char(ch, modifiers)
    }

    /// Return `true` when preset rename text editing is active.
    #[cfg(any(feature = "vst3", windows))]
    pub fn text_edit_active(&self) -> bool {
        self.window.text_edit_active()
    }

    /// Resolve one registered shortcut action key from input.
    #[cfg(any(feature = "vst3", windows))]
    pub fn shortcut_action_for_input(
        &self,
        ch: char,
        modifiers: ShortcutModifiers,
    ) -> Option<String> {
        self.window.shortcut_action_for_input(ch, modifiers)
    }

    /// Return true when host-driven resizing is enabled.
    pub fn host_resize_enabled(&self) -> bool {
        self.window.host_resize_enabled()
    }

    /// Resolve host-adjusted size according to the configured resize policy.
    pub fn adjust_host_size(&self, size: GuiSize) -> Option<GuiSize> {
        self.window
            .adjust_host_size(size)
            .map(constrained_host_size)
    }

    /// Apply a host-provided size using Toybox's canonical resize behavior.
    pub fn apply_host_size(&self, size: GuiSize) {
        self.window.apply_host_size(constrained_host_size(size));
    }

    /// Close editor if it is open.
    pub fn close(&mut self) {
        self.window.hide();
    }

    /// Return last known logical size.
    pub fn last_size(&self) -> Option<(u32, u32)> {
        self.window.last_size()
    }
}

/// Return the preferred logical Pump window size measured from declarative layout.
///
/// Hosts may query a plugin view size before the GUI is opened. This helper
/// provides a stable measured fallback so host-side parent windows are large
/// enough for the current declarative content on first attach.
pub(crate) fn preferred_window_size() -> (u32, u32) {
    static PREFERRED_SIZE: OnceLock<(u32, u32)> = OnceLock::new();
    *PREFERRED_SIZE.get_or_init(|| {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        state.measured_open_size()
    })
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
    drag_mode: Option<CurveDragMode>,
    curve_hovered: bool,
    curve_local_pointer: Point,
    curve_size: Size,
    last_division_change_micros: Option<u64>,
    preset_rename_active: bool,
    preset_rename_target: usize,
    preset_name_draft: String,
    preset_warning_frames: u8,
    preset_warning_text: Option<&'static str>,
    pointer_primary_down: bool,
    active_knob_gesture_param: Option<ClapId>,
}

#[derive(Clone, Debug)]
enum CurveDragMode {
    MoveNode {
        origin_index: usize,
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

/// Snapshot of host-automation control values used to build one UI frame.
#[derive(Clone, Copy, Debug)]
struct ControlSnapshot {
    mix: f32,
    depth: f32,
    phase_offset: f32,
    output_gain_db: f32,
    division: usize,
}

/// Snapshot of preset-bank state needed for header rendering.
#[derive(Clone, Debug)]
struct PresetSnapshot {
    names: Vec<String>,
    selected: usize,
    dirty: bool,
    rename_active: bool,
    rename_draft: String,
    warning_blink_visible: bool,
}

/// Snapshot of curve-editor hover/selection state used for drawing.
#[derive(Clone, Copy, Debug)]
struct CurveRenderState {
    selected_node: Option<usize>,
    hovered_node: Option<usize>,
    hovered_segment: Option<usize>,
    preview_node: Option<CurveNode>,
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
