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

use crate::curve::{points_to_curve, FreehandPoint, CURVE_TABLE_LEN};
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
    drawing_curve: bool,
    stroke_points: Vec<FreehandPoint>,
}

impl GuiRuntime {
    fn new() -> Self {
        Self {
            last_pointer: Point { x: 0, y: 0 },
            drawing_curve: false,
            stroke_points: Vec::with_capacity(1024),
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
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.last_pointer = input.pointer_pos;
        }

        let mix = self.params.mix();
        let depth = self.params.depth();
        let phase_offset = self.params.phase_offset();
        let output_gain_db = self.params.output_gain_db();
        let division = self.params.sync_division();

        let curve = self.params.curve_snapshot();
        let draw_commands = self.build_curve_draw_commands(&curve);

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
                label("Freehand Beat-Synced Ducking").text_color(Color::rgb(168, 176, 192)),
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

        let point = point_from_pointer(runtime.last_pointer);

        match kind {
            RegionInteractionKind::Pressed => {
                runtime.drawing_curve = true;
                runtime.stroke_points.clear();
                runtime.stroke_points.push(point);
            }
            RegionInteractionKind::Dragged => {
                if runtime.drawing_curve {
                    runtime.stroke_points.push(point);
                }
            }
            RegionInteractionKind::Released => {
                if runtime.drawing_curve {
                    runtime.stroke_points.push(point);
                    let fallback = self.params.curve_snapshot();
                    let curve = points_to_curve(&runtime.stroke_points, &fallback);
                    self.params.set_curve(&curve);
                }
                runtime.drawing_curve = false;
            }
            RegionInteractionKind::DoubleClicked | RegionInteractionKind::SecondaryClicked => {
                self.params.reset_curve_to_default();
                runtime.drawing_curve = false;
                runtime.stroke_points.clear();
            }
        }
    }

    fn build_curve_draw_commands(&self, curve: &[f32; CURVE_TABLE_LEN]) -> Vec<DrawCommand> {
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

fn point_from_pointer(pointer: Point) -> FreehandPoint {
    let local_x = (pointer.x - CURVE_X) as f32;
    let local_y = (pointer.y - CURVE_Y) as f32;
    let x = (local_x / CURVE_W.max(1) as f32).clamp(0.0, 1.0);
    let y = (1.0 - (local_y / CURVE_H.max(1) as f32)).clamp(0.0, 1.0);
    FreehandPoint { x, y }
}
