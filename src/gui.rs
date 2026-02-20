//! Declarative curve-editor GUI for Pump.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use toybox::clack_extensions::gui::{GuiSize, Window};
use toybox::clack_plugin::plugin::PluginError;
use toybox::clack_plugin::utils::ClapId;
use toybox::clap::automation::{AutomationConfig, AutomationQueue};
use toybox::clap::gui::{
    GuiHostWindow, GuiOpenRequest, HostParamRequester, InputState, ShortcutBinding,
    ShortcutModifiers,
};
use toybox::gui::declarative::{
    button, column, column_slots, dropdown, grid, indicator, knob, panel, root_frame_sized,
    row_slots, spacer, surface, textbox, weighted_slot, weighted_slot_lengths, GridTemplate,
    LayoutBox, Node, OverflowPolicy, RegionInteractionKind, RootScaleMode, Slot, SlotAlign,
    SlotCrossSize, SlotParams, SurfaceCommand, ThemeTokens, TrackSize, UiAction, UiSpec,
};
use toybox::gui::{Color, MainPalette, Point, Rect, Size};
use toybox::raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use crate::curve::{
    sample_editable_curve, CurveNode, CurveSegment, EditableCurve, MAX_EDITABLE_NODES,
    MAX_SEGMENT_TENSION, MIN_SEGMENT_TENSION,
};
use crate::params::{
    sync_division_label, PumpParams, SavePresetOutcome, DEFAULT_PRESET_NAME, MAX_DEPTH, MAX_MIX,
    MAX_OUTPUT_GAIN_DB, MAX_PHASE_OFFSET, MAX_PRESET_NAME_CHARS, MAX_SYNC_DIVISION, MIN_DEPTH,
    MIN_MIX, MIN_OUTPUT_GAIN_DB, MIN_PHASE_OFFSET, PARAM_DEPTH_ID, PARAM_MIX_ID,
    PARAM_OUTPUT_GAIN_ID, PARAM_PHASE_OFFSET_ID, PARAM_SYNC_DIVISION_ID,
};
use crate::GuiStatus;

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
const HEADER_CONTROL_GAP: i32 = 4;
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
const PRESET_WARNING_MAX: &str = "MAX";
const PRESET_WARNING_INIT: &str = "INIT";
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

const fn fixed_box(width: u32, height: u32) -> LayoutBox {
    LayoutBox::fixed(width, height).max(width, height)
}

fn curve_scale_for_size(curve_size: Size) -> f32 {
    let width_scale = curve_size.width.max(1) as f32 / WINDOW_WIDTH as f32;
    let height_scale = curve_size.height.max(1) as f32 / WINDOW_HEIGHT as f32;
    width_scale.min(height_scale).clamp(0.2, 4.0)
}

fn scaled_curve_i32(base: i32, curve_size: Size) -> i32 {
    (base as f32 * curve_scale_for_size(curve_size))
        .round()
        .max(1.0) as i32
}

fn scaled_curve_u32(base: u32, curve_size: Size) -> u32 {
    (base as f32 * curve_scale_for_size(curve_size))
        .round()
        .max(1.0) as u32
}

fn scaled_curve_tension_pixel_scale(curve_size: Size) -> f32 {
    CURVE_TENSION_PIXEL_SCALE * curve_scale_for_size(curve_size)
}

fn monotonic_micros() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = EPOCH.get_or_init(Instant::now);
    epoch.elapsed().as_micros().min(u64::MAX as u128) as u64
}

fn node_hit_radius(curve_size: Size) -> i32 {
    scaled_curve_i32(NODE_HIT_RADIUS, curve_size)
}

fn segment_near_hit_radius(curve_size: Size) -> i32 {
    scaled_curve_i32(SEGMENT_NEAR_HIT_RADIUS, curve_size)
}

fn segment_direct_hit_radius(curve_size: Size) -> i32 {
    scaled_curve_i32(SEGMENT_DIRECT_HIT_RADIUS, curve_size)
}

fn node_insert_guard_radius(curve_size: Size) -> i32 {
    scaled_curve_i32(NODE_INSERT_GUARD_RADIUS, curve_size)
}

fn curve_drag_threshold_px(curve_size: Size) -> i32 {
    scaled_curve_i32(CURVE_DRAG_START_THRESHOLD_PX, curve_size)
}

fn node_push_through_threshold_px(curve_size: Size) -> i32 {
    scaled_curve_i32(NODE_PUSH_THROUGH_PX, curve_size)
}

fn curve_tension_pixel_scale(curve_size: Size) -> f32 {
    scaled_curve_tension_pixel_scale(curve_size)
}

/// Return the internal tension-sign multiplier that produces visual upward bend.
///
/// Rising segments require negative tension for upward bend while falling
/// segments require positive tension, so drag logic must compensate by segment.
fn segment_upward_tension_sign(curve: &EditableCurve, segment_index: usize) -> f32 {
    let left = curve.nodes.get(segment_index).copied();
    let right = curve.nodes.get(segment_index + 1).copied();
    match (left, right) {
        (Some(left_node), Some(right_node)) if right_node.y > left_node.y => -1.0,
        _ => 1.0,
    }
}

/// Convert vertical drag delta into internal segment tension delta.
///
/// Dragging upward (smaller `y`) always returns a positive visual bend amount.
fn tension_delta_from_drag_for_segment(
    curve: &EditableCurve,
    segment_index: usize,
    start_pointer: Point,
    raw_local_pointer: Point,
    curve_size: Size,
) -> f32 {
    let drag_units =
        (start_pointer.y - raw_local_pointer.y) as f32 / curve_tension_pixel_scale(curve_size);
    drag_units * segment_upward_tension_sign(curve, segment_index)
}

#[cfg(all(test, feature = "screenshot-test", not(target_os = "windows")))]
mod screenshot_tests {
    // Policy anchor: screenshot_renders_initial_ui
    include!("gui/screenshot_non_windows_tests.rs");
}

const fn u32_max(left: u32, right: u32) -> u32 {
    if left > right {
        left
    } else {
        right
    }
}

/// Enforce Pump's host-negotiated minimum window size.
fn constrained_host_size(size: GuiSize) -> GuiSize {
    GuiSize {
        width: size.width.max(WINDOW_WIDTH),
        height: size.height.max(WINDOW_HEIGHT),
    }
}

const fn resolve_vertical_slot_heights(total_height: u32) -> (u32, u32, u32) {
    let clamped_total = u32_max(total_height, 1);
    let header_h =
        clamped_total.saturating_mul(HEADER_SECTION_WEIGHT as u32) / ROOT_SECTION_WEIGHT_SUM;
    let controls_h =
        clamped_total.saturating_mul(CONTROLS_SECTION_WEIGHT as u32) / ROOT_SECTION_WEIGHT_SUM;
    let consumed = header_h.saturating_add(controls_h);
    let curve_h = clamped_total.saturating_sub(consumed);
    (header_h, curve_h, controls_h)
}

fn resolve_runtime_controls_slot_widths(total_width: u32) -> (u32, u32) {
    let widths = weighted_slot_lengths(
        total_width.max(1),
        &[KNOBS_SECTION_WEIGHT, DROPDOWN_SECTION_WEIGHT],
    );
    (
        widths.first().copied().unwrap_or(1),
        widths.get(1).copied().unwrap_or(1),
    )
}

fn scaled_line_height(text_scale: u32) -> u32 {
    BASE_CONTROL_LINE_UNIT.saturating_mul(text_scale.max(1))
}

/// Resolve a stable parameter id for one knob action key.
fn knob_param_id(key: &str) -> Option<ClapId> {
    match key {
        MIX_KEY => Some(PARAM_MIX_ID),
        DEPTH_KEY => Some(PARAM_DEPTH_ID),
        PHASE_KEY => Some(PARAM_PHASE_OFFSET_ID),
        OUTPUT_KEY => Some(PARAM_OUTPUT_GAIN_ID),
        _ => None,
    }
}

/// Shared Pump color/style tokens derived from the canonical Patchbay theme.
#[derive(Clone, Copy, Debug)]
struct PumpTheme {
    tokens: ThemeTokens,
    subtitle_text: Color,
    hint_text: Color,
    curve_bg: Color,
    curve_border: Color,
    curve_grid_vertical: Color,
    curve_grid_horizontal: Color,
    curve_line: Color,
    curve_line_highlight: Color,
    curve_line_highlight_glow: Color,
    preset_title_bg: Color,
    preset_title_dirty_bg: Color,
    preset_title_hover_bg: Color,
    preset_title_dirty_hover_bg: Color,
    preset_title_active_bg: Color,
    preset_title_dirty_active_bg: Color,
    preset_title_outline: Color,
    preset_add_warning_text: Color,
    preview_fill: Color,
    preview_stroke: Color,
    node_fill: Color,
    node_hover_fill: Color,
    node_selected_fill: Color,
    node_stroke: Color,
    node_hover_stroke: Color,
    node_selected_stroke: Color,
    node_hover_ring: Color,
    node_selected_ring: Color,
    playhead_dot_core: Color,
    playhead_dot_glow: Color,
    playhead_dot_stroke: Color,
    meter_outline: Color,
    meter_fill: Color,
}

impl PumpTheme {
    /// Return the canonical Pump GUI theme.
    fn main(metrics: UiLayoutMetrics) -> Self {
        let palette = MainPalette::main();
        let mut tokens = ThemeTokens::main();
        tokens.typography.text_scale = metrics.text_scale;
        tokens.controls.knob_diameter = metrics.knob_diameter;
        tokens.controls.dropdown_height = metrics.dropdown_control_h;
        tokens.controls.button_height = metrics.button_control_h;
        Self {
            tokens,
            subtitle_text: palette.syntax_emphasis,
            hint_text: palette.text_muted,
            curve_bg: palette.background_primary,
            curve_border: palette.ui_secondary,
            curve_grid_vertical: palette.background_secondary,
            curve_grid_horizontal: palette.ui_secondary,
            curve_line: palette.syntax_emphasis,
            curve_line_highlight: palette.accent_focus,
            curve_line_highlight_glow: palette.text_primary,
            preset_title_bg: palette.background_secondary,
            preset_title_dirty_bg: Color::rgb(150, 44, 44),
            preset_title_hover_bg: palette.syntax_emphasis,
            preset_title_dirty_hover_bg: Color::rgb(184, 64, 64),
            preset_title_active_bg: palette.accent_focus,
            preset_title_dirty_active_bg: Color::rgb(205, 84, 84),
            preset_title_outline: palette.ui_secondary,
            preset_add_warning_text: Color::rgb(255, 170, 170),
            preview_fill: palette.literals,
            preview_stroke: palette.identifiers,
            node_fill: palette.text_primary,
            node_hover_fill: palette.identifiers,
            node_selected_fill: palette.accent_focus,
            node_stroke: palette.ui_secondary,
            node_hover_stroke: palette.syntax_emphasis,
            node_selected_stroke: palette.text_primary,
            node_hover_ring: palette.syntax_emphasis,
            node_selected_ring: palette.accent_focus,
            playhead_dot_core: Color::rgba(255, 255, 255, 220),
            playhead_dot_glow: Color::rgba(
                palette.accent_focus.r,
                palette.accent_focus.g,
                palette.accent_focus.b,
                180,
            ),
            playhead_dot_stroke: palette.accent_focus,
            meter_outline: palette.ui_secondary,
            meter_fill: palette.literals,
        }
    }
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

    /// Inject one text character into the native editor window.
    #[cfg(any(feature = "vst3", windows))]
    pub fn post_text_char(&self, ch: char) -> bool {
        self.window.post_text_char(ch)
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

#[derive(Copy, Clone, Debug)]
enum CurveDragMode {
    MoveNode {
        index: usize,
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

/// Layout dimensions used to author Pump controls in design space.
///
/// Pump authors all widget geometry at a fixed logical design resolution.
/// Patchbay applies uniform root scaling at render time so host window size
/// changes do not alter declarative layout structure.
#[derive(Clone, Copy, Debug)]
struct UiLayoutMetrics {
    content_w: u32,
    content_h: u32,
    curve_h: u32,
    controls_gap: i32,
    meter_x_offset: i32,
    meter_y_offset: i32,
    meter_width: u32,
    meter_stroke: u32,
    dropdown_control_w: u32,
    dropdown_control_h: u32,
    button_control_h: u32,
    transport_indicator_size: u32,
    curve_size: Size,
    knob_track_w: u32,
    knob_diameter: u32,
    text_scale: u32,
    label_line_h: u32,
}

impl UiLayoutMetrics {
    /// Resolve all layout dimensions from the fixed design resolution.
    fn design_space() -> Self {
        let content_w = WINDOW_WIDTH;
        let content_h = WINDOW_HEIGHT;
        let (_header_h, curve_h, controls_h) = resolve_vertical_slot_heights(content_h);
        let (knobs_slot_w, dropdown_slot_w) = resolve_runtime_controls_slot_widths(content_w);
        let controls_gap = HEADER_CONTROL_GAP.max(0);
        let text_scale = BASE_TEXT_SCALE.max(1);
        let knob_track_width = knobs_slot_w.saturating_div(KNOBS_PER_ROW as u32);
        let knob_diameter = BASE_KNOB_DIAMETER.min(knob_track_width.max(1));
        let knob_track_w = knob_diameter.max(1);
        let label_line_h = scaled_line_height(text_scale);
        let expanded_control_h = controls_h
            .saturating_sub(label_line_h)
            .saturating_sub((controls_gap.max(0) as u32).saturating_mul(2))
            .saturating_div(2)
            .max(BASE_DROPDOWN_CONTROL_H.max(1));
        let dropdown_control_h = expanded_control_h;
        let button_control_h = expanded_control_h;
        let dropdown_control_w = dropdown_slot_w.max(1);
        let transport_indicator_size = TRANSPORT_INDICATOR_SIZE.max(1);
        let curve_size = Size {
            width: content_w,
            height: curve_h,
        };
        let meter_x_offset = METER_X_OFFSET.max(0);
        let meter_y_offset = METER_Y_OFFSET.max(0);
        let meter_width = METER_WIDTH.max(0) as u32;
        let meter_stroke = METER_STROKE.max(0) as u32;
        Self {
            content_w,
            content_h,
            curve_h,
            controls_gap,
            meter_x_offset,
            meter_y_offset,
            meter_width,
            meter_stroke,
            dropdown_control_w,
            dropdown_control_h,
            button_control_h,
            transport_indicator_size,
            curve_size,
            knob_track_w,
            knob_diameter,
            text_scale,
            label_line_h,
        }
    }
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
    warning_text: Option<&'static str>,
}

/// Snapshot of curve-editor hover/selection state used for drawing.
#[derive(Clone, Copy, Debug)]
struct CurveRenderState {
    selected_node: Option<usize>,
    hovered_node: Option<usize>,
    hovered_segment: Option<usize>,
    preview_node: Option<CurveNode>,
}

impl GuiRuntime {
    fn new() -> Self {
        Self {
            selected_node: None,
            drag_mode: None,
            curve_hovered: false,
            curve_local_pointer: Point { x: 0, y: 0 },
            curve_size: Size {
                width: CURVE_W,
                height: CURVE_H,
            },
            last_division_change_micros: None,
            preset_rename_active: false,
            preset_rename_target: 0,
            preset_name_draft: String::new(),
            preset_warning_frames: 0,
            preset_warning_text: None,
            pointer_primary_down: false,
            active_knob_gesture_param: None,
        }
    }
}

impl GuiState {
    fn new(
        params: Arc<PumpParams>,
        status: Arc<GuiStatus>,
        automation_queue: Arc<AutomationQueue>,
        param_requester: Option<HostParamRequester>,
    ) -> Self {
        Self {
            params,
            status,
            automation_queue,
            automation_config: AutomationConfig::default(),
            param_requester,
            runtime: Mutex::new(GuiRuntime::new()),
        }
    }

    /// Snapshot runtime pointer/selection state and update curve dimensions.
    fn snapshot_curve_runtime(&self, curve_size: Size) -> (Option<usize>, bool, Point) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.curve_size = curve_size;
            (
                runtime.selected_node,
                runtime.curve_hovered,
                runtime.curve_local_pointer,
            )
        } else {
            (None, false, Point { x: 0, y: 0 })
        }
    }

    /// Snapshot current plugin control values for UI rendering.
    fn snapshot_controls(&self) -> ControlSnapshot {
        ControlSnapshot {
            mix: self.params.mix(),
            depth: self.params.depth(),
            phase_offset: self.params.phase_offset(),
            output_gain_db: self.params.output_gain_db(),
            division: self.params.sync_division(),
        }
    }

    /// Snapshot preset-bank state and transient header interaction flags.
    fn snapshot_presets(&self) -> PresetSnapshot {
        let bank = self.params.preset_bank_snapshot();
        let names = if bank.presets.is_empty() {
            vec![DEFAULT_PRESET_NAME.to_string()]
        } else {
            bank.presets
                .iter()
                .map(|preset| preset.name.clone())
                .collect()
        };
        let selected = bank.selected.min(names.len().saturating_sub(1));
        let dirty = self.params.current_state_differs_from_selected_preset();

        let mut rename_active = false;
        let mut rename_draft = String::new();
        let mut warning_text = None;
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.preset_rename_active {
                runtime.preset_rename_target = runtime
                    .preset_rename_target
                    .min(names.len().saturating_sub(1));
                rename_active = true;
                rename_draft = runtime.preset_name_draft.clone();
            }
            if runtime.preset_warning_frames > 0 {
                warning_text = runtime.preset_warning_text;
                runtime.preset_warning_frames = runtime.preset_warning_frames.saturating_sub(1);
            }
        }

        PresetSnapshot {
            names,
            selected,
            dirty,
            rename_active,
            rename_draft,
            warning_text,
        }
    }

    fn mark_division_change(&self) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.last_division_change_micros = Some(monotonic_micros());
        }
    }

    fn set_preset_warning(&self, text: &'static str) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.preset_warning_text = Some(text);
            runtime.preset_warning_frames = PRESET_WARNING_FRAMES;
        }
    }

    fn consume_recent_division_change_guard(&self) -> bool {
        let Ok(mut runtime) = self.runtime.lock() else {
            return false;
        };
        let Some(last_division_change_micros) = runtime.last_division_change_micros else {
            return false;
        };
        let elapsed = monotonic_micros().saturating_sub(last_division_change_micros);
        if elapsed <= RESET_GUARD_AFTER_DROPDOWN_MICROS {
            // Swallow one immediate reset press after dropdown selection to avoid
            // accidental click-through while popup selection closes.
            runtime.last_division_change_micros = None;
            return true;
        }
        false
    }

    /// Build the top header slot node.
    fn build_header_slot(
        &self,
        metrics: UiLayoutMetrics,
        theme: PumpTheme,
        presets: &PresetSnapshot,
    ) -> Node {
        let header_h = resolve_vertical_slot_heights(metrics.content_h).0.max(1);
        let header_slot_widths = weighted_slot_lengths(
            metrics.content_w.max(1),
            &[
                HEADER_EMPTY_SECTION_PERCENT as u16,
                HEADER_INDICATOR_SECTION_PERCENT as u16,
            ],
        );
        let left_width = header_slot_widths.first().copied().unwrap_or(1).max(1);
        let action_button_width = (left_width / 8).max(metrics.transport_indicator_size.max(1));
        let preset_title_width = left_width.saturating_sub(action_button_width).max(1);
        let indicator_node = Node::align_box(
            indicator(
                Size {
                    width: metrics.transport_indicator_size,
                    height: metrics.transport_indicator_size,
                },
                self.status.transport_beat_blink_active(),
            )
            .widget_layout(fixed_box(
                metrics.transport_indicator_size,
                metrics.transport_indicator_size,
            )),
        )
        .slot_align(SlotAlign::Center, SlotAlign::Center)
        .fill();

        let preset_dropdown_or_edit = if presets.rename_active {
            textbox(presets.rename_draft.clone())
                .text_color(theme.subtitle_text)
                .text_editable(PRESET_RENAME_KEY, true)
                .text_edit_max_chars(MAX_PRESET_NAME_CHARS)
                .widget_layout(LayoutBox::fill())
                .fill()
        } else {
            dropdown(
                PRESET_DROPDOWN_KEY,
                presets.names.len().max(1),
                presets.selected.min(presets.names.len().saturating_sub(1)),
            )
            .dropdown_option_labels(presets.names.clone())
            .control_size(Size {
                width: preset_title_width,
                height: header_h,
            })
            .dropdown_background_color(if presets.dirty {
                theme.preset_title_dirty_bg
            } else {
                theme.preset_title_bg
            })
            .dropdown_hover_background_color(if presets.dirty {
                theme.preset_title_dirty_hover_bg
            } else {
                theme.preset_title_hover_bg
            })
            .dropdown_active_background_color(if presets.dirty {
                theme.preset_title_dirty_active_bg
            } else {
                theme.preset_title_active_bg
            })
            .dropdown_outline_color(theme.preset_title_outline)
            .dropdown_text_color(theme.subtitle_text)
            .fill()
        };
        let preset_title = panel("preset-title", preset_dropdown_or_edit.fill())
            .background(if presets.dirty {
                theme.preset_title_dirty_bg
            } else {
                theme.preset_title_bg
            })
            .outline(theme.preset_title_outline)
            .pad_all(0)
            .fill();

        let action_button_slot = |node: Node| {
            Slot::with_params(
                node,
                SlotParams::intrinsic()
                    .cross_size(SlotCrossSize::Intrinsic)
                    .align(SlotAlign::Start, SlotAlign::Center),
            )
        };

        let rename_button = button(PRESET_RENAME_BUTTON_KEY)
            .button_label("R")
            .control_size(Size {
                width: action_button_width,
                height: header_h,
            })
            .fill();
        let save_button = button(PRESET_SAVE_KEY)
            .button_label("S")
            .control_size(Size {
                width: action_button_width,
                height: header_h,
            })
            .fill();
        let add_button = button(PRESET_ADD_KEY)
            .button_label("+")
            .control_size(Size {
                width: action_button_width,
                height: header_h,
            })
            .fill();
        let action_buttons = row_slots(vec![
            action_button_slot(rename_button),
            action_button_slot(save_button),
            action_button_slot(add_button),
            weighted_slot(
                spacer(Size {
                    width: 1,
                    height: 1,
                }),
                1,
            ),
        ])
        .gap(0)
        .container_overflow(OverflowPolicy::Compress)
        .fill();
        let left_controls = row_slots(vec![
            weighted_slot(preset_title, 82),
            weighted_slot(action_buttons, 18),
        ])
        .gap(HEADER_CONTROL_GAP.max(0))
        .container_overflow(OverflowPolicy::Compress)
        .fill();
        let left_content = if let Some(warning_text) = presets.warning_text {
            let warning_row = Node::align_box(
                textbox(warning_text)
                    .text_color(theme.preset_add_warning_text)
                    .widget_layout(LayoutBox::fill()),
            )
            .slot_align(SlotAlign::End, SlotAlign::Center)
            .fill();
            column_slots(vec![
                weighted_slot(left_controls, 82),
                weighted_slot(warning_row, 18),
            ])
            .gap(0)
            .container_overflow(OverflowPolicy::Compress)
            .fill()
        } else {
            left_controls
        };

        let header_content = row_slots(vec![
            weighted_slot(left_content, HEADER_EMPTY_SECTION_PERCENT as u16),
            weighted_slot(indicator_node, HEADER_INDICATOR_SECTION_PERCENT as u16),
        ])
        .container_overflow(OverflowPolicy::Compress);
        panel("header", header_content).pad_all(0)
    }

    /// Build the spline/curve slot node.
    fn build_spline_slot(
        &self,
        metrics: UiLayoutMetrics,
        theme: PumpTheme,
        draw_commands: Vec<SurfaceCommand>,
    ) -> Node {
        let spline_content = surface(
            CURVE_KEY,
            Size {
                width: metrics.content_w,
                height: metrics.curve_h,
            },
            draw_commands,
        )
        .fill();
        panel("spline", spline_content)
            .background(theme.curve_bg)
            .outline(theme.curve_border)
            .pad_all(0)
    }

    /// Build the controls slot node.
    fn build_controls_slot(
        &self,
        metrics: UiLayoutMetrics,
        theme: PumpTheme,
        controls: ControlSnapshot,
    ) -> Node {
        const KNOB_TEXT_MAX_CHARS: u32 = 8;
        const MONO_CHAR_CELL_WIDTH_PX: u32 = 6;
        let knob_text_scale = metrics
            .knob_track_w
            .saturating_div(KNOB_TEXT_MAX_CHARS.saturating_mul(MONO_CHAR_CELL_WIDTH_PX))
            .max(1)
            .min(metrics.text_scale.max(1));
        let knob_label_h = metrics
            .label_line_h
            .max(scaled_line_height(knob_text_scale));
        let knob_cell = |key: &'static str,
                         label: &'static str,
                         value: f32,
                         range: (f32, f32),
                         value_text: String| {
            let title = Node::align_box(
                textbox(label)
                    .text_align_center()
                    .text_color(theme.subtitle_text)
                    .widget_layout(fixed_box(metrics.knob_track_w, knob_label_h)),
            )
            .slot_align(SlotAlign::Center, SlotAlign::Start)
            .fill();
            let value_label = Node::align_box(
                textbox(value_text)
                    .text_align_center()
                    .text_color(theme.hint_text)
                    .widget_layout(fixed_box(metrics.knob_track_w, knob_label_h)),
            )
            .slot_align(SlotAlign::Center, SlotAlign::End)
            .fill();
            let knob_body = Node::align_box(
                knob(key, value, range)
                    .control_size(Size {
                        width: metrics.knob_diameter,
                        height: metrics.knob_diameter,
                    })
                    .widget_layout(fixed_box(metrics.knob_diameter, metrics.knob_diameter)),
            )
            .slot_align(SlotAlign::Center, SlotAlign::Center)
            .fill();
            column_slots(vec![
                weighted_slot(title, 15),
                weighted_slot(knob_body, 70),
                weighted_slot(value_label, 15),
            ])
            .gap(0)
            .container_overflow(OverflowPolicy::Compress)
        };
        let knobs_grid = grid(
            GridTemplate::new(vec![TrackSize::Auto; KNOBS_PER_ROW])
                .rows(vec![TrackSize::Auto])
                .gap(0)
                .justify_start(),
            vec![
                knob_cell(
                    MIX_KEY,
                    "Mix",
                    controls.mix,
                    (MIN_MIX, MAX_MIX),
                    format!("{:.0}%", controls.mix * 100.0),
                ),
                knob_cell(
                    DEPTH_KEY,
                    "Depth",
                    controls.depth,
                    (MIN_DEPTH, MAX_DEPTH),
                    format!("{:.0}%", controls.depth * 100.0),
                ),
                knob_cell(
                    PHASE_KEY,
                    "Phase",
                    controls.phase_offset,
                    (MIN_PHASE_OFFSET, MAX_PHASE_OFFSET),
                    format!("{:.0}%", controls.phase_offset * 100.0),
                ),
                knob_cell(
                    OUTPUT_KEY,
                    "Output",
                    controls.output_gain_db,
                    (MIN_OUTPUT_GAIN_DB, MAX_OUTPUT_GAIN_DB),
                    format!("{:+.0}dB", controls.output_gain_db),
                ),
            ],
        )
        .fill()
        .container_overflow(OverflowPolicy::Compress);

        let knobs_slot = panel("knobs", knobs_grid.fill()).pad_all(0);
        let dropdown_slot_content = column(vec![
            dropdown(
                DIVISION_KEY,
                MAX_SYNC_DIVISION as usize + 1,
                controls.division.min(MAX_SYNC_DIVISION as usize),
            )
            .dropdown_option_labels(
                (0..=MAX_SYNC_DIVISION as usize)
                    .map(|index| sync_division_label(index).to_string())
                    .collect(),
            )
            .control_size(Size {
                width: metrics.dropdown_control_w,
                height: metrics.dropdown_control_h,
            })
            .fill(),
            button(RESET_KEY)
                .button_label("Reset")
                .control_size(Size {
                    width: metrics.dropdown_control_w,
                    height: metrics.button_control_h,
                })
                .fill(),
        ])
        .gap(metrics.controls_gap.max(0))
        .pad_all(0)
        .fill()
        .container_overflow(OverflowPolicy::Compress);
        let dropdown_slot = panel("dropdown", dropdown_slot_content.fill()).pad_all(0);

        let controls_row = row_slots(vec![
            weighted_slot(knobs_slot, KNOBS_SECTION_WEIGHT),
            weighted_slot(dropdown_slot, DROPDOWN_SECTION_WEIGHT)
                .align(SlotAlign::End, SlotAlign::Start),
        ])
        .container_overflow(OverflowPolicy::Compress);
        panel("controls", controls_row).pad_all(0)
    }

    /// Resolve curve-hover and preview state for frame rendering.
    fn compute_curve_render_state(
        &self,
        editable_curve: &EditableCurve,
        selected_node: Option<usize>,
        curve_hovered: bool,
        curve_local_pointer: Point,
        alt_down: bool,
        curve_size: Size,
    ) -> CurveRenderState {
        let hovered_node = curve_hovered
            .then(|| {
                find_node_hit_for_size(
                    editable_curve,
                    curve_local_pointer,
                    node_hit_radius(curve_size),
                    curve_size,
                )
            })
            .flatten();
        let direct_segment = curve_hovered
            .then(|| {
                find_segment_line_hit_within_for_size(
                    editable_curve,
                    curve_local_pointer,
                    segment_direct_hit_radius(curve_size),
                    curve_size,
                )
            })
            .flatten();
        let preview_node = (curve_hovered
            && !alt_down
            && hovered_node.is_none()
            && direct_segment.is_some())
        .then(|| preview_node_on_curve_for_size(editable_curve, curve_local_pointer, curve_size))
        .flatten();
        let hovered_segment = (curve_hovered && preview_node.is_none())
            .then(|| {
                find_segment_line_hit_within_for_size(
                    editable_curve,
                    curve_local_pointer,
                    segment_near_hit_radius(curve_size),
                    curve_size,
                )
            })
            .flatten();
        CurveRenderState {
            selected_node,
            hovered_node,
            hovered_segment,
            preview_node,
        }
    }

    /// Build the root UI spec for the current frame dimensions and content tree.
    fn build_root_spec(&self, metrics: UiLayoutMetrics, theme: PumpTheme, content: Node) -> UiSpec {
        let design_size = Size {
            width: metrics.content_w,
            height: metrics.content_h,
        };
        UiSpec::new(
            root_frame_sized(ROOT_KEY, content, design_size)
                .padding(0)
                .scale_mode(RootScaleMode::UniformFit)
                .tokens(theme.tokens),
        )
    }

    /// Build curve draw commands for the current frame input and runtime state.
    fn build_curve_commands_for_frame(
        &self,
        input: &InputState,
        metrics: UiLayoutMetrics,
        theme: PumpTheme,
    ) -> Vec<SurfaceCommand> {
        let (selected_node, curve_hovered, curve_local_pointer) =
            self.snapshot_curve_runtime(metrics.curve_size);
        let editable_curve = self.params.editable_curve_snapshot();
        let curve_state = self.compute_curve_render_state(
            &editable_curve,
            selected_node,
            curve_hovered,
            curve_local_pointer,
            input.alt_down,
            metrics.curve_size,
        );
        self.build_curve_draw_commands(&editable_curve, metrics, curve_state, &theme)
    }

    fn build_ui(&self, input: &InputState) -> UiSpec {
        self.sync_knob_gesture_state(input.mouse_down);
        let metrics = UiLayoutMetrics::design_space();
        let theme = PumpTheme::main(metrics);
        let controls = self.snapshot_controls();
        let presets = self.snapshot_presets();
        let draw_commands = self.build_curve_commands_for_frame(input, metrics, theme);

        let header_slot = self.build_header_slot(metrics, theme, &presets);
        let spline_slot = self.build_spline_slot(metrics, theme, draw_commands);
        let controls_slot = self.build_controls_slot(metrics, theme, controls);

        let content = column_slots(vec![
            weighted_slot(header_slot, HEADER_SECTION_WEIGHT),
            weighted_slot(spline_slot, CURVE_SECTION_WEIGHT),
            weighted_slot(controls_slot, CONTROLS_SECTION_WEIGHT),
        ])
        .container_overflow(OverflowPolicy::Compress);
        self.build_root_spec(metrics, theme, content)
    }

    fn measured_open_size(&self) -> (u32, u32) {
        // Open at baseline design size so initial rendering is true 1:1.
        (WINDOW_WIDTH, WINDOW_HEIGHT)
    }

    fn reduce_action(&mut self, action: UiAction) {
        match action {
            UiAction::KnobChanged { key, value } => self.reduce_knob(key.as_str(), value),
            UiAction::DropdownSelected { key, index } => self.reduce_dropdown(key.as_str(), index),
            UiAction::ButtonPressed { key } if key == PRESET_RENAME_BUTTON_KEY => {
                self.begin_preset_rename();
            }
            UiAction::ButtonPressed { key } if key == RESET_KEY => {
                if self.consume_recent_division_change_guard() {
                    return;
                }
                self.params.reset_curve_to_default();
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.selected_node = None;
                    runtime.drag_mode = None;
                }
            }
            UiAction::ButtonPressed { key } if key == PRESET_ADD_KEY => {
                if self.params.add_preset_from_current_state().is_some() {
                    if let Ok(mut runtime) = self.runtime.lock() {
                        runtime.preset_rename_active = false;
                        runtime.preset_name_draft.clear();
                        runtime.preset_warning_frames = 0;
                        runtime.preset_warning_text = None;
                    }
                } else {
                    self.set_preset_warning(PRESET_WARNING_MAX);
                }
            }
            UiAction::ButtonPressed { key } if key == PRESET_SAVE_KEY => {
                self.save_current_preset_by_name();
            }
            UiAction::TextBoxEdited { key, text } if key == PRESET_RENAME_KEY => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.preset_name_draft = text;
                }
            }
            UiAction::TextBoxEditCommitted { key, text } if key == PRESET_RENAME_KEY => {
                self.commit_preset_rename(text.as_str());
            }
            UiAction::TextBoxEditCanceled { key } if key == PRESET_RENAME_KEY => {
                self.cancel_preset_rename();
            }
            UiAction::RegionHover {
                key,
                hovered,
                local_pointer,
            } if key == CURVE_KEY => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.curve_hovered = hovered;
                    runtime.curve_local_pointer =
                        scale_point_to_design(local_pointer, runtime.curve_size);
                }
            }
            UiAction::RegionHover { .. } => {}
            UiAction::RegionInteracted {
                key,
                kind,
                local_pointer,
                raw_local_pointer,
                alt_down,
            } if key == CURVE_KEY => {
                self.reduce_curve_interaction(kind, local_pointer, raw_local_pointer, alt_down)
            }
            UiAction::RegionInteracted { .. } => {}
            _ => {}
        }
    }

    fn reduce_knob(&mut self, key: &str, value: f32) {
        let Some(param_id) = knob_param_id(key) else {
            return;
        };
        match key {
            MIX_KEY => {
                self.params.set_mix(value);
            }
            DEPTH_KEY => {
                self.params.set_depth(value);
            }
            PHASE_KEY => {
                self.params.set_phase_offset(value);
            }
            OUTPUT_KEY => {
                self.params.set_output_gain_db(value);
            }
            _ => return,
        }
        self.push_knob_value_update(param_id, value as f64);
    }

    fn reduce_dropdown(&mut self, key: &str, index: usize) {
        if key == PRESET_DROPDOWN_KEY {
            if let Some(selected) = self.params.load_preset(index) {
                self.push_all_param_updates();
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.preset_rename_active = false;
                    runtime.preset_name_draft.clear();
                    runtime.preset_rename_target = selected;
                    runtime.preset_warning_frames = 0;
                    runtime.preset_warning_text = None;
                }
            }
            return;
        }

        if key != DIVISION_KEY {
            return;
        }

        let clamped = index.min(MAX_SYNC_DIVISION as usize);
        self.mark_division_change();
        self.params.set_sync_division(clamped as f32);
        self.push_single_value_update(PARAM_SYNC_DIVISION_ID, clamped as f64);
    }

    fn begin_preset_rename(&self) {
        let bank = self.params.preset_bank_snapshot();
        if bank.presets.is_empty() {
            return;
        }
        let selected = bank.selected.min(bank.presets.len().saturating_sub(1));
        if self.params.is_preset_read_only(selected) {
            self.set_preset_warning(PRESET_WARNING_INIT);
            return;
        }
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.preset_rename_target = selected;
            runtime.preset_name_draft = bank.presets[runtime.preset_rename_target].name.clone();
            runtime.preset_rename_active = true;
            runtime.preset_warning_frames = 0;
            runtime.preset_warning_text = None;
        }
    }

    fn commit_preset_rename(&self, text: &str) {
        let Ok(mut runtime) = self.runtime.lock() else {
            return;
        };
        if !runtime.preset_rename_active {
            return;
        }
        let target = runtime.preset_rename_target;
        let renamed = self.params.rename_preset(target, text);
        runtime.preset_rename_active = false;
        runtime.preset_name_draft.clear();
        if renamed {
            runtime.preset_warning_frames = 0;
            runtime.preset_warning_text = None;
        } else {
            runtime.preset_warning_text = Some(PRESET_WARNING_INIT);
            runtime.preset_warning_frames = PRESET_WARNING_FRAMES;
        }
    }

    fn cancel_preset_rename(&self) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.preset_rename_active = false;
            runtime.preset_name_draft.clear();
        }
    }

    fn save_current_preset_by_name(&self) {
        let bank = self.params.preset_bank_snapshot();
        let selected = bank.selected.min(bank.presets.len().saturating_sub(1));
        let fallback_name = bank
            .presets
            .get(selected)
            .map(|preset| preset.name.clone())
            .unwrap_or_else(|| DEFAULT_PRESET_NAME.to_string());
        let candidate = self
            .runtime
            .lock()
            .ok()
            .and_then(|runtime| {
                if runtime.preset_rename_active {
                    Some(runtime.preset_name_draft.clone())
                } else {
                    None
                }
            })
            .unwrap_or(fallback_name);
        match self.params.save_current_state_by_name(&candidate) {
            SavePresetOutcome::Overwritten { index } | SavePresetOutcome::Created { index } => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.preset_rename_active = false;
                    runtime.preset_name_draft.clear();
                    runtime.preset_rename_target = index;
                    runtime.preset_warning_frames = 0;
                    runtime.preset_warning_text = None;
                }
            }
            SavePresetOutcome::BlockedReadOnly => self.set_preset_warning(PRESET_WARNING_INIT),
            SavePresetOutcome::BlockedFull => self.set_preset_warning(PRESET_WARNING_MAX),
            SavePresetOutcome::InvalidName => self.set_preset_warning(PRESET_WARNING_NAME),
        }
    }

    fn reduce_curve_interaction(
        &mut self,
        kind: RegionInteractionKind,
        local_pointer: Point,
        raw_local_pointer: Point,
        alt_down: bool,
    ) {
        let Ok(mut runtime) = self.runtime.lock() else {
            return;
        };

        let local_pointer = scale_point_to_design(local_pointer, runtime.curve_size);
        let raw_local_pointer = scale_point_to_design(raw_local_pointer, runtime.curve_size);
        let normalized_pointer = node_from_local_for_size(local_pointer, runtime.curve_size);
        let raw_normalized_pointer =
            node_from_local_for_size(raw_local_pointer, runtime.curve_size);

        match kind {
            RegionInteractionKind::Pressed => {
                let mut editable = self.params.editable_curve_snapshot();
                if let Some(index) = find_node_hit_for_size(
                    &editable,
                    local_pointer,
                    node_hit_radius(runtime.curve_size),
                    runtime.curve_size,
                ) {
                    runtime.selected_node = Some(index);
                    runtime.drag_mode = Some(CurveDragMode::MoveNode {
                        index,
                        start_pointer: local_pointer,
                        dragging: false,
                    });
                    return;
                }

                if !alt_down
                    && find_segment_line_hit_within_for_size(
                        &editable,
                        local_pointer,
                        segment_direct_hit_radius(runtime.curve_size),
                        runtime.curve_size,
                    )
                    .is_some()
                {
                    let preview_node = preview_node_on_curve_for_size(
                        &editable,
                        local_pointer,
                        runtime.curve_size,
                    )
                    .unwrap_or(normalized_pointer);
                    let inserted_index =
                        insert_node_for_size(&mut editable, preview_node, runtime.curve_size);
                    runtime.selected_node = Some(inserted_index);
                    runtime.drag_mode = Some(CurveDragMode::MoveNode {
                        index: inserted_index,
                        start_pointer: local_pointer,
                        dragging: false,
                    });
                    enforce_wrapped_endpoints(&mut editable);
                    self.params.set_editable_curve(&editable);
                    return;
                }

                if let Some(index) = find_segment_line_hit_within_for_size(
                    &editable,
                    local_pointer,
                    segment_near_hit_radius(runtime.curve_size),
                    runtime.curve_size,
                ) {
                    runtime.drag_mode = if alt_down {
                        let start_tension = editable
                            .segments
                            .get(index)
                            .copied()
                            .unwrap_or(CurveSegment { tension: 0.0 })
                            .tension;
                        Some(CurveDragMode::AdjustSegmentCurve {
                            index,
                            start_pointer: local_pointer,
                            start_tension,
                            dragging: false,
                        })
                    } else {
                        let right_index = (index + 1).min(editable.nodes.len().saturating_sub(1));
                        Some(CurveDragMode::MoveSegment {
                            index,
                            start_pointer: local_pointer,
                            start_left_x: editable.nodes[index].x,
                            start_right_x: editable.nodes[right_index].x,
                            start_left_y: editable.nodes[index].y,
                            start_right_y: editable.nodes[right_index].y,
                            dragging: false,
                        })
                    };
                    return;
                }

                if let Some(index) = find_node_hit_within_for_size(
                    &editable,
                    local_pointer,
                    node_insert_guard_radius(runtime.curve_size),
                    runtime.curve_size,
                ) {
                    runtime.selected_node = Some(index);
                    runtime.drag_mode = Some(CurveDragMode::MoveNode {
                        index,
                        start_pointer: local_pointer,
                        dragging: false,
                    });
                    return;
                }

                let inserted_index =
                    insert_node_for_size(&mut editable, normalized_pointer, runtime.curve_size);
                runtime.selected_node = Some(inserted_index);
                runtime.drag_mode = Some(CurveDragMode::MoveNode {
                    index: inserted_index,
                    start_pointer: local_pointer,
                    dragging: false,
                });
                enforce_wrapped_endpoints(&mut editable);
                self.params.set_editable_curve(&editable);
            }
            RegionInteractionKind::Dragged => {
                if let Some(mut drag_mode) = runtime.drag_mode {
                    let mut editable = self.params.editable_curve_snapshot();
                    let mut curve_changed = false;
                    match drag_mode {
                        CurveDragMode::MoveNode {
                            index,
                            start_pointer,
                            mut dragging,
                        } => {
                            if !dragging
                                && !drag_threshold_crossed(
                                    start_pointer,
                                    local_pointer,
                                    curve_drag_threshold_px(runtime.curve_size),
                                )
                            {
                                return;
                            }
                            dragging = true;
                            let moved_index = move_node_with_push_through_for_size(
                                &mut editable,
                                index,
                                raw_normalized_pointer,
                                node_push_through_threshold_px(runtime.curve_size),
                                runtime.curve_size,
                            );
                            runtime.selected_node = Some(moved_index);
                            drag_mode = CurveDragMode::MoveNode {
                                index: moved_index,
                                start_pointer,
                                dragging,
                            };
                            curve_changed = true;
                        }
                        CurveDragMode::MoveSegment {
                            index,
                            start_pointer,
                            start_left_x,
                            start_right_x,
                            start_left_y,
                            start_right_y,
                            mut dragging,
                        } => {
                            if !dragging
                                && !drag_threshold_crossed(
                                    start_pointer,
                                    local_pointer,
                                    curve_drag_threshold_px(runtime.curve_size),
                                )
                            {
                                return;
                            }
                            dragging = true;
                            let curve_width = runtime.curve_size.width.max(2);
                            let curve_height = runtime.curve_size.height.max(2);
                            let delta_x = (raw_local_pointer.x - start_pointer.x) as f32
                                / (curve_width - 1) as f32;
                            let delta_y = (start_pointer.y - raw_local_pointer.y) as f32
                                / (curve_height - 1) as f32;
                            move_segment_translated(
                                &mut editable,
                                index,
                                (start_left_x, start_left_y),
                                (start_right_x, start_right_y),
                                (delta_x, delta_y),
                            );
                            drag_mode = CurveDragMode::MoveSegment {
                                index,
                                start_pointer,
                                start_left_x,
                                start_right_x,
                                start_left_y,
                                start_right_y,
                                dragging,
                            };
                            curve_changed = true;
                        }
                        CurveDragMode::AdjustSegmentCurve {
                            index,
                            start_pointer,
                            start_tension,
                            mut dragging,
                        } => {
                            if !dragging
                                && !drag_threshold_crossed(
                                    start_pointer,
                                    local_pointer,
                                    curve_drag_threshold_px(runtime.curve_size),
                                )
                            {
                                return;
                            }
                            dragging = true;
                            let delta = tension_delta_from_drag_for_segment(
                                &editable,
                                index,
                                start_pointer,
                                raw_local_pointer,
                                runtime.curve_size,
                            );
                            if let Some(segment) = editable.segments.get_mut(index) {
                                segment.tension = (start_tension + delta)
                                    .clamp(MIN_SEGMENT_TENSION, MAX_SEGMENT_TENSION);
                                curve_changed = true;
                            }
                            drag_mode = CurveDragMode::AdjustSegmentCurve {
                                index,
                                start_pointer,
                                start_tension,
                                dragging,
                            };
                        }
                    }
                    runtime.drag_mode = Some(drag_mode);
                    if curve_changed {
                        enforce_wrapped_endpoints(&mut editable);
                        self.params.set_editable_curve(&editable);
                    }
                }
            }
            RegionInteractionKind::Released => {
                runtime.drag_mode = None;
            }
            RegionInteractionKind::SecondaryClicked => {}
            RegionInteractionKind::DoubleClicked => {
                let mut editable = self.params.editable_curve_snapshot();
                if let Some(index) =
                    find_deletable_node_hit_for_size(&editable, local_pointer, runtime.curve_size)
                {
                    editable.nodes.remove(index);
                    let remove_segment = index
                        .saturating_sub(1)
                        .min(editable.segments.len().saturating_sub(1));
                    if !editable.segments.is_empty() {
                        editable.segments.remove(remove_segment);
                    }
                    enforce_wrapped_endpoints(&mut editable);
                    runtime.selected_node = None;
                    runtime.drag_mode = None;
                    self.params.set_editable_curve(&editable);
                }
            }
        }
    }

    fn build_curve_draw_commands(
        &self,
        editable_curve: &EditableCurve,
        metrics: UiLayoutMetrics,
        state: CurveRenderState,
        theme: &PumpTheme,
    ) -> Vec<SurfaceCommand> {
        let curve_size = metrics.curve_size;
        let rect = Rect {
            origin: Point { x: 0, y: 0 },
            size: Size {
                width: curve_size.width,
                height: curve_size.height,
            },
        };
        let to_canvas = |point: Point| scale_point_from_design(point, curve_size);
        let border_stroke = scaled_curve_u32(METER_STROKE.max(0) as u32, curve_size);
        let node_radius = scaled_curve_i32(NODE_DRAW_RADIUS, curve_size);
        let node_hover_radius = scaled_curve_i32(NODE_DRAW_RADIUS + 1, curve_size);
        let node_preview_radius = scaled_curve_i32(NODE_DRAW_RADIUS + 1, curve_size);
        let node_preview_stroke_radius = scaled_curve_i32(NODE_DRAW_RADIUS + 2, curve_size);
        let node_ring_radius = scaled_curve_i32(NODE_DRAW_RADIUS + 3, curve_size);
        let playhead_core_radius = scaled_curve_i32(PLAYHEAD_DOT_CORE_RADIUS, curve_size);
        let playhead_glow_radius = scaled_curve_i32(PLAYHEAD_DOT_GLOW_RADIUS, curve_size);
        let node_stroke = scaled_curve_i32(METER_STROKE, curve_size);
        let highlight_offset = scaled_curve_i32(1, curve_size);
        let meter_x_offset = metrics.meter_x_offset.max(0);
        let meter_y_offset = metrics.meter_y_offset.max(0);
        let meter_width = metrics.meter_width.max(1);
        let meter_width_i32 = i32::try_from(meter_width).unwrap_or(i32::MAX);
        let meter_stroke_u32 = metrics.meter_stroke.max(1);
        let meter_inner_width = metrics
            .meter_width
            .max(1)
            .saturating_sub(meter_stroke_u32.saturating_mul(2));

        let mut commands = Vec::with_capacity(1024);
        commands.push(SurfaceCommand::FillRect {
            rect,
            color: theme.curve_bg,
        });
        commands.push(SurfaceCommand::StrokeRect {
            rect,
            thickness: border_stroke,
            color: theme.curve_border,
        });

        for step in 1..16 {
            let x = ((curve_size.width as i32 - 1) * step) / 16;
            commands.push(SurfaceCommand::Line {
                start: Point { x, y: 0 },
                end: Point {
                    x,
                    y: curve_size.height as i32 - 1,
                },
                color: theme.curve_grid_vertical,
            });
        }

        for step in 1..4 {
            let y = ((curve_size.height as i32 - 1) * step) / 4;
            commands.push(SurfaceCommand::Line {
                start: Point { x: 0, y },
                end: Point {
                    x: curve_size.width as i32 - 1,
                    y,
                },
                color: theme.curve_grid_horizontal,
            });
        }

        for segment_index in 0..editable_curve.segments.len() {
            let left = editable_curve.nodes[segment_index];
            let right =
                editable_curve.nodes[(segment_index + 1).min(editable_curve.nodes.len() - 1)];
            let left_x = local_from_node_for_size(CurveNode { x: left.x, y: 0.0 }, curve_size).x;
            let right_x = local_from_node_for_size(CurveNode { x: right.x, y: 0.0 }, curve_size).x;
            let segment_width = (right_x - left_x).abs().max(2);
            let steps = segment_width.clamp(2, 96) as usize;
            let mut prev = to_canvas(local_from_node_for_size(
                CurveNode {
                    x: left.x,
                    y: sample_editable_curve(editable_curve, left.x),
                },
                curve_size,
            ));
            let highlight =
                state.preview_node.is_none() && state.hovered_segment == Some(segment_index);
            let line_color = if highlight {
                theme.curve_line_highlight
            } else {
                theme.curve_line
            };
            for step in 1..=steps {
                let t = step as f32 / steps as f32;
                let x = left.x + (right.x - left.x) * t;
                let point = to_canvas(local_from_node_for_size(
                    CurveNode {
                        x,
                        y: sample_editable_curve(editable_curve, x),
                    },
                    curve_size,
                ));
                commands.push(SurfaceCommand::Line {
                    start: prev,
                    end: point,
                    color: line_color,
                });
                if highlight {
                    commands.push(SurfaceCommand::Line {
                        start: Point {
                            x: prev.x,
                            y: prev.y + highlight_offset,
                        },
                        end: Point {
                            x: point.x,
                            y: point.y + highlight_offset,
                        },
                        color: theme.curve_line_highlight_glow,
                    });
                }
                prev = point;
            }
        }

        if let Some(preview) = state.preview_node {
            let center = to_canvas(local_from_node_for_size(preview, curve_size));
            commands.push(SurfaceCommand::FillCircle {
                center,
                radius: node_preview_radius,
                color: theme.preview_fill,
            });
            commands.push(SurfaceCommand::StrokeCircle {
                center,
                radius: node_preview_stroke_radius,
                thickness: node_stroke,
                color: theme.preview_stroke,
            });
        }

        for (index, node) in editable_curve.nodes.iter().copied().enumerate() {
            let center = to_canvas(local_from_node_for_size(node, curve_size));
            let selected = state.selected_node == Some(index);
            let hovered = state.hovered_node == Some(index);
            let fill_color = if selected {
                theme.node_selected_fill
            } else if hovered {
                theme.node_hover_fill
            } else {
                theme.node_fill
            };
            let stroke_color = if selected {
                theme.node_selected_stroke
            } else if hovered {
                theme.node_hover_stroke
            } else {
                theme.node_stroke
            };
            commands.push(SurfaceCommand::FillCircle {
                center,
                radius: if selected || hovered {
                    node_hover_radius
                } else {
                    node_radius
                },
                color: fill_color,
            });
            commands.push(SurfaceCommand::StrokeCircle {
                center,
                radius: node_radius,
                thickness: node_stroke,
                color: stroke_color,
            });
            if selected || hovered {
                commands.push(SurfaceCommand::StrokeCircle {
                    center,
                    radius: node_ring_radius,
                    thickness: node_stroke,
                    color: if selected {
                        theme.node_selected_ring
                    } else {
                        theme.node_hover_ring
                    },
                });
            }
        }

        if self.status.has_host_beats_timeline() || self.status.is_playing() {
            let phase = self.status.phase();
            let point = to_canvas(local_from_node_for_size(
                CurveNode {
                    x: phase,
                    y: sample_editable_curve(editable_curve, phase).clamp(0.0, 1.0),
                },
                curve_size,
            ));
            commands.push(SurfaceCommand::FillCircle {
                center: point,
                radius: playhead_glow_radius,
                color: theme.playhead_dot_glow,
            });
            commands.push(SurfaceCommand::FillCircle {
                center: point,
                radius: playhead_core_radius,
                color: theme.playhead_dot_core,
            });
            commands.push(SurfaceCommand::StrokeCircle {
                center: point,
                radius: playhead_core_radius.saturating_add(1),
                thickness: node_stroke.max(1),
                color: theme.playhead_dot_stroke,
            });
        }

        let reduction = (1.0 - self.status.gain().clamp(0.0, 1.0)).clamp(0.0, 1.0);
        let meter_rect = Rect {
            origin: Point {
                x: curve_size.width as i32 - meter_x_offset - meter_width_i32,
                y: meter_y_offset,
            },
            size: Size {
                width: meter_width,
                height: curve_size
                    .height
                    .saturating_sub((meter_y_offset.saturating_mul(2)).max(0) as u32),
            },
        };
        commands.push(SurfaceCommand::StrokeRect {
            rect: meter_rect,
            thickness: meter_stroke_u32,
            color: theme.meter_outline,
        });
        let fill_height = ((meter_rect.size.height as f32) * reduction).round() as u32;
        if fill_height > 0 {
            commands.push(SurfaceCommand::FillRect {
                rect: Rect {
                    origin: Point {
                        x: meter_rect.origin.x
                            + i32::try_from(meter_stroke_u32).unwrap_or(i32::MAX),
                        y: meter_rect.origin.y
                            + i32::try_from(meter_stroke_u32).unwrap_or(i32::MAX),
                    },
                    size: Size {
                        width: meter_inner_width,
                        height: fill_height,
                    },
                },
                color: theme.meter_fill,
            });
        }

        commands
    }

    fn request_flush(&self) {
        if let Some(requester) = self.param_requester {
            requester.request_flush();
        }
    }

    /// Close any active knob automation gesture when pointer drag ends.
    fn sync_knob_gesture_state(&self, mouse_down: bool) {
        let mut ended_param = None;
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.pointer_primary_down && !mouse_down {
                ended_param = runtime.active_knob_gesture_param.take();
            }
            runtime.pointer_primary_down = mouse_down;
        }
        if let Some(param_id) = ended_param {
            self.end_param_gesture(param_id);
        }
    }

    fn push_all_param_updates(&self) {
        self.push_single_value_update(PARAM_MIX_ID, self.params.mix() as f64);
        self.push_single_value_update(PARAM_DEPTH_ID, self.params.depth() as f64);
        self.push_single_value_update(PARAM_PHASE_OFFSET_ID, self.params.phase_offset() as f64);
        self.push_single_value_update(PARAM_OUTPUT_GAIN_ID, self.params.output_gain_db() as f64);
        self.push_single_value_update(PARAM_SYNC_DIVISION_ID, self.params.sync_division() as f64);
    }

    fn push_single_value_update(&self, param_id: ClapId, value: f64) {
        self.begin_param_gesture(param_id);
        self.push_param_value(param_id, value);
        self.end_param_gesture(param_id);
    }

    /// Push one knob update using drag-aware gesture boundaries.
    fn push_knob_value_update(&self, param_id: ClapId, value: f64) {
        let mut should_begin = false;
        let mut should_end_previous = None;
        let mut immediate = false;
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.pointer_primary_down {
                if runtime.active_knob_gesture_param != Some(param_id) {
                    should_end_previous = runtime.active_knob_gesture_param.take();
                    runtime.active_knob_gesture_param = Some(param_id);
                    should_begin = true;
                }
            } else {
                should_end_previous = runtime.active_knob_gesture_param.take();
                immediate = true;
            }
        } else {
            immediate = true;
        }

        if let Some(previous) = should_end_previous {
            self.end_param_gesture(previous);
        }
        if immediate {
            self.push_single_value_update(param_id, value);
            return;
        }
        if should_begin {
            self.begin_param_gesture(param_id);
        }
        self.push_param_value(param_id, value);
    }

    /// Begin one automation gesture event.
    fn begin_param_gesture(&self, param_id: ClapId) {
        self.automation_queue
            .push_gesture_begin(&self.automation_config, param_id);
        self.request_flush();
    }

    /// Push one automation value event.
    fn push_param_value(&self, param_id: ClapId, value: f64) {
        self.automation_queue
            .push_value(&self.automation_config, param_id, value);
        self.request_flush();
    }

    /// End one automation gesture event.
    fn end_param_gesture(&self, param_id: ClapId) {
        self.automation_queue
            .push_gesture_end(&self.automation_config, param_id);
        self.request_flush();
    }
}

#[allow(dead_code)]
fn local_from_node(node: CurveNode) -> Point {
    local_from_node_for_size(
        node,
        Size {
            width: CURVE_W,
            height: CURVE_H,
        },
    )
}

fn local_from_node_for_size(node: CurveNode, curve_size: Size) -> Point {
    let width = curve_size.width.max(1) as f32 - 1.0;
    let height = curve_size.height.max(1) as f32 - 1.0;
    let x = (node.x.clamp(0.0, 1.0) * width).round() as i32;
    let y = ((1.0 - node.y.clamp(0.0, 1.0)) * height).round() as i32;
    Point { x, y }
}

#[allow(dead_code)]
fn node_from_local(local: Point) -> CurveNode {
    node_from_local_for_size(
        local,
        Size {
            width: CURVE_W,
            height: CURVE_H,
        },
    )
}

fn node_from_local_for_size(local: Point, curve_size: Size) -> CurveNode {
    let width = (curve_size.width.max(1) as f32 - 1.0).max(1.0);
    let height = (curve_size.height.max(1) as f32 - 1.0).max(1.0);
    let x = (local.x as f32 / width).clamp(0.0, 1.0);
    let y = (1.0 - (local.y as f32 / height)).clamp(0.0, 1.0);
    CurveNode { x, y }
}

fn scale_point_from_design(point: Point, curve_size: Size) -> Point {
    Point {
        x: point.x.clamp(0, curve_size.width.max(1) as i32 - 1),
        y: point.y.clamp(0, curve_size.height.max(1) as i32 - 1),
    }
}

fn scale_point_to_design(point: Point, curve_size: Size) -> Point {
    scale_point_from_design(point, curve_size)
}

#[allow(dead_code)]
fn find_node_hit(curve: &EditableCurve, local_pointer: Point) -> Option<usize> {
    find_node_hit_for_size(
        curve,
        local_pointer,
        node_hit_radius(Size {
            width: CURVE_W,
            height: CURVE_H,
        }),
        Size {
            width: CURVE_W,
            height: CURVE_H,
        },
    )
}

fn find_node_hit_for_size(
    curve: &EditableCurve,
    local_pointer: Point,
    radius: i32,
    curve_size: Size,
) -> Option<usize> {
    find_node_hit_within_for_size(curve, local_pointer, radius, curve_size)
}

fn find_node_hit_within_for_size(
    curve: &EditableCurve,
    local_pointer: Point,
    radius: i32,
    curve_size: Size,
) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    let radius_squared = radius.max(0) as i64 * radius.max(0) as i64;
    for (index, node) in curve.nodes.iter().copied().enumerate() {
        let center = local_from_node_for_size(node, curve_size);
        let distance = distance_squared(center, local_pointer);
        if distance <= radius_squared {
            match best {
                Some((_, best_distance)) if distance >= best_distance => {}
                _ => best = Some((index, distance)),
            }
        }
    }
    best.map(|(index, _)| index)
}

#[allow(dead_code)]
fn find_node_hit_within(curve: &EditableCurve, local_pointer: Point, radius: i32) -> Option<usize> {
    find_node_hit_within_for_size(
        curve,
        local_pointer,
        radius,
        Size {
            width: CURVE_W,
            height: CURVE_H,
        },
    )
}

#[allow(dead_code)]
fn find_segment_line_hit_within(
    curve: &EditableCurve,
    local_pointer: Point,
    radius: i32,
) -> Option<usize> {
    find_segment_line_hit_within_for_size(
        curve,
        local_pointer,
        radius,
        Size {
            width: CURVE_W,
            height: CURVE_H,
        },
    )
}

fn find_segment_line_hit_within_for_size(
    curve: &EditableCurve,
    local_pointer: Point,
    radius: i32,
    curve_size: Size,
) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    let radius_squared = (radius.max(0) * radius.max(0)) as f32;
    for index in 0..curve.segments.len() {
        let distance =
            segment_polyline_distance_squared_for_size(curve, index, local_pointer, curve_size);
        if distance <= radius_squared {
            match best {
                Some((_, best_distance)) if distance >= best_distance => {}
                _ => best = Some((index, distance)),
            }
        }
    }
    best.map(|(index, _)| index)
}

#[allow(dead_code)]
fn insert_node(curve: &mut EditableCurve, node: CurveNode) -> usize {
    insert_node_for_size(
        curve,
        node,
        Size {
            width: CURVE_W,
            height: CURVE_H,
        },
    )
}

fn insert_node_for_size(curve: &mut EditableCurve, node: CurveNode, curve_size: Size) -> usize {
    if curve.nodes.len() >= MAX_EDITABLE_NODES {
        return find_nearest_node_for_size(
            curve,
            local_from_node_for_size(node, curve_size),
            curve_size,
        )
        .unwrap_or(0);
    }

    let mut insert_at = curve.nodes.partition_point(|existing| existing.x < node.x);
    insert_at = insert_at.clamp(1, curve.nodes.len().saturating_sub(1));

    let left_limit = curve.nodes[insert_at - 1].x + NODE_X_MIN_SPACING;
    let right_limit = curve.nodes[insert_at].x - NODE_X_MIN_SPACING;
    if left_limit >= right_limit {
        return insert_at.saturating_sub(1);
    }

    let x = node.x.clamp(left_limit, right_limit);
    let y = node.y.clamp(0.0, 1.0);
    curve.nodes.insert(insert_at, CurveNode { x, y });

    let inherited = curve
        .segments
        .get(insert_at.saturating_sub(1))
        .copied()
        .unwrap_or(CurveSegment { tension: 0.0 });
    curve
        .segments
        .insert(insert_at.saturating_sub(1), inherited);
    insert_at
}

#[allow(dead_code)]
fn move_node_with_push_through(
    curve: &mut EditableCurve,
    index: usize,
    target: CurveNode,
    push_threshold_px: i32,
) -> usize {
    move_node_with_push_through_for_size(
        curve,
        index,
        target,
        push_threshold_px,
        Size {
            width: CURVE_W,
            height: CURVE_H,
        },
    )
}

fn move_node_with_push_through_for_size(
    curve: &mut EditableCurve,
    index: usize,
    target: CurveNode,
    push_threshold_px: i32,
    curve_size: Size,
) -> usize {
    if index >= curve.nodes.len() {
        return index;
    }

    let y = target.y.clamp(0.0, 1.0);
    let last_index = curve.nodes.len() - 1;
    if index == 0 {
        set_wrapped_endpoint_y(curve, y);
        return 0;
    }
    if index == last_index {
        set_wrapped_endpoint_y(curve, y);
        return curve.nodes.len().saturating_sub(1);
    }

    let mut moved_index = index;
    let threshold_x = push_threshold_px.max(0) as f32 / (curve_size.width.max(2) - 1) as f32;
    while moved_index + 1 < curve.nodes.len().saturating_sub(1)
        && target.x > curve.nodes[moved_index + 1].x + threshold_x
    {
        remove_interior_node(curve, moved_index + 1);
    }
    while moved_index > 1 && target.x < curve.nodes[moved_index - 1].x - threshold_x {
        remove_interior_node(curve, moved_index - 1);
        moved_index = moved_index.saturating_sub(1);
    }

    let min_x = curve.nodes[moved_index - 1].x + NODE_X_MIN_SPACING;
    let max_x = curve.nodes[moved_index + 1].x - NODE_X_MIN_SPACING;
    curve.nodes[moved_index].x = target.x.clamp(min_x, max_x);
    curve.nodes[moved_index].y = y;
    enforce_wrapped_endpoints(curve);
    moved_index
}

fn move_segment_translated(
    curve: &mut EditableCurve,
    segment_index: usize,
    start_left: (f32, f32),
    start_right: (f32, f32),
    delta: (f32, f32),
) {
    let (start_left_x, start_left_y) = start_left;
    let (start_right_x, start_right_y) = start_right;
    let (delta_x, delta_y) = delta;
    if curve.nodes.len() < 2 || segment_index >= curve.nodes.len() - 1 {
        return;
    }

    let right_index = segment_index + 1;
    let mut applied_dx = delta_x;
    if segment_index == 0 || right_index == curve.nodes.len() - 1 {
        applied_dx = 0.0;
    } else {
        let min_dx = curve.nodes[segment_index - 1].x + NODE_X_MIN_SPACING - start_left_x;
        let max_dx = curve.nodes[right_index + 1].x - NODE_X_MIN_SPACING - start_right_x;
        applied_dx = applied_dx.clamp(min_dx, max_dx);
    }
    curve.nodes[segment_index].x = (start_left_x + applied_dx).clamp(0.0, 1.0);
    curve.nodes[right_index].x = (start_right_x + applied_dx).clamp(0.0, 1.0);
    curve.nodes[segment_index].y = (start_left_y + delta_y).clamp(0.0, 1.0);
    curve.nodes[right_index].y = (start_right_y + delta_y).clamp(0.0, 1.0);

    if segment_index == 0 {
        set_wrapped_endpoint_y(curve, curve.nodes[0].y);
    }
    if right_index == curve.nodes.len() - 1 {
        set_wrapped_endpoint_y(curve, curve.nodes[right_index].y);
    }
    enforce_wrapped_endpoints(curve);
}

fn set_wrapped_endpoint_y(curve: &mut EditableCurve, y: f32) {
    if curve.nodes.len() < 2 {
        return;
    }
    let clamped = y.clamp(0.0, 1.0);
    let last_index = curve.nodes.len() - 1;
    curve.nodes[0].x = 0.0;
    curve.nodes[0].y = clamped;
    curve.nodes[last_index].x = 1.0;
    curve.nodes[last_index].y = clamped;
}

fn enforce_wrapped_endpoints(curve: &mut EditableCurve) {
    if curve.nodes.len() < 2 {
        return;
    }
    let clamped = curve.nodes[0].y.clamp(0.0, 1.0);
    let last_index = curve.nodes.len() - 1;
    curve.nodes[0].x = 0.0;
    curve.nodes[0].y = clamped;
    curve.nodes[last_index].x = 1.0;
    curve.nodes[last_index].y = clamped;
}

fn remove_interior_node(curve: &mut EditableCurve, remove_index: usize) {
    let last_index = curve.nodes.len().saturating_sub(1);
    if remove_index == 0 || remove_index >= last_index {
        return;
    }

    let left_segment_index = remove_index.saturating_sub(1);
    let right_segment_index = remove_index.min(curve.segments.len().saturating_sub(1));
    let left_tension = curve
        .segments
        .get(left_segment_index)
        .copied()
        .unwrap_or(CurveSegment { tension: 0.0 })
        .tension;
    let right_tension = curve
        .segments
        .get(right_segment_index)
        .copied()
        .unwrap_or(CurveSegment {
            tension: left_tension,
        })
        .tension;
    let merged_tension =
        ((left_tension + right_tension) * 0.5).clamp(MIN_SEGMENT_TENSION, MAX_SEGMENT_TENSION);

    curve.nodes.remove(remove_index);
    if !curve.segments.is_empty() {
        if right_segment_index < curve.segments.len() {
            curve.segments.remove(right_segment_index);
        } else {
            curve.segments.pop();
        }
        if left_segment_index < curve.segments.len() {
            curve.segments[left_segment_index].tension = merged_tension;
        }
    }
}

#[allow(dead_code)]
fn find_nearest_node(curve: &EditableCurve, local_pointer: Point) -> Option<usize> {
    find_nearest_node_for_size(
        curve,
        local_pointer,
        Size {
            width: CURVE_W,
            height: CURVE_H,
        },
    )
}

fn find_nearest_node_for_size(
    curve: &EditableCurve,
    local_pointer: Point,
    curve_size: Size,
) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    for (index, node) in curve.nodes.iter().copied().enumerate() {
        let distance = distance_squared(local_from_node_for_size(node, curve_size), local_pointer);
        match best {
            Some((_, best_distance)) if distance >= best_distance => {}
            _ => best = Some((index, distance)),
        }
    }
    best.map(|(index, _)| index)
}

fn distance_squared(a: Point, b: Point) -> i64 {
    let dx = a.x as i64 - b.x as i64;
    let dy = a.y as i64 - b.y as i64;
    dx * dx + dy * dy
}

#[allow(dead_code)]
fn segment_polyline_distance_squared(curve: &EditableCurve, index: usize, point: Point) -> f32 {
    segment_polyline_distance_squared_for_size(
        curve,
        index,
        point,
        Size {
            width: CURVE_W,
            height: CURVE_H,
        },
    )
}

fn segment_polyline_distance_squared_for_size(
    curve: &EditableCurve,
    index: usize,
    point: Point,
    curve_size: Size,
) -> f32 {
    let left = curve.nodes[index];
    let right = curve.nodes[(index + 1).min(curve.nodes.len() - 1)];
    let width = ((right.x - left.x).abs() * (curve_size.width.max(1) as f32 - 1.0))
        .round()
        .max(2.0) as i32;
    let steps = width.clamp(2, 96) as usize;
    let mut prev = local_from_node_for_size(
        CurveNode {
            x: left.x,
            y: sample_editable_curve(curve, left.x),
        },
        curve_size,
    );
    let mut best = f32::MAX;
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let x = left.x + (right.x - left.x) * t;
        let current = local_from_node_for_size(
            CurveNode {
                x,
                y: sample_editable_curve(curve, x),
            },
            curve_size,
        );
        let distance = point_to_segment_distance_squared(point, prev, current);
        if distance < best {
            best = distance;
        }
        prev = current;
    }
    best
}

fn point_to_segment_distance_squared(point: Point, a: Point, b: Point) -> f32 {
    let px = point.x as f32;
    let py = point.y as f32;
    let ax = a.x as f32;
    let ay = a.y as f32;
    let bx = b.x as f32;
    let by = b.y as f32;
    let abx = bx - ax;
    let aby = by - ay;
    let ab_len2 = abx * abx + aby * aby;
    if ab_len2 <= f32::EPSILON {
        let dx = px - ax;
        let dy = py - ay;
        return dx * dx + dy * dy;
    }
    let apx = px - ax;
    let apy = py - ay;
    let t = ((apx * abx + apy * aby) / ab_len2).clamp(0.0, 1.0);
    let cx = ax + abx * t;
    let cy = ay + aby * t;
    let dx = px - cx;
    let dy = py - cy;
    dx * dx + dy * dy
}

#[allow(dead_code)]
fn preview_node_on_curve(curve: &EditableCurve, local_pointer: Point) -> Option<CurveNode> {
    preview_node_on_curve_for_size(
        curve,
        local_pointer,
        Size {
            width: CURVE_W,
            height: CURVE_H,
        },
    )
}

fn preview_node_on_curve_for_size(
    curve: &EditableCurve,
    local_pointer: Point,
    curve_size: Size,
) -> Option<CurveNode> {
    if curve.nodes.len() < 2 {
        return None;
    }
    let x = node_from_local_for_size(local_pointer, curve_size).x;
    Some(CurveNode {
        x,
        y: sample_editable_curve(curve, x).clamp(0.0, 1.0),
    })
}

fn drag_threshold_crossed(start_pointer: Point, current_pointer: Point, threshold_px: i32) -> bool {
    let threshold = threshold_px.max(0) as i64;
    distance_squared(start_pointer, current_pointer) >= threshold * threshold
}

#[allow(dead_code)]
fn find_deletable_node_hit(curve: &EditableCurve, local_pointer: Point) -> Option<usize> {
    find_deletable_node_hit_for_size(
        curve,
        local_pointer,
        Size {
            width: CURVE_W,
            height: CURVE_H,
        },
    )
}

fn find_deletable_node_hit_for_size(
    curve: &EditableCurve,
    local_pointer: Point,
    curve_size: Size,
) -> Option<usize> {
    let index = find_node_hit_for_size(
        curve,
        local_pointer,
        node_hit_radius(curve_size),
        curve_size,
    )?;
    if index == 0 || index + 1 == curve.nodes.len() {
        return None;
    }
    Some(index)
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
