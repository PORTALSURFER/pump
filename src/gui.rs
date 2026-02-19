//! Declarative curve-editor GUI for Pump.

use std::sync::{Arc, Mutex, OnceLock};

use toybox::clack_extensions::gui::{GuiSize, Window};
use toybox::clack_plugin::plugin::PluginError;
use toybox::clack_plugin::utils::ClapId;
use toybox::clap::automation::{AutomationConfig, AutomationQueue};
use toybox::clap::gui::{GuiHostWindow, GuiOpenRequest, HostParamRequester, InputState};
use toybox::gui::declarative::{
    button, column, column_slots, dropdown, fill_slot, fraction_slot, grid, indicator, knob, panel,
    root_frame_sized, row_slots, stack, surface, switch_layout, textbox, weighted_slot,
    weighted_slot_lengths, when_width_ge, GridTemplate, LayoutBox, Node, OverflowPolicy,
    RegionInteractionKind, RootScaleMode, SlotAlign, SurfaceCommand, ThemeTokens, TrackSize,
    UiAction, UiSpec,
};
use toybox::gui::{Color, MainPalette, Point, Rect, Size};
use toybox::raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use crate::curve::{
    sample_editable_curve, CurveNode, CurveSegment, EditableCurve, MAX_EDITABLE_NODES,
    MAX_SEGMENT_TENSION, MIN_SEGMENT_TENSION,
};
use crate::params::{
    sync_division_label, PumpParams, MAX_DEPTH, MAX_MIX, MAX_OUTPUT_GAIN_DB, MAX_PHASE_OFFSET,
    MAX_SYNC_DIVISION, MIN_DEPTH, MIN_MIX, MIN_OUTPUT_GAIN_DB, MIN_PHASE_OFFSET, PARAM_DEPTH_ID,
    PARAM_MIX_ID, PARAM_OUTPUT_GAIN_ID, PARAM_PHASE_OFFSET_ID, PARAM_SYNC_DIVISION_ID,
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

const BASE_PADDING_X: i32 = 18;
const HEADER_SECTION_WEIGHT: u16 = 7;
const CURVE_SECTION_WEIGHT: u16 = 63;
const CONTROLS_SECTION_WEIGHT: u16 = 30;
const ROOT_SECTION_WEIGHT_SUM: u32 =
    HEADER_SECTION_WEIGHT as u32 + CURVE_SECTION_WEIGHT as u32 + CONTROLS_SECTION_WEIGHT as u32;
const KNOBS_SECTION_WEIGHT: u16 = 70;
const DROPDOWN_SECTION_WEIGHT: u16 = 30;
const HEADER_SWITCH_WIDE_MIN_WIDTH: u32 = 560;
const CURVE_W: u32 = WINDOW_WIDTH;
const CURVE_H: u32 = resolve_vertical_slot_heights(WINDOW_HEIGHT).1;
const TITLE_LABEL_W: u32 = 64;
const TITLE_LABEL_RIGHT_GAP: i32 = 8;
const HEADER_CONTROL_GAP: i32 = 4;
const METER_X_OFFSET: i32 = 12;
const METER_Y_OFFSET: i32 = 10;
const METER_WIDTH: i32 = 6;
const METER_STROKE: i32 = 1;
const BASE_KNOB_DIAMETER: u32 = 48;
const BASE_TEXT_SCALE: u32 = 2;
const KNOBS_PER_ROW: usize = 4;
const BASE_CONTROL_LINE_UNIT: u32 = 8;
const BASE_DROPDOWN_CONTROL_H: u32 = 24;
const TRANSPORT_INDICATOR_SIZE: u32 = 10;
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
    use std::sync::Arc;

    use toybox::gui::{screenshot_harness, Size};

    use super::{AutomationQueue, GuiState, GuiStatus, PumpParams, WINDOW_HEIGHT, WINDOW_WIDTH};

    #[test]
    fn screenshot_renders_initial_ui() {
        let params = Arc::new(PumpParams::new());
        let status = Arc::new(GuiStatus::default());
        let queue = Arc::new(AutomationQueue::default());
        let state = GuiState::new(params, status, queue, None);

        screenshot_harness::capture_initial_ui_screenshots_if_enabled(
            env!("CARGO_PKG_NAME"),
            Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            |input| state.build_ui(input),
        )
        .expect("failed to capture pump headless screenshots");
    }
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

/// Shared Pump color/style tokens derived from the canonical Patchbay theme.
#[derive(Clone, Copy, Debug)]
struct PumpTheme {
    tokens: ThemeTokens,
    title_text: Color,
    subtitle_text: Color,
    hint_text: Color,
    curve_bg: Color,
    curve_border: Color,
    curve_grid_vertical: Color,
    curve_grid_horizontal: Color,
    curve_line: Color,
    curve_line_highlight: Color,
    curve_line_highlight_glow: Color,
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
            title_text: palette.accent_focus,
            subtitle_text: palette.syntax_emphasis,
            hint_text: palette.text_muted,
            curve_bg: palette.background_primary,
            curve_border: palette.ui_secondary,
            curve_grid_vertical: palette.background_secondary,
            curve_grid_horizontal: palette.ui_secondary,
            curve_line: palette.syntax_emphasis,
            curve_line_highlight: palette.accent_focus,
            curve_line_highlight_glow: palette.text_primary,
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
    padding_x: i32,
    title_label_w: u32,
    controls_gap: i32,
    meter_x_offset: i32,
    meter_y_offset: i32,
    meter_width: u32,
    meter_stroke: u32,
    dropdown_control_w: u32,
    dropdown_control_h: u32,
    button_control_h: u32,
    subtitle_label_w: u32,
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
        let padding_x = BASE_PADDING_X.max(0);
        let title_label_w = TITLE_LABEL_W.max(1);
        let title_label_gap = TITLE_LABEL_RIGHT_GAP.max(0);
        let controls_gap = HEADER_CONTROL_GAP.max(0);
        let text_scale = BASE_TEXT_SCALE.max(1);
        let knob_track_width = knobs_slot_w.saturating_div(KNOBS_PER_ROW as u32);
        let knob_diameter = BASE_KNOB_DIAMETER.min(knob_track_width.max(1));
        let knob_track_w = knob_track_width.max(1);
        let label_line_h = scaled_line_height(text_scale);
        let expanded_control_h = controls_h
            .saturating_sub(label_line_h)
            .saturating_sub((controls_gap.max(0) as u32).saturating_mul(2))
            .saturating_div(2)
            .max(BASE_DROPDOWN_CONTROL_H.max(1));
        let dropdown_control_h = expanded_control_h;
        let button_control_h = expanded_control_h;
        let dropdown_control_w = dropdown_slot_w.max(1);
        let subtitle_label_x = padding_x
            .saturating_add(title_label_w as i32)
            .saturating_add(title_label_gap)
            .max(0) as u32;
        let subtitle_label_w = content_w.saturating_sub(subtitle_label_x).max(1);
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
            padding_x,
            title_label_w,
            controls_gap,
            meter_x_offset,
            meter_y_offset,
            meter_width,
            meter_stroke,
            dropdown_control_w,
            dropdown_control_h,
            button_control_h,
            subtitle_label_w,
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

    /// Build the top header slot node.
    fn build_header_slot(&self, metrics: UiLayoutMetrics, theme: PumpTheme) -> Node {
        let header_indicator = || {
            Node::align_box(
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
            .slot_align(SlotAlign::End, SlotAlign::Center)
            .fill()
        };
        let wide_header = row_slots(vec![
            fraction_slot(
                panel(
                    "header-title-wide",
                    textbox("PUMP")
                        .text_color(theme.title_text)
                        .widget_layout(fixed_box(metrics.title_label_w, metrics.label_line_h)),
                )
                .pad_xy(metrics.padding_x.max(0), 0),
                18,
            ),
            fill_slot(
                panel(
                    "header-subtitle-wide",
                    stack(vec![
                        textbox("Spline Beat-Synced Ducking")
                            .text_color(theme.subtitle_text)
                            .widget_layout(fixed_box(
                                metrics.subtitle_label_w,
                                metrics.label_line_h,
                            )),
                        header_indicator(),
                    ])
                    .fill(),
                )
                .pad_xy(0, 0),
            ),
        ])
        .container_overflow(OverflowPolicy::Compress);
        let compact_header = column_slots(vec![
            fill_slot(
                panel(
                    "header-title-compact",
                    textbox("PUMP")
                        .text_color(theme.title_text)
                        .widget_layout(fixed_box(metrics.title_label_w, metrics.label_line_h)),
                )
                .pad_xy(metrics.padding_x.max(0), 0),
            ),
            fill_slot(
                panel(
                    "header-subtitle-compact",
                    stack(vec![
                        textbox("Spline Beat-Synced Ducking")
                            .text_color(theme.subtitle_text)
                            .widget_layout(fixed_box(
                                metrics.subtitle_label_w,
                                metrics.label_line_h,
                            )),
                        header_indicator(),
                    ])
                    .fill(),
                )
                .pad_xy(0, 0),
            ),
        ])
        .container_overflow(OverflowPolicy::Compress);
        let header_content = switch_layout(
            vec![when_width_ge(HEADER_SWITCH_WIDE_MIN_WIDTH, wide_header)],
            compact_header,
        )
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
        let knob_cell = |key: &'static str,
                         label: &'static str,
                         value: f32,
                         range: (f32, f32),
                         value_text: String| {
            let title = Node::align_box(
                textbox(label)
                    .text_color(theme.subtitle_text)
                    .widget_layout(fixed_box(metrics.knob_track_w, metrics.label_line_h)),
            )
            .slot_align(SlotAlign::Center, SlotAlign::Start)
            .fill();
            let value_label = Node::align_box(
                textbox(value_text)
                    .text_color(theme.hint_text)
                    .widget_layout(fixed_box(metrics.knob_track_w, metrics.label_line_h)),
            )
            .slot_align(SlotAlign::Center, SlotAlign::End)
            .fill();
            stack(vec![
                knob(key, value, range).widget_layout(LayoutBox::auto()),
                title,
                value_label,
            ])
            .fill()
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
            stack(vec![
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
                }),
                Node::align_box(
                    textbox(sync_division_label(controls.division))
                        .text_color(theme.subtitle_text)
                        .widget_layout(fixed_box(
                            metrics.dropdown_control_w.saturating_sub(20).max(1),
                            metrics.label_line_h,
                        )),
                )
                .slot_align(SlotAlign::Center, SlotAlign::Center)
                .fill(),
            ])
            .fill(),
            stack(vec![
                button(RESET_KEY).control_size(Size {
                    width: metrics.dropdown_control_w,
                    height: metrics.button_control_h,
                }),
                Node::align_box(
                    textbox("Reset Curve")
                        .text_color(theme.subtitle_text)
                        .widget_layout(fixed_box(metrics.dropdown_control_w, metrics.label_line_h)),
                )
                .slot_align(SlotAlign::Center, SlotAlign::Center)
                .fill(),
            ])
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
        let metrics = UiLayoutMetrics::design_space();
        let theme = PumpTheme::main(metrics);
        let controls = self.snapshot_controls();
        let draw_commands = self.build_curve_commands_for_frame(input, metrics, theme);

        let header_slot = self.build_header_slot(metrics, theme);
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
            UiAction::ButtonPressed { key } if key == RESET_KEY => {
                self.params.reset_curve_to_default();
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.selected_node = None;
                    runtime.drag_mode = None;
                }
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
        match key {
            MIX_KEY => {
                self.params.set_mix(value);
                self.push_single_value_update(PARAM_MIX_ID, value as f64);
            }
            DEPTH_KEY => {
                self.params.set_depth(value);
                self.push_single_value_update(PARAM_DEPTH_ID, value as f64);
            }
            PHASE_KEY => {
                self.params.set_phase_offset(value);
                self.push_single_value_update(PARAM_PHASE_OFFSET_ID, value as f64);
            }
            OUTPUT_KEY => {
                self.params.set_output_gain_db(value);
                self.push_single_value_update(PARAM_OUTPUT_GAIN_ID, value as f64);
            }
            _ => {}
        }
    }

    fn reduce_dropdown(&mut self, key: &str, index: usize) {
        if key != DIVISION_KEY {
            return;
        }

        let clamped = index.min(MAX_SYNC_DIVISION as usize);
        self.params.set_sync_division(clamped as f32);
        self.push_single_value_update(PARAM_SYNC_DIVISION_ID, clamped as f64);
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
                        y: meter_rect.origin.y + meter_rect.size.height as i32 - fill_height as i32,
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

    fn push_single_value_update(&self, param_id: ClapId, value: f64) {
        self.automation_queue
            .push_gesture_begin(&self.automation_config, param_id);
        self.automation_queue
            .push_value(&self.automation_config, param_id, value);
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
    use super::{
        constrained_host_size, find_deletable_node_hit, find_segment_line_hit_within,
        local_from_node, move_node_with_push_through, move_segment_translated,
        preferred_window_size, preview_node_on_curve, resolve_runtime_controls_slot_widths,
        resolve_vertical_slot_heights, segment_upward_tension_sign,
        tension_delta_from_drag_for_segment, CurveRenderState, GuiState, PumpTheme,
        UiLayoutMetrics, CURVE_H, CURVE_KEY, CURVE_W, DIVISION_KEY, HEADER_SWITCH_WIDE_MIN_WIDTH,
        TRANSPORT_INDICATOR_SIZE, WINDOW_HEIGHT, WINDOW_WIDTH,
    };
    use crate::curve::{sample_editable_curve, CurveNode, CurveSegment, EditableCurve};
    use crate::params::{sync_division_label, PumpParams};
    use crate::GuiStatus;
    use std::sync::Arc;
    use toybox::clack_extensions::gui::GuiSize;
    use toybox::clap::automation::AutomationQueue;
    use toybox::clap::gui::InputState;
    use toybox::gui::declarative::{
        measure_checked, ContainerLayout, ContainerLength, GridKind, Node, PanelSpec,
        RootScaleMode, SurfaceCommand, SwitchLayoutSpec, UiSpec,
    };
    use toybox::gui::{render_spec_to_frame, Color, MainPalette, Point, Size};

    fn expect_slot_child<'a>(node: &'a Node, label: &str) -> &'a Node {
        match node {
            Node::Slot(slot) => slot.child(),
            other => panic!("expected {label} slot wrapper, got {other:?}"),
        }
    }

    fn expect_slot_panel<'a>(node: &'a Node, label: &str) -> &'a PanelSpec {
        match expect_slot_child(node, label) {
            Node::Row(row) => match row.children() {
                [child] => match expect_slot_child(child, label) {
                    Node::Panel(panel) => panel,
                    other => panic!("expected {label} row to wrap panel, got {other:?}"),
                },
                _ => panic!("expected {label} row to contain exactly one child"),
            },
            Node::Panel(panel) => panel,
            other => panic!("expected {label} panel (or row wrapper), got {other:?}"),
        }
    }

    fn assert_container_layout_host_derived(layout: ContainerLayout) {
        assert!(matches!(
            layout.width,
            ContainerLength::Auto | ContainerLength::Fill(_)
        ));
        assert!(matches!(
            layout.height,
            ContainerLength::Auto | ContainerLength::Fill(_)
        ));
    }

    fn assert_slot_tree_node(node: &Node) {
        match node {
            Node::Slot(slot) => {
                let child = slot.child();
                assert!(
                    !matches!(child, Node::Slot(_)),
                    "slot child must not be another slot"
                );
                assert_slot_tree_node(child);
            }
            Node::Panel(panel) => {
                assert_container_layout_host_derived(panel.container_layout());
                assert!(matches!(panel.content(), Node::Slot(_)));
                assert_slot_tree_node(panel.content());
            }
            Node::PaddingBox(padding_box) => {
                assert_container_layout_host_derived(padding_box.container_layout());
                assert!(matches!(padding_box.content(), Node::Slot(_)));
                assert_slot_tree_node(padding_box.content());
            }
            Node::AlignBox(align_box) => {
                assert_container_layout_host_derived(align_box.container_layout());
                assert!(matches!(align_box.content(), Node::Slot(_)));
                assert_slot_tree_node(align_box.content());
            }
            Node::AspectBox(aspect_box) => {
                assert_container_layout_host_derived(aspect_box.container_layout());
                assert!(matches!(aspect_box.content(), Node::Slot(_)));
                assert_slot_tree_node(aspect_box.content());
            }
            Node::Row(row) => {
                assert_container_layout_host_derived(row.container_layout());
                for child in row.children() {
                    assert!(matches!(child, Node::Slot(_)));
                    assert_slot_tree_node(child);
                }
            }
            Node::Column(column) => {
                assert_container_layout_host_derived(column.container_layout());
                for child in column.children() {
                    assert!(matches!(child, Node::Slot(_)));
                    assert_slot_tree_node(child);
                }
            }
            Node::Grid(grid) => {
                assert_container_layout_host_derived(grid.container_layout());
                for child in grid.children() {
                    assert!(matches!(child, Node::Slot(_)));
                    assert_slot_tree_node(child);
                }
            }
            Node::Absolute(absolute) => {
                assert_container_layout_host_derived(absolute.container_layout());
                for child in absolute.children() {
                    assert!(matches!(child.node(), Node::Slot(_)));
                    assert_slot_tree_node(child.node());
                }
            }
            Node::Stack(stack) => {
                assert_container_layout_host_derived(stack.container_layout());
                for child in stack.children() {
                    assert!(matches!(child, Node::Slot(_)));
                    assert_slot_tree_node(child);
                }
            }
            Node::ScrollView(scroll_view) => {
                assert_container_layout_host_derived(scroll_view.container_layout());
                assert!(matches!(scroll_view.content(), Node::Slot(_)));
                assert_slot_tree_node(scroll_view.content());
            }
            Node::Wrap(wrap) => {
                assert_container_layout_host_derived(wrap.container_layout());
                for child in wrap.children() {
                    assert!(matches!(child, Node::Slot(_)));
                    assert_slot_tree_node(child);
                }
            }
            Node::SwitchLayout(switch_layout) => {
                assert_container_layout_host_derived(switch_layout.container_layout());
                assert!(matches!(switch_layout.fallback(), Node::Slot(_)));
                assert_slot_tree_node(switch_layout.fallback());
                for case_entry in switch_layout.cases() {
                    assert!(matches!(case_entry.child(), Node::Slot(_)));
                    assert_slot_tree_node(case_entry.child());
                }
            }
            Node::TextBox(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::Region(_)
            | Node::Indicator(_) => {}
        }
    }

    fn assert_emitted_slot_tree_invariants(spec: &UiSpec) {
        let root = spec.root.content();
        assert!(matches!(root, Node::Slot(_)));
        let root_child = expect_slot_child(root, "root");
        assert!(
            matches!(
                root_child,
                Node::Panel(_)
                    | Node::PaddingBox(_)
                    | Node::AlignBox(_)
                    | Node::AspectBox(_)
                    | Node::Row(_)
                    | Node::Column(_)
                    | Node::Grid(_)
                    | Node::Absolute(_)
                    | Node::Stack(_)
                    | Node::ScrollView(_)
                    | Node::Wrap(_)
                    | Node::SwitchLayout(_)
            ),
            "root slot child must be a container"
        );
        assert_slot_tree_node(root_child);
    }

    fn extract_header_switch(spec: &UiSpec) -> &SwitchLayoutSpec {
        let root_grid = match expect_slot_child(spec.root.content(), "root") {
            Node::Grid(grid) => grid,
            other => panic!("expected root content grid, got {other:?}"),
        };
        let header_panel = expect_slot_panel(&root_grid.children()[0], "header");
        match expect_slot_child(header_panel.content(), "header") {
            Node::SwitchLayout(switch_layout) => switch_layout,
            other => panic!("expected header switch-layout, got {other:?}"),
        }
    }

    #[test]
    fn delete_hit_ignores_endpoints_and_targets_interior_nodes() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.3, y: 0.4 },
                CurveNode { x: 0.6, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
            ],
        };

        let near_start = local_from_node(curve.nodes[0]);
        assert_eq!(find_deletable_node_hit(&curve, near_start), None);

        let near_middle = local_from_node(curve.nodes[1]);
        assert_eq!(find_deletable_node_hit(&curve, near_middle), Some(1));
    }

    #[test]
    fn delete_hit_returns_none_outside_node_hit_radius() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.5, y: 0.2 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }, CurveSegment { tension: 0.0 }],
        };

        let far_away = Point { x: 0, y: 0 };
        assert_eq!(find_deletable_node_hit(&curve, far_away), None);
    }

    #[test]
    fn segment_line_hit_detects_nearby_curve_segment() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.3, y: 0.2 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }, CurveSegment { tension: 0.0 }],
        };

        let near_segment = local_from_node(CurveNode { x: 0.2, y: 0.45 });
        assert_eq!(
            find_segment_line_hit_within(&curve, near_segment, 24),
            Some(0)
        );
    }

    #[test]
    fn push_through_drag_consumes_crossed_interior_nodes_only() {
        let mut curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.25, y: 0.6 },
                CurveNode { x: 0.5, y: 0.3 },
                CurveNode { x: 0.75, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
            ],
        };

        let moved_index =
            move_node_with_push_through(&mut curve, 2, CurveNode { x: 0.95, y: 0.4 }, 0);
        assert_eq!(moved_index, 2);
        assert_eq!(curve.nodes.len(), 4);
        assert_eq!(curve.nodes[0].x, 0.0);
        assert_eq!(curve.nodes[curve.nodes.len() - 1].x, 1.0);
        assert!(curve.nodes.iter().all(|node| node.x <= 1.0));
    }

    #[test]
    fn wrapped_endpoints_move_together() {
        let mut curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.5, y: 0.25 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }, CurveSegment { tension: 0.0 }],
        };

        move_node_with_push_through(&mut curve, 0, CurveNode { x: 0.0, y: 0.31 }, 10);
        let last_index = curve.nodes.len() - 1;
        assert!((curve.nodes[0].y - curve.nodes[last_index].y).abs() <= f32::EPSILON);
    }

    #[test]
    fn preview_node_snaps_to_curve_value_at_pointer_x() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.5, y: 0.0 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }, CurveSegment { tension: 0.0 }],
        };
        let pointer = local_from_node(CurveNode { x: 0.5, y: 0.9 });
        let preview = preview_node_on_curve(&curve, pointer).expect("preview exists");
        let expected = sample_editable_curve(&curve, preview.x);
        assert!((preview.y - expected).abs() < 1.0e-6);
    }

    #[test]
    fn segment_translation_moves_interior_segment_horizontally() {
        let mut curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.3, y: 0.5 },
                CurveNode { x: 0.6, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
            ],
        };
        move_segment_translated(&mut curve, 1, (0.3, 0.5), (0.6, 0.5), (0.1, 0.1));
        assert!((curve.nodes[1].x - 0.4).abs() < 1.0e-6);
        assert!((curve.nodes[2].x - 0.7).abs() < 1.0e-6);
        assert!((curve.nodes[1].y - 0.6).abs() < 1.0e-6);
        assert!((curve.nodes[2].y - 0.6).abs() < 1.0e-6);
    }

    #[test]
    fn upward_bend_sign_tracks_segment_direction() {
        let rising = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.2 }, CurveNode { x: 1.0, y: 0.8 }],
            segments: vec![CurveSegment { tension: 0.0 }],
        };
        let falling = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.8 }, CurveNode { x: 1.0, y: 0.2 }],
            segments: vec![CurveSegment { tension: 0.0 }],
        };

        assert_eq!(segment_upward_tension_sign(&rising, 0), -1.0);
        assert_eq!(segment_upward_tension_sign(&falling, 0), 1.0);
    }

    #[test]
    fn upward_drag_bends_rising_segment_upward() {
        let mut curve = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.2 }, CurveNode { x: 1.0, y: 0.8 }],
            segments: vec![CurveSegment { tension: 0.0 }],
        };
        let baseline_mid = sample_editable_curve(&curve, 0.5);
        let delta = tension_delta_from_drag_for_segment(
            &curve,
            0,
            Point { x: 0, y: 80 },
            Point { x: 0, y: 40 },
            Size {
                width: CURVE_W,
                height: CURVE_H,
            },
        );
        curve.segments[0].tension = (curve.segments[0].tension + delta).clamp(
            crate::curve::MIN_SEGMENT_TENSION,
            crate::curve::MAX_SEGMENT_TENSION,
        );
        let dragged_mid = sample_editable_curve(&curve, 0.5);
        assert!(
            dragged_mid > baseline_mid,
            "upward drag should move midpoint up for rising segment"
        );
    }

    #[test]
    fn upward_drag_bends_falling_segment_upward() {
        let mut curve = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.8 }, CurveNode { x: 1.0, y: 0.2 }],
            segments: vec![CurveSegment { tension: 0.0 }],
        };
        let baseline_mid = sample_editable_curve(&curve, 0.5);
        let delta = tension_delta_from_drag_for_segment(
            &curve,
            0,
            Point { x: 0, y: 80 },
            Point { x: 0, y: 40 },
            Size {
                width: CURVE_W,
                height: CURVE_H,
            },
        );
        curve.segments[0].tension = (curve.segments[0].tension + delta).clamp(
            crate::curve::MIN_SEGMENT_TENSION,
            crate::curve::MAX_SEGMENT_TENSION,
        );
        let dragged_mid = sample_editable_curve(&curve, 0.5);
        assert!(
            dragged_mid > baseline_mid,
            "upward drag should move midpoint up for falling segment"
        );
    }

    fn curve_draw_commands_with_transport(
        phase: f32,
        is_playing: bool,
        has_host_beats_timeline: bool,
    ) -> (Vec<SurfaceCommand>, EditableCurve, PumpTheme) {
        let params = Arc::new(PumpParams::new());
        let status = Arc::new(GuiStatus::default());
        status.update(
            phase,
            1.0,
            is_playing,
            has_host_beats_timeline,
            phase.rem_euclid(1.0),
        );
        let state = GuiState::new(
            Arc::clone(&params),
            status,
            Arc::new(AutomationQueue::default()),
            None,
        );
        let metrics = UiLayoutMetrics::design_space();
        let theme = PumpTheme::main(metrics);
        let curve = params.editable_curve_snapshot();
        let commands = state.build_curve_draw_commands(
            &curve,
            metrics,
            CurveRenderState {
                selected_node: None,
                hovered_node: None,
                hovered_segment: None,
                preview_node: None,
            },
            &theme,
        );
        (commands, curve, theme)
    }

    fn fill_circle_centers_for_color(commands: &[SurfaceCommand], color: Color) -> Vec<Point> {
        commands
            .iter()
            .filter_map(|command| match command {
                SurfaceCommand::FillCircle {
                    center,
                    color: command_color,
                    ..
                } if *command_color == color => Some(*center),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn playhead_dot_hidden_when_transport_stopped_without_host_timeline() {
        let (commands, _curve, theme) = curve_draw_commands_with_transport(0.25, false, false);
        assert!(
            fill_circle_centers_for_color(&commands, theme.playhead_dot_glow).is_empty(),
            "glow dot should be hidden when transport is stopped and host timeline is unavailable"
        );
        assert!(
            fill_circle_centers_for_color(&commands, theme.playhead_dot_core).is_empty(),
            "core dot should be hidden when transport is stopped and host timeline is unavailable"
        );
    }

    #[test]
    fn playhead_dot_visible_when_transport_stopped_with_host_timeline() {
        let (commands, _curve, theme) = curve_draw_commands_with_transport(0.25, false, true);
        assert!(
            !fill_circle_centers_for_color(&commands, theme.playhead_dot_glow).is_empty(),
            "glow dot should remain visible when host timeline is available"
        );
        assert!(
            !fill_circle_centers_for_color(&commands, theme.playhead_dot_core).is_empty(),
            "core dot should remain visible when host timeline is available"
        );
    }

    #[test]
    fn playhead_dot_visible_without_host_beats_timeline_when_playing() {
        let (commands, _curve, theme) = curve_draw_commands_with_transport(0.25, true, false);
        assert!(
            !fill_circle_centers_for_color(&commands, theme.playhead_dot_glow).is_empty(),
            "glow dot should remain visible while transport is playing"
        );
        assert!(
            !fill_circle_centers_for_color(&commands, theme.playhead_dot_core).is_empty(),
            "core dot should remain visible while transport is playing"
        );
    }

    #[test]
    fn playhead_dot_visible_when_transport_running_with_beats_timeline() {
        let (commands, _curve, theme) = curve_draw_commands_with_transport(0.25, true, true);
        let glow_centers = fill_circle_centers_for_color(&commands, theme.playhead_dot_glow);
        let core_centers = fill_circle_centers_for_color(&commands, theme.playhead_dot_core);
        assert_eq!(glow_centers.len(), 1, "expected one glow playhead dot");
        assert_eq!(core_centers.len(), 1, "expected one core playhead dot");
        assert_eq!(
            core_centers[0], glow_centers[0],
            "playhead glow and core should share the same center"
        );
    }

    #[test]
    fn playhead_dot_tracks_curve_sample_at_host_phase() {
        let phase = 0.37;
        let (commands, curve, theme) = curve_draw_commands_with_transport(phase, true, true);
        let core_centers = fill_circle_centers_for_color(&commands, theme.playhead_dot_core);
        assert_eq!(core_centers.len(), 1, "expected one core playhead dot");
        let phase = phase.rem_euclid(1.0);
        let expected = local_from_node(CurveNode {
            x: phase,
            y: sample_editable_curve(&curve, phase).clamp(0.0, 1.0),
        });
        assert_eq!(
            core_centers[0], expected,
            "playhead dot should map to the sampled curve point at host phase"
        );
    }

    #[test]
    fn measured_open_size_is_at_least_default_window_baseline() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let (width, height) = state.measured_open_size();
        assert_eq!(width, WINDOW_WIDTH);
        assert_eq!(height, WINDOW_HEIGHT);
    }

    #[test]
    fn preferred_window_size_tracks_measured_layout() {
        let (preferred_width, preferred_height) = preferred_window_size();
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let (measured_width, measured_height) = state.measured_open_size();
        assert_eq!(preferred_width, measured_width);
        assert_eq!(preferred_height, measured_height);
        assert_eq!(preferred_width, WINDOW_WIDTH);
        assert_eq!(preferred_height, WINDOW_HEIGHT);
    }

    #[test]
    fn constrained_host_size_enforces_baseline_minimums() {
        assert_eq!(
            constrained_host_size(GuiSize {
                width: 1,
                height: 1,
            }),
            GuiSize {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            }
        );
        assert_eq!(
            constrained_host_size(GuiSize {
                width: WINDOW_WIDTH * 2,
                height: 40,
            }),
            GuiSize {
                width: WINDOW_WIDTH * 2,
                height: WINDOW_HEIGHT,
            }
        );
        assert_eq!(
            constrained_host_size(GuiSize {
                width: 64,
                height: WINDOW_HEIGHT * 2,
            }),
            GuiSize {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT * 2,
            }
        );
    }

    #[test]
    fn slot_height_split_matches_expected_ratios() {
        let (header_h, curve_h, controls_h) = resolve_vertical_slot_heights(WINDOW_HEIGHT);
        assert_eq!(header_h, 18);
        assert_eq!(curve_h, 163);
        assert_eq!(controls_h, 77);
        assert_eq!(header_h + curve_h + controls_h, WINDOW_HEIGHT);
    }

    #[test]
    fn bottom_row_split_matches_expected_ratio() {
        let (knobs_w, dropdown_w) = resolve_runtime_controls_slot_widths(WINDOW_WIDTH);
        assert_eq!(knobs_w, 294);
        assert_eq!(dropdown_w, 126);
        assert_eq!(knobs_w + dropdown_w, WINDOW_WIDTH);
    }

    #[test]
    fn runtime_slot_splits_consume_full_parent_extent() {
        let (header_h, curve_h, controls_h) = resolve_vertical_slot_heights(259);
        assert_eq!(header_h + curve_h + controls_h, 259);

        let (knobs_w, dropdown_w) = resolve_runtime_controls_slot_widths(799);
        assert_eq!(knobs_w + dropdown_w, 799);
    }

    #[test]
    fn build_ui_places_curve_region_at_full_spline_extent() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let spec = state.build_ui(&InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        });
        let region =
            find_curve_region_node(spec.root.content()).expect("curve region should exist");
        assert_eq!(region.width, WINDOW_WIDTH);
        assert_eq!(region.height, CURVE_H);
    }

    #[test]
    fn build_ui_keeps_design_sized_root_across_host_resize_sequences() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let input_sizes = [
            Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            Size {
                width: WINDOW_WIDTH * 3,
                height: WINDOW_HEIGHT * 3,
            },
            Size {
                width: WINDOW_WIDTH / 2,
                height: WINDOW_HEIGHT / 2,
            },
            Size {
                width: WINDOW_WIDTH * 2,
                height: WINDOW_HEIGHT * 2,
            },
        ];
        for window_size in input_sizes {
            let input = InputState {
                window_size,
                ..InputState::default()
            };
            let spec = state.build_ui(&input);
            let measured = measure_checked(&spec).expect("measurement should succeed");
            assert_eq!(measured.width, WINDOW_WIDTH);
            assert_eq!(measured.height, WINDOW_HEIGHT);
            assert_eq!(spec.root.scale_mode, RootScaleMode::UniformFit);
            assert_eq!(
                spec.root.design_size_value(),
                Some(Size {
                    width: WINDOW_WIDTH,
                    height: WINDOW_HEIGHT,
                })
            );
        }
    }

    #[test]
    fn build_ui_handles_tiny_window_sizes_without_measurement_errors() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let tiny_sizes = [
            Size {
                width: 1,
                height: 1,
            },
            Size {
                width: 2,
                height: 3,
            },
            Size {
                width: 3,
                height: 2,
            },
            Size {
                width: 8,
                height: 8,
            },
        ];

        for input_size in tiny_sizes {
            let input = InputState {
                window_size: input_size,
                ..InputState::default()
            };
            let spec = state.build_ui(&input);
            let measured = measure_checked(&spec).expect("measurement should succeed");
            assert_eq!(measured.width, WINDOW_WIDTH);
            assert_eq!(measured.height, WINDOW_HEIGHT);
            assert_eq!(spec.root.scale_mode, RootScaleMode::UniformFit);
            assert_eq!(
                spec.root.design_size_value(),
                Some(Size {
                    width: WINDOW_WIDTH,
                    height: WINDOW_HEIGHT,
                })
            );
        }
    }

    #[test]
    fn build_ui_handles_host_resize_jitter_without_layout_regressions() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let jitter_sizes = [
            Size {
                width: 1,
                height: 1,
            },
            Size {
                width: 640,
                height: 360,
            },
            Size {
                width: 2,
                height: 3,
            },
            Size {
                width: 1024,
                height: 256,
            },
            Size {
                width: 3,
                height: 2,
            },
            Size {
                width: 700,
                height: 700,
            },
            Size {
                width: 1,
                height: 1,
            },
            Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
        ];

        for input_size in jitter_sizes {
            let input = InputState {
                window_size: input_size,
                ..InputState::default()
            };
            let spec = state.build_ui(&input);
            let measured = measure_checked(&spec).expect("measurement should succeed");

            assert_eq!(measured.width, WINDOW_WIDTH);
            assert_eq!(measured.height, WINDOW_HEIGHT);
            assert_eq!(spec.root.scale_mode, RootScaleMode::UniformFit);
            assert_eq!(
                spec.root.design_size_value(),
                Some(Size {
                    width: WINDOW_WIDTH,
                    height: WINDOW_HEIGHT,
                })
            );

            let curve_region = find_curve_region_node(spec.root.content())
                .expect("curve region should exist for all measured sizes");
            assert_eq!(curve_region.width, WINDOW_WIDTH);
            assert_eq!(curve_region.height, CURVE_H);

            let dropdown_size = find_dropdown_control_size(spec.root.content(), DIVISION_KEY)
                .expect("division dropdown control size should exist for all measured sizes");
            let (_, expected_dropdown_w) = resolve_runtime_controls_slot_widths(WINDOW_WIDTH);
            assert_eq!(dropdown_size.width, expected_dropdown_w);
        }
    }

    #[test]
    fn build_ui_root_content_is_three_slot_column() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let spec = state.build_ui(&InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        });
        let root_grid = match expect_slot_child(spec.root.content(), "root") {
            Node::Grid(grid) => grid,
            other => panic!("expected root content grid, got {other:?}"),
        };
        assert_eq!(root_grid.children().len(), 3);
        let _header_panel = expect_slot_panel(&root_grid.children()[0], "header");
        let _curve_panel = expect_slot_panel(&root_grid.children()[1], "curve");
        let header_panel = expect_slot_panel(&root_grid.children()[0], "header");
        let header_switch = match expect_slot_child(header_panel.content(), "header") {
            Node::SwitchLayout(switch_layout) => switch_layout,
            other => panic!("expected header switch-layout in panel, got {other:?}"),
        };
        assert_eq!(header_switch.cases().len(), 1);
        let wide_case_child = header_switch
            .cases()
            .first()
            .expect("expected one wide-case child")
            .child();
        let wide_grid = match expect_slot_child(wide_case_child, "header-wide") {
            Node::Grid(grid) => grid,
            other => panic!("expected wide header grid, got {other:?}"),
        };
        assert_eq!(wide_grid.children().len(), 2);
        let _title_panel = expect_slot_panel(&wide_grid.children()[0], "header-title");
        let _subtitle_panel = expect_slot_panel(&wide_grid.children()[1], "header-subtitle");
        let compact_grid = match expect_slot_child(header_switch.fallback(), "header-compact") {
            Node::Grid(grid) => grid,
            other => panic!("expected compact header grid fallback, got {other:?}"),
        };
        assert_eq!(compact_grid.children().len(), 2);

        let controls_panel = expect_slot_panel(&root_grid.children()[2], "controls");
        let controls_grid = match expect_slot_child(controls_panel.content(), "controls") {
            Node::Grid(grid) => grid,
            other => panic!("expected controls grid in panel, got {other:?}"),
        };
        assert_eq!(controls_grid.children().len(), 2);
        let _knobs_panel = expect_slot_panel(&controls_grid.children()[0], "knobs");
        let _dropdown_panel = expect_slot_panel(&controls_grid.children()[1], "dropdowns");
    }

    #[test]
    fn build_ui_includes_textboxes_for_control_captions() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let spec = state.build_ui(&InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        });
        let mut texts = Vec::new();
        collect_textbox_texts(spec.root.content(), &mut texts);

        for expected in [
            "Mix",
            "Depth",
            "Phase",
            "Output",
            sync_division_label(crate::params::DEFAULT_SYNC_DIVISION_INDEX),
            "Reset Curve",
        ] {
            assert!(
                texts.iter().any(|text| text == expected),
                "expected textbox caption `{expected}` in {:?}",
                texts
            );
        }
    }

    #[test]
    fn header_transport_indicator_reflects_transport_blink_state() {
        let params = Arc::new(PumpParams::new());
        let status = Arc::new(GuiStatus::default());
        let state = GuiState::new(
            params,
            Arc::clone(&status),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let input = InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        };

        status.update(0.0, 1.0, true, true, 0.05);
        let lit_spec = state.build_ui(&input);
        let lit = find_first_indicator_active(lit_spec.root.content())
            .expect("header transport indicator should exist");
        assert!(lit, "indicator should blink on at beat onset");

        status.update(0.0, 1.0, true, true, 0.5);
        let dim_spec = state.build_ui(&input);
        let dim = find_first_indicator_active(dim_spec.root.content())
            .expect("header transport indicator should exist");
        assert!(!dim, "indicator should be off between beat flashes");

        status.update(0.0, 1.0, true, false, 0.05);
        let no_timeline_spec = state.build_ui(&input);
        let no_timeline = find_first_indicator_active(no_timeline_spec.root.content())
            .expect("header transport indicator should exist");
        assert!(
            !no_timeline,
            "indicator should stay off when host beat timeline is unavailable"
        );
    }

    #[test]
    fn header_transport_indicator_has_expected_fixed_size() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let spec = state.build_ui(&InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        });
        let size = find_first_indicator_size(spec.root.content())
            .expect("header transport indicator should exist");
        assert_eq!(
            size,
            Size {
                width: TRANSPORT_INDICATOR_SIZE,
                height: TRANSPORT_INDICATOR_SIZE,
            }
        );
    }

    #[test]
    fn header_switch_selects_compact_and_wide_variants_by_root_width() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let compact_spec = state.build_ui(&InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        });
        let compact_switch = extract_header_switch(&compact_spec);
        let compact_child = compact_switch.selected_child(WINDOW_WIDTH);
        let compact_grid = match expect_slot_child(compact_child, "compact-header") {
            Node::Grid(grid) => grid,
            other => panic!("expected compact header grid, got {other:?}"),
        };
        assert_eq!(compact_grid.kind(), GridKind::SlotColumn);

        let wide_width = HEADER_SWITCH_WIDE_MIN_WIDTH.saturating_add(1);
        let wide_spec = state.build_ui(&InputState {
            window_size: Size {
                width: wide_width,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        });
        let wide_switch = extract_header_switch(&wide_spec);
        let wide_child = wide_switch.selected_child(wide_width);
        let wide_grid = match expect_slot_child(wide_child, "wide-header") {
            Node::Grid(grid) => grid,
            other => panic!("expected wide header grid, got {other:?}"),
        };
        assert_eq!(wide_grid.kind(), GridKind::SlotRow);
    }

    #[test]
    fn emitted_ui_spec_passes_strict_slot_validation() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let sizes = [
            Size {
                width: 1,
                height: 1,
            },
            Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            Size {
                width: WINDOW_WIDTH * 2,
                height: WINDOW_HEIGHT * 2,
            },
        ];

        for size in sizes {
            let spec = state.build_ui(&InputState {
                window_size: size,
                ..InputState::default()
            });
            measure_checked(&spec).expect("emitted tree must pass strict declarative validation");
            assert_emitted_slot_tree_invariants(&spec);
        }
    }

    #[test]
    fn pump_knob_block_top_borders_tile_without_horizontal_gaps() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let frame = render_spec_to_frame(
            Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            |input| state.build_ui(input),
        )
        .expect("pump frame should render");

        let (header_h, curve_h, _) = resolve_vertical_slot_heights(WINDOW_HEIGHT);
        let controls_top = header_h.saturating_add(curve_h);
        let base_row = controls_top.min(frame.height.saturating_sub(1));
        let (knobs_w, _) = resolve_runtime_controls_slot_widths(WINDOW_WIDTH);
        let border = MainPalette::main().text_primary;

        let end_row = base_row
            .saturating_add(12)
            .min(frame.height.saturating_sub(1));
        let mut best_runs: Vec<(u32, u32)> = Vec::new();
        let mut best_coverage = 0u32;
        for y in base_row..=end_row {
            let runs = color_runs_on_row(
                &frame.pixels,
                frame.width,
                y,
                0,
                knobs_w.saturating_sub(1),
                border,
            );
            let coverage = runs
                .iter()
                .map(|(start, end)| end.saturating_sub(*start).saturating_add(1))
                .sum::<u32>();
            if coverage > best_coverage {
                best_coverage = coverage;
                best_runs = runs;
            }
        }

        let significant_runs: Vec<(u32, u32)> = best_runs
            .into_iter()
            .filter(|(start, end)| end.saturating_sub(*start).saturating_add(1) >= 12)
            .collect();

        assert_eq!(
            significant_runs.len(),
            1,
            "expected one contiguous knob-border run in pump knob slot, got {:?}",
            significant_runs
        );
    }

    fn find_curve_region_node(node: &Node) -> Option<Size> {
        match node {
            Node::Slot(slot) => find_curve_region_node(slot.child()),
            Node::Region(region) if region.key == CURVE_KEY => Some(region.size),
            Node::Panel(panel) => find_curve_region_node(panel.content()),
            Node::PaddingBox(padding_box) => find_curve_region_node(padding_box.content()),
            Node::AlignBox(align_box) => find_curve_region_node(align_box.content()),
            Node::AspectBox(aspect_box) => find_curve_region_node(aspect_box.content()),
            Node::Row(flex) | Node::Column(flex) => {
                flex.children().iter().find_map(find_curve_region_node)
            }
            Node::Grid(grid) => grid.children().iter().find_map(find_curve_region_node),
            Node::Stack(stack) => stack.children().iter().find_map(find_curve_region_node),
            Node::ScrollView(scroll_view) => find_curve_region_node(scroll_view.content()),
            Node::Wrap(wrap) => wrap.children().iter().find_map(find_curve_region_node),
            Node::SwitchLayout(switch_layout) => switch_layout
                .cases()
                .iter()
                .find_map(|case_entry| find_curve_region_node(case_entry.child()))
                .or_else(|| find_curve_region_node(switch_layout.fallback())),
            Node::Region(_) => None,
            Node::TextBox(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::Indicator(_)
            | Node::Absolute(_) => None,
        }
    }

    fn collect_textbox_texts(node: &Node, texts: &mut Vec<String>) {
        match node {
            Node::Slot(slot) => collect_textbox_texts(slot.child(), texts),
            Node::Panel(panel) => collect_textbox_texts(panel.content(), texts),
            Node::PaddingBox(padding_box) => collect_textbox_texts(padding_box.content(), texts),
            Node::AlignBox(align_box) => collect_textbox_texts(align_box.content(), texts),
            Node::AspectBox(aspect_box) => collect_textbox_texts(aspect_box.content(), texts),
            Node::Row(flex) | Node::Column(flex) => {
                for child in flex.children() {
                    collect_textbox_texts(child, texts);
                }
            }
            Node::Grid(grid) => {
                for child in grid.children() {
                    collect_textbox_texts(child, texts);
                }
            }
            Node::Absolute(absolute) => {
                for child in absolute.children() {
                    collect_textbox_texts(child.node(), texts);
                }
            }
            Node::Stack(stack) => {
                for child in stack.children() {
                    collect_textbox_texts(child, texts);
                }
            }
            Node::ScrollView(scroll_view) => collect_textbox_texts(scroll_view.content(), texts),
            Node::Wrap(wrap) => {
                for child in wrap.children() {
                    collect_textbox_texts(child, texts);
                }
            }
            Node::SwitchLayout(switch_layout) => {
                for case_entry in switch_layout.cases() {
                    collect_textbox_texts(case_entry.child(), texts);
                }
                collect_textbox_texts(switch_layout.fallback(), texts);
            }
            Node::TextBox(text_box) => texts.push(text_box.text.clone()),
            Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::Region(_)
            | Node::Indicator(_) => {}
        }
    }

    fn find_dropdown_control_size(node: &Node, key: &str) -> Option<Size> {
        match node {
            Node::Slot(slot) => find_dropdown_control_size(slot.child(), key),
            Node::Dropdown(dropdown) if dropdown.key == key => dropdown.control_size,
            Node::Panel(panel) => find_dropdown_control_size(panel.content(), key),
            Node::PaddingBox(padding_box) => find_dropdown_control_size(padding_box.content(), key),
            Node::AlignBox(align_box) => find_dropdown_control_size(align_box.content(), key),
            Node::AspectBox(aspect_box) => find_dropdown_control_size(aspect_box.content(), key),
            Node::Row(flex) | Node::Column(flex) => flex
                .children()
                .iter()
                .find_map(|child| find_dropdown_control_size(child, key)),
            Node::Grid(grid) => grid
                .children()
                .iter()
                .find_map(|child| find_dropdown_control_size(child, key)),
            Node::Stack(stack) => stack
                .children()
                .iter()
                .find_map(|child| find_dropdown_control_size(child, key)),
            Node::ScrollView(scroll_view) => find_dropdown_control_size(scroll_view.content(), key),
            Node::Wrap(wrap) => wrap
                .children()
                .iter()
                .find_map(|child| find_dropdown_control_size(child, key)),
            Node::SwitchLayout(switch_layout) => switch_layout
                .cases()
                .iter()
                .find_map(|case_entry| find_dropdown_control_size(case_entry.child(), key))
                .or_else(|| find_dropdown_control_size(switch_layout.fallback(), key)),
            Node::Dropdown(_) => None,
            Node::TextBox(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Region(_)
            | Node::Indicator(_)
            | Node::Absolute(_) => None,
        }
    }

    fn find_first_indicator_active(node: &Node) -> Option<bool> {
        match node {
            Node::Slot(slot) => find_first_indicator_active(slot.child()),
            Node::Indicator(indicator) => Some(indicator.active),
            Node::Panel(panel) => find_first_indicator_active(panel.content()),
            Node::PaddingBox(padding_box) => find_first_indicator_active(padding_box.content()),
            Node::AlignBox(align_box) => find_first_indicator_active(align_box.content()),
            Node::AspectBox(aspect_box) => find_first_indicator_active(aspect_box.content()),
            Node::Row(flex) | Node::Column(flex) => {
                flex.children().iter().find_map(find_first_indicator_active)
            }
            Node::Grid(grid) => grid.children().iter().find_map(find_first_indicator_active),
            Node::Stack(stack) => stack
                .children()
                .iter()
                .find_map(find_first_indicator_active),
            Node::ScrollView(scroll_view) => find_first_indicator_active(scroll_view.content()),
            Node::Wrap(wrap) => wrap.children().iter().find_map(find_first_indicator_active),
            Node::SwitchLayout(switch_layout) => switch_layout
                .cases()
                .iter()
                .find_map(|case_entry| find_first_indicator_active(case_entry.child()))
                .or_else(|| find_first_indicator_active(switch_layout.fallback())),
            Node::Absolute(absolute) => absolute
                .children()
                .iter()
                .find_map(|child| find_first_indicator_active(child.node())),
            Node::TextBox(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::Region(_) => None,
        }
    }

    fn find_first_indicator_size(node: &Node) -> Option<Size> {
        match node {
            Node::Slot(slot) => find_first_indicator_size(slot.child()),
            Node::Indicator(indicator) => Some(indicator.size),
            Node::Panel(panel) => find_first_indicator_size(panel.content()),
            Node::PaddingBox(padding_box) => find_first_indicator_size(padding_box.content()),
            Node::AlignBox(align_box) => find_first_indicator_size(align_box.content()),
            Node::AspectBox(aspect_box) => find_first_indicator_size(aspect_box.content()),
            Node::Row(flex) | Node::Column(flex) => {
                flex.children().iter().find_map(find_first_indicator_size)
            }
            Node::Grid(grid) => grid.children().iter().find_map(find_first_indicator_size),
            Node::Stack(stack) => stack.children().iter().find_map(find_first_indicator_size),
            Node::ScrollView(scroll_view) => find_first_indicator_size(scroll_view.content()),
            Node::Wrap(wrap) => wrap.children().iter().find_map(find_first_indicator_size),
            Node::SwitchLayout(switch_layout) => switch_layout
                .cases()
                .iter()
                .find_map(|case_entry| find_first_indicator_size(case_entry.child()))
                .or_else(|| find_first_indicator_size(switch_layout.fallback())),
            Node::Absolute(absolute) => absolute
                .children()
                .iter()
                .find_map(|child| find_first_indicator_size(child.node())),
            Node::TextBox(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::Region(_) => None,
        }
    }

    fn color_runs_on_row(
        pixels: &[u8],
        frame_width: u32,
        y: u32,
        x_start: u32,
        x_end: u32,
        color: toybox::gui::Color,
    ) -> Vec<(u32, u32)> {
        if frame_width == 0 || pixels.is_empty() || x_start > x_end {
            return Vec::new();
        }
        let mut runs = Vec::new();
        let mut active_start: Option<u32> = None;
        for x in x_start..=x_end {
            let idx =
                ((y.saturating_mul(frame_width).saturating_add(x)).saturating_mul(4)) as usize;
            if idx + 3 >= pixels.len() {
                break;
            }
            let matches = pixels[idx] == color.r
                && pixels[idx + 1] == color.g
                && pixels[idx + 2] == color.b
                && pixels[idx + 3] != 0;
            match (active_start, matches) {
                (None, true) => active_start = Some(x),
                (Some(start), false) => {
                    runs.push((start, x.saturating_sub(1)));
                    active_start = None;
                }
                _ => {}
            }
        }
        if let Some(start) = active_start {
            runs.push((start, x_end));
        }
        runs
    }
}

#[cfg(all(test, feature = "screenshot-test", target_os = "windows"))]
mod screenshot_tests {
    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crate::GuiStatus;
    use raw_window_handle::{RawWindowHandle, Win32WindowHandle};
    use toybox::gui::screenshot_harness;
    use windows::core::w;
    use windows::Win32::Foundation::{HINSTANCE, HMENU, HWND};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetClientRect,
        GetDIBits, GetWindowDC, Rect, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, ShowWindow, UpdateWindow, SW_SHOW, WS_OVERLAPPEDWINDOW,
    };

    use super::{AutomationQueue, PumpGui, PumpParams, WINDOW_HEIGHT, WINDOW_WIDTH};

    #[test]
    fn screenshot_renders_initial_ui() {
        if !screenshot_harness::screenshots_enabled() {
            return;
        }

        let sizes = screenshot_harness::default_screenshot_sizes(toybox::gui::Size {
            width: WINDOW_WIDTH,
            height: WINDOW_HEIGHT,
        });
        for size in sizes {
            render_and_capture_at_size(size.width, size.height);
        }
    }

    fn render_and_capture_at_size(width: u32, height: u32) {
        let mut gui = PumpGui::default();
        let params = Arc::new(PumpParams::new());
        let status = Arc::new(GuiStatus::default());
        let host = parent_window(width, height);
        let queue = Arc::new(AutomationQueue::default());

        gui.set_parent_raw(host.raw_handle());
        gui.open(&params, &status, queue, None)
            .expect("open should succeed");
        let hwnd = wait_for_window_handle(&gui);
        wait_for_any_logical_size(&gui);
        gui.request_resize(width, height);
        wait_for_logical_size(&gui, (width, height));

        std::thread::sleep(Duration::from_millis(75));

        let path = screenshot_path(env!("CARGO_PKG_NAME"), width, height);
        let (captured_width, captured_height) =
            capture_hwnd(hwnd, &path).expect("failed to capture screenshot");
        assert!(
            captured_width == width && captured_height == height,
            "captured image should be exactly {width}x{height}, got {captured_width}x{captured_height}"
        );

        gui.close();
        assert!(path.exists());
    }

    fn parent_window(width: u32, height: u32) -> ScreenshotParentWindow {
        ScreenshotParentWindow::new(width, height)
    }

    fn wait_for_any_logical_size(gui: &PumpGui) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Some(size) = gui.last_size() {
                if size.0 > 0 && size.1 > 0 {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("plugin GUI never reported any logical size");
    }

    fn wait_for_logical_size(gui: &PumpGui, min_size: (u32, u32)) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Some((width, height)) = gui.last_size() {
                if width >= min_size.0 && height >= min_size.1 {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "plugin GUI never reached logical size >= {min_size:?}, last size={:?}",
            gui.last_size()
        );
    }

    fn wait_for_window_handle(gui: &PumpGui) -> HWND {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Some(handle) = gui.handle() {
                if handle.is_valid() {
                    return handle.hwnd();
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("plugin GUI handle did not become available");
    }

    fn screenshot_path(plugin: &str, width: u32, height: u32) -> PathBuf {
        screenshot_harness::screenshot_output_path(plugin, width, height)
            .expect("resolve screenshot path")
    }

    fn capture_hwnd(hwnd: HWND, out: &PathBuf) -> Result<(u32, u32), String> {
        let mut client = Rect::default();
        unsafe {
            GetClientRect(hwnd, &mut client)
                .map_err(|err| format!("GetClientRect failed: {err}"))?;
        };

        let width = u32::try_from(client.right.saturating_sub(client.left))
            .map_err(|_| "invalid client width".to_string())?;
        let height = u32::try_from(client.bottom.saturating_sub(client.top))
            .map_err(|_| "invalid client height".to_string())?;

        if width == 0 || height == 0 {
            return Err("screenshot target has empty geometry".into());
        }

        let source_dc = unsafe { GetWindowDC(hwnd) };
        if source_dc.is_invalid() {
            return Err("GetWindowDC returned invalid DC".into());
        }

        let memory_dc = unsafe { CreateCompatibleDC(source_dc) };
        if memory_dc.is_invalid() {
            unsafe {
                let _ = ReleaseDC(hwnd, source_dc);
            }
            return Err("CreateCompatibleDC returned invalid DC".into());
        }

        let bitmap = unsafe { CreateCompatibleBitmap(source_dc, width as i32, height as i32) };
        if bitmap.is_invalid() {
            unsafe {
                let _ = DeleteDC(memory_dc);
                let _ = ReleaseDC(hwnd, source_dc);
            }
            return Err("CreateCompatibleBitmap returned invalid bitmap".into());
        }

        let old_object = unsafe { SelectObject(memory_dc, HGDIOBJ::from(bitmap)) };
        let bitblt_ok = unsafe {
            BitBlt(
                memory_dc,
                0,
                0,
                width as i32,
                height as i32,
                source_dc,
                0,
                0,
                SRCCOPY,
            )
            .is_ok()
        };
        if !bitblt_ok {
            unsafe {
                let _ = SelectObject(memory_dc, old_object);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(memory_dc);
                let _ = ReleaseDC(hwnd, source_dc);
            }
            return Err("BitBlt failed".into());
        }

        let mut bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [Default::default(); 1],
        };

        let pixel_len = usize::try_from(width)
            .ok()
            .and_then(|w| {
                usize::try_from(height)
                    .ok()
                    .map(|h| w.saturating_mul(h).saturating_mul(4))
            })
            .ok_or_else(|| "invalid pixel dimensions".to_string())?;
        let mut pixels = vec![0_u8; pixel_len];

        let got = unsafe {
            GetDIBits(
                memory_dc,
                bitmap,
                0,
                height,
                Some(pixels.as_mut_ptr().cast()),
                &mut bitmap_info,
                DIB_RGB_COLORS,
            )
        };

        unsafe {
            let _ = SelectObject(memory_dc, old_object);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory_dc);
            let _ = ReleaseDC(hwnd, source_dc);
        }

        if got == 0 {
            return Err("GetDIBits returned no rows".into());
        }

        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }

        screenshot_harness::write_png_rgba8(out, width, height, pixels)?;
        Ok((width, height))
    }

    struct ScreenshotParentWindow {
        hwnd: HWND,
    }

    impl ScreenshotParentWindow {
        fn new(width: u32, height: u32) -> Self {
            let hwnd = unsafe {
                CreateWindowExW(
                    Default::default(),
                    w!("STATIC"),
                    w!("toybox-screenshot-host"),
                    WS_OVERLAPPEDWINDOW,
                    0,
                    0,
                    width as i32,
                    height as i32,
                    HWND::default(),
                    HMENU::default(),
                    HINSTANCE::default(),
                    None,
                )
            };

            if hwnd.0 == 0 {
                panic!("CreateWindowExW failed");
            }

            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = UpdateWindow(hwnd);
            }

            Self { hwnd }
        }

        fn raw_handle(&self) -> RawWindowHandle {
            let mut handle = Win32WindowHandle::empty();
            handle.hwnd = self.hwnd.0 as *mut c_void;
            handle.hinstance = HINSTANCE::default().0 as *mut c_void;
            RawWindowHandle::Win32(handle)
        }
    }

    impl Drop for ScreenshotParentWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}
