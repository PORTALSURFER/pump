//! Declarative curve-editor GUI for Pump.

use std::sync::{Arc, Mutex, OnceLock};

use toybox::clack_extensions::gui::{GuiSize, Window};
use toybox::clack_plugin::plugin::PluginError;
use toybox::clack_plugin::utils::ClapId;
use toybox::clap::automation::{AutomationConfig, AutomationQueue};
use toybox::clap::gui::{GuiHostWindow, InputState};
use toybox::gui::declarative::{
    button, column, column_sections, dropdown, grid, knob, label, panel, root_frame_sized,
    row_sections, weighted, AbsoluteChild, AbsoluteSpec, DrawCommand, GridTemplate, LayoutBox,
    Node, RegionInteractionKind, RegionSpec, ThemeTokens, TrackSize, UiAction, UiSpec,
};
use toybox::gui::{Color, MainPalette, Point, Rect, Size};
use toybox::raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use crate::curve::{
    sample_editable_curve, CurveNode, CurveSegment, EditableCurve, MAX_EDITABLE_NODES,
    MAX_SEGMENT_TENSION, MIN_SEGMENT_TENSION,
};
use crate::params::{
    sync_division_label, sync_division_labels, PumpParams, MAX_DEPTH, MAX_MIX, MAX_OUTPUT_GAIN_DB,
    MAX_PHASE_OFFSET, MAX_SYNC_DIVISION, MIN_DEPTH, MIN_MIX, MIN_OUTPUT_GAIN_DB, MIN_PHASE_OFFSET,
    PARAM_DEPTH_ID, PARAM_MIX_ID, PARAM_OUTPUT_GAIN_ID, PARAM_PHASE_OFFSET_ID,
    PARAM_SYNC_DIVISION_ID,
};
use crate::{GuiStatus, HostParamRequester};

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

const ROOT_KEY: &str = "pump-root";
const CURVE_KEY: &str = "curve";
const MIX_KEY: &str = "mix";
const DEPTH_KEY: &str = "depth";
const PHASE_KEY: &str = "phase";
const OUTPUT_KEY: &str = "output";
const DIVISION_KEY: &str = "division";
const RESET_KEY: &str = "reset";

const PADDING_X: i32 = 18;
const HEADER_SECTION_WEIGHT: u16 = 7;
const CURVE_SECTION_WEIGHT: u16 = 63;
const CONTROLS_SECTION_WEIGHT: u16 = 30;
const ROOT_SECTION_WEIGHT_SUM: u32 =
    HEADER_SECTION_WEIGHT as u32 + CURVE_SECTION_WEIGHT as u32 + CONTROLS_SECTION_WEIGHT as u32;
const KNOBS_SECTION_WEIGHT: u16 = 70;
const DROPDOWN_SECTION_WEIGHT: u16 = 30;
const CONTROLS_SECTION_WEIGHT_SUM: u32 =
    KNOBS_SECTION_WEIGHT as u32 + DROPDOWN_SECTION_WEIGHT as u32;
const SPLINE_TITLE_Y: i32 = 4;
const CURVE_W: u32 = WINDOW_WIDTH;
const CURVE_H: u32 = resolve_vertical_section_heights(WINDOW_HEIGHT).1;
const TITLE_LABEL_W: u32 = 64;
const LABEL_LINE_H: u32 = 16;
const NODE_DRAW_RADIUS: i32 = 4;
const NODE_HIT_RADIUS: i32 = 8;
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

const fn u32_max(left: u32, right: u32) -> u32 {
    if left > right {
        left
    } else {
        right
    }
}

const fn resolve_vertical_section_heights(total_height: u32) -> (u32, u32, u32) {
    let clamped_total = u32_max(total_height, 1);
    let header_h =
        clamped_total.saturating_mul(HEADER_SECTION_WEIGHT as u32) / ROOT_SECTION_WEIGHT_SUM;
    let controls_h =
        clamped_total.saturating_mul(CONTROLS_SECTION_WEIGHT as u32) / ROOT_SECTION_WEIGHT_SUM;
    let consumed = header_h.saturating_add(controls_h);
    let curve_h = clamped_total.saturating_sub(consumed);
    (header_h, curve_h, controls_h)
}

const fn resolve_controls_section_widths(total_width: u32) -> (u32, u32) {
    let clamped_total = u32_max(total_width, 1);
    let mut knobs_w =
        clamped_total.saturating_mul(KNOBS_SECTION_WEIGHT as u32) / CONTROLS_SECTION_WEIGHT_SUM;
    if knobs_w >= clamped_total {
        knobs_w = clamped_total.saturating_sub(1);
    }
    let dropdown_w = u32_max(clamped_total.saturating_sub(knobs_w), 1);
    (knobs_w, dropdown_w)
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
    playhead: Color,
    meter_outline: Color,
    meter_fill: Color,
}

impl PumpTheme {
    /// Return the canonical Pump GUI theme.
    fn main() -> Self {
        let palette = MainPalette::main();
        let tokens = ThemeTokens::main();
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
            playhead: palette.accent_focus,
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
        let state = GuiState::new(
            Arc::clone(params),
            Arc::clone(status),
            automation_queue,
            param_requester,
        );
        let open_size = state.measured_open_size();

        self.window.open_parented(
            "pump".to_string(),
            open_size,
            state,
            |_state| {},
            |input, state| state.build_ui(input),
            |state, action| state.reduce_action(action),
        )
    }

    /// Request a logical resize from the GUI thread.
    pub fn request_resize(&self, width: u32, height: u32) {
        self.window.request_resize(width, height);
    }

    /// Return true when host-driven resizing is enabled.
    pub fn host_resize_enabled(&self) -> bool {
        self.window.host_resize_enabled()
    }

    /// Resolve host-adjusted size according to the configured resize policy.
    pub fn adjust_host_size(&self, size: GuiSize) -> Option<GuiSize> {
        self.window.adjust_host_size(size)
    }

    /// Apply a host-provided size using Toybox's canonical resize behavior.
    pub fn apply_host_size(&self, size: GuiSize) {
        self.window.apply_host_size(size);
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

    fn build_ui(&self, input: &InputState) -> UiSpec {
        let content_w = input.window_size.width.max(WINDOW_WIDTH);
        let content_h = input.window_size.height.max(WINDOW_HEIGHT);
        let theme = PumpTheme::main();
        let (_header_h, curve_h, _controls_h) = resolve_vertical_section_heights(content_h);
        let (_knobs_section_w, dropdown_section_w) = resolve_controls_section_widths(content_w);
        let dropdown_control_w = dropdown_section_w.saturating_sub(8).max(64);
        let label_line_h = LABEL_LINE_H;
        let padding_x = PADDING_X;
        let spline_title_y = SPLINE_TITLE_Y;
        let spline_tip_y = curve_h as i32 - label_line_h as i32 - 4;
        let subtitle_label_w =
            content_w.saturating_sub((padding_x.max(0) as u32).saturating_add(72));
        let subtitle_label_w = subtitle_label_w.max(1);
        let tip_label_w = content_w.saturating_sub((padding_x.max(0) as u32).saturating_add(8));
        let tip_label_w = tip_label_w.max(1);

        let curve_size = Size {
            width: content_w,
            height: curve_h,
        };
        let (selected_node, curve_hovered, curve_local_pointer) =
            if let Ok(mut runtime) = self.runtime.lock() {
                runtime.curve_size = curve_size;
                (
                    runtime.selected_node,
                    runtime.curve_hovered,
                    runtime.curve_local_pointer,
                )
            } else {
                (None, false, Point { x: 0, y: 0 })
            };

        let mix = self.params.mix();
        let depth = self.params.depth();
        let phase_offset = self.params.phase_offset();
        let output_gain_db = self.params.output_gain_db();
        let division = self.params.sync_division();

        let editable_curve = self.params.editable_curve_snapshot();
        let hovered_node = curve_hovered
            .then(|| find_node_hit(&editable_curve, curve_local_pointer))
            .flatten();
        let direct_segment = curve_hovered
            .then(|| {
                find_segment_line_hit_within(
                    &editable_curve,
                    curve_local_pointer,
                    SEGMENT_DIRECT_HIT_RADIUS,
                )
            })
            .flatten();
        let preview_node = (curve_hovered
            && !input.alt_down
            && hovered_node.is_none()
            && direct_segment.is_some())
        .then(|| preview_node_on_curve(&editable_curve, curve_local_pointer))
        .flatten();
        let hovered_segment = (curve_hovered && preview_node.is_none())
            .then(|| {
                find_segment_line_hit_within(
                    &editable_curve,
                    curve_local_pointer,
                    SEGMENT_NEAR_HIT_RADIUS,
                )
            })
            .flatten();
        let draw_commands = self.build_curve_draw_commands(
            &editable_curve,
            curve_size,
            selected_node,
            hovered_node,
            hovered_segment,
            preview_node,
            &theme,
        );
        let spline_controls = vec![
            AbsoluteChild::new(
                Point { x: 0, y: 0 },
                Node::Region(
                    RegionSpec::new(
                        CURVE_KEY,
                        Size {
                            width: content_w,
                            height: curve_h,
                        },
                    )
                    .draw_commands(draw_commands),
                ),
            ),
            AbsoluteChild::new(
                Point {
                    x: padding_x,
                    y: spline_tip_y,
                },
                label("Tip: double-click node deletes; direct-curve click adds node; near drag moves line; Alt+drag adjusts curve.")
                    .text_color(theme.hint_text)
                    .layout(fixed_box(tip_label_w, label_line_h)),
            ),
        ];

        let header_controls = vec![
            AbsoluteChild::new(
                Point {
                    x: padding_x,
                    y: spline_title_y,
                },
                label("PUMP")
                    .text_color(theme.title_text)
                    .layout(fixed_box(TITLE_LABEL_W, label_line_h)),
            ),
            AbsoluteChild::new(
                Point {
                    x: padding_x + 72,
                    y: spline_title_y,
                },
                label("Spline Beat-Synced Ducking")
                    .text_color(theme.subtitle_text)
                    .layout(fixed_box(subtitle_label_w, label_line_h)),
            ),
        ];

        let header_content =
            Node::Absolute(AbsoluteSpec::new(header_controls).layout(LayoutBox::fill()))
                .layout(LayoutBox::fill());

        let spline_content =
            Node::Absolute(AbsoluteSpec::new(spline_controls).layout(LayoutBox::fill()))
                .layout(LayoutBox::fill());

        let header_section = panel("header", header_content).pad_all(0);

        let spline_section = panel("spline", spline_content)
            .background(theme.curve_bg)
            .outline(theme.curve_border)
            .pad_all(0);

        let knobs_grid = grid(
            GridTemplate::new(vec![TrackSize::Auto; 4])
                .rows(vec![TrackSize::Fr(1)])
                .gap(0)
                .justify_start(),
            vec![
                knob(MIX_KEY, "Mix", mix, (MIN_MIX, MAX_MIX))
                    .value_label(format!("{:.0}%", mix * 100.0)),
                knob(DEPTH_KEY, "Depth", depth, (MIN_DEPTH, MAX_DEPTH))
                    .value_label(format!("{:.0}%", depth * 100.0)),
                knob(
                    PHASE_KEY,
                    "Phase",
                    phase_offset,
                    (MIN_PHASE_OFFSET, MAX_PHASE_OFFSET),
                )
                .value_label(format!("{:.0}%", phase_offset * 100.0)),
                knob(
                    OUTPUT_KEY,
                    "Output",
                    output_gain_db,
                    (MIN_OUTPUT_GAIN_DB, MAX_OUTPUT_GAIN_DB),
                )
                .value_label(format!("{output_gain_db:+.1} dB")),
            ],
        )
        .layout(LayoutBox::fill());

        let knobs_section = panel("knobs", knobs_grid.layout(LayoutBox::fill())).pad_all(8);

        let dropdown_section_content = column(vec![
            dropdown(
                DIVISION_KEY,
                "Division",
                sync_division_labels(),
                division.min(MAX_SYNC_DIVISION as usize),
            )
            .control_size(Size {
                width: dropdown_control_w,
                height: 24,
            }),
            button(RESET_KEY, "Reset Curve").control_size(Size {
                width: dropdown_control_w,
                height: 24,
            }),
            label(format!("Cycle: {}", sync_division_label(division)))
                .text_color(theme.hint_text)
                .layout(fixed_box(dropdown_control_w, label_line_h)),
        ])
        .gap(4)
        .pad_all(0)
        .layout(LayoutBox::fill());

        let dropdown_section = panel(
            "dropdown",
            dropdown_section_content.layout(LayoutBox::fill()),
        )
        .pad_all(8);

        let controls_row = row_sections(vec![
            weighted(knobs_section, KNOBS_SECTION_WEIGHT),
            weighted(dropdown_section, DROPDOWN_SECTION_WEIGHT),
        ]);

        let controls_section = panel("controls", controls_row).pad_all(0);

        let content = column_sections(vec![
            weighted(header_section, HEADER_SECTION_WEIGHT),
            weighted(spline_section, CURVE_SECTION_WEIGHT),
            weighted(controls_section, CONTROLS_SECTION_WEIGHT),
        ]);

        UiSpec::new(
            root_frame_sized(
                ROOT_KEY,
                content,
                Size {
                    width: WINDOW_WIDTH,
                    height: WINDOW_HEIGHT,
                },
                input.window_size,
            )
            .padding(0)
            .tokens(theme.tokens),
        )
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
        let normalized_pointer = node_from_local(local_pointer);
        let raw_normalized_pointer = node_from_local(raw_local_pointer);

        match kind {
            RegionInteractionKind::Pressed => {
                let mut editable = self.params.editable_curve_snapshot();
                if let Some(index) = find_node_hit(&editable, local_pointer) {
                    runtime.selected_node = Some(index);
                    runtime.drag_mode = Some(CurveDragMode::MoveNode {
                        index,
                        start_pointer: local_pointer,
                        dragging: false,
                    });
                    return;
                }

                if !alt_down
                    && find_segment_line_hit_within(
                        &editable,
                        local_pointer,
                        SEGMENT_DIRECT_HIT_RADIUS,
                    )
                    .is_some()
                {
                    let preview_node = preview_node_on_curve(&editable, local_pointer)
                        .unwrap_or(normalized_pointer);
                    let inserted_index = insert_node(&mut editable, preview_node);
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

                if let Some(index) =
                    find_segment_line_hit_within(&editable, local_pointer, SEGMENT_NEAR_HIT_RADIUS)
                {
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

                if let Some(index) =
                    find_node_hit_within(&editable, local_pointer, NODE_INSERT_GUARD_RADIUS)
                {
                    runtime.selected_node = Some(index);
                    runtime.drag_mode = Some(CurveDragMode::MoveNode {
                        index,
                        start_pointer: local_pointer,
                        dragging: false,
                    });
                    return;
                }

                let inserted_index = insert_node(&mut editable, normalized_pointer);
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
                                    CURVE_DRAG_START_THRESHOLD_PX,
                                )
                            {
                                return;
                            }
                            dragging = true;
                            let moved_index = move_node_with_push_through(
                                &mut editable,
                                index,
                                raw_normalized_pointer,
                                NODE_PUSH_THROUGH_PX,
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
                                    CURVE_DRAG_START_THRESHOLD_PX,
                                )
                            {
                                return;
                            }
                            dragging = true;
                            let delta_x = (raw_local_pointer.x - start_pointer.x) as f32
                                / (CURVE_W.max(2) - 1) as f32;
                            let delta_y = (start_pointer.y - raw_local_pointer.y) as f32
                                / (CURVE_H.max(2) - 1) as f32;
                            move_segment_translated(
                                &mut editable,
                                index,
                                start_left_x,
                                start_right_x,
                                start_left_y,
                                start_right_y,
                                delta_x,
                                delta_y,
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
                                    CURVE_DRAG_START_THRESHOLD_PX,
                                )
                            {
                                return;
                            }
                            dragging = true;
                            if let Some(segment) = editable.segments.get_mut(index) {
                                let delta = (raw_local_pointer.y - start_pointer.y) as f32
                                    / CURVE_TENSION_PIXEL_SCALE;
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
                if let Some(index) = find_deletable_node_hit(&editable, local_pointer) {
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
        curve_size: Size,
        selected_node: Option<usize>,
        hovered_node: Option<usize>,
        hovered_segment: Option<usize>,
        preview_node: Option<CurveNode>,
        theme: &PumpTheme,
    ) -> Vec<DrawCommand> {
        let rect = Rect {
            origin: Point { x: 0, y: 0 },
            size: Size {
                width: curve_size.width,
                height: curve_size.height,
            },
        };
        let to_canvas = |point: Point| scale_point_from_design(point, curve_size);

        let mut commands = Vec::with_capacity(1024);
        commands.push(DrawCommand::FillRect {
            rect,
            color: theme.curve_bg,
        });
        commands.push(DrawCommand::StrokeRect {
            rect,
            thickness: 1,
            color: theme.curve_border,
        });

        for step in 1..16 {
            let x = ((curve_size.width as i32 - 1) * step) / 16;
            commands.push(DrawCommand::Line {
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
            commands.push(DrawCommand::Line {
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
            let segment_width = ((right.x - left.x).abs() * (CURVE_W as f32 - 1.0))
                .round()
                .max(2.0) as i32;
            let steps = segment_width.clamp(2, 96) as usize;
            let mut prev = to_canvas(local_from_node(CurveNode {
                x: left.x,
                y: sample_editable_curve(editable_curve, left.x),
            }));
            let highlight = preview_node.is_none() && hovered_segment == Some(segment_index);
            let line_color = if highlight {
                theme.curve_line_highlight
            } else {
                theme.curve_line
            };
            for step in 1..=steps {
                let t = step as f32 / steps as f32;
                let x = left.x + (right.x - left.x) * t;
                let point = to_canvas(local_from_node(CurveNode {
                    x,
                    y: sample_editable_curve(editable_curve, x),
                }));
                commands.push(DrawCommand::Line {
                    start: prev,
                    end: point,
                    color: line_color,
                });
                if highlight {
                    commands.push(DrawCommand::Line {
                        start: Point {
                            x: prev.x,
                            y: prev.y + 1,
                        },
                        end: Point {
                            x: point.x,
                            y: point.y + 1,
                        },
                        color: theme.curve_line_highlight_glow,
                    });
                }
                prev = point;
            }
        }

        if let Some(preview) = preview_node {
            let center = to_canvas(local_from_node(preview));
            commands.push(DrawCommand::FillCircle {
                center,
                radius: NODE_DRAW_RADIUS + 1,
                color: theme.preview_fill,
            });
            commands.push(DrawCommand::StrokeCircle {
                center,
                radius: NODE_DRAW_RADIUS + 2,
                thickness: 1,
                color: theme.preview_stroke,
            });
        }

        for (index, node) in editable_curve.nodes.iter().copied().enumerate() {
            let center = to_canvas(local_from_node(node));
            let selected = selected_node == Some(index);
            let hovered = hovered_node == Some(index);
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
            commands.push(DrawCommand::FillCircle {
                center,
                radius: if selected || hovered {
                    NODE_DRAW_RADIUS + 1
                } else {
                    NODE_DRAW_RADIUS
                },
                color: fill_color,
            });
            commands.push(DrawCommand::StrokeCircle {
                center,
                radius: NODE_DRAW_RADIUS,
                thickness: 1,
                color: stroke_color,
            });
            if selected || hovered {
                commands.push(DrawCommand::StrokeCircle {
                    center,
                    radius: NODE_DRAW_RADIUS + 3,
                    thickness: 1,
                    color: if selected {
                        theme.node_selected_ring
                    } else {
                        theme.node_hover_ring
                    },
                });
            }
        }

        let phase = self.status.phase();
        let playhead_x = (phase * (curve_size.width as f32 - 1.0)).round() as i32;
        commands.push(DrawCommand::Line {
            start: Point {
                x: playhead_x,
                y: 0,
            },
            end: Point {
                x: playhead_x,
                y: curve_size.height as i32 - 1,
            },
            color: theme.playhead,
        });

        let reduction = (1.0 - self.status.gain().clamp(0.0, 1.0)).clamp(0.0, 1.0);
        let meter_rect = Rect {
            origin: Point {
                x: curve_size.width as i32 - 12,
                y: 10,
            },
            size: Size {
                width: 6,
                height: curve_size.height.saturating_sub(20),
            },
        };
        commands.push(DrawCommand::StrokeRect {
            rect: meter_rect,
            thickness: 1,
            color: theme.meter_outline,
        });
        let fill_height = ((meter_rect.size.height as f32) * reduction).round() as u32;
        if fill_height > 0 {
            commands.push(DrawCommand::FillRect {
                rect: Rect {
                    origin: Point {
                        x: meter_rect.origin.x + 1,
                        y: meter_rect.origin.y + meter_rect.size.height as i32 - fill_height as i32,
                    },
                    size: Size {
                        width: meter_rect.size.width.saturating_sub(2),
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

fn local_from_node(node: CurveNode) -> Point {
    let x = (node.x.clamp(0.0, 1.0) * (CURVE_W as f32 - 1.0)).round() as i32;
    let y = ((1.0 - node.y.clamp(0.0, 1.0)) * (CURVE_H as f32 - 1.0)).round() as i32;
    Point { x, y }
}

fn node_from_local(local: Point) -> CurveNode {
    let x = (local.x as f32 / CURVE_W.max(1) as f32).clamp(0.0, 1.0);
    let y = (1.0 - (local.y as f32 / CURVE_H.max(1) as f32)).clamp(0.0, 1.0);
    CurveNode { x, y }
}

fn scale_point_from_design(point: Point, curve_size: Size) -> Point {
    let width = curve_size.width.max(1) as i64;
    let height = curve_size.height.max(1) as i64;
    let x = (point.x as i64 * width / CURVE_W.max(1) as i64) as i32;
    let y = (point.y as i64 * height / CURVE_H.max(1) as i64) as i32;
    Point { x, y }
}

fn scale_point_to_design(point: Point, curve_size: Size) -> Point {
    let width = curve_size.width.max(1) as i64;
    let height = curve_size.height.max(1) as i64;
    let x = (point.x as i64 * CURVE_W.max(1) as i64 / width) as i32;
    let y = (point.y as i64 * CURVE_H.max(1) as i64 / height) as i32;
    Point { x, y }
}

fn find_node_hit(curve: &EditableCurve, local_pointer: Point) -> Option<usize> {
    find_node_hit_within(curve, local_pointer, NODE_HIT_RADIUS)
}

fn find_node_hit_within(curve: &EditableCurve, local_pointer: Point, radius: i32) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    let radius_squared = radius.max(0) as i64 * radius.max(0) as i64;
    for (index, node) in curve.nodes.iter().copied().enumerate() {
        let center = local_from_node(node);
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

fn find_segment_line_hit_within(
    curve: &EditableCurve,
    local_pointer: Point,
    radius: i32,
) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    let radius_squared = (radius.max(0) * radius.max(0)) as f32;
    for index in 0..curve.segments.len() {
        let distance = segment_polyline_distance_squared(curve, index, local_pointer);
        if distance <= radius_squared {
            match best {
                Some((_, best_distance)) if distance >= best_distance => {}
                _ => best = Some((index, distance)),
            }
        }
    }
    best.map(|(index, _)| index)
}

fn insert_node(curve: &mut EditableCurve, node: CurveNode) -> usize {
    if curve.nodes.len() >= MAX_EDITABLE_NODES {
        return find_nearest_node(curve, local_from_node(node)).unwrap_or(0);
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

fn move_node_with_push_through(
    curve: &mut EditableCurve,
    index: usize,
    target: CurveNode,
    push_threshold_px: i32,
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
    let threshold_x = push_threshold_px.max(0) as f32 / (CURVE_W.max(2) - 1) as f32;
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
    start_left_x: f32,
    start_right_x: f32,
    start_left_y: f32,
    start_right_y: f32,
    delta_x: f32,
    delta_y: f32,
) {
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

fn find_nearest_node(curve: &EditableCurve, local_pointer: Point) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    for (index, node) in curve.nodes.iter().copied().enumerate() {
        let distance = distance_squared(local_from_node(node), local_pointer);
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

fn segment_polyline_distance_squared(curve: &EditableCurve, index: usize, point: Point) -> f32 {
    let left = curve.nodes[index];
    let right = curve.nodes[(index + 1).min(curve.nodes.len() - 1)];
    let width = ((right.x - left.x).abs() * (CURVE_W as f32 - 1.0))
        .round()
        .max(2.0) as i32;
    let steps = width.clamp(2, 96) as usize;
    let mut prev = local_from_node(CurveNode {
        x: left.x,
        y: sample_editable_curve(curve, left.x),
    });
    let mut best = f32::MAX;
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let x = left.x + (right.x - left.x) * t;
        let current = local_from_node(CurveNode {
            x,
            y: sample_editable_curve(curve, x),
        });
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

fn preview_node_on_curve(curve: &EditableCurve, local_pointer: Point) -> Option<CurveNode> {
    if curve.nodes.len() < 2 {
        return None;
    }
    let x = node_from_local(local_pointer).x;
    Some(CurveNode {
        x,
        y: sample_editable_curve(curve, x).clamp(0.0, 1.0),
    })
}

fn drag_threshold_crossed(start_pointer: Point, current_pointer: Point, threshold_px: i32) -> bool {
    let threshold = threshold_px.max(0) as i64;
    distance_squared(start_pointer, current_pointer) >= threshold * threshold
}

fn find_deletable_node_hit(curve: &EditableCurve, local_pointer: Point) -> Option<usize> {
    let index = find_node_hit(curve, local_pointer)?;
    if index == 0 || index + 1 == curve.nodes.len() {
        return None;
    }
    Some(index)
}

#[cfg(test)]
mod tests {
    use super::{
        find_deletable_node_hit, find_segment_line_hit_within, local_from_node,
        move_node_with_push_through, move_segment_translated, preferred_window_size,
        preview_node_on_curve, resolve_controls_section_widths, resolve_vertical_section_heights,
        GuiState, CURVE_H, CURVE_KEY, WINDOW_HEIGHT, WINDOW_WIDTH,
    };
    use crate::curve::{sample_editable_curve, CurveNode, CurveSegment, EditableCurve};
    use crate::params::PumpParams;
    use crate::GuiStatus;
    use std::sync::Arc;
    use toybox::clap::automation::AutomationQueue;
    use toybox::clap::gui::InputState;
    use toybox::gui::declarative::{measure_checked, Node, RootScaleMode};
    use toybox::gui::{Point, Size};

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
        move_segment_translated(&mut curve, 1, 0.3, 0.6, 0.5, 0.5, 0.1, 0.1);
        assert!((curve.nodes[1].x - 0.4).abs() < 1.0e-6);
        assert!((curve.nodes[2].x - 0.7).abs() < 1.0e-6);
        assert!((curve.nodes[1].y - 0.6).abs() < 1.0e-6);
        assert!((curve.nodes[2].y - 0.6).abs() < 1.0e-6);
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
    fn section_height_split_matches_expected_ratios() {
        let (header_h, curve_h, controls_h) = resolve_vertical_section_heights(WINDOW_HEIGHT);
        assert_eq!(header_h, 18);
        assert_eq!(curve_h, 163);
        assert_eq!(controls_h, 77);
        assert_eq!(header_h + curve_h + controls_h, WINDOW_HEIGHT);
    }

    #[test]
    fn bottom_row_split_matches_expected_ratio() {
        let (knobs_w, dropdown_w) = resolve_controls_section_widths(WINDOW_WIDTH);
        assert_eq!(knobs_w, 294);
        assert_eq!(dropdown_w, 126);
        assert_eq!(knobs_w + dropdown_w, WINDOW_WIDTH);
    }

    #[test]
    fn build_ui_places_curve_region_at_full_spline_extent() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let spec = state.build_ui(&InputState::default());
        let region = find_curve_region_node(&spec.root.content).expect("curve region should exist");
        assert_eq!(region.size.width, WINDOW_WIDTH);
        assert_eq!(region.size.height, CURVE_H);
    }

    #[test]
    fn build_ui_accepts_resize_sequences_with_window_sized_root_layout() {
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
                width: WINDOW_WIDTH + 87,
                height: WINDOW_HEIGHT + 11,
            },
            Size {
                width: WINDOW_WIDTH + 141,
                height: WINDOW_HEIGHT + 27,
            },
        ];
        for window_size in input_sizes {
            let mut input = InputState::default();
            input.window_size = window_size;
            let spec = state.build_ui(&input);
            let measured = measure_checked(&spec).expect("measurement should succeed");
            assert!(measured.width >= window_size.width.max(WINDOW_WIDTH));
            assert!(measured.height >= window_size.height.max(WINDOW_HEIGHT));
            assert_eq!(spec.root.scale_mode, RootScaleMode::None);
            assert_eq!(spec.root.design_size, None);
        }
    }

    #[test]
    fn build_ui_root_content_is_three_section_column() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let spec = state.build_ui(&InputState::default());
        let root_column = match spec.root.content.as_ref() {
            Node::Column(column) => column,
            other => panic!("expected root content column, got {other:?}"),
        };
        assert_eq!(root_column.children.len(), 3);
        assert!(matches!(root_column.children[0], Node::Panel(_)));
        assert!(matches!(root_column.children[1], Node::Panel(_)));
        let controls_panel = match &root_column.children[2] {
            Node::Panel(panel) => panel,
            other => panic!("expected controls panel, got {other:?}"),
        };
        let controls_row = match controls_panel.content.as_ref() {
            Node::Row(row) => row,
            other => panic!("expected controls row in panel, got {other:?}"),
        };
        assert_eq!(controls_row.children.len(), 2);
        assert!(matches!(controls_row.children[0], Node::Panel(_)));
        assert!(matches!(controls_row.children[1], Node::Panel(_)));
    }

    fn find_curve_region_node(node: &Node) -> Option<&toybox::gui::declarative::RegionSpec> {
        match node {
            Node::Region(region) if region.key == CURVE_KEY => Some(region),
            Node::Panel(panel) => find_curve_region_node(&panel.content),
            Node::Row(flex) | Node::Column(flex) => {
                flex.children.iter().find_map(find_curve_region_node)
            }
            Node::Grid(grid) => grid.children.iter().find_map(find_curve_region_node),
            Node::Absolute(absolute) => absolute
                .children
                .iter()
                .find_map(|child| find_curve_region_node(&child.node)),
            Node::Region(_) => None,
            Node::Label(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::Indicator(_) => None,
        }
    }
}
