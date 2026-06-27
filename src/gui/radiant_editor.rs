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

use crate::curve::{
    sample_editable_curve, CurveNode, CurveSegment, EditableCurve, MAX_EDITABLE_NODES,
    MAX_SEGMENT_TENSION, MIN_SEGMENT_TENSION,
};
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
const CURVE_PREVIEW_NODE_SIZE: f32 = 7.0;
const CURVE_NODE_HIT_RADIUS: f32 = 10.0;
const CURVE_NODE_INSERT_GUARD_RADIUS: f32 = 12.0;
const CURVE_SEGMENT_HOVER_RADIUS: f32 = 7.0;
const CURVE_SEGMENT_TENSION_PIXEL_SCALE: f32 = 120.0;
const CURVE_NODE_MIN_SPACING_X: f32 = 1.0e-3;

#[derive(Clone)]
struct ActiveCurveSegmentDrag {
    index: usize,
    origin_curve: EditableCurve,
    start_pointer: Point,
    start_tension: f32,
}

#[derive(Clone)]
struct RadiantEditorState {
    params: Arc<PumpParams>,
    active_curve_node: Option<usize>,
    active_curve_segment: Option<ActiveCurveSegmentDrag>,
    preview_curve_node: Option<CurveNode>,
    hover_curve_segment: Option<usize>,
    option_hover_held: bool,
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
                    active_curve_segment: None,
                    preview_curve_node: None,
                    hover_curve_segment: None,
                    option_hover_held: false,
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
            active_curve_segment: None,
            preview_curve_node: None,
            hover_curve_segment: None,
            option_hover_held: false,
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
                CurvePreviewWidget::new(
                    params.editable_curve_snapshot(),
                    state.active_curve_node,
                    state.active_curve_segment.as_ref().map(|drag| drag.index),
                    state.preview_curve_node,
                    state.hover_curve_segment,
                    state.option_hover_held,
                ),
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
        CurvePreviewMessage::Hover {
            preview_node,
            segment,
        } => {
            state.preview_curve_node = preview_node;
            state.hover_curve_segment = segment;
        }
        CurvePreviewMessage::OptionHoverChanged { option_held } => {
            state.option_hover_held = option_held;
            if option_held {
                state.preview_curve_node = None;
            }
        }
        CurvePreviewMessage::PressNode { index } => {
            state.active_curve_node = Some(index);
            state.active_curve_segment = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::InsertNode { node } => {
            let mut curve = state.params.editable_curve_snapshot();
            state.active_curve_segment = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            if let Some(index) = insert_curve_node(&mut curve, node) {
                state.params.set_editable_curve(&curve);
                state.active_curve_node = Some(index);
            }
        }
        CurvePreviewMessage::DragNode { index, node } => {
            let mut curve = state.params.editable_curve_snapshot();
            update_curve_node(&mut curve, index, node);
            state.params.set_editable_curve(&curve);
            state.active_curve_node = Some(index);
            state.active_curve_segment = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::ReleaseNode { index, node } => {
            let mut curve = state.params.editable_curve_snapshot();
            update_curve_node(&mut curve, index, node);
            state.params.set_editable_curve(&curve);
            state.active_curve_node = None;
            state.active_curve_segment = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::PressSegment { index, position } => {
            let curve = state.params.editable_curve_snapshot();
            if let Some(drag) = start_curve_segment_drag(&curve, index, position) {
                state.active_curve_node = None;
                state.active_curve_segment = Some(drag);
                state.preview_curve_node = None;
                state.hover_curve_segment = Some(index);
            }
        }
        CurvePreviewMessage::DragSegment { index: _, position } => {
            if let Some(drag) = state.active_curve_segment.as_ref() {
                let curve = curve_with_adjusted_segment_tension(drag, position);
                state.params.set_editable_curve(&curve);
                state.preview_curve_node = None;
                state.hover_curve_segment = Some(drag.index);
            }
        }
        CurvePreviewMessage::ReleaseSegment { index: _, position } => {
            if let Some(drag) = state.active_curve_segment.take() {
                let curve = curve_with_adjusted_segment_tension(&drag, position);
                state.params.set_editable_curve(&curve);
                state.preview_curve_node = None;
                state.hover_curve_segment = state.option_hover_held.then_some(drag.index);
            }
        }
        CurvePreviewMessage::Cancel => {
            state.active_curve_node = None;
            state.active_curve_segment = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            state.option_hover_held = false;
        }
    }
}

fn start_curve_segment_drag(
    curve: &EditableCurve,
    index: usize,
    start_pointer: Point,
) -> Option<ActiveCurveSegmentDrag> {
    let normalized = curve.clone().normalized();
    let start_tension = normalized.segments.get(index).copied()?.tension;
    (index + 1 < normalized.nodes.len()).then_some(ActiveCurveSegmentDrag {
        index,
        origin_curve: normalized,
        start_pointer,
        start_tension,
    })
}

fn curve_with_adjusted_segment_tension(
    drag: &ActiveCurveSegmentDrag,
    current_pointer: Point,
) -> EditableCurve {
    let mut curve = drag.origin_curve.clone();
    let delta = segment_tension_delta_from_drag(
        &drag.origin_curve,
        drag.index,
        drag.start_pointer,
        current_pointer,
    );
    if let Some(segment) = curve.segments.get_mut(drag.index) {
        segment.tension =
            (drag.start_tension + delta).clamp(MIN_SEGMENT_TENSION, MAX_SEGMENT_TENSION);
    }
    curve.normalize_in_place();
    curve
}

fn segment_tension_delta_from_drag(
    curve: &EditableCurve,
    segment_index: usize,
    start_pointer: Point,
    current_pointer: Point,
) -> f32 {
    let drag_units = (start_pointer.y - current_pointer.y) / CURVE_SEGMENT_TENSION_PIXEL_SCALE;
    drag_units * segment_upward_tension_sign(curve, segment_index)
}

fn segment_upward_tension_sign(curve: &EditableCurve, segment_index: usize) -> f32 {
    let left = curve.nodes.get(segment_index).copied();
    let right = curve.nodes.get(segment_index + 1).copied();
    match (left, right) {
        (Some(left_node), Some(right_node)) if right_node.y > left_node.y => -1.0,
        _ => 1.0,
    }
}

fn insert_curve_node(curve: &mut EditableCurve, node: CurveNode) -> Option<usize> {
    curve.normalize_in_place();
    if curve.nodes.len() < 2 || curve.nodes.len() >= MAX_EDITABLE_NODES {
        return None;
    }

    let mut insert_at = curve.nodes.partition_point(|existing| existing.x < node.x);
    insert_at = insert_at.clamp(1, curve.nodes.len().saturating_sub(1));

    let left_limit = curve.nodes[insert_at - 1].x + CURVE_NODE_MIN_SPACING_X;
    let right_limit = curve.nodes[insert_at].x - CURVE_NODE_MIN_SPACING_X;
    if left_limit >= right_limit {
        return None;
    }

    curve.nodes.insert(
        insert_at,
        CurveNode {
            x: node.x.clamp(left_limit, right_limit),
            y: node.y.clamp(0.0, 1.0),
        },
    );
    let inherited = curve
        .segments
        .get(insert_at.saturating_sub(1))
        .copied()
        .unwrap_or(CurveSegment { tension: 0.0 });
    curve
        .segments
        .insert(insert_at.saturating_sub(1), inherited);
    curve.normalize_in_place();
    Some(insert_at)
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
    active_segment: Option<usize>,
    preview_node: Option<CurveNode>,
    hover_segment: Option<usize>,
    option_hover_held: bool,
}

impl CurvePreviewWidget {
    fn new(
        curve: EditableCurve,
        active_node: Option<usize>,
        active_segment: Option<usize>,
        preview_node: Option<CurveNode>,
        hover_segment: Option<usize>,
        option_hover_held: bool,
    ) -> Self {
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
            active_segment,
            preview_node,
            hover_segment,
            option_hover_held,
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
        self.hit_node_within(bounds, position, CURVE_NODE_HIT_RADIUS)
    }

    fn hit_node_within(&self, bounds: Rect, position: Point, radius: f32) -> Option<usize> {
        let radius_squared = radius.max(0.0) * radius.max(0.0);
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

    fn insert_node_at(&self, bounds: Rect, position: Point) -> Option<CurveNode> {
        if !bounds.contains(position)
            || self.curve.nodes.len() < 2
            || self.curve.nodes.len() >= MAX_EDITABLE_NODES
        {
            return None;
        }

        Some(Self::node_from_point(bounds, position))
    }

    fn hover_at(&self, bounds: Rect, position: Point) -> CurveHoverState {
        let segment = if bounds.contains(position) {
            self.hit_segment(bounds, position, CURVE_SEGMENT_HOVER_RADIUS)
        } else {
            None
        };
        let preview_node = if self.option_hover_held && segment.is_some() {
            None
        } else {
            self.preview_node_at(bounds, position, segment)
        };
        CurveHoverState {
            preview_node,
            segment,
        }
    }

    fn preview_node_at(
        &self,
        bounds: Rect,
        position: Point,
        segment: Option<usize>,
    ) -> Option<CurveNode> {
        if !bounds.contains(position)
            || self.curve.nodes.len() < 2
            || self.curve.nodes.len() >= MAX_EDITABLE_NODES
            || self
                .hit_node_within(bounds, position, CURVE_NODE_INSERT_GUARD_RADIUS)
                .is_some()
            || segment.is_none()
        {
            return None;
        }

        let x = Self::node_from_point(bounds, position).x;
        if x <= CURVE_NODE_MIN_SPACING_X || x >= 1.0 - CURVE_NODE_MIN_SPACING_X {
            return None;
        }
        Some(CurveNode {
            x,
            y: sample_editable_curve(&self.curve, x).clamp(0.0, 1.0),
        })
    }

    fn hit_segment(&self, bounds: Rect, position: Point, radius: f32) -> Option<usize> {
        let radius_squared = radius.max(0.0) * radius.max(0.0);
        (0..self.curve.nodes.len().saturating_sub(1))
            .filter_map(|index| {
                let distance = self.segment_polyline_distance_squared(bounds, index, position);
                (distance <= radius_squared).then_some((index, distance))
            })
            .min_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
    }

    fn segment_polyline_distance_squared(
        &self,
        bounds: Rect,
        index: usize,
        position: Point,
    ) -> f32 {
        let Some(left) = self.curve.nodes.get(index).copied() else {
            return f32::MAX;
        };
        let Some(right) = self.curve.nodes.get(index + 1).copied() else {
            return f32::MAX;
        };

        let left_x = Self::curve_point(bounds, CurveNode { x: left.x, y: 0.0 }).x;
        let right_x = Self::curve_point(bounds, CurveNode { x: right.x, y: 0.0 }).x;
        let steps = (right_x - left_x).abs().round().clamp(2.0, 96.0) as usize;
        let mut previous = Self::curve_point(
            bounds,
            CurveNode {
                x: left.x,
                y: sample_editable_curve(&self.curve, left.x),
            },
        );
        let mut best = f32::MAX;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let x = left.x + (right.x - left.x) * t;
            let current = Self::curve_point(
                bounds,
                CurveNode {
                    x,
                    y: sample_editable_curve(&self.curve, x),
                },
            );
            best = best.min(point_to_segment_distance_squared(
                position, previous, current,
            ));
            previous = current;
        }
        best
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
        let points = self.sample_curve_points(bounds);
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: Arc::from(points),
            color: theme.accent_mint,
            width: 2.0,
        }));

        let highlighted_segment = self.active_segment.or_else(|| {
            self.option_hover_held
                .then_some(self.hover_segment)
                .flatten()
        });
        if let Some(segment) = highlighted_segment {
            let points = self.sample_segment_points(bounds, segment);
            if points.len() > 1 {
                primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                    widget_id: self.common.id,
                    points: Arc::from(points),
                    color: theme.accent_warning,
                    width: 3.5,
                }));
            }
        }
    }

    fn sample_curve_points(&self, bounds: Rect) -> Vec<Point> {
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
        points
    }

    fn sample_segment_points(&self, bounds: Rect, index: usize) -> Vec<Point> {
        let Some(left) = self.curve.nodes.get(index).copied() else {
            return Vec::new();
        };
        let Some(right) = self.curve.nodes.get(index + 1).copied() else {
            return Vec::new();
        };
        let left_x = Self::curve_point(bounds, CurveNode { x: left.x, y: 0.0 }).x;
        let right_x = Self::curve_point(bounds, CurveNode { x: right.x, y: 0.0 }).x;
        let steps = (right_x - left_x).abs().round().clamp(2.0, 96.0) as usize;
        let mut points = Vec::with_capacity(steps + 1);
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            let x = left.x + (right.x - left.x) * t;
            points.push(Self::curve_point(
                bounds,
                CurveNode {
                    x,
                    y: sample_editable_curve(&self.curve, x),
                },
            ));
        }
        points
    }

    fn push_nodes(&self, primitives: &mut Vec<PaintPrimitive>, bounds: Rect, theme: &ThemeTokens) {
        if let Some(preview) = self.preview_node {
            let center = Self::curve_point(bounds, preview);
            let radius = CURVE_PREVIEW_NODE_SIZE * 0.5;
            let rect = Rect::from_xy_size(
                center.x - radius,
                center.y - radius,
                CURVE_PREVIEW_NODE_SIZE,
                CURVE_PREVIEW_NODE_SIZE,
            );
            primitives.push(PaintPrimitive::FillRect(PaintFillRect {
                widget_id: self.common.id,
                rect,
                color: theme.accent_warning,
            }));
            primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
                widget_id: self.common.id,
                rect,
                color: theme.accent_mint,
                width: 1.0,
            }));
        }

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
                modifiers,
            } => {
                if let Some(index) = self.hit_node(bounds, position) {
                    Some(CurvePreviewMessage::PressNode { index })
                } else {
                    let hover = self.hover_at(bounds, position);
                    let option_held = self.option_hover_held || modifiers.alt;
                    if option_held {
                        hover
                            .segment
                            .map(|index| CurvePreviewMessage::PressSegment { index, position })
                    } else {
                        hover
                            .preview_node
                            .or_else(|| self.insert_node_at(bounds, position))
                            .map(|node| CurvePreviewMessage::InsertNode { node })
                    }
                }
            }
            WidgetInput::PointerMove { position } => {
                if let Some(index) = self.active_node {
                    Some(CurvePreviewMessage::DragNode {
                        index,
                        node: Self::node_from_point(bounds, position),
                    })
                } else if let Some(index) = self.active_segment {
                    Some(CurvePreviewMessage::DragSegment { index, position })
                } else {
                    let hover = self.hover_at(bounds, position);
                    (hover.preview_node != self.preview_node || hover.segment != self.hover_segment)
                        .then_some(CurvePreviewMessage::Hover {
                            preview_node: hover.preview_node,
                            segment: hover.segment,
                        })
                }
            }
            WidgetInput::PointerModifiersChanged { modifiers } => (modifiers.alt
                != self.option_hover_held)
                .then_some(CurvePreviewMessage::OptionHoverChanged {
                    option_held: modifiers.alt,
                }),
            WidgetInput::PointerRelease {
                position,
                button: PointerButton::Primary,
                ..
            }
            | WidgetInput::PointerDrop {
                position,
                button: PointerButton::Primary,
                ..
            } => {
                if let Some(index) = self.active_node {
                    Some(CurvePreviewMessage::ReleaseNode {
                        index,
                        node: Self::node_from_point(bounds, position),
                    })
                } else {
                    self.active_segment
                        .map(|index| CurvePreviewMessage::ReleaseSegment { index, position })
                }
            }
            WidgetInput::FocusChanged(false) => (self.active_node.is_some()
                || self.active_segment.is_some()
                || self.preview_node.is_some()
                || self.hover_segment.is_some()
                || self.option_hover_held)
                .then_some(CurvePreviewMessage::Cancel),
            _ => None,
        }?;
        Some(WidgetOutput::typed(message))
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
    Hover {
        preview_node: Option<CurveNode>,
        segment: Option<usize>,
    },
    OptionHoverChanged {
        option_held: bool,
    },
    PressNode {
        index: usize,
    },
    InsertNode {
        node: CurveNode,
    },
    DragNode {
        index: usize,
        node: CurveNode,
    },
    ReleaseNode {
        index: usize,
        node: CurveNode,
    },
    PressSegment {
        index: usize,
        position: Point,
    },
    DragSegment {
        index: usize,
        position: Point,
    },
    ReleaseSegment {
        index: usize,
        position: Point,
    },
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CurveHoverState {
    preview_node: Option<CurveNode>,
    segment: Option<usize>,
}

fn point_to_segment_distance_squared(point: Point, a: Point, b: Point) -> f32 {
    let abx = b.x - a.x;
    let aby = b.y - a.y;
    let ab_len_squared = abx * abx + aby * aby;
    if ab_len_squared <= f32::EPSILON {
        let dx = point.x - a.x;
        let dy = point.y - a.y;
        return dx * dx + dy * dy;
    }

    let apx = point.x - a.x;
    let apy = point.y - a.y;
    let t = ((apx * abx + apy * aby) / ab_len_squared).clamp(0.0, 1.0);
    let closest = Point::new(a.x + abx * t, a.y + aby * t);
    let dx = point.x - closest.x;
    let dy = point.y - closest.y;
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;
    use radiant::runtime::PaintPrimitive;
    use radiant::widgets::PointerModifiers;

    #[test]
    fn radiant_editor_reduces_slider_messages_to_params() {
        let params = Arc::new(PumpParams::new());
        let mut state = RadiantEditorState {
            params: Arc::clone(&params),
            active_curve_node: None,
            active_curve_segment: None,
            preview_curve_node: None,
            hover_curve_segment: None,
            option_hover_held: false,
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
            active_curve_segment: None,
            preview_curve_node: None,
            hover_curve_segment: None,
            option_hover_held: false,
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
    fn radiant_editor_curve_insert_adds_preview_node_to_params() {
        let params = Arc::new(PumpParams::new());
        let mut state = RadiantEditorState {
            params: Arc::clone(&params),
            active_curve_node: None,
            active_curve_segment: None,
            preview_curve_node: Some(CurveNode { x: 0.2, y: 0.0 }),
            hover_curve_segment: Some(1),
            option_hover_held: true,
        };
        let before = params.editable_curve_snapshot();
        let node = CurveNode {
            x: 0.2,
            y: sample_editable_curve(&before, 0.2),
        };

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::InsertNode { node }),
        );

        let after = params.editable_curve_snapshot();
        assert_eq!(after.nodes.len(), before.nodes.len() + 1);
        assert_eq!(after.segments.len(), after.nodes.len() - 1);
        assert_eq!(state.active_curve_node, Some(2));
        assert_eq!(state.preview_curve_node, None);
        assert_eq!(state.hover_curve_segment, None);
        assert!(after
            .nodes
            .iter()
            .any(|inserted| (inserted.x - node.x).abs() < 1.0e-6
                && (inserted.y - node.y).abs() < 1.0e-6));
    }

    #[test]
    fn radiant_editor_option_hover_clears_insert_preview() {
        let params = Arc::new(PumpParams::new());
        let mut state = RadiantEditorState {
            params,
            active_curve_node: None,
            active_curve_segment: None,
            preview_curve_node: Some(CurveNode { x: 0.2, y: 0.3 }),
            hover_curve_segment: Some(1),
            option_hover_held: false,
        };

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::OptionHoverChanged {
                option_held: true,
            }),
        );

        assert_eq!(state.preview_curve_node, None);
        assert_eq!(state.hover_curve_segment, Some(1));
        assert!(state.option_hover_held);
    }

    #[test]
    fn radiant_editor_segment_drag_bends_with_pointer_direction() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.2 },
                CurveNode { x: 0.5, y: 0.8 },
                CurveNode { x: 1.0, y: 0.2 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }, CurveSegment { tension: 0.0 }],
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = RadiantEditorState {
            params: Arc::clone(&params),
            active_curve_node: None,
            active_curve_segment: None,
            preview_curve_node: None,
            hover_curve_segment: None,
            option_hover_held: true,
        };
        let start = Point::new(120.0, 48.0);
        let baseline_midpoint = sample_editable_curve(&curve, 0.25);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressSegment {
                index: 0,
                position: start,
            }),
        );
        assert!(state.active_curve_segment.is_some());
        assert_eq!(state.preview_curve_node, None);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragSegment {
                index: 0,
                position: Point::new(start.x, start.y - 24.0),
            }),
        );
        let upward_midpoint = sample_editable_curve(&params.editable_curve_snapshot(), 0.25);
        assert!(upward_midpoint > baseline_midpoint);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragSegment {
                index: 0,
                position: Point::new(start.x, start.y + 24.0),
            }),
        );
        let downward_midpoint = sample_editable_curve(&params.editable_curve_snapshot(), 0.25);
        assert!(downward_midpoint < baseline_midpoint);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleaseSegment {
                index: 0,
                position: Point::new(start.x, start.y + 24.0),
            }),
        );
        assert!(state.active_curve_segment.is_none());
    }

    #[test]
    fn curve_preview_widget_emits_press_for_hit_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, false);
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
    fn curve_preview_widget_emits_preview_for_segment_hover() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let expected = CurveNode {
            x: 0.2,
            y: sample_editable_curve(&curve, 0.2),
        };
        let position = CurvePreviewWidget::curve_point(bounds, expected);

        let output = widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .expect("segment hover should emit a preview message");

        match output.typed_copied() {
            Some(CurvePreviewMessage::Hover {
                preview_node: Some(node),
                segment: Some(1),
            }) => {
                assert!((node.x - expected.x).abs() < 1.0e-6);
                assert!((node.y - expected.y).abs() < 1.0e-6);
            }
            other => panic!("unexpected segment hover output: {other:?}"),
        }
    }

    #[test]
    fn curve_preview_widget_suppresses_preview_for_option_hovered_segment() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let expected = CurveNode {
            x: 0.2,
            y: sample_editable_curve(&curve, 0.2),
        };
        let position = CurvePreviewWidget::curve_point(bounds, expected);

        let output = widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .expect("option line hover should emit segment hover state");

        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::Hover {
                preview_node: None,
                segment: Some(1),
            })
        );
    }

    #[test]
    fn curve_preview_widget_emits_insert_for_segment_press() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let expected = CurveNode {
            x: 0.2,
            y: sample_editable_curve(&curve, 0.2),
        };
        let position = CurvePreviewWidget::curve_point(bounds, expected);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: Default::default(),
                },
            )
            .expect("segment press should emit an insert message");

        match output.typed_copied() {
            Some(CurvePreviewMessage::InsertNode { node }) => {
                assert!((node.x - expected.x).abs() < 1.0e-6);
                assert!((node.y - expected.y).abs() < 1.0e-6);
            }
            other => panic!("unexpected segment press output: {other:?}"),
        }
    }

    #[test]
    fn curve_preview_widget_emits_segment_press_when_option_held() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let expected = CurveNode {
            x: 0.2,
            y: sample_editable_curve(&curve, 0.2),
        };
        let position = CurvePreviewWidget::curve_point(bounds, expected);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        alt: true,
                        ..PointerModifiers::default()
                    },
                },
            )
            .expect("option segment press should begin segment drag");

        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::PressSegment { index: 1, position })
        );
    }

    #[test]
    fn curve_preview_widget_emits_insert_for_empty_canvas_press() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let expected = CurveNode { x: 0.72, y: 0.18 };
        let position = CurvePreviewWidget::curve_point(bounds, expected);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: Default::default(),
                },
            )
            .expect("empty canvas press should emit an insert message");

        match output.typed_copied() {
            Some(CurvePreviewMessage::InsertNode { node }) => {
                assert!((node.x - expected.x).abs() < 1.0e-6);
                assert!((node.y - expected.y).abs() < 1.0e-6);
            }
            other => panic!("unexpected empty canvas press output: {other:?}"),
        }
    }

    #[test]
    fn radiant_editor_canvas_insert_can_drag_inserted_node() {
        let params = Arc::new(PumpParams::new());
        let mut state = RadiantEditorState {
            params: Arc::clone(&params),
            active_curve_node: None,
            active_curve_segment: None,
            preview_curve_node: None,
            hover_curve_segment: None,
            option_hover_held: false,
        };

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::InsertNode {
                node: CurveNode { x: 0.72, y: 0.18 },
            }),
        );
        let inserted = state
            .active_curve_node
            .expect("inserted node should become active");

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: inserted,
                node: CurveNode { x: 0.62, y: 0.42 },
            }),
        );
        let curve = params.editable_curve_snapshot();
        assert!((curve.nodes[inserted].x - 0.62).abs() < 1.0e-6);
        assert!((curve.nodes[inserted].y - 0.42).abs() < 1.0e-6);
    }

    #[test]
    fn curve_preview_widget_tracks_option_hovered_segment() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(
            bounds,
            CurveNode {
                x: 0.2,
                y: sample_editable_curve(&curve, 0.2),
            },
        );

        let output = widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .expect("line hover should emit hover state");
        assert!(matches!(
            output.typed_copied(),
            Some(CurvePreviewMessage::Hover {
                segment: Some(1),
                ..
            })
        ));

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerModifiersChanged {
                    modifiers: PointerModifiers {
                        alt: true,
                        ..PointerModifiers::default()
                    },
                },
            )
            .expect("option press should emit option-hover state");
        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::OptionHoverChanged { option_held: true })
        );
    }

    #[test]
    fn curve_preview_widget_paints_preview_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let preview = CurveNode {
            x: 0.2,
            y: sample_editable_curve(&curve, 0.2),
        };
        let widget = CurvePreviewWidget::new(curve, None, None, Some(preview), None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::FillRect(fill)
                    if fill.color == theme.accent_warning
                        && (fill.rect.width() - CURVE_PREVIEW_NODE_SIZE).abs() < 1.0e-6
                        && (fill.rect.height() - CURVE_PREVIEW_NODE_SIZE).abs() < 1.0e-6
            )
        }));
    }

    #[test]
    fn curve_preview_widget_paints_option_hovered_segment() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, None, None, None, Some(1), true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokePolyline(polyline)
                    if polyline.color == theme.accent_warning
                        && (polyline.width - 3.5).abs() < 1.0e-6
                        && polyline.points.len() > 2
            )
        }));
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
