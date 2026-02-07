//! Declarative curve-editor GUI for Pump.

use std::sync::{Arc, Mutex};

use toybox::clack_extensions::gui::Window;
use toybox::clack_plugin::plugin::PluginError;
use toybox::clack_plugin::utils::ClapId;
use toybox::clap::automation::{AutomationConfig, AutomationQueue};
use toybox::clap::gui::{GuiHostWindow, InputState};
use toybox::gui::declarative::{
    button, dropdown, knob, label, panel, AbsoluteChild, AbsoluteSpec, DrawCommand, LayoutBox,
    Node, RegionInteractionKind, RegionSpec, RootFrameSpec, UiAction, UiSpec,
};
use toybox::gui::{Color, Point, Rect, Size};
use toybox::raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

use crate::curve::{
    sample_editable_curve, CurveNode, CurveSegment, EditableCurve, CURVE_TABLE_LEN,
    MAX_EDITABLE_NODES, MAX_SEGMENT_TENSION, MIN_SEGMENT_TENSION,
};
use crate::params::{
    sync_division_label, sync_division_labels, PumpParams, MAX_DEPTH, MAX_MIX, MAX_OUTPUT_GAIN_DB,
    MAX_PHASE_OFFSET, MAX_SYNC_DIVISION, MIN_DEPTH, MIN_MIX, MIN_OUTPUT_GAIN_DB, MIN_PHASE_OFFSET,
    PARAM_DEPTH_ID, PARAM_MIX_ID, PARAM_OUTPUT_GAIN_ID, PARAM_PHASE_OFFSET_ID,
    PARAM_SYNC_DIVISION_ID,
};
use crate::{GuiStatus, HostParamRequester};

/// Default width for the plugin editor window.
pub const WINDOW_WIDTH: u32 = 700;
/// Default height for the plugin editor window.
pub const WINDOW_HEIGHT: u32 = 430;

const ROOT_KEY: &str = "pump-root";
const CURVE_KEY: &str = "curve";
const MIX_KEY: &str = "mix";
const DEPTH_KEY: &str = "depth";
const PHASE_KEY: &str = "phase";
const OUTPUT_KEY: &str = "output";
const DIVISION_KEY: &str = "division";
const RESET_KEY: &str = "reset";

const PADDING_X: i32 = 18;
const TITLE_Y: i32 = 14;
const CURVE_X: i32 = PADDING_X;
const CURVE_Y: i32 = 44;
const CURVE_W: u32 = 664;
const CURVE_H: u32 = 222;
const CONTROL_Y: i32 = 290;
const CONTROL_STEP: i32 = 156;
const KNOB_X: [i32; 4] = [
    PADDING_X,
    PADDING_X + CONTROL_STEP,
    PADDING_X + CONTROL_STEP * 2,
    PADDING_X + CONTROL_STEP * 3,
];
const DROPDOWN_X: i32 = PADDING_X + CONTROL_STEP * 4;
const DROPDOWN_W: u32 = 132;
const NODE_DRAW_RADIUS: i32 = 4;
const NODE_HIT_RADIUS: i32 = 8;
const HANDLE_DRAW_RADIUS: i32 = 4;
const HANDLE_HIT_RADIUS: i32 = 8;
const NODE_INSERT_GUARD_RADIUS: i32 = 12;
const HANDLE_INSERT_GUARD_RADIUS: i32 = 11;
const CURVE_DRAG_START_THRESHOLD_PX: i32 = 2;
const HANDLE_TENSION_PIXEL_SCALE: f32 = 28.0;
const NODE_X_MIN_SPACING: f32 = 1.0e-3;

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

        self.window.open_parented(
            "pump".to_string(),
            (WINDOW_WIDTH, WINDOW_HEIGHT),
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

    /// Close editor if it is open.
    pub fn close(&mut self) {
        self.window.hide();
    }

    /// Return last known logical size.
    pub fn last_size(&self) -> Option<(u32, u32)> {
        self.window.last_size()
    }
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
    last_pointer: Point,
    selected_node: Option<usize>,
    drag_mode: Option<CurveDragMode>,
}

#[derive(Copy, Clone, Debug)]
enum CurveDragMode {
    MoveNode {
        index: usize,
        start_pointer: Point,
        dragging: bool,
    },
    AdjustSegment {
        index: usize,
        start_tension: f32,
        start_pointer_y: i32,
        start_pointer: Point,
        dragging: bool,
    },
}

impl GuiRuntime {
    fn new() -> Self {
        Self {
            last_pointer: Point { x: 0, y: 0 },
            selected_node: None,
            drag_mode: None,
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
        let selected_node = if let Ok(mut runtime) = self.runtime.lock() {
            runtime.last_pointer = input.pointer_pos;
            runtime.selected_node
        } else {
            None
        };

        let mix = self.params.mix();
        let depth = self.params.depth();
        let phase_offset = self.params.phase_offset();
        let output_gain_db = self.params.output_gain_db();
        let division = self.params.sync_division();

        let curve = self.params.curve_snapshot();
        let editable_curve = self.params.editable_curve_snapshot();
        let local_pointer = local_from_pointer(input.pointer_pos);
        let hovered_node = find_node_hit(&editable_curve, local_pointer);
        let hovered_segment = find_segment_handle_hit(&editable_curve, local_pointer);
        let draw_commands = self.build_curve_draw_commands(
            &curve,
            &editable_curve,
            selected_node,
            hovered_node,
            hovered_segment,
        );

        let controls = vec![
            AbsoluteChild::new(
                Point {
                    x: PADDING_X,
                    y: TITLE_Y,
                },
                label("PUMP").text_color(Color::rgb(242, 244, 248)),
            ),
            AbsoluteChild::new(
                Point {
                    x: PADDING_X + 72,
                    y: TITLE_Y,
                },
                label("Spline Beat-Synced Ducking").text_color(Color::rgb(168, 176, 192)),
            ),
            AbsoluteChild::new(
                Point {
                    x: CURVE_X,
                    y: CURVE_Y,
                },
                Node::Region(
                    RegionSpec::new(
                        CURVE_KEY,
                        Size {
                            width: CURVE_W,
                            height: CURVE_H,
                        },
                    )
                    .draw_commands(draw_commands),
                ),
            ),
            AbsoluteChild::new(
                Point {
                    x: KNOB_X[0],
                    y: CONTROL_Y,
                },
                knob(MIX_KEY, "Mix", mix, (MIN_MIX, MAX_MIX))
                    .value_label(format!("{:.0}%", mix * 100.0)),
            ),
            AbsoluteChild::new(
                Point {
                    x: KNOB_X[1],
                    y: CONTROL_Y,
                },
                knob(DEPTH_KEY, "Depth", depth, (MIN_DEPTH, MAX_DEPTH))
                    .value_label(format!("{:.0}%", depth * 100.0)),
            ),
            AbsoluteChild::new(
                Point {
                    x: KNOB_X[2],
                    y: CONTROL_Y,
                },
                knob(
                    PHASE_KEY,
                    "Phase",
                    phase_offset,
                    (MIN_PHASE_OFFSET, MAX_PHASE_OFFSET),
                )
                .value_label(format!("{:.0}%", phase_offset * 100.0)),
            ),
            AbsoluteChild::new(
                Point {
                    x: KNOB_X[3],
                    y: CONTROL_Y,
                },
                knob(
                    OUTPUT_KEY,
                    "Output",
                    output_gain_db,
                    (MIN_OUTPUT_GAIN_DB, MAX_OUTPUT_GAIN_DB),
                )
                .value_label(format!("{output_gain_db:+.1} dB")),
            ),
            AbsoluteChild::new(
                Point {
                    x: DROPDOWN_X,
                    y: CONTROL_Y + 12,
                },
                dropdown(
                    DIVISION_KEY,
                    "Division",
                    sync_division_labels(),
                    division.min(MAX_SYNC_DIVISION as usize),
                )
                .control_size(Size {
                    width: DROPDOWN_W,
                    height: 24,
                }),
            ),
            AbsoluteChild::new(
                Point {
                    x: DROPDOWN_X,
                    y: CONTROL_Y + 56,
                },
                button(RESET_KEY, "Reset Curve").control_size(Size {
                    width: DROPDOWN_W,
                    height: 24,
                }),
            ),
            AbsoluteChild::new(
                Point {
                    x: DROPDOWN_X,
                    y: CONTROL_Y + 88,
                },
                label(format!("Cycle: {}", sync_division_label(division)))
                    .text_color(Color::rgb(173, 182, 198)),
            ),
            AbsoluteChild::new(
                Point {
                    x: CURVE_X,
                    y: CURVE_Y + CURVE_H as i32 + 6,
                },
                label("Tip: right-click a node to delete it.")
                    .text_color(Color::rgb(132, 142, 160)),
            ),
        ];

        let content = Node::Absolute(
            AbsoluteSpec::new(controls).layout(LayoutBox::fixed(WINDOW_WIDTH, WINDOW_HEIGHT)),
        );

        UiSpec::new(
            RootFrameSpec::new(
                ROOT_KEY,
                panel("pump-main", content)
                    .pad_all(0)
                    .background(Color::rgb(22, 26, 34))
                    .outline(Color::rgb(62, 69, 84))
                    .layout(LayoutBox::fixed(WINDOW_WIDTH, WINDOW_HEIGHT)),
            )
            .title("pump")
            .padding(0)
            .layout(LayoutBox::fixed(WINDOW_WIDTH, WINDOW_HEIGHT)),
        )
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
            UiAction::RegionInteracted { key, kind } if key == CURVE_KEY => {
                self.reduce_curve_interaction(kind)
            }
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

    fn reduce_curve_interaction(&mut self, kind: RegionInteractionKind) {
        let Ok(mut runtime) = self.runtime.lock() else {
            return;
        };

        let local_pointer = local_from_pointer(runtime.last_pointer);
        let normalized_pointer = node_from_local(local_pointer);

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

                if let Some(index) = find_segment_handle_hit(&editable, local_pointer) {
                    let start_tension = editable
                        .segments
                        .get(index)
                        .copied()
                        .unwrap_or(CurveSegment { tension: 0.0 })
                        .tension;
                    runtime.drag_mode = Some(CurveDragMode::AdjustSegment {
                        index,
                        start_tension,
                        start_pointer_y: local_pointer.y,
                        start_pointer: local_pointer,
                        dragging: false,
                    });
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

                if let Some(index) = find_segment_handle_hit_within(
                    &editable,
                    local_pointer,
                    HANDLE_INSERT_GUARD_RADIUS,
                ) {
                    let start_tension = editable
                        .segments
                        .get(index)
                        .copied()
                        .unwrap_or(CurveSegment { tension: 0.0 })
                        .tension;
                    runtime.drag_mode = Some(CurveDragMode::AdjustSegment {
                        index,
                        start_tension,
                        start_pointer_y: local_pointer.y,
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
                            move_node(&mut editable, index, normalized_pointer);
                            runtime.selected_node = Some(index);
                            drag_mode = CurveDragMode::MoveNode {
                                index,
                                start_pointer,
                                dragging,
                            };
                            curve_changed = true;
                        }
                        CurveDragMode::AdjustSegment {
                            index,
                            start_tension,
                            start_pointer_y,
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
                            if let Some(segment) = editable.segments.get_mut(index) {
                                let delta = (start_pointer_y - local_pointer.y) as f32
                                    / HANDLE_TENSION_PIXEL_SCALE;
                                segment.tension = (start_tension + delta)
                                    .clamp(MIN_SEGMENT_TENSION, MAX_SEGMENT_TENSION);
                                curve_changed = true;
                            }
                            drag_mode = CurveDragMode::AdjustSegment {
                                index,
                                start_tension,
                                start_pointer_y,
                                start_pointer,
                                dragging,
                            };
                        }
                    }
                    runtime.drag_mode = Some(drag_mode);
                    if curve_changed {
                        self.params.set_editable_curve(&editable);
                    }
                }
            }
            RegionInteractionKind::Released => {
                runtime.drag_mode = None;
            }
            RegionInteractionKind::SecondaryClicked => {
                let mut editable = self.params.editable_curve_snapshot();
                if let Some(index) = find_node_hit(&editable, local_pointer) {
                    if index > 0 && index + 1 < editable.nodes.len() {
                        editable.nodes.remove(index);
                        let remove_segment = index
                            .saturating_sub(1)
                            .min(editable.segments.len().saturating_sub(1));
                        if !editable.segments.is_empty() {
                            editable.segments.remove(remove_segment);
                        }
                        runtime.selected_node = None;
                        runtime.drag_mode = None;
                        self.params.set_editable_curve(&editable);
                    }
                }
            }
            RegionInteractionKind::DoubleClicked => {}
        }
    }

    fn build_curve_draw_commands(
        &self,
        curve: &[f32; CURVE_TABLE_LEN],
        editable_curve: &EditableCurve,
        selected_node: Option<usize>,
        hovered_node: Option<usize>,
        hovered_segment: Option<usize>,
    ) -> Vec<DrawCommand> {
        let rect = Rect {
            origin: Point { x: 0, y: 0 },
            size: Size {
                width: CURVE_W,
                height: CURVE_H,
            },
        };

        let mut commands = Vec::with_capacity(1024);
        commands.push(DrawCommand::FillRect {
            rect,
            color: Color::rgb(15, 18, 24),
        });
        commands.push(DrawCommand::StrokeRect {
            rect,
            thickness: 1,
            color: Color::rgb(58, 65, 80),
        });

        for step in 1..16 {
            let x = ((CURVE_W as i32 - 1) * step) / 16;
            commands.push(DrawCommand::Line {
                start: Point { x, y: 0 },
                end: Point {
                    x,
                    y: CURVE_H as i32 - 1,
                },
                color: Color::rgb(33, 39, 50),
            });
        }

        for step in 1..4 {
            let y = ((CURVE_H as i32 - 1) * step) / 4;
            commands.push(DrawCommand::Line {
                start: Point { x: 0, y },
                end: Point {
                    x: CURVE_W as i32 - 1,
                    y,
                },
                color: Color::rgb(30, 36, 46),
            });
        }

        let mut prev: Option<Point> = None;
        for index in 0..220 {
            let t = index as f32 / 219.0;
            let sample_index = ((CURVE_TABLE_LEN - 1) as f32 * t).round() as usize;
            let sample = curve[sample_index.min(CURVE_TABLE_LEN - 1)];
            let x = (t * (CURVE_W as f32 - 1.0)).round() as i32;
            let y = ((1.0 - sample) * (CURVE_H as f32 - 1.0)).round() as i32;
            let point = Point { x, y };
            if let Some(previous) = prev {
                commands.push(DrawCommand::Line {
                    start: previous,
                    end: point,
                    color: Color::rgb(134, 206, 255),
                });
            }
            prev = Some(point);
        }

        for segment_index in 0..editable_curve.segments.len() {
            let (base_point, handle_point) = segment_handle_points(editable_curve, segment_index);
            let hovered = hovered_segment == Some(segment_index);
            let guide_color = if hovered {
                Color::rgb(134, 157, 198)
            } else {
                Color::rgb(86, 98, 122)
            };
            let fill_color = if hovered {
                Color::rgb(191, 214, 255)
            } else {
                Color::rgb(129, 146, 176)
            };
            let stroke_color = if hovered {
                Color::rgb(226, 238, 255)
            } else {
                Color::rgb(193, 205, 228)
            };
            commands.push(DrawCommand::Line {
                start: base_point,
                end: handle_point,
                color: guide_color,
            });
            commands.push(DrawCommand::FillCircle {
                center: handle_point,
                radius: HANDLE_DRAW_RADIUS,
                color: fill_color,
            });
            commands.push(DrawCommand::StrokeCircle {
                center: handle_point,
                radius: HANDLE_DRAW_RADIUS,
                thickness: 1,
                color: stroke_color,
            });
        }

        for (index, node) in editable_curve.nodes.iter().copied().enumerate() {
            let center = local_from_node(node);
            let selected = selected_node == Some(index);
            let hovered = hovered_node == Some(index);
            let fill_color = if selected {
                Color::rgb(255, 206, 118)
            } else if hovered {
                Color::rgb(187, 223, 255)
            } else {
                Color::rgb(230, 240, 255)
            };
            let stroke_color = if selected {
                Color::rgb(255, 242, 206)
            } else if hovered {
                Color::rgb(220, 236, 255)
            } else {
                Color::rgb(116, 129, 148)
            };
            commands.push(DrawCommand::FillCircle {
                center,
                radius: NODE_DRAW_RADIUS,
                color: fill_color,
            });
            commands.push(DrawCommand::StrokeCircle {
                center,
                radius: NODE_DRAW_RADIUS,
                thickness: 1,
                color: stroke_color,
            });
        }

        let phase = self.status.phase();
        let playhead_x = (phase * (CURVE_W as f32 - 1.0)).round() as i32;
        commands.push(DrawCommand::Line {
            start: Point {
                x: playhead_x,
                y: 0,
            },
            end: Point {
                x: playhead_x,
                y: CURVE_H as i32 - 1,
            },
            color: Color::rgb(245, 192, 118),
        });

        let reduction = (1.0 - self.status.gain().clamp(0.0, 1.0)).clamp(0.0, 1.0);
        let meter_rect = Rect {
            origin: Point {
                x: CURVE_W as i32 - 12,
                y: 10,
            },
            size: Size {
                width: 6,
                height: CURVE_H - 20,
            },
        };
        commands.push(DrawCommand::StrokeRect {
            rect: meter_rect,
            thickness: 1,
            color: Color::rgb(71, 79, 96),
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
                color: Color::rgb(255, 120, 88),
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

fn local_from_pointer(pointer: Point) -> Point {
    let x = (pointer.x - CURVE_X).clamp(0, CURVE_W.saturating_sub(1) as i32);
    let y = (pointer.y - CURVE_Y).clamp(0, CURVE_H.saturating_sub(1) as i32);
    Point { x, y }
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

fn find_segment_handle_hit(curve: &EditableCurve, local_pointer: Point) -> Option<usize> {
    find_segment_handle_hit_within(curve, local_pointer, HANDLE_HIT_RADIUS)
}

fn find_segment_handle_hit_within(
    curve: &EditableCurve,
    local_pointer: Point,
    radius: i32,
) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    let radius_squared = radius.max(0) as i64 * radius.max(0) as i64;
    for index in 0..curve.segments.len() {
        let (_, handle) = segment_handle_points(curve, index);
        let distance = distance_squared(handle, local_pointer);
        if distance <= radius_squared {
            match best {
                Some((_, best_distance)) if distance >= best_distance => {}
                _ => best = Some((index, distance)),
            }
        }
    }
    best.map(|(index, _)| index)
}

fn segment_handle_points(curve: &EditableCurve, index: usize) -> (Point, Point) {
    let left = curve.nodes[index];
    let right = curve.nodes[(index + 1).min(curve.nodes.len() - 1)];
    let mid_x = (left.x + right.x) * 0.5;
    let mid_y = sample_editable_curve(curve, mid_x);
    let base = local_from_node(CurveNode { x: mid_x, y: mid_y });
    let tension = curve
        .segments
        .get(index)
        .copied()
        .unwrap_or(CurveSegment { tension: 0.0 })
        .tension;
    let handle = Point {
        x: base.x,
        y: (base.y as f32 - tension * HANDLE_TENSION_PIXEL_SCALE).round() as i32,
    };
    (base, handle)
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

fn move_node(curve: &mut EditableCurve, index: usize, target: CurveNode) {
    if index >= curve.nodes.len() {
        return;
    }

    let y = target.y.clamp(0.0, 1.0);
    let last_index = curve.nodes.len() - 1;
    if index == 0 {
        curve.nodes[0].x = 0.0;
        curve.nodes[0].y = y;
        return;
    }
    if index == last_index {
        curve.nodes[last_index].x = 1.0;
        curve.nodes[last_index].y = y;
        return;
    }

    let min_x = curve.nodes[index - 1].x + NODE_X_MIN_SPACING;
    let max_x = curve.nodes[index + 1].x - NODE_X_MIN_SPACING;
    curve.nodes[index].x = target.x.clamp(min_x, max_x);
    curve.nodes[index].y = y;
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

fn drag_threshold_crossed(start_pointer: Point, current_pointer: Point, threshold_px: i32) -> bool {
    let threshold = threshold_px.max(0) as i64;
    distance_squared(start_pointer, current_pointer) >= threshold * threshold
}
