//! Shared Radiant surface for Pump editor hosts.

use std::sync::Arc;

use radiant::gui::types::{Point, Rect, Vector2};
use radiant::layout::LayoutOutput;
use radiant::prelude::{
    column, custom_widget_mapped, row, slider, text, IntoView, UiSurface, ViewNode,
};
#[cfg(test)]
use radiant::runtime::SurfaceFrame;
use radiant::runtime::{
    DeclarativeSurfaceRuntime, PaintFillRect, PaintPrimitive, PaintStrokePolyline, PaintStrokeRect,
};
#[cfg(feature = "vst3")]
use radiant::runtime::{Event, SurfacePaintPlan};
use radiant::theme::ThemeTokens;
use radiant::widgets::{PointerButton, Widget, WidgetCommon, WidgetInput, WidgetOutput};

use crate::curve::{sample_editable_curve, CurveNode, EditableCurve};
use crate::params::{
    sync_division_label, PumpParams, MAX_OUTPUT_GAIN_DB, MAX_SYNC_DIVISION, MIN_OUTPUT_GAIN_DB,
};

use super::{WINDOW_HEIGHT, WINDOW_WIDTH};

const TITLE_HEIGHT: f32 = 24.0;
const CURVE_PREVIEW_HEIGHT: f32 = 96.0;
const CONTROL_ROW_HEIGHT: f32 = 22.0;
const CONTROL_LABEL_WIDTH: f32 = 54.0;
const CONTROL_VALUE_WIDTH: f32 = 60.0;
const SURFACE_PADDING: f32 = 12.0;
const SURFACE_SPACING: f32 = 6.0;
const CURVE_GRID_COLUMNS: usize = 8;
const CURVE_GRID_ROWS: usize = 4;
const CURVE_SAMPLE_COUNT: usize = 96;
const CURVE_NODE_SIZE: f32 = 5.0;
const CURVE_NODE_HIT_RADIUS: f32 = 10.0;
const CURVE_NODE_MIN_SPACING_X: f32 = 1.0e-3;

#[derive(Clone)]
struct RadiantEditorState {
    params: Arc<PumpParams>,
    active_curve_node: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
enum RadiantEditorMessage {
    Mix(f32),
    Phase(f32),
    OutputGain(f32),
    SyncDivision(f32),
    Curve(CurvePreviewMessage),
}

type EditorProjector = fn(&mut RadiantEditorState) -> Arc<UiSurface<RadiantEditorMessage>>;
type EditorReducer = fn(&mut RadiantEditorState, RadiantEditorMessage);
type EditorSurfaceRuntime = DeclarativeSurfaceRuntime<
    RadiantEditorState,
    RadiantEditorMessage,
    EditorProjector,
    EditorReducer,
>;

/// Host-controlled Radiant editor surface used by embedded Pump GUIs.
#[cfg(feature = "vst3")]
pub(crate) struct RadiantPumpEditor {
    runtime: EditorSurfaceRuntime,
    theme: ThemeTokens,
    paint_plan: SurfacePaintPlan,
}

#[cfg(feature = "vst3")]
impl RadiantPumpEditor {
    /// Build a Radiant editor runtime at the provided logical viewport.
    pub(crate) fn new(params: Arc<PumpParams>, width: u32, height: u32) -> Self {
        let theme = ThemeTokens::default();
        let viewport = Vector2::new(width.max(1) as f32, height.max(1) as f32);
        Self {
            runtime: EditorSurfaceRuntime::new_declarative(
                RadiantEditorState {
                    params,
                    active_curve_node: None,
                },
                viewport,
                project_editor_surface,
                reduce_editor_message,
            ),
            paint_plan: SurfacePaintPlan::empty(&theme),
            theme,
        }
    }

    /// Apply a host-driven viewport resize.
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.runtime.dispatch_event(Event::resize(Vector2::new(
            width.max(1) as f32,
            height.max(1) as f32,
        )));
    }

    /// Route a backend-neutral event into the Radiant runtime.
    pub(crate) fn dispatch_event(&mut self, event: Event) {
        let _ = self.runtime.dispatch_event(event);
    }

    /// Refresh and return the current backend-neutral paint plan.
    pub(crate) fn paint_plan(&mut self) -> &SurfacePaintPlan {
        let _ = self
            .runtime
            .borrowed_frame_into(&self.theme, &mut self.paint_plan);
        &self.paint_plan
    }
}

/// Build one Radiant frame for tests and non-retained preview hosts.
#[cfg(test)]
pub(crate) fn radiant_editor_frame_for_params(
    params: Arc<PumpParams>,
    viewport: Vector2,
) -> SurfaceFrame {
    EditorSurfaceRuntime::new_declarative(
        RadiantEditorState {
            params,
            active_curve_node: None,
        },
        viewport,
        project_editor_surface,
        reduce_editor_message,
    )
    .frame(&ThemeTokens::default())
}

fn project_editor_surface(state: &mut RadiantEditorState) -> Arc<UiSurface<RadiantEditorMessage>> {
    let params = state.params.as_ref();
    let output = params.output_gain_db();
    let sync = params.sync_division();
    Arc::new(
        column([
            text("PUMP").height(TITLE_HEIGHT).fill_width(),
            custom_widget_mapped(
                CurvePreviewWidget::new(params.editable_curve_snapshot(), state.active_curve_node),
                RadiantEditorMessage::Curve,
            )
            .fill_width()
            .height(CURVE_PREVIEW_HEIGHT),
            control_row(
                "Mix",
                format!("{:.0}%", params.mix() * 100.0),
                params.mix(),
                RadiantEditorMessage::Mix,
            ),
            control_row(
                "Phase",
                format!("{:.0}%", params.phase_offset() * 100.0),
                params.phase_offset(),
                RadiantEditorMessage::Phase,
            ),
            control_row(
                "Output",
                format!("{output:+.1} dB"),
                normalize_output_gain(output),
                RadiantEditorMessage::OutputGain,
            ),
            control_row(
                "Sync",
                sync_division_label(sync).to_string(),
                normalize_sync_division(sync),
                RadiantEditorMessage::SyncDivision,
            ),
        ])
        .padding(SURFACE_PADDING)
        .spacing(SURFACE_SPACING)
        .size(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32)
        .into_surface(),
    )
}

fn control_row(
    label: &'static str,
    value_label: String,
    value: f32,
    message: fn(f32) -> RadiantEditorMessage,
) -> ViewNode<RadiantEditorMessage> {
    row([
        text(label)
            .width(CONTROL_LABEL_WIDTH)
            .height(CONTROL_ROW_HEIGHT),
        slider(value.clamp(0.0, 1.0))
            .message(message)
            .fill_width()
            .height(CONTROL_ROW_HEIGHT),
        text(value_label)
            .width(CONTROL_VALUE_WIDTH)
            .height(CONTROL_ROW_HEIGHT),
    ])
    .spacing(8.0)
    .fill_width()
    .height(CONTROL_ROW_HEIGHT)
}

fn reduce_editor_message(state: &mut RadiantEditorState, message: RadiantEditorMessage) {
    match message {
        RadiantEditorMessage::Mix(value) => state.params.set_mix(value),
        RadiantEditorMessage::Phase(value) => state.params.set_phase_offset(value),
        RadiantEditorMessage::OutputGain(value) => {
            state
                .params
                .set_output_gain_db(denormalize_output_gain(value));
        }
        RadiantEditorMessage::SyncDivision(value) => {
            state
                .params
                .set_sync_division((value.clamp(0.0, 1.0) * MAX_SYNC_DIVISION).round());
        }
        RadiantEditorMessage::Curve(message) => reduce_curve_message(state, message),
    }
}

fn reduce_curve_message(state: &mut RadiantEditorState, message: CurvePreviewMessage) {
    match message {
        CurvePreviewMessage::PressNode { index } => state.active_curve_node = Some(index),
        CurvePreviewMessage::DragNode { index, node } => {
            let mut curve = state.params.editable_curve_snapshot();
            update_curve_node(&mut curve, index, node);
            state.params.set_editable_curve(&curve);
            state.active_curve_node = Some(index);
        }
        CurvePreviewMessage::ReleaseNode { index, node } => {
            let mut curve = state.params.editable_curve_snapshot();
            update_curve_node(&mut curve, index, node);
            state.params.set_editable_curve(&curve);
            state.active_curve_node = None;
        }
        CurvePreviewMessage::Cancel => state.active_curve_node = None,
    }
}

fn update_curve_node(curve: &mut EditableCurve, index: usize, node: CurveNode) {
    if curve.nodes.is_empty() || index >= curve.nodes.len() {
        return;
    }
    if index == 0 || index + 1 == curve.nodes.len() {
        let y = node.y.clamp(0.0, 1.0);
        if let Some(first) = curve.nodes.first_mut() {
            first.x = 0.0;
            first.y = y;
        }
        if let Some(last) = curve.nodes.last_mut() {
            last.x = 1.0;
            last.y = y;
        }
        curve.normalize_in_place();
        return;
    }

    let previous_x = curve.nodes[index - 1].x + CURVE_NODE_MIN_SPACING_X;
    let next_x = curve.nodes[index + 1].x - CURVE_NODE_MIN_SPACING_X;
    if previous_x >= next_x {
        return;
    }
    curve.nodes[index] = CurveNode {
        x: node.x.clamp(previous_x, next_x),
        y: node.y.clamp(0.0, 1.0),
    };
    curve.normalize_in_place();
}

fn normalize_output_gain(value: f32) -> f32 {
    ((value - MIN_OUTPUT_GAIN_DB) / (MAX_OUTPUT_GAIN_DB - MIN_OUTPUT_GAIN_DB)).clamp(0.0, 1.0)
}

fn denormalize_output_gain(value: f32) -> f32 {
    MIN_OUTPUT_GAIN_DB + value.clamp(0.0, 1.0) * (MAX_OUTPUT_GAIN_DB - MIN_OUTPUT_GAIN_DB)
}

fn normalize_sync_division(value: usize) -> f32 {
    (value as f32 / MAX_SYNC_DIVISION).clamp(0.0, 1.0)
}

#[derive(Clone)]
struct CurvePreviewWidget {
    common: WidgetCommon,
    curve: EditableCurve,
    active_node: Option<usize>,
}

impl CurvePreviewWidget {
    fn new(curve: EditableCurve, active_node: Option<usize>) -> Self {
        Self {
            common: WidgetCommon::fixed(
                0,
                (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0),
                CURVE_PREVIEW_HEIGHT,
            )
            .with_pointer_focus()
            .without_default_chrome(),
            curve: curve.normalized(),
            active_node,
        }
    }

    fn curve_point(bounds: Rect, node: CurveNode) -> Point {
        let width = bounds.width().max(1.0) - 1.0;
        let height = bounds.height().max(1.0) - 1.0;
        Point::new(
            bounds.min.x + node.x.clamp(0.0, 1.0) * width,
            bounds.min.y + (1.0 - node.y.clamp(0.0, 1.0)) * height,
        )
    }

    fn node_from_point(bounds: Rect, position: Point) -> CurveNode {
        let width = (bounds.width().max(1.0) - 1.0).max(1.0);
        let height = (bounds.height().max(1.0) - 1.0).max(1.0);
        CurveNode {
            x: ((position.x - bounds.min.x) / width).clamp(0.0, 1.0),
            y: (1.0 - ((position.y - bounds.min.y) / height)).clamp(0.0, 1.0),
        }
    }

    fn hit_node(&self, bounds: Rect, position: Point) -> Option<usize> {
        let radius_squared = CURVE_NODE_HIT_RADIUS * CURVE_NODE_HIT_RADIUS;
        self.curve
            .nodes
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, node)| {
                let center = Self::curve_point(bounds, node);
                let dx = center.x - position.x;
                let dy = center.y - position.y;
                let distance_squared = dx * dx + dy * dy;
                (distance_squared <= radius_squared).then_some((index, distance_squared))
            })
            .min_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
    }

    fn push_grid(&self, primitives: &mut Vec<PaintPrimitive>, bounds: Rect, theme: &ThemeTokens) {
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.common.id,
            rect: bounds,
            color: theme.bg_secondary,
        }));
        for step in 1..CURVE_GRID_COLUMNS {
            let x = bounds.min.x + bounds.width() * (step as f32 / CURVE_GRID_COLUMNS as f32);
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.common.id,
                points: Arc::from([Point::new(x, bounds.min.y), Point::new(x, bounds.max.y)]),
                color: theme.grid_soft,
                width: 1.0,
            }));
        }
        for step in 1..CURVE_GRID_ROWS {
            let y = bounds.min.y + bounds.height() * (step as f32 / CURVE_GRID_ROWS as f32);
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.common.id,
                points: Arc::from([Point::new(bounds.min.x, y), Point::new(bounds.max.x, y)]),
                color: theme.grid_soft,
                width: 1.0,
            }));
        }
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.common.id,
            rect: bounds,
            color: theme.border_emphasis,
            width: 1.0,
        }));
    }

    fn push_curve(&self, primitives: &mut Vec<PaintPrimitive>, bounds: Rect, theme: &ThemeTokens) {
        let mut points = Vec::with_capacity(CURVE_SAMPLE_COUNT + 1);
        for step in 0..=CURVE_SAMPLE_COUNT {
            let phase = step as f32 / CURVE_SAMPLE_COUNT as f32;
            points.push(Self::curve_point(
                bounds,
                CurveNode {
                    x: phase,
                    y: sample_editable_curve(&self.curve, phase),
                },
            ));
        }
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: Arc::from(points),
            color: theme.accent_mint,
            width: 2.0,
        }));
    }

    fn push_nodes(&self, primitives: &mut Vec<PaintPrimitive>, bounds: Rect, theme: &ThemeTokens) {
        for (index, node) in self.curve.nodes.iter().copied().enumerate() {
            let center = Self::curve_point(bounds, node);
            let active = self.active_node == Some(index);
            let size = if active {
                CURVE_NODE_SIZE + 2.0
            } else {
                CURVE_NODE_SIZE
            };
            let radius = size * 0.5;
            let rect = Rect::from_xy_size(center.x - radius, center.y - radius, size, size);
            primitives.push(PaintPrimitive::FillRect(PaintFillRect {
                widget_id: self.common.id,
                rect,
                color: if active {
                    theme.accent_warning
                } else {
                    theme.surface_raised
                },
            }));
            primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
                widget_id: self.common.id,
                rect,
                color: theme.accent_copper,
                width: 1.0,
            }));
        }
    }
}

impl Widget for CurvePreviewWidget {
    fn common(&self) -> &WidgetCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        &mut self.common
    }

    fn handle_input(&mut self, bounds: Rect, input: WidgetInput) -> Option<WidgetOutput> {
        let message = match input {
            WidgetInput::PointerPress {
                position,
                button: PointerButton::Primary,
                ..
            } => self
                .hit_node(bounds, position)
                .map(|index| CurvePreviewMessage::PressNode { index }),
            WidgetInput::PointerMove { position } => {
                self.active_node.map(|index| CurvePreviewMessage::DragNode {
                    index,
                    node: Self::node_from_point(bounds, position),
                })
            }
            WidgetInput::PointerRelease {
                position,
                button: PointerButton::Primary,
                ..
            }
            | WidgetInput::PointerDrop {
                position,
                button: PointerButton::Primary,
                ..
            } => self
                .active_node
                .map(|index| CurvePreviewMessage::ReleaseNode {
                    index,
                    node: Self::node_from_point(bounds, position),
                }),
            WidgetInput::FocusChanged(false) => {
                self.active_node.map(|_| CurvePreviewMessage::Cancel)
            }
            _ => None,
        }?;
        Some(WidgetOutput::typed(message))
    }

    fn accepts_pointer_move(&self) -> bool {
        self.active_node.is_some()
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        self.push_grid(primitives, bounds, theme);
        self.push_curve(primitives, bounds, theme);
        self.push_nodes(primitives, bounds, theme);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CurvePreviewMessage {
    PressNode { index: usize },
    DragNode { index: usize, node: CurveNode },
    ReleaseNode { index: usize, node: CurveNode },
    Cancel,
}

#[cfg(test)]
mod tests {
    use super::*;
    use radiant::runtime::PaintPrimitive;

    #[test]
    fn radiant_editor_reduces_slider_messages_to_params() {
        let params = Arc::new(PumpParams::new());
        let mut state = RadiantEditorState {
            params: Arc::clone(&params),
            active_curve_node: None,
        };

        reduce_editor_message(&mut state, RadiantEditorMessage::Mix(0.25));
        reduce_editor_message(&mut state, RadiantEditorMessage::Phase(0.5));
        reduce_editor_message(&mut state, RadiantEditorMessage::OutputGain(0.5));
        reduce_editor_message(&mut state, RadiantEditorMessage::SyncDivision(1.0));

        assert!((params.mix() - 0.25).abs() < f32::EPSILON);
        assert!((params.phase_offset() - 0.5).abs() < f32::EPSILON);
        assert!((params.output_gain_db() + 6.0).abs() < f32::EPSILON);
        assert_eq!(params.sync_division(), MAX_SYNC_DIVISION as usize);
    }

    #[test]
    fn radiant_editor_curve_drag_updates_editable_curve() {
        let params = Arc::new(PumpParams::new());
        let mut state = RadiantEditorState {
            params: Arc::clone(&params),
            active_curve_node: None,
        };

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressNode { index: 1 }),
        );
        assert_eq!(state.active_curve_node, Some(1));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.2, y: 0.25 },
            }),
        );
        let curve = params.editable_curve_snapshot();
        assert!((curve.nodes[1].x - 0.2).abs() < f32::EPSILON);
        assert!((curve.nodes[1].y - 0.25).abs() < f32::EPSILON);
        assert_eq!(state.active_curve_node, Some(1));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleaseNode {
                index: 1,
                node: CurveNode { x: 0.24, y: 0.3 },
            }),
        );
        let curve = params.editable_curve_snapshot();
        assert!((curve.nodes[1].x - 0.24).abs() < f32::EPSILON);
        assert!((curve.nodes[1].y - 0.3).abs() < f32::EPSILON);
        assert_eq!(state.active_curve_node, None);
    }

    #[test]
    fn curve_preview_widget_emits_press_for_hit_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve.clone(), None);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: Default::default(),
                },
            )
            .expect("hit node should emit a press message");

        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::PressNode { index: 1 })
        );
    }

    #[test]
    fn radiant_editor_surface_emits_visible_paint() {
        let frame = radiant_editor_frame_for_params(
            Arc::new(PumpParams::new()),
            Vector2::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
        );

        assert!(frame.paint_plan.primitives.iter().any(|primitive| {
            matches!(primitive, PaintPrimitive::Text(text) if text.text.as_str() == "PUMP")
        }));
        assert!(frame
            .paint_plan
            .primitives
            .iter()
            .any(|primitive| matches!(primitive, PaintPrimitive::FillRect(_))));
        assert!(frame.paint_plan.primitives.iter().any(|primitive| {
            matches!(primitive, PaintPrimitive::StrokePolyline(polyline) if polyline.points.len() > 16)
        }));
    }
}
