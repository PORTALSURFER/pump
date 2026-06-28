//! Shared Radiant surface for Pump editor hosts.

use std::sync::Arc;

use radiant::gui::types::{Point, Rect, Vector2};
use radiant::layout::LayoutOutput;
use radiant::prelude::{
    column, custom_widget_mapped, row, slider, text, IntoView, TextAlign, UiSurface, ViewNode,
};
#[cfg(test)]
use radiant::runtime::SurfaceFrame;
use radiant::runtime::{
    DeclarativeSurfaceRuntime, PaintFillRect, PaintPrimitive, PaintStrokePolyline, PaintStrokeRect,
    PaintText, PaintTextAlign, PaintTextRun,
};
#[cfg(feature = "vst3")]
use radiant::runtime::{Event, SurfacePaintPlan};
use radiant::theme::ThemeTokens;
use radiant::widgets::{
    FocusBehavior, PointerButton, TextWrap, Widget, WidgetCommon, WidgetInput, WidgetKey,
    WidgetOutput,
};
use toybox::clack_plugin::utils::ClapId;
use toybox::clap::automation::{AutomationConfig, AutomationQueue};

use crate::curve::{
    sample_editable_curve, CurveNode, CurveSegment, EditableCurve, MAX_EDITABLE_NODES,
    MAX_SEGMENT_TENSION, MIN_SEGMENT_TENSION,
};
use crate::params::{
    format_plain_value_text, parse_plain_value_text, sync_division_label, PumpParams,
    GLOBAL_CURVE_SLOT_COUNT, MAX_OUTPUT_GAIN_DB, MAX_SYNC_DIVISION, MIN_OUTPUT_GAIN_DB,
    PARAM_MIX_ID, PARAM_OUTPUT_GAIN_ID, PARAM_PHASE_OFFSET_ID,
};
use crate::GuiStatus;

use super::{build_version_label, WINDOW_HEIGHT, WINDOW_WIDTH};

const BUILD_LABEL_HEIGHT: f32 = 16.0;
const CURVE_PREVIEW_HEIGHT: f32 = 72.0;
const CURVE_SLOT_ROW_HEIGHT: f32 = 22.0;
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
const CURVE_PLAYHEAD_MARKER_SIZE: f32 = 5.5;
const CURVE_PLAYHEAD_MARKER_GLOW_SIZE: f32 = 9.5;
const CURVE_SLOT_PREVIEW_STEPS: usize = 24;
const CURVE_SLOT_MARGIN: f32 = 3.0;
const VALUE_ENTRY_MAX_CHARS: usize = 16;
const VALUE_LABEL_FONT_SIZE: f32 = 12.0;

#[derive(Clone)]
struct ActiveCurveSegmentDrag {
    index: usize,
    origin_curve: EditableCurve,
    start_pointer: Point,
    start_tension: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NumericEntryTarget {
    Mix,
    Phase,
    OutputGain,
}

impl NumericEntryTarget {
    fn param_id(self) -> ClapId {
        match self {
            Self::Mix => PARAM_MIX_ID,
            Self::Phase => PARAM_PHASE_OFFSET_ID,
            Self::OutputGain => PARAM_OUTPUT_GAIN_ID,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Mix => "Mix",
            Self::Phase => "Phase",
            Self::OutputGain => "Output",
        }
    }

    fn widget_key(self) -> &'static str {
        match self {
            Self::Mix => "numeric-entry-mix",
            Self::Phase => "numeric-entry-phase",
            Self::OutputGain => "numeric-entry-output",
        }
    }

    fn current_plain_value(self, params: &PumpParams) -> f64 {
        match self {
            Self::Mix => params.mix() as f64,
            Self::Phase => params.phase_offset() as f64,
            Self::OutputGain => params.output_gain_db() as f64,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct NumericEntryState {
    target: NumericEntryTarget,
    draft: String,
    dirty: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum NumericEntryMessage {
    Begin {
        target: NumericEntryTarget,
    },
    DraftChanged {
        target: NumericEntryTarget,
        draft: String,
        dirty: bool,
    },
    Commit {
        target: NumericEntryTarget,
        draft: String,
    },
    Cancel {
        target: NumericEntryTarget,
    },
}

#[derive(Clone)]
struct RadiantEditorState {
    params: Arc<PumpParams>,
    status: Arc<GuiStatus>,
    automation_queue: Arc<AutomationQueue>,
    automation_config: AutomationConfig,
    active_curve_node: Option<usize>,
    active_curve_segment: Option<ActiveCurveSegmentDrag>,
    hover_curve_node: Option<usize>,
    preview_curve_node: Option<CurveNode>,
    hover_curve_segment: Option<usize>,
    option_hover_held: bool,
    loaded_global_curve_slot: Option<usize>,
    numeric_entry: Option<NumericEntryState>,
}

#[derive(Clone, Debug, PartialEq)]
enum RadiantEditorMessage {
    Mix(f32),
    Phase(f32),
    OutputGain(f32),
    SyncDivision(f32),
    Curve(CurvePreviewMessage),
    CurveSlot(CurveSlotMessage),
    NumericEntry(NumericEntryMessage),
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
    status: Arc<GuiStatus>,
    theme: ThemeTokens,
    paint_plan: SurfacePaintPlan,
}

#[cfg(feature = "vst3")]
impl RadiantPumpEditor {
    /// Build a Radiant editor runtime at the provided logical viewport.
    pub(crate) fn new(
        params: Arc<PumpParams>,
        status: Arc<GuiStatus>,
        automation_queue: Arc<AutomationQueue>,
        width: u32,
        height: u32,
    ) -> Self {
        let theme = ThemeTokens::default();
        let viewport = Vector2::new(width.max(1) as f32, height.max(1) as f32);
        Self {
            runtime: EditorSurfaceRuntime::new_declarative(
                RadiantEditorState::new(params, Arc::clone(&status), automation_queue),
                viewport,
                project_editor_surface,
                reduce_editor_message,
            ),
            status,
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

    /// Route a focused key press into the Radiant runtime.
    pub(crate) fn dispatch_key_press(&mut self, key: WidgetKey) -> bool {
        self.runtime.dispatch_event(Event::key_press(key)).is_some()
    }

    /// Route a focused text character into the Radiant runtime.
    pub(crate) fn dispatch_character(&mut self, ch: char) -> bool {
        self.runtime.dispatch_event(Event::character(ch)).is_some()
    }

    /// Cancel active numeric value entry, if any.
    pub(crate) fn cancel_numeric_entry(&mut self) -> bool {
        if self.runtime.bridge().state().numeric_entry.is_none() {
            return false;
        }
        let _ = self.runtime.dispatch_event(Event::clear_focus());
        if self.runtime.bridge().state().numeric_entry.is_some() {
            self.runtime.bridge_mut().state_mut().numeric_entry = None;
            self.runtime.refresh();
        }
        true
    }

    /// Return whether the hosted view should keep repainting without input.
    pub(crate) fn needs_realtime_redraw(&self) -> bool {
        self.status.has_host_beats_timeline() || self.status.is_playing()
    }

    /// Refresh and return the current backend-neutral paint plan.
    pub(crate) fn paint_plan(&mut self) -> &SurfacePaintPlan {
        if self.needs_realtime_redraw() {
            self.runtime.refresh();
        }
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
    status: Arc<GuiStatus>,
    viewport: Vector2,
) -> SurfaceFrame {
    EditorSurfaceRuntime::new_declarative(
        RadiantEditorState::new(params, status, Arc::new(AutomationQueue::default())),
        viewport,
        project_editor_surface,
        reduce_editor_message,
    )
    .frame(&ThemeTokens::default())
}

impl RadiantEditorState {
    fn new(
        params: Arc<PumpParams>,
        status: Arc<GuiStatus>,
        automation_queue: Arc<AutomationQueue>,
    ) -> Self {
        Self {
            params,
            status,
            automation_queue,
            automation_config: AutomationConfig::default(),
            active_curve_node: None,
            active_curve_segment: None,
            hover_curve_node: None,
            preview_curve_node: None,
            hover_curve_segment: None,
            option_hover_held: false,
            loaded_global_curve_slot: None,
            numeric_entry: None,
        }
    }
}

fn project_editor_surface(state: &mut RadiantEditorState) -> Arc<UiSurface<RadiantEditorMessage>> {
    let params = state.params.as_ref();
    let output = params.output_gain_db();
    let sync = params.sync_division();
    let playhead_phase = (state.status.has_host_beats_timeline() || state.status.is_playing())
        .then_some(state.status.phase());
    Arc::new(
        column([
            text(build_version_label())
                .muted_text()
                .align_text(TextAlign::Right)
                .height(BUILD_LABEL_HEIGHT)
                .fill_width(),
            custom_widget_mapped(
                CurvePreviewWidget::new(
                    params.editable_curve_snapshot(),
                    state.active_curve_node,
                    state.active_curve_segment.as_ref().map(|drag| drag.index),
                    state.hover_curve_node,
                    state.preview_curve_node,
                    state.hover_curve_segment,
                    state.option_hover_held,
                )
                .with_playhead_phase(playhead_phase),
                RadiantEditorMessage::Curve,
            )
            .fill_width()
            .height(CURVE_PREVIEW_HEIGHT),
            curve_slot_row(state),
            control_row(
                NumericEntryTarget::Mix,
                format!("{:.0}%", params.mix() * 100.0),
                params.mix(),
                state.numeric_entry.as_ref(),
                RadiantEditorMessage::Mix,
            ),
            control_row(
                NumericEntryTarget::Phase,
                format!("{:.0}%", params.phase_offset() * 100.0),
                params.phase_offset(),
                state.numeric_entry.as_ref(),
                RadiantEditorMessage::Phase,
            ),
            control_row(
                NumericEntryTarget::OutputGain,
                format!("{output:+.1} dB"),
                normalize_output_gain(output),
                state.numeric_entry.as_ref(),
                RadiantEditorMessage::OutputGain,
            ),
            enum_control_row(
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
    target: NumericEntryTarget,
    value_label: String,
    value: f32,
    active_entry: Option<&NumericEntryState>,
    message: fn(f32) -> RadiantEditorMessage,
) -> ViewNode<RadiantEditorMessage> {
    let value_label = value_label_node(target, value_label, active_entry);
    slider_control_row(target.label(), value_label, value, message)
}

fn enum_control_row(
    label: &'static str,
    value_label: String,
    value: f32,
    message: fn(f32) -> RadiantEditorMessage,
) -> ViewNode<RadiantEditorMessage> {
    slider_control_row(
        label,
        text(value_label)
            .align_text(TextAlign::Right)
            .width(CONTROL_VALUE_WIDTH)
            .height(CONTROL_ROW_HEIGHT),
        value,
        message,
    )
}

fn slider_control_row(
    label: &'static str,
    value_label: ViewNode<RadiantEditorMessage>,
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
        value_label,
    ])
    .spacing(8.0)
    .fill_width()
    .height(CONTROL_ROW_HEIGHT)
}

fn value_label_node(
    target: NumericEntryTarget,
    value_label: String,
    active_entry: Option<&NumericEntryState>,
) -> ViewNode<RadiantEditorMessage> {
    let (display, editing, dirty) = active_entry
        .filter(|entry| entry.target == target)
        .map(|entry| (entry.draft.clone(), true, entry.dirty))
        .unwrap_or((value_label, false, false));
    custom_widget_mapped(
        NumericValueLabelWidget::new(target, display, editing, dirty),
        RadiantEditorMessage::NumericEntry,
    )
    .key(target.widget_key())
    .width(CONTROL_VALUE_WIDTH)
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
        RadiantEditorMessage::CurveSlot(message) => reduce_curve_slot_message(state, message),
        RadiantEditorMessage::NumericEntry(message) => reduce_numeric_entry_message(state, message),
    }
}

fn curve_slot_row(state: &RadiantEditorState) -> ViewNode<RadiantEditorMessage> {
    let slots = state.params.global_curve_slots_snapshot();
    let loaded_slot = state.loaded_global_curve_slot;
    let deviated_slot =
        loaded_slot.filter(|index| state.params.current_curve_deviates_from_global_slot(*index));
    let slot_nodes: [ViewNode<RadiantEditorMessage>; GLOBAL_CURVE_SLOT_COUNT] =
        std::array::from_fn(|index| {
            let curve = slots.get(index).and_then(|slot| slot.curve.clone());
            custom_widget_mapped(
                CurveSlotWidget::new(
                    index,
                    curve,
                    loaded_slot == Some(index),
                    deviated_slot == Some(index),
                ),
                RadiantEditorMessage::CurveSlot,
            )
            .height(CURVE_SLOT_ROW_HEIGHT)
        });
    row(slot_nodes)
        .spacing(4.0)
        .fill_width()
        .height(CURVE_SLOT_ROW_HEIGHT)
}

fn reduce_curve_slot_message(state: &mut RadiantEditorState, message: CurveSlotMessage) {
    match message {
        CurveSlotMessage::Load { index } => {
            let Some(curve) = state.params.global_curve_slot_curve(index) else {
                return;
            };
            state.params.set_editable_curve(&curve);
            state.active_curve_node = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            state.loaded_global_curve_slot = Some(index);
        }
        CurveSlotMessage::Store { index } => {
            let curve = state.params.editable_curve_snapshot();
            if state.params.set_global_curve_slot_curve(index, &curve) {
                state.loaded_global_curve_slot = Some(index);
            }
        }
    }
}

fn reduce_numeric_entry_message(state: &mut RadiantEditorState, message: NumericEntryMessage) {
    match message {
        NumericEntryMessage::Begin { target } => {
            let value = target.current_plain_value(state.params.as_ref());
            let draft = format_plain_value_text(target.param_id(), value)
                .unwrap_or_else(|| value.to_string());
            state.numeric_entry = Some(NumericEntryState {
                target,
                draft,
                dirty: false,
            });
        }
        NumericEntryMessage::DraftChanged {
            target,
            draft,
            dirty,
        } => {
            if state
                .numeric_entry
                .as_ref()
                .is_some_and(|entry| entry.target == target)
            {
                state.numeric_entry = Some(NumericEntryState {
                    target,
                    draft,
                    dirty,
                });
            }
        }
        NumericEntryMessage::Commit { target, draft } => {
            let entry_active = state
                .numeric_entry
                .as_ref()
                .is_some_and(|entry| entry.target == target);
            if entry_active {
                let Some(value) = parse_plain_value_text(target.param_id(), draft.trim()) else {
                    return;
                };
                apply_numeric_entry_value(state, target, value);
                state.numeric_entry = None;
            }
        }
        NumericEntryMessage::Cancel { target } => {
            if state
                .numeric_entry
                .as_ref()
                .is_some_and(|entry| entry.target == target)
            {
                state.numeric_entry = None;
            }
        }
    }
}

fn apply_numeric_entry_value(
    state: &mut RadiantEditorState,
    target: NumericEntryTarget,
    value: f64,
) {
    match target {
        NumericEntryTarget::Mix => state.params.set_mix(value as f32),
        NumericEntryTarget::Phase => state.params.set_phase_offset(value as f32),
        NumericEntryTarget::OutputGain => state.params.set_output_gain_db(value as f32),
    }

    let param_id = target.param_id();
    let _ = state
        .automation_queue
        .push_gesture_begin(&state.automation_config, param_id);
    let _ = state
        .automation_queue
        .push_value(&state.automation_config, param_id, value);
    let _ = state
        .automation_queue
        .push_gesture_end(&state.automation_config, param_id);
}

fn reduce_curve_message(state: &mut RadiantEditorState, message: CurvePreviewMessage) {
    match message {
        CurvePreviewMessage::Hover {
            node,
            preview_node,
            segment,
        } => {
            state.hover_curve_node = node;
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
            state.hover_curve_node = Some(index);
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::InsertNode { node } => {
            let mut curve = state.params.editable_curve_snapshot();
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            if let Some(index) = insert_curve_node(&mut curve, node) {
                state.params.set_editable_curve(&curve);
                state.active_curve_node = Some(index);
                state.hover_curve_node = Some(index);
            }
        }
        CurvePreviewMessage::DeleteNode { index } => {
            let mut curve = state.params.editable_curve_snapshot();
            if delete_curve_node(&mut curve, index) {
                state.params.set_editable_curve(&curve);
            }
            state.active_curve_node = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::DragNode { index, node } => {
            let mut curve = state.params.editable_curve_snapshot();
            update_curve_node(&mut curve, index, node);
            state.params.set_editable_curve(&curve);
            state.active_curve_node = Some(index);
            state.active_curve_segment = None;
            state.hover_curve_node = Some(index);
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::ReleaseNode { index, node } => {
            let mut curve = state.params.editable_curve_snapshot();
            update_curve_node(&mut curve, index, node);
            state.params.set_editable_curve(&curve);
            state.active_curve_node = None;
            state.active_curve_segment = None;
            state.hover_curve_node = Some(index);
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::PressSegment { index, position } => {
            let curve = state.params.editable_curve_snapshot();
            if let Some(drag) = start_curve_segment_drag(&curve, index, position) {
                state.active_curve_node = None;
                state.active_curve_segment = Some(drag);
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = Some(index);
            }
        }
        CurvePreviewMessage::DragSegment { index: _, position } => {
            if let Some(drag) = state.active_curve_segment.as_ref() {
                let curve = curve_with_adjusted_segment_tension(drag, position);
                state.params.set_editable_curve(&curve);
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = Some(drag.index);
            }
        }
        CurvePreviewMessage::ReleaseSegment { index: _, position } => {
            if let Some(drag) = state.active_curve_segment.take() {
                let curve = curve_with_adjusted_segment_tension(&drag, position);
                state.params.set_editable_curve(&curve);
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = state.option_hover_held.then_some(drag.index);
            }
        }
        CurvePreviewMessage::Cancel => {
            state.active_curve_node = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
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

fn delete_curve_node(curve: &mut EditableCurve, index: usize) -> bool {
    curve.normalize_in_place();
    if index == 0 || index + 1 >= curve.nodes.len() {
        return false;
    }

    curve.nodes.remove(index);
    if !curve.segments.is_empty() {
        let remove_segment = index
            .saturating_sub(1)
            .min(curve.segments.len().saturating_sub(1));
        curve.segments.remove(remove_segment);
    }
    curve.normalize_in_place();
    true
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
struct NumericValueLabelWidget {
    common: WidgetCommon,
    target: NumericEntryTarget,
    text: String,
    editing: bool,
    dirty: bool,
}

impl NumericValueLabelWidget {
    fn new(target: NumericEntryTarget, text: String, editing: bool, dirty: bool) -> Self {
        Self {
            common: WidgetCommon::fixed(0, CONTROL_VALUE_WIDTH, CONTROL_ROW_HEIGHT)
                .with_focus(FocusBehavior::Keyboard)
                .without_default_chrome(),
            target,
            text,
            editing,
            dirty,
        }
    }

    fn draft_with_character(&self, ch: char) -> Option<String> {
        if !is_numeric_entry_char(ch) {
            return None;
        }
        let mut draft = if self.dirty {
            self.text.clone()
        } else {
            String::new()
        };
        if draft.chars().count() >= VALUE_ENTRY_MAX_CHARS {
            return None;
        }
        draft.push(ch);
        Some(draft)
    }

    fn draft_after_backspace(&self) -> String {
        if !self.dirty {
            return String::new();
        }
        let mut draft = self.text.clone();
        draft.pop();
        draft
    }

    fn draft_after_delete(&self) -> String {
        if self.dirty {
            self.text.clone()
        } else {
            String::new()
        }
    }

    fn changed(&self, draft: String) -> NumericEntryMessage {
        NumericEntryMessage::DraftChanged {
            target: self.target,
            draft,
            dirty: true,
        }
    }
}

impl Widget for NumericValueLabelWidget {
    fn common(&self) -> &WidgetCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        &mut self.common
    }

    fn handle_input(&mut self, bounds: Rect, input: WidgetInput) -> Option<WidgetOutput> {
        let message = match input {
            WidgetInput::PointerMove { position } => {
                self.common.state.hovered = bounds.contains(position);
                None
            }
            WidgetInput::PointerPress {
                position,
                button: PointerButton::Primary,
                modifiers,
            } if bounds.contains(position) => {
                self.common.state.focused = true;
                self.common.state.hovered = true;
                modifiers.command.then_some(NumericEntryMessage::Begin {
                    target: self.target,
                })
            }
            WidgetInput::FocusChanged(focused) => {
                self.common.state.focused = focused;
                (!focused && self.editing).then_some(NumericEntryMessage::Cancel {
                    target: self.target,
                })
            }
            WidgetInput::Character('\u{1b}') if self.editing => Some(NumericEntryMessage::Cancel {
                target: self.target,
            }),
            WidgetInput::Character('\r' | '\n') if self.editing => {
                Some(NumericEntryMessage::Commit {
                    target: self.target,
                    draft: self.text.clone(),
                })
            }
            WidgetInput::Character(ch) if self.editing => self
                .draft_with_character(ch)
                .map(|draft| self.changed(draft)),
            WidgetInput::KeyPress(WidgetKey::Enter) if self.editing => {
                Some(NumericEntryMessage::Commit {
                    target: self.target,
                    draft: self.text.clone(),
                })
            }
            WidgetInput::KeyPress(WidgetKey::Backspace) if self.editing => {
                Some(self.changed(self.draft_after_backspace()))
            }
            WidgetInput::KeyPress(WidgetKey::Delete) if self.editing => {
                Some(self.changed(self.draft_after_delete()))
            }
            _ => None,
        }?;
        Some(WidgetOutput::typed(message))
    }

    fn accepts_text_input(&self) -> bool {
        self.editing
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        if self.editing {
            primitives.push(PaintPrimitive::FillRect(PaintFillRect {
                widget_id: self.common.id,
                rect: bounds,
                color: theme.bg_primary,
            }));
            primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
                widget_id: self.common.id,
                rect: bounds,
                color: if self.common.state.focused {
                    theme.accent_warning
                } else {
                    theme.border_emphasis
                },
                width: 1.0,
            }));
        }
        primitives.push(PaintPrimitive::Text(PaintTextRun {
            widget_id: self.common.id,
            text: PaintText::from(self.text.clone()),
            rect: bounds,
            font_size: VALUE_LABEL_FONT_SIZE,
            baseline: None,
            color: if self.editing {
                theme.text_primary
            } else {
                theme.text_muted
            },
            align: if self.editing {
                PaintTextAlign::Left
            } else {
                PaintTextAlign::Right
            },
            wrap: TextWrap::None,
        }));
    }

    fn automation_label(&self) -> Option<String> {
        Some(format!("{} value", self.target.label()))
    }

    fn automation_value_text(&self) -> Option<String> {
        Some(self.text.clone())
    }
}

fn is_numeric_entry_char(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+' | '%' | ' ' | 'd' | 'D' | 'b' | 'B')
}

#[derive(Clone)]
struct CurveSlotWidget {
    common: WidgetCommon,
    index: usize,
    curve: Option<EditableCurve>,
    loaded: bool,
    deviated: bool,
}

impl CurveSlotWidget {
    fn new(index: usize, curve: Option<EditableCurve>, loaded: bool, deviated: bool) -> Self {
        Self {
            common: WidgetCommon::fixed(
                0,
                ((WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0) / GLOBAL_CURVE_SLOT_COUNT as f32)
                    .max(1.0),
                CURVE_SLOT_ROW_HEIGHT,
            )
            .with_pointer_focus()
            .without_default_chrome(),
            index,
            curve: curve.map(|curve| curve.normalized()),
            loaded,
            deviated,
        }
    }

    fn sample_points(&self, bounds: Rect) -> Arc<[Point]> {
        let margin = CURVE_SLOT_MARGIN
            .min(bounds.width() * 0.25)
            .min(bounds.height() * 0.25)
            .max(1.0);
        let inner_w = (bounds.width() - margin * 2.0).max(1.0);
        let inner_h = (bounds.height() - margin * 2.0).max(1.0);
        if let Some(curve) = self.curve.as_ref() {
            let points: Vec<Point> = (0..CURVE_SLOT_PREVIEW_STEPS.max(2))
                .map(|step| {
                    let steps = CURVE_SLOT_PREVIEW_STEPS.max(2);
                    let t = step as f32 / (steps - 1) as f32;
                    Point::new(
                        bounds.min.x + margin + t * inner_w,
                        bounds.min.y
                            + margin
                            + (1.0 - sample_editable_curve(curve, t).clamp(0.0, 1.0)) * inner_h,
                    )
                })
                .collect();
            Arc::from(points)
        } else {
            let y = bounds.min.y + margin + inner_h * 0.5;
            Arc::from([
                Point::new(bounds.min.x + margin, y),
                Point::new(bounds.max.x - margin, y),
            ])
        }
    }
}

impl Widget for CurveSlotWidget {
    fn common(&self) -> &WidgetCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        &mut self.common
    }

    fn handle_input(&mut self, bounds: Rect, input: WidgetInput) -> Option<WidgetOutput> {
        let message = match input {
            WidgetInput::PointerMove { position } => {
                self.common.state.hovered = bounds.contains(position);
                None
            }
            WidgetInput::PointerPress {
                position,
                button: PointerButton::Primary,
                modifiers,
            } if bounds.contains(position) => {
                self.common.state.focused = true;
                self.common.state.hovered = true;
                if modifiers.command {
                    Some(CurveSlotMessage::Store { index: self.index })
                } else {
                    Some(CurveSlotMessage::Load { index: self.index })
                }
            }
            WidgetInput::FocusChanged(focused) => {
                self.common.state.focused = focused;
                None
            }
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
        let hovered = self.common.state.hovered;
        let fill = if self.deviated {
            theme.accent_danger
        } else if self.loaded {
            theme.surface_raised
        } else if hovered {
            theme.bg_secondary
        } else {
            theme.bg_primary
        };
        let outline = if self.deviated {
            theme.accent_danger
        } else if hovered || self.loaded {
            theme.accent_warning
        } else {
            theme.border
        };
        let curve_color = if self.deviated {
            theme.text_primary
        } else if self.curve.is_some() {
            theme.accent_mint
        } else {
            theme.text_muted
        };
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.common.id,
            rect: bounds,
            color: fill,
        }));
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.common.id,
            rect: bounds,
            color: outline,
            width: 1.0,
        }));
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: self.sample_points(bounds),
            color: curve_color,
            width: if hovered || self.loaded || self.deviated {
                2.0
            } else {
                1.0
            },
        }));
    }

    fn automation_label(&self) -> Option<String> {
        Some(format!("Curve slot {}", self.index + 1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CurveSlotMessage {
    Load { index: usize },
    Store { index: usize },
}

#[derive(Clone)]
struct CurvePreviewWidget {
    common: WidgetCommon,
    curve: EditableCurve,
    active_node: Option<usize>,
    active_segment: Option<usize>,
    hover_node: Option<usize>,
    preview_node: Option<CurveNode>,
    hover_segment: Option<usize>,
    option_hover_held: bool,
    playhead_phase: Option<f32>,
}

impl CurvePreviewWidget {
    fn new(
        curve: EditableCurve,
        active_node: Option<usize>,
        active_segment: Option<usize>,
        hover_node: Option<usize>,
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
            hover_node,
            preview_node,
            hover_segment,
            option_hover_held,
            playhead_phase: None,
        }
    }

    fn with_playhead_phase(mut self, playhead_phase: Option<f32>) -> Self {
        self.playhead_phase = playhead_phase.map(|phase| phase.rem_euclid(1.0));
        self
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
        let node = if bounds.contains(position) {
            self.hit_node(bounds, position)
        } else {
            None
        };
        let segment = if node.is_none() && bounds.contains(position) {
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
            node,
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
            let hovered = self.hover_node == Some(index);
            let size = if active {
                CURVE_NODE_SIZE + 2.0
            } else if hovered {
                CURVE_NODE_SIZE + 1.5
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
                } else if hovered {
                    theme.accent_mint
                } else {
                    theme.surface_raised
                },
            }));
            primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
                widget_id: self.common.id,
                rect,
                color: if active && hovered {
                    theme.accent_mint
                } else if hovered {
                    theme.accent_warning
                } else {
                    theme.accent_copper
                },
                width: if hovered { 1.5 } else { 1.0 },
            }));
        }
    }

    fn push_playhead(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        theme: &ThemeTokens,
    ) {
        let Some(phase) = self.playhead_phase else {
            return;
        };
        let sample = sample_editable_curve(&self.curve, phase).clamp(0.0, 1.0);
        let center = Self::curve_point(
            bounds,
            CurveNode {
                x: phase,
                y: sample,
            },
        );
        let glow_radius = CURVE_PLAYHEAD_MARKER_GLOW_SIZE * 0.5;
        let core_radius = CURVE_PLAYHEAD_MARKER_SIZE * 0.5;
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.common.id,
            rect: Rect::from_xy_size(
                center.x - glow_radius,
                center.y - glow_radius,
                CURVE_PLAYHEAD_MARKER_GLOW_SIZE,
                CURVE_PLAYHEAD_MARKER_GLOW_SIZE,
            ),
            color: theme.accent_copper,
        }));
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.common.id,
            rect: Rect::from_xy_size(
                center.x - core_radius,
                center.y - core_radius,
                CURVE_PLAYHEAD_MARKER_SIZE,
                CURVE_PLAYHEAD_MARKER_SIZE,
            ),
            color: theme.accent_warning,
        }));
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.common.id,
            rect: Rect::from_xy_size(
                center.x - core_radius,
                center.y - core_radius,
                CURVE_PLAYHEAD_MARKER_SIZE,
                CURVE_PLAYHEAD_MARKER_SIZE,
            ),
            color: theme.accent_mint,
            width: 1.0,
        }));
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
            WidgetInput::PointerDoubleClick {
                position,
                button: PointerButton::Primary,
                ..
            } => self
                .hit_node(bounds, position)
                .filter(|index| *index > 0 && *index + 1 < self.curve.nodes.len())
                .map(|index| CurvePreviewMessage::DeleteNode { index }),
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
                    (hover.node != self.hover_node
                        || hover.preview_node != self.preview_node
                        || hover.segment != self.hover_segment)
                        .then_some(CurvePreviewMessage::Hover {
                            node: hover.node,
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
                || self.hover_node.is_some()
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
        self.push_playhead(primitives, bounds, theme);
        self.push_nodes(primitives, bounds, theme);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CurvePreviewMessage {
    Hover {
        node: Option<usize>,
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
    DeleteNode {
        index: usize,
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
    node: Option<usize>,
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
    #[cfg(feature = "vst3")]
    use crate::GuiTransportTelemetry;
    use radiant::runtime::PaintPrimitive;
    use radiant::widgets::PointerModifiers;

    fn editor_state(params: Arc<PumpParams>) -> RadiantEditorState {
        RadiantEditorState::new(
            params,
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
        )
    }

    #[test]
    fn curve_slot_widget_click_loads_and_command_click_stores() {
        let bounds = Rect::from_xy_size(0.0, 0.0, 48.0, CURVE_SLOT_ROW_HEIGHT);
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurveSlotWidget::new(3, Some(curve), false, false);

        let load = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: Point::new(10.0, 10.0),
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers::default(),
                },
            )
            .expect("normal slot click should emit load");
        assert_eq!(
            load.typed_copied(),
            Some(CurveSlotMessage::Load { index: 3 })
        );

        let store = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: Point::new(10.0, 10.0),
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        command: true,
                        ..PointerModifiers::default()
                    },
                },
            )
            .expect("command slot click should emit store");
        assert_eq!(
            store.typed_copied(),
            Some(CurveSlotMessage::Store { index: 3 })
        );
    }

    #[test]
    fn curve_slot_widget_paints_deviation_in_danger_color() {
        let bounds = Rect::from_xy_size(0.0, 0.0, 48.0, CURVE_SLOT_ROW_HEIGHT);
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurveSlotWidget::new(0, Some(curve), true, true);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::FillRect(fill) if fill.color == theme.accent_danger
            )
        }));
        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokeRect(stroke) if stroke.color == theme.accent_danger
            )
        }));
    }

    #[test]
    fn radiant_editor_reduces_slider_messages_to_params() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));

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
    fn radiant_numeric_entry_commit_valid_value_updates_param() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::Begin {
                target: NumericEntryTarget::Mix,
            }),
        );
        assert_eq!(
            state
                .numeric_entry
                .as_ref()
                .map(|entry| entry.draft.as_str()),
            Some("100%")
        );

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::DraftChanged {
                target: NumericEntryTarget::Mix,
                draft: "25".to_string(),
                dirty: true,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::Commit {
                target: NumericEntryTarget::Mix,
                draft: "25".to_string(),
            }),
        );

        assert!((params.mix() - 0.25).abs() < f32::EPSILON);
        assert!(state.numeric_entry.is_none());
    }

    #[test]
    fn radiant_numeric_entry_rejects_invalid_commit_without_corrupting_param() {
        let params = Arc::new(PumpParams::new());
        params.set_output_gain_db(-3.0);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::Begin {
                target: NumericEntryTarget::OutputGain,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::Commit {
                target: NumericEntryTarget::OutputGain,
                draft: "not a number".to_string(),
            }),
        );

        assert!((params.output_gain_db() + 3.0).abs() < f32::EPSILON);
        assert_eq!(
            state.numeric_entry.as_ref().map(|entry| entry.target),
            Some(NumericEntryTarget::OutputGain)
        );
    }

    #[test]
    fn radiant_numeric_entry_cancel_leaves_prior_value() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::Begin {
                target: NumericEntryTarget::Phase,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::DraftChanged {
                target: NumericEntryTarget::Phase,
                draft: "75".to_string(),
                dirty: true,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::Cancel {
                target: NumericEntryTarget::Phase,
            }),
        );

        assert!((params.phase_offset() - 0.0).abs() < f32::EPSILON);
        assert!(state.numeric_entry.is_none());
    }

    #[test]
    fn numeric_value_label_starts_edit_only_on_command_click() {
        let bounds = Rect::from_xy_size(0.0, 0.0, CONTROL_VALUE_WIDTH, CONTROL_ROW_HEIGHT);
        let mut widget =
            NumericValueLabelWidget::new(NumericEntryTarget::Mix, "50%".to_string(), false, false);

        assert!(widget
            .handle_input(bounds, WidgetInput::primary_press(Point::new(8.0, 8.0)))
            .is_none());

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: Point::new(8.0, 8.0),
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        command: true,
                        ..PointerModifiers::default()
                    },
                },
            )
            .expect("command-click should begin numeric entry");

        assert_eq!(
            output.typed_cloned::<NumericEntryMessage>(),
            Some(NumericEntryMessage::Begin {
                target: NumericEntryTarget::Mix
            })
        );
    }

    #[test]
    fn numeric_value_label_edits_commits_and_cancels_draft() {
        let bounds = Rect::from_xy_size(0.0, 0.0, CONTROL_VALUE_WIDTH, CONTROL_ROW_HEIGHT);
        let mut widget =
            NumericValueLabelWidget::new(NumericEntryTarget::Mix, "100%".to_string(), true, false);

        let output = widget
            .handle_input(bounds, WidgetInput::Character('2'))
            .expect("first typed character should replace selected label");
        assert_eq!(
            output.typed_cloned::<NumericEntryMessage>(),
            Some(NumericEntryMessage::DraftChanged {
                target: NumericEntryTarget::Mix,
                draft: "2".to_string(),
                dirty: true,
            })
        );

        widget.text = "2".to_string();
        widget.dirty = true;
        let output = widget
            .handle_input(bounds, WidgetInput::Character('5'))
            .expect("second typed character should append");
        assert_eq!(
            output.typed_cloned::<NumericEntryMessage>(),
            Some(NumericEntryMessage::DraftChanged {
                target: NumericEntryTarget::Mix,
                draft: "25".to_string(),
                dirty: true,
            })
        );

        widget.text = "25".to_string();
        let output = widget
            .handle_input(bounds, WidgetInput::KeyPress(WidgetKey::Enter))
            .expect("enter should commit");
        assert_eq!(
            output.typed_cloned::<NumericEntryMessage>(),
            Some(NumericEntryMessage::Commit {
                target: NumericEntryTarget::Mix,
                draft: "25".to_string(),
            })
        );

        let output = widget
            .handle_input(bounds, WidgetInput::FocusChanged(false))
            .expect("focus loss should cancel active edit");
        assert_eq!(
            output.typed_cloned::<NumericEntryMessage>(),
            Some(NumericEntryMessage::Cancel {
                target: NumericEntryTarget::Mix,
            })
        );
    }

    #[test]
    fn radiant_editor_curve_drag_updates_editable_curve() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));

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
    fn radiant_editor_curve_delete_removes_interior_node() {
        let params = Arc::new(PumpParams::new());
        let before = params.editable_curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));
        state.active_curve_node = Some(1);
        state.hover_curve_node = Some(1);
        state.preview_curve_node = Some(CurveNode { x: 0.2, y: 0.3 });
        state.hover_curve_segment = Some(0);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DeleteNode { index: 1 }),
        );

        let after = params.editable_curve_snapshot();
        assert_eq!(after.nodes.len() + 1, before.nodes.len());
        assert_eq!(after.segments.len(), after.nodes.len() - 1);
        assert_eq!(state.active_curve_node, None);
        assert!(state.active_curve_segment.is_none());
        assert_eq!(state.hover_curve_node, None);
        assert_eq!(state.preview_curve_node, None);
        assert_eq!(state.hover_curve_segment, None);
    }

    #[test]
    fn radiant_editor_curve_delete_ignores_endpoints() {
        let params = Arc::new(PumpParams::new());
        let before = params.editable_curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));
        state.hover_curve_node = Some(0);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DeleteNode { index: 0 }),
        );

        assert_eq!(params.editable_curve_snapshot().nodes, before.nodes);
        assert_eq!(params.editable_curve_snapshot().segments, before.segments);
    }

    #[test]
    fn radiant_editor_curve_insert_adds_preview_node_to_params() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));
        state.preview_curve_node = Some(CurveNode { x: 0.2, y: 0.0 });
        state.hover_curve_segment = Some(1);
        state.option_hover_held = true;
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
        let mut state = editor_state(params);
        state.preview_curve_node = Some(CurveNode { x: 0.2, y: 0.3 });
        state.hover_curve_segment = Some(1);

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
        let mut state = editor_state(Arc::clone(&params));
        state.option_hover_held = true;
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
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
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
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
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
                node: None,
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
        let mut widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, true);
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
                node: None,
                preview_node: None,
                segment: Some(1),
            })
        );
    }

    #[test]
    fn curve_preview_widget_emits_insert_for_segment_press() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
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
        let mut widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, true);
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
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false);
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
        let mut state = editor_state(Arc::clone(&params));

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
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
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
    fn curve_preview_widget_emits_hover_for_hit_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);

        let output = widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .expect("node hover should emit hover state");

        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::Hover {
                node: Some(1),
                preview_node: None,
                segment: None,
            })
        );
    }

    #[test]
    fn curve_preview_widget_clears_node_hover_off_hit_target() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, Some(1), None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let node_position = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);
        let position = Point::new(
            node_position.x + CURVE_NODE_HIT_RADIUS + 2.0,
            node_position.y,
        );

        let output = widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .expect("moving off the node should clear hover state");

        assert!(matches!(
            output.typed_copied(),
            Some(CurvePreviewMessage::Hover { node: None, .. })
        ));
    }

    #[test]
    fn curve_preview_widget_emits_delete_for_double_clicked_interior_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);

        let output = widget
            .handle_input(bounds, WidgetInput::primary_double_click(position))
            .expect("interior node double-click should emit delete");

        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::DeleteNode { index: 1 })
        );
    }

    #[test]
    fn curve_preview_widget_ignores_double_clicked_endpoint() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, curve.nodes[0]);

        assert_eq!(
            widget.handle_input(bounds, WidgetInput::primary_double_click(position)),
            None
        );
    }

    #[test]
    fn curve_preview_widget_paints_preview_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let preview = CurveNode {
            x: 0.2,
            y: sample_editable_curve(&curve, 0.2),
        };
        let widget = CurvePreviewWidget::new(curve, None, None, None, Some(preview), None, false);
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
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, Some(1), true);
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
    fn curve_preview_widget_paints_hovered_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, None, None, Some(1), None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::FillRect(fill)
                    if fill.color == theme.accent_mint
                        && (fill.rect.width() - (CURVE_NODE_SIZE + 1.5)).abs() < 1.0e-6
                        && (fill.rect.height() - (CURVE_NODE_SIZE + 1.5)).abs() < 1.0e-6
            )
        }));
        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokeRect(stroke)
                    if stroke.color == theme.accent_warning
                        && (stroke.rect.width() - (CURVE_NODE_SIZE + 1.5)).abs() < 1.0e-6
                        && (stroke.width - 1.5).abs() < 1.0e-6
            )
        }));
    }

    #[test]
    fn curve_preview_widget_paints_hovered_active_node_outline() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, Some(1), None, Some(1), None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::FillRect(fill)
                    if fill.color == theme.accent_warning
                        && (fill.rect.width() - (CURVE_NODE_SIZE + 2.0)).abs() < 1.0e-6
                        && (fill.rect.height() - (CURVE_NODE_SIZE + 2.0)).abs() < 1.0e-6
            )
        }));
        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokeRect(stroke)
                    if stroke.color == theme.accent_mint
                        && (stroke.rect.width() - (CURVE_NODE_SIZE + 2.0)).abs() < 1.0e-6
                        && (stroke.width - 1.5).abs() < 1.0e-6
            )
        }));
    }

    #[test]
    fn curve_preview_widget_paints_playhead_marker_at_sampled_phase() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let phase = 0.37;
        let widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false)
            .with_playhead_phase(Some(phase));
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let expected_center = CurvePreviewWidget::curve_point(
            bounds,
            CurveNode {
                x: phase,
                y: sample_editable_curve(&curve, phase).clamp(0.0, 1.0),
            },
        );
        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::FillRect(fill)
                    if fill.color == theme.accent_warning
                        && (fill.rect.width() - CURVE_PLAYHEAD_MARKER_SIZE).abs() < 1.0e-4
                        && (fill.rect.height() - CURVE_PLAYHEAD_MARKER_SIZE).abs() < 1.0e-4
                        && ((fill.rect.min.x + fill.rect.width() * 0.5) - expected_center.x).abs()
                            < 1.0e-4
                        && ((fill.rect.min.y + fill.rect.height() * 0.5) - expected_center.y).abs()
                            < 1.0e-4
            )
        }));
    }

    #[cfg(feature = "vst3")]
    #[test]
    fn radiant_editor_reprojects_playhead_from_status_without_pointer_event() {
        let params = Arc::new(PumpParams::new());
        let status = Arc::new(GuiStatus::default());
        status.update(
            0.1,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.1,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        let mut editor = RadiantPumpEditor::new(
            Arc::clone(&params),
            Arc::clone(&status),
            Arc::new(AutomationQueue::default()),
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        );
        let first_center = playhead_marker_center(editor.paint_plan())
            .expect("initial paint plan should include a playhead marker");

        status.update(
            0.6,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.6,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        let second_center = playhead_marker_center(editor.paint_plan())
            .expect("refreshed paint plan should include a playhead marker");

        assert!(
            (second_center.x - first_center.x).abs() > 32.0,
            "playhead marker should move after status changes without relying on pointer events (first={first_center:?}, second={second_center:?})"
        );
    }

    #[cfg(feature = "vst3")]
    fn playhead_marker_center(plan: &SurfacePaintPlan) -> Option<Point> {
        let theme = ThemeTokens::default();
        plan.primitives
            .iter()
            .find_map(|primitive| match primitive {
                PaintPrimitive::FillRect(fill)
                    if fill.color == theme.accent_warning
                        && (fill.rect.width() - CURVE_PLAYHEAD_MARKER_SIZE).abs() < 1.0e-6
                        && (fill.rect.height() - CURVE_PLAYHEAD_MARKER_SIZE).abs() < 1.0e-6 =>
                {
                    Some(Point::new(
                        fill.rect.min.x + fill.rect.width() * 0.5,
                        fill.rect.min.y + fill.rect.height() * 0.5,
                    ))
                }
                _ => None,
            })
    }

    #[test]
    fn radiant_editor_surface_emits_visible_paint() {
        let frame = radiant_editor_frame_for_params(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Vector2::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
        );
        let version_label = build_version_label();

        assert!(frame.paint_plan.primitives.iter().any(|primitive| {
            matches!(primitive, PaintPrimitive::Text(text) if text.text.as_str() == version_label)
        }));
        assert!(!frame.paint_plan.primitives.iter().any(|primitive| {
            matches!(primitive, PaintPrimitive::Text(text) if text.text.eq_ignore_ascii_case("pump"))
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
