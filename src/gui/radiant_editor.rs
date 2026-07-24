//! Shared Radiant surface for Pump editor hosts.

use std::sync::Arc;

use radiant::gui::types::{Point, Rect, Rgba8, Vector2};
use radiant::gui::visualization::{
    push_sampled_curve_area_fill, SampledCurveAreaBaseline, SampledCurveAreaFillParts,
};
use radiant::layout::LayoutOutput;
use radiant::prelude::{
    column, custom_widget, custom_widget_mapped, row, slider, text, IntoView, TextAlign, ViewNode,
};
#[cfg(test)]
use radiant::runtime::SurfaceFrame;
use radiant::runtime::{
    DeclarativeSurfaceRuntime, PaintBrush, PaintFillRect, PaintLinearGradient, PaintPrimitive,
    PaintStrokePolyline, PaintStrokeRect, PaintText, PaintTextAlign, PaintTextRun, UiSurface,
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
    cyclically_offset_editable_curve, sample_editable_curve, CurveNode, CurveSegment,
    EditableCurve, MAX_EDITABLE_NODES, MAX_SEGMENT_TENSION, MIN_SEGMENT_TENSION,
};
use crate::incoming_waveform::IncomingWaveformSnapshot;
use crate::params::{
    format_plain_value_text, parse_plain_value_text, sync_division_label, PumpParams,
    GLOBAL_CURVE_SLOT_COUNT, MAX_DEPTH_DB, MAX_FLOOR_DB, MAX_OUTPUT_GAIN_DB, MAX_SYNC_DIVISION,
    MIN_DEPTH_DB, MIN_FLOOR_DB, MIN_OUTPUT_GAIN_DB, PARAM_DEPTH_ID, PARAM_FLOOR_ID, PARAM_MIX_ID,
    PARAM_OUTPUT_GAIN_ID, PARAM_PHASE_OFFSET_ID, PARAM_SMOOTH_ID, TRIGGER_MODE_LABELS,
    TRIGGER_MODE_SIDECHAIN,
};
use crate::GuiStatus;

use super::{build_version_label, snap_curve_time_to_beat_grid, WINDOW_HEIGHT, WINDOW_WIDTH};

const BUILD_LABEL_HEIGHT: f32 = 16.0;
const CURVE_PREVIEW_HEIGHT: f32 = 68.0;
const GAIN_REDUCTION_METER_WIDTH: f32 = 34.0;
const GAIN_REDUCTION_METER_BAR_WIDTH: f32 = 8.0;
const CURVE_SLOT_ROW_HEIGHT: f32 = 22.0;
const CURVE_SLOT_SPACING: f32 = 4.0;
const CURVE_SLOT_VISIBLE_COUNT: usize = 6;
const CURVE_SLOT_NAV_WIDTH: f32 = 20.0;
const CURVE_SLOT_CORAL: Rgba8 = Rgba8::new(255, 128, 128, 255);
const CURVE_SLOT_CORAL_SOFT: Rgba8 = Rgba8::new(255, 128, 128, 96);
const CURVE_SLOT_WIDTH: f32 = ((WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0)
    - CURVE_SLOT_NAV_WIDTH
    - CURVE_SLOT_SPACING * CURVE_SLOT_VISIBLE_COUNT as f32)
    / CURVE_SLOT_VISIBLE_COUNT as f32;
const CONTROL_ROW_HEIGHT: f32 = 22.0;
const CONTROL_LABEL_WIDTH: f32 = 54.0;
const CONTROL_VALUE_WIDTH: f32 = 60.0;
const SURFACE_PADDING: f32 = 12.0;
const SURFACE_SPACING: f32 = 6.0;
const CURVE_SAMPLE_COUNT: usize = 96;
const CURVE_FILL_TOP_ALPHA: u8 = 96;
const CURVE_FILL_BOTTOM_ALPHA: u8 = 12;
const CURVE_NODE_SIZE: f32 = 5.0;
const CURVE_PREVIEW_NODE_SIZE: f32 = 7.0;
const CURVE_NODE_HIT_RADIUS: f32 = 10.0;
const CURVE_NODE_INSERT_GUARD_RADIUS: f32 = 12.0;
const CURVE_SEGMENT_HOVER_RADIUS: f32 = 7.0;
const CURVE_SEGMENT_TENSION_PIXEL_SCALE: f32 = 120.0;
const CURVE_NODE_PUSH_THROUGH_MARGIN_PX: f32 = 10.0;
const CURVE_NODE_MIN_SPACING_X: f32 = 1.0e-3;
const CURVE_PLAYHEAD_MARKER_SIZE: f32 = 5.5;
const CURVE_PLAYHEAD_MARKER_GLOW_SIZE: f32 = 9.5;
const CURVE_PLAYHEAD_GLOW_COLOR: Rgba8 = Rgba8::new(255, 96, 208, 112);
const CURVE_PLAYHEAD_CORE_COLOR: Rgba8 = Rgba8::new(255, 96, 208, 255);
const CURVE_PLAYHEAD_STROKE_COLOR: Rgba8 = Rgba8::new(255, 196, 232, 255);
const CURVE_SEGMENT_MOVE_COLOR: Rgba8 = Rgba8::new(96, 176, 255, 255);
const CURVE_REFERENCE_GUTTER_WIDTH: f32 = 52.0;
const CURVE_REFERENCE_LABEL_HEIGHT: f32 = 12.0;
const CURVE_REFERENCE_FONT_SIZE: f32 = 9.0;
const CURVE_SLOT_PREVIEW_STEPS: usize = 24;
const CURVE_SLOT_MARGIN: f32 = 3.0;
const VALUE_ENTRY_MAX_CHARS: usize = 16;
const VALUE_LABEL_FONT_SIZE: f32 = 12.0;

fn curve_reference_gutter_width(preview_width: f32) -> f32 {
    CURVE_REFERENCE_GUTTER_WIDTH.min((preview_width - 1.0).max(0.0))
}

fn curve_viewport_width(preview_width: f32) -> f32 {
    (preview_width - curve_reference_gutter_width(preview_width)).max(1.0)
}

fn curve_node_push_through_threshold_x(preview_width: f32) -> f32 {
    CURVE_NODE_PUSH_THROUGH_MARGIN_PX
        / (curve_viewport_width(preview_width).max(1.0) - 1.0).max(1.0)
}

fn curve_width_from_push_through_threshold_x(threshold_x: f32) -> f32 {
    if threshold_x > f32::EPSILON {
        CURVE_NODE_PUSH_THROUGH_MARGIN_PX / threshold_x + 1.0
    } else {
        1.0
    }
}

#[derive(Clone)]
struct ActiveCurveNodeDrag {
    origin_index: usize,
    origin_curve: EditableCurve,
    horizontal_gain_anchor: Option<f32>,
    vertical_time_anchor: Option<f32>,
    last_pointer_x: f32,
    last_pointer_y: f32,
    unconstrained_x_offset: f32,
    unconstrained_y_offset: f32,
    suppress_command_snap_once: bool,
}

#[derive(Clone, Copy)]
struct CurvePointDragModifiers {
    shift_held: bool,
    option_held: bool,
    command_held: bool,
}

impl ActiveCurveNodeDrag {
    fn set_constraints(&mut self, shift_held: bool, option_held: bool, current: CurveNode) {
        let vertical_active = shift_held && option_held;
        let horizontal_active = shift_held && !vertical_active;
        match (self.vertical_time_anchor, vertical_active) {
            (None, true) => {
                self.vertical_time_anchor = Some(current.x.clamp(0.0, 1.0));
            }
            (Some(anchor), false) => {
                self.unconstrained_x_offset = anchor - self.last_pointer_x;
                self.vertical_time_anchor = None;
                self.suppress_command_snap_once = true;
            }
            _ => {}
        }
        match (self.horizontal_gain_anchor, horizontal_active) {
            (None, true) => {
                self.horizontal_gain_anchor = Some(current.y.clamp(0.0, 1.0));
            }
            (Some(anchor), false) => {
                self.unconstrained_y_offset = anchor - self.last_pointer_y;
                self.horizontal_gain_anchor = None;
            }
            _ => {}
        }
    }

    fn target_for_pointer(
        &mut self,
        target: CurveNode,
        modifiers: CurvePointDragModifiers,
        sync_division: usize,
        curve_width: f32,
        current: CurveNode,
    ) -> CurveNode {
        self.last_pointer_x = target.x;
        self.last_pointer_y = target.y;
        self.set_constraints(modifiers.shift_held, modifiers.option_held, current);
        let suppress_command_snap = self.suppress_command_snap_once;
        self.suppress_command_snap_once = false;
        let mut effective = CurveNode {
            x: self
                .vertical_time_anchor
                .unwrap_or(target.x + self.unconstrained_x_offset)
                .clamp(0.0, 1.0),
            y: self
                .horizontal_gain_anchor
                .unwrap_or(target.y + self.unconstrained_y_offset)
                .clamp(0.0, 1.0),
        };
        if modifiers.command_held && self.vertical_time_anchor.is_none() && !suppress_command_snap {
            effective.x = snap_curve_time_to_beat_grid(sync_division, curve_width, effective.x);
        }
        effective
    }
}

#[derive(Clone)]
struct ActiveCurveSegmentDrag {
    index: usize,
    origin_curve: EditableCurve,
    start_pointer: Point,
    mode: CurveSegmentDragMode,
}

#[derive(Clone)]
struct ActiveCurveOffsetDrag {
    origin_curve: EditableCurve,
    start_pointer_x: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CurveSegmentDragMode {
    AdjustTension { start_tension: f32 },
    MovePair,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NumericEntryTarget {
    Mix,
    Depth,
    Floor,
    Phase,
    OutputGain,
    Smooth,
}

impl NumericEntryTarget {
    fn param_id(self) -> ClapId {
        match self {
            Self::Mix => PARAM_MIX_ID,
            Self::Depth => PARAM_DEPTH_ID,
            Self::Floor => PARAM_FLOOR_ID,
            Self::Phase => PARAM_PHASE_OFFSET_ID,
            Self::OutputGain => PARAM_OUTPUT_GAIN_ID,
            Self::Smooth => PARAM_SMOOTH_ID,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Mix => "Mix",
            Self::Depth => "Depth",
            Self::Floor => "Floor",
            Self::Phase => "Phase",
            Self::OutputGain => "Output",
            Self::Smooth => "Smooth",
        }
    }

    fn widget_key(self) -> &'static str {
        match self {
            Self::Mix => "numeric-entry-mix",
            Self::Depth => "numeric-entry-depth",
            Self::Floor => "numeric-entry-floor",
            Self::Phase => "numeric-entry-phase",
            Self::OutputGain => "numeric-entry-output",
            Self::Smooth => "numeric-entry-smooth",
        }
    }

    fn current_plain_value(self, params: &PumpParams) -> f64 {
        match self {
            Self::Mix => params.mix() as f64,
            Self::Depth => params.depth_db() as f64,
            Self::Floor => params.floor_db() as f64,
            Self::Phase => params.phase_offset() as f64,
            Self::OutputGain => params.output_gain_db() as f64,
            Self::Smooth => params.smooth() as f64,
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
    active_curve_node_drag: Option<ActiveCurveNodeDrag>,
    active_curve_segment: Option<ActiveCurveSegmentDrag>,
    active_curve_offset: Option<ActiveCurveOffsetDrag>,
    preview_curve_offset: Option<EditableCurve>,
    hover_curve_node: Option<usize>,
    preview_curve_node: Option<CurveNode>,
    hover_curve_segment: Option<usize>,
    option_hover_held: bool,
    command_hover_held: bool,
    shift_hover_held: bool,
    loaded_global_curve_slot: Option<usize>,
    curve_slot_scroll_offset: usize,
    numeric_entry: Option<NumericEntryState>,
}

#[derive(Clone, Debug, PartialEq)]
enum RadiantEditorMessage {
    Mix(f32),
    Depth(f32),
    Floor(f32),
    Phase(f32),
    OutputGain(f32),
    Smooth(f32),
    SyncDivision(f32),
    TriggerMode(f32),
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
        self.status.has_host_beats_timeline()
            || self.status.is_playing()
            || self.status.gain_reduction_needs_redraw()
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

#[cfg(all(feature = "vst3", target_os = "macos"))]
impl toybox::vst3::gui::RadiantVst3Editor for RadiantPumpEditor {
    fn resize(&mut self, width: u32, height: u32) {
        Self::resize(self, width, height);
    }

    fn dispatch_event(&mut self, event: Event) {
        Self::dispatch_event(self, event);
    }

    fn paint_plan(&mut self) -> &SurfacePaintPlan {
        Self::paint_plan(self)
    }

    fn needs_realtime_redraw(&self) -> bool {
        Self::needs_realtime_redraw(self)
    }

    fn dispatch_key_press(&mut self, key: WidgetKey) -> bool {
        Self::dispatch_key_press(self, key)
    }

    fn dispatch_character(&mut self, character: char) -> bool {
        Self::dispatch_character(self, character)
    }

    fn cancel_text_entry(&mut self) -> bool {
        Self::cancel_numeric_entry(self)
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
            active_curve_node_drag: None,
            active_curve_segment: None,
            active_curve_offset: None,
            preview_curve_offset: None,
            hover_curve_node: None,
            preview_curve_node: None,
            hover_curve_segment: None,
            option_hover_held: false,
            command_hover_held: false,
            shift_hover_held: false,
            loaded_global_curve_slot: None,
            curve_slot_scroll_offset: 0,
            numeric_entry: None,
        }
    }
}

fn project_editor_surface(state: &mut RadiantEditorState) -> Arc<UiSurface<RadiantEditorMessage>> {
    let params = state.params.as_ref();
    let curve = state
        .preview_curve_offset
        .clone()
        .unwrap_or_else(|| params.editable_curve_snapshot());
    let output = params.output_gain_db();
    let smooth = params.smooth();
    let depth = params.depth_db();
    let floor = params.floor_db();
    let sync = params.sync_division();
    let playhead_phase = (state.status.has_host_beats_timeline() || state.status.is_playing())
        .then_some(state.status.phase());
    Arc::new(
        column([
            text(if params.preset_persistence_warning().is_some() {
                super::PRESET_WARNING_STORAGE.to_string()
            } else {
                build_version_label()
            })
            .muted_text()
            .align_text(TextAlign::Right)
            .height(BUILD_LABEL_HEIGHT)
            .fill_width(),
            row([
                custom_widget_mapped(
                    CurvePreviewWidget::new(
                        curve,
                        state.active_curve_node,
                        state.active_curve_segment.as_ref().map(|drag| drag.index),
                        state.hover_curve_node,
                        state.preview_curve_node,
                        state.hover_curve_segment,
                        state.option_hover_held,
                    )
                    .with_command_hover_held(state.command_hover_held)
                    .with_shift_hover_held(state.shift_hover_held)
                    .with_active_segment_move(
                        state
                            .active_curve_segment
                            .as_ref()
                            .is_some_and(|drag| drag.mode == CurveSegmentDragMode::MovePair),
                    )
                    .with_active_curve_offset(
                        state
                            .active_curve_offset
                            .as_ref()
                            .map(|drag| drag.start_pointer_x),
                    )
                    .with_incoming_waveform(state.status.incoming_waveform_snapshot())
                    .with_sync_division(sync)
                    .with_gain_mapping(depth, floor)
                    .with_playhead_phase(playhead_phase),
                    RadiantEditorMessage::Curve,
                )
                .fill_width()
                .height(CURVE_PREVIEW_HEIGHT),
                custom_widget(
                    GainReductionMeterWidget::new(state.status.gain_reduction_db()),
                    |_| None,
                )
                .width(GAIN_REDUCTION_METER_WIDTH)
                .height(CURVE_PREVIEW_HEIGHT),
            ])
            .spacing(4.0)
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
                NumericEntryTarget::Depth,
                format!("{depth:.0} dB"),
                normalize_depth(depth),
                state.numeric_entry.as_ref(),
                RadiantEditorMessage::Depth,
            ),
            control_row(
                NumericEntryTarget::Floor,
                if floor <= MIN_FLOOR_DB {
                    "−∞".to_string()
                } else {
                    format!("{floor:.0} dB")
                },
                normalize_floor(floor),
                state.numeric_entry.as_ref(),
                RadiantEditorMessage::Floor,
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
            control_row(
                NumericEntryTarget::Smooth,
                format!("{:.0}%", smooth * 100.0),
                smooth,
                state.numeric_entry.as_ref(),
                RadiantEditorMessage::Smooth,
            ),
            enum_control_row(
                "Sync",
                sync_division_label(sync).to_string(),
                normalize_sync_division(sync),
                RadiantEditorMessage::SyncDivision,
            ),
            enum_control_row(
                "Trigger",
                if state.status.sidechain_available() {
                    TRIGGER_MODE_LABELS[params.trigger_mode()].to_string()
                } else if params.trigger_mode() == TRIGGER_MODE_SIDECHAIN {
                    "Sidechain (unavailable)".to_string()
                } else {
                    TRIGGER_MODE_LABELS[params.trigger_mode()].to_string()
                },
                params.trigger_mode() as f32,
                RadiantEditorMessage::TriggerMode,
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
        RadiantEditorMessage::Depth(value) => state.params.set_depth_db(denormalize_depth(value)),
        RadiantEditorMessage::Floor(value) => state.params.set_floor_db(denormalize_floor(value)),
        RadiantEditorMessage::Phase(value) => state.params.set_phase_offset(value),
        RadiantEditorMessage::OutputGain(value) => {
            state
                .params
                .set_output_gain_db(denormalize_output_gain(value));
        }
        RadiantEditorMessage::Smooth(value) => state.params.set_smooth(value),
        RadiantEditorMessage::SyncDivision(value) => {
            state
                .params
                .set_sync_division((value.clamp(0.0, 1.0) * MAX_SYNC_DIVISION).round());
        }
        RadiantEditorMessage::TriggerMode(value) => {
            let mode = value.round().clamp(0.0, TRIGGER_MODE_SIDECHAIN as f32) as usize;
            if mode == TRIGGER_MODE_SIDECHAIN && !state.status.sidechain_available() {
                return;
            }
            state.params.set_trigger_mode(mode as f32);
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
    let max_offset = curve_slot_scroll_max();
    let offset = state.curve_slot_scroll_offset.min(max_offset);
    let visible_end = (offset + CURVE_SLOT_VISIBLE_COUNT).min(GLOBAL_CURVE_SLOT_COUNT);
    let mut slot_nodes: Vec<ViewNode<RadiantEditorMessage>> = (offset..visible_end)
        .map(|index| {
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
            .width(CURVE_SLOT_WIDTH)
            .height(CURVE_SLOT_ROW_HEIGHT)
        })
        .collect();
    if max_offset > 0 {
        slot_nodes.push(
            custom_widget_mapped(
                CurveSlotNavigationWidget::new(if offset == 0 { 1 } else { -1 }),
                RadiantEditorMessage::CurveSlot,
            )
            .width(CURVE_SLOT_NAV_WIDTH)
            .height(CURVE_SLOT_ROW_HEIGHT),
        );
    }
    row(slot_nodes)
        .spacing(CURVE_SLOT_SPACING)
        .fill_width()
        .height(CURVE_SLOT_ROW_HEIGHT)
}

fn curve_slot_scroll_max() -> usize {
    GLOBAL_CURVE_SLOT_COUNT.saturating_sub(CURVE_SLOT_VISIBLE_COUNT)
}

fn ensure_curve_slot_visible(state: &mut RadiantEditorState, index: usize) {
    if index < state.curve_slot_scroll_offset {
        state.curve_slot_scroll_offset = index;
    } else if index >= state.curve_slot_scroll_offset + CURVE_SLOT_VISIBLE_COUNT {
        state.curve_slot_scroll_offset = index + 1 - CURVE_SLOT_VISIBLE_COUNT;
    }
    state.curve_slot_scroll_offset = state.curve_slot_scroll_offset.min(curve_slot_scroll_max());
}

fn reduce_curve_slot_message(state: &mut RadiantEditorState, message: CurveSlotMessage) {
    match message {
        CurveSlotMessage::Navigate { delta } => {
            let max_offset = curve_slot_scroll_max();
            state.curve_slot_scroll_offset =
                (state.curve_slot_scroll_offset as i8 + delta).clamp(0, max_offset as i8) as usize;
        }
        CurveSlotMessage::Load { index } => {
            let Some(curve) = state.params.global_curve_slot_curve(index) else {
                return;
            };
            state.params.set_editable_curve_preserving_phase(&curve);
            state.active_curve_node = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            state.loaded_global_curve_slot = Some(index);
            ensure_curve_slot_visible(state, index);
        }
        CurveSlotMessage::Store { index } => {
            let curve = state.params.editable_curve_snapshot();
            if state.params.set_global_curve_slot_curve(index, &curve) {
                state.loaded_global_curve_slot = Some(index);
                ensure_curve_slot_visible(state, index);
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
        NumericEntryTarget::Depth => state.params.set_depth_db(value as f32),
        NumericEntryTarget::Floor => state.params.set_floor_db(value as f32),
        NumericEntryTarget::Phase => state.params.set_phase_offset(value as f32),
        NumericEntryTarget::OutputGain => state.params.set_output_gain_db(value as f32),
        NumericEntryTarget::Smooth => state.params.set_smooth(value as f32),
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
        CurvePreviewMessage::ModifiersChanged {
            option_held,
            command_held,
            shift_held,
        } => {
            let command_released = state.command_hover_held && !command_held;
            let constraint_changed =
                state.shift_hover_held != shift_held || state.option_hover_held != option_held;
            state.option_hover_held = option_held;
            state.command_hover_held = command_held;
            state.shift_hover_held = shift_held;
            if constraint_changed {
                let curve = state.params.editable_curve_snapshot();
                let active_index = state.active_curve_node;
                if let Some(drag) = state.active_curve_node_drag.as_mut() {
                    let current = active_index
                        .and_then(|index| curve.nodes.get(index))
                        .copied()
                        .unwrap_or_else(|| drag.origin_curve.nodes[drag.origin_index]);
                    drag.set_constraints(shift_held, option_held, current);
                }
            }
            if option_held || command_held {
                state.preview_curve_node = None;
            }
            if command_released {
                if state
                    .active_curve_segment
                    .as_ref()
                    .is_some_and(|drag| drag.mode == CurveSegmentDragMode::MovePair)
                {
                    state.active_curve_segment = None;
                }
                state.hover_curve_segment = None;
            }
        }
        CurvePreviewMessage::PressNode {
            index,
            pointer,
            shift_held,
            option_held,
            command_held,
        } => {
            let curve = state.params.editable_curve_snapshot();
            if let Some(drag) =
                start_curve_node_drag(&curve, index, pointer, shift_held, option_held)
            {
                state.option_hover_held = option_held;
                state.command_hover_held = command_held;
                state.active_curve_node = Some(index);
                state.active_curve_node_drag = Some(drag);
                state.active_curve_segment = None;
                state.active_curve_offset = None;
                state.preview_curve_offset = None;
                state.shift_hover_held = shift_held;
                state.hover_curve_node = Some(index);
                state.preview_curve_node = None;
                state.hover_curve_segment = None;
            }
        }
        CurvePreviewMessage::PressCurveOffset { pointer_x } => {
            let origin_curve = state.params.editable_curve_snapshot();
            state.active_curve_offset = Some(ActiveCurveOffsetDrag {
                origin_curve: origin_curve.clone(),
                start_pointer_x: pointer_x,
            });
            state.preview_curve_offset = Some(origin_curve);
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            state.command_hover_held = true;
            state.shift_hover_held = true;
        }
        CurvePreviewMessage::InsertNode { node, command_held } => {
            let mut curve = state.params.editable_curve_snapshot();
            state.command_hover_held = command_held;
            state.active_curve_node_drag = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            if let Some(index) = insert_curve_node(&mut curve, node) {
                state.params.set_editable_curve(&curve);
                state.active_curve_node = Some(index);
                state.active_curve_node_drag =
                    start_curve_node_drag(&curve, index, node, false, false);
                state.hover_curve_node = Some(index);
            }
        }
        CurvePreviewMessage::DeleteNode { index } => {
            let mut curve = state.params.editable_curve_snapshot();
            if delete_curve_node(&mut curve, index) {
                state.params.set_editable_curve(&curve);
            }
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::DragNode {
            index,
            node,
            push_through_threshold_x,
        } => {
            let current_curve = state.params.editable_curve_snapshot();
            let current = state
                .active_curve_node
                .and_then(|active_index| current_curve.nodes.get(active_index))
                .copied()
                .unwrap_or(node);
            let shift_held = state.shift_hover_held;
            let option_held = state.option_hover_held;
            let command_held = state.command_hover_held;
            let (curve, moved_index) = if let Some(drag) = state.active_curve_node_drag.as_mut() {
                let target = drag.target_for_pointer(
                    node,
                    CurvePointDragModifiers {
                        shift_held,
                        option_held,
                        command_held,
                    },
                    state.params.sync_division(),
                    curve_width_from_push_through_threshold_x(push_through_threshold_x),
                    current,
                );
                curve_with_dragged_node(drag, target, push_through_threshold_x)
            } else {
                let mut curve = current_curve;
                let mut target = node;
                if command_held {
                    target.x = snap_curve_time_to_beat_grid(
                        state.params.sync_division(),
                        curve_width_from_push_through_threshold_x(push_through_threshold_x),
                        target.x,
                    );
                }
                update_curve_node(&mut curve, index, target);
                (curve, index)
            };
            state.params.set_editable_curve(&curve);
            state.active_curve_node = Some(moved_index);
            state.active_curve_segment = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = Some(moved_index);
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::DragCurveOffset { delta } => {
            let Some(drag) = state.active_curve_offset.as_ref() else {
                return;
            };
            let curve = cyclically_offset_editable_curve(&drag.origin_curve, delta);
            state.preview_curve_offset = Some(curve);
            state.active_curve_node = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::ReleaseCurveOffset { delta } => {
            if let Some(drag) = state.active_curve_offset.take() {
                let curve = cyclically_offset_editable_curve(&drag.origin_curve, delta);
                state.params.set_editable_curve_preserving_phase(&curve);
            }
            state.preview_curve_offset = None;
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::ReleaseNode {
            index,
            node,
            push_through_threshold_x,
            shift_held,
            option_held,
            command_held,
        } => {
            let current_curve = state.params.editable_curve_snapshot();
            let current = state
                .active_curve_node
                .and_then(|active_index| current_curve.nodes.get(active_index))
                .copied()
                .unwrap_or(node);
            let (curve, moved_index) = if let Some(drag) = state.active_curve_node_drag.as_mut() {
                let target = drag.target_for_pointer(
                    node,
                    CurvePointDragModifiers {
                        shift_held,
                        option_held,
                        command_held,
                    },
                    state.params.sync_division(),
                    curve_width_from_push_through_threshold_x(push_through_threshold_x),
                    current,
                );
                curve_with_dragged_node(drag, target, push_through_threshold_x)
            } else {
                let mut curve = current_curve;
                let mut target = node;
                if command_held {
                    target.x = snap_curve_time_to_beat_grid(
                        state.params.sync_division(),
                        curve_width_from_push_through_threshold_x(push_through_threshold_x),
                        target.x,
                    );
                }
                update_curve_node(&mut curve, index, target);
                (curve, index)
            };
            state.params.set_editable_curve(&curve);
            state.option_hover_held = option_held;
            state.command_hover_held = command_held;
            state.shift_hover_held = shift_held;
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_segment = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = Some(moved_index);
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::PressSegment { index, position } => {
            let curve = state.params.editable_curve_snapshot();
            if let Some(drag) = start_curve_segment_tension_drag(&curve, index, position) {
                state.active_curve_node = None;
                state.active_curve_node_drag = None;
                state.active_curve_offset = None;
                state.preview_curve_offset = None;
                state.active_curve_segment = Some(drag);
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = Some(index);
            }
        }
        CurvePreviewMessage::PressSegmentMove { index, position } => {
            let curve = state.params.editable_curve_snapshot();
            if let Some(drag) = start_curve_segment_move_drag(&curve, index, position) {
                state.active_curve_node = None;
                state.active_curve_node_drag = None;
                state.active_curve_offset = None;
                state.preview_curve_offset = None;
                state.active_curve_segment = Some(drag);
                state.command_hover_held = true;
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = Some(index);
            }
        }
        CurvePreviewMessage::DragSegment {
            index: _,
            position,
            curve_size,
        } => {
            if let Some(drag) = state.active_curve_segment.as_ref() {
                let curve = curve_with_dragged_segment(drag, position, curve_size);
                state.params.set_editable_curve(&curve);
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = Some(drag.index);
            }
        }
        CurvePreviewMessage::ReleaseSegment {
            index: _,
            position,
            curve_size,
        } => {
            if let Some(drag) = state.active_curve_segment.take() {
                let curve = curve_with_dragged_segment(&drag, position, curve_size);
                state.params.set_editable_curve(&curve);
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = match drag.mode {
                    CurveSegmentDragMode::MovePair => {
                        state.command_hover_held.then_some(drag.index)
                    }
                    CurveSegmentDragMode::AdjustTension { .. } => {
                        state.option_hover_held.then_some(drag.index)
                    }
                };
            }
        }
        CurvePreviewMessage::Cancel => {
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            state.option_hover_held = false;
            state.command_hover_held = false;
            state.shift_hover_held = false;
        }
    }
}

fn start_curve_node_drag(
    curve: &EditableCurve,
    index: usize,
    pointer: CurveNode,
    shift_held: bool,
    option_held: bool,
) -> Option<ActiveCurveNodeDrag> {
    let normalized = curve.clone().normalized();
    let origin = *normalized.nodes.get(index)?;
    let vertical_active = shift_held && option_held;
    let horizontal_active = shift_held && !vertical_active;
    Some(ActiveCurveNodeDrag {
        origin_index: index,
        origin_curve: normalized,
        horizontal_gain_anchor: horizontal_active.then_some(origin.y),
        vertical_time_anchor: vertical_active.then_some(origin.x),
        last_pointer_x: pointer.x.clamp(0.0, 1.0),
        last_pointer_y: pointer.y.clamp(0.0, 1.0),
        unconstrained_x_offset: 0.0,
        unconstrained_y_offset: 0.0,
        suppress_command_snap_once: false,
    })
}

fn curve_with_dragged_node(
    drag: &ActiveCurveNodeDrag,
    target: CurveNode,
    push_through_threshold_x: f32,
) -> (EditableCurve, usize) {
    let mut curve = drag.origin_curve.clone();
    let moved_index = move_curve_node_with_push_through(
        &mut curve,
        drag.origin_index,
        target,
        push_through_threshold_x,
    );
    curve.normalize_in_place();
    (curve, moved_index)
}

fn move_curve_node_with_push_through(
    curve: &mut EditableCurve,
    index: usize,
    target: CurveNode,
    push_through_threshold_x: f32,
) -> usize {
    if index >= curve.nodes.len() {
        return index;
    }

    let y = target.y.clamp(0.0, 1.0);
    let last_index = curve.nodes.len().saturating_sub(1);
    if index == 0 {
        set_wrapped_curve_endpoint_y(curve, y);
        return 0;
    }
    if index == last_index {
        set_wrapped_curve_endpoint_y(curve, y);
        return curve.nodes.len().saturating_sub(1);
    }

    let mut moved_index = index;
    let threshold_x = push_through_threshold_x.max(0.0);
    while moved_index + 1 < curve.nodes.len().saturating_sub(1)
        && target.x > curve.nodes[moved_index + 1].x + threshold_x
    {
        remove_interior_curve_node(curve, moved_index + 1);
    }
    while moved_index > 1 && target.x < curve.nodes[moved_index - 1].x - threshold_x {
        remove_interior_curve_node(curve, moved_index - 1);
        moved_index = moved_index.saturating_sub(1);
    }

    let min_x = curve.nodes[moved_index - 1].x + CURVE_NODE_MIN_SPACING_X;
    let max_x = curve.nodes[moved_index + 1].x - CURVE_NODE_MIN_SPACING_X;
    curve.nodes[moved_index] = CurveNode {
        x: target.x.clamp(min_x, max_x),
        y,
    };
    enforce_wrapped_curve_endpoints(curve);
    moved_index
}

fn remove_interior_curve_node(curve: &mut EditableCurve, remove_index: usize) {
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

fn set_wrapped_curve_endpoint_y(curve: &mut EditableCurve, y: f32) {
    if curve.nodes.len() < 2 {
        return;
    }
    let clamped = y.clamp(0.0, 1.0);
    let last_index = curve.nodes.len() - 1;
    curve.nodes[0] = CurveNode { x: 0.0, y: clamped };
    curve.nodes[last_index] = CurveNode { x: 1.0, y: clamped };
}

fn enforce_wrapped_curve_endpoints(curve: &mut EditableCurve) {
    if curve.nodes.len() < 2 {
        return;
    }
    set_wrapped_curve_endpoint_y(curve, curve.nodes[0].y);
}

fn start_curve_segment_tension_drag(
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
        mode: CurveSegmentDragMode::AdjustTension { start_tension },
    })
}

fn start_curve_segment_move_drag(
    curve: &EditableCurve,
    index: usize,
    start_pointer: Point,
) -> Option<ActiveCurveSegmentDrag> {
    let normalized = curve.clone().normalized();
    (index + 1 < normalized.nodes.len()).then_some(ActiveCurveSegmentDrag {
        index,
        origin_curve: normalized,
        start_pointer,
        mode: CurveSegmentDragMode::MovePair,
    })
}

fn curve_with_dragged_segment(
    drag: &ActiveCurveSegmentDrag,
    current_pointer: Point,
    curve_size: Vector2,
) -> EditableCurve {
    let mut curve = drag.origin_curve.clone();
    match drag.mode {
        CurveSegmentDragMode::AdjustTension { start_tension } => {
            let delta = segment_tension_delta_from_drag(
                &drag.origin_curve,
                drag.index,
                drag.start_pointer,
                current_pointer,
            );
            if let Some(segment) = curve.segments.get_mut(drag.index) {
                segment.tension =
                    (start_tension + delta).clamp(MIN_SEGMENT_TENSION, MAX_SEGMENT_TENSION);
            }
        }
        CurveSegmentDragMode::MovePair => {
            let curve_width = curve_size.x.max(2.0);
            let curve_height = curve_size.y.max(2.0);
            let delta = (
                (current_pointer.x - drag.start_pointer.x) / (curve_width - 1.0),
                (drag.start_pointer.y - current_pointer.y) / (curve_height - 1.0),
            );
            let left = drag.origin_curve.nodes[drag.index];
            let right = drag.origin_curve.nodes[drag.index + 1];
            super::move_segment_translated(
                &mut curve,
                drag.index,
                (left.x, left.y),
                (right.x, right.y),
                delta,
            );
        }
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

fn normalize_depth(value: f32) -> f32 {
    ((value - MIN_DEPTH_DB) / (MAX_DEPTH_DB - MIN_DEPTH_DB)).clamp(0.0, 1.0)
}

fn denormalize_depth(value: f32) -> f32 {
    MIN_DEPTH_DB + value.clamp(0.0, 1.0) * (MAX_DEPTH_DB - MIN_DEPTH_DB)
}

fn normalize_floor(value: f32) -> f32 {
    ((value - MIN_FLOOR_DB) / (MAX_FLOOR_DB - MIN_FLOOR_DB)).clamp(0.0, 1.0)
}

fn denormalize_floor(value: f32) -> f32 {
    MIN_FLOOR_DB + value.clamp(0.0, 1.0) * (MAX_FLOOR_DB - MIN_FLOOR_DB)
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
    command_hovered: bool,
}

impl CurveSlotWidget {
    fn new(index: usize, curve: Option<EditableCurve>, loaded: bool, deviated: bool) -> Self {
        Self {
            common: WidgetCommon::fixed(0, CURVE_SLOT_WIDTH.max(1.0), CURVE_SLOT_ROW_HEIGHT)
                .with_keyboard_focus()
                .without_default_chrome(),
            index,
            curve: curve.map(|curve| curve.normalized()),
            loaded,
            deviated,
            command_hovered: false,
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
            WidgetInput::PointerModifiersChanged { modifiers } => {
                self.command_hovered = modifiers.command;
                None
            }
            WidgetInput::PointerPress {
                position,
                button: PointerButton::Primary,
                modifiers,
            } if bounds.contains(position) => {
                self.common.state.focused = true;
                self.common.state.hovered = true;
                self.common.state.pressed = true;
                self.command_hovered = modifiers.command;
                if modifiers.command {
                    Some(CurveSlotMessage::Store { index: self.index })
                } else {
                    Some(CurveSlotMessage::Load { index: self.index })
                }
            }
            WidgetInput::PointerRelease { .. } | WidgetInput::PointerDrop { .. } => {
                self.common.state.pressed = false;
                None
            }
            WidgetInput::FocusChanged(focused) => {
                self.common.state.focused = focused;
                None
            }
            WidgetInput::Wheel { delta, .. } => Some(CurveSlotMessage::Navigate {
                delta: if delta.y < 0.0 { 1 } else { -1 },
            }),
            WidgetInput::KeyPress(key) if self.common.state.focused => match key {
                WidgetKey::ArrowLeft | WidgetKey::Home => {
                    Some(CurveSlotMessage::Navigate { delta: -1 })
                }
                WidgetKey::ArrowRight | WidgetKey::End => {
                    Some(CurveSlotMessage::Navigate { delta: 1 })
                }
                _ => None,
            },
            _ => None,
        }?;
        Some(WidgetOutput::typed(message))
    }

    fn accepts_wheel_input(&self) -> bool {
        true
    }

    fn synchronize_from_previous(&mut self, previous: &dyn Widget) {
        let Some(previous) = previous.as_any().downcast_ref::<Self>() else {
            return;
        };
        self.common.state.hovered = previous.common.state.hovered;
        self.common.state.pressed = previous.common.state.pressed;
        self.common.state.focused = previous.common.state.focused;
        self.command_hovered = previous.command_hovered;
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        let hovered = self.common.state.hovered;
        let pressed = self.common.state.pressed;
        let fill = if self.deviated {
            theme.accent_danger
        } else if pressed {
            CURVE_SLOT_CORAL_SOFT
        } else if self.loaded {
            theme.surface_raised
        } else if hovered {
            theme.bg_secondary
        } else {
            theme.bg_primary
        };
        let outline = if self.deviated {
            theme.accent_danger
        } else if self.loaded || hovered || pressed {
            CURVE_SLOT_CORAL
        } else {
            theme.border
        };
        let curve_color = if self.deviated || self.curve.is_some() {
            CURVE_SLOT_CORAL
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
        if self.loaded {
            primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
                widget_id: self.common.id,
                rect: bounds.inset(2.0, 2.0, 2.0, 2.0),
                color: CURVE_SLOT_CORAL,
                width: 1.0,
            }));
        }
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: self.sample_points(bounds),
            color: curve_color,
            width: if hovered || pressed || self.loaded || self.deviated {
                2.0
            } else {
                1.0
            },
        }));
        if self.command_hovered {
            let center = Point::new(bounds.max.x - 5.0, bounds.min.y + 5.0);
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.common.id,
                points: Arc::from([
                    Point::new(center.x - 2.0, center.y),
                    Point::new(center.x + 2.0, center.y),
                ]),
                color: CURVE_SLOT_CORAL,
                width: 1.0,
            }));
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.common.id,
                points: Arc::from([
                    Point::new(center.x, center.y - 2.0),
                    Point::new(center.x, center.y + 2.0),
                ]),
                color: CURVE_SLOT_CORAL,
                width: 1.0,
            }));
        }
    }

    fn automation_label(&self) -> Option<String> {
        Some(format!("Curve slot {}", self.index + 1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CurveSlotMessage {
    Load { index: usize },
    Store { index: usize },
    Navigate { delta: i8 },
}

#[derive(Clone)]
struct CurveSlotNavigationWidget {
    common: WidgetCommon,
    direction: i8,
}

impl CurveSlotNavigationWidget {
    fn new(direction: i8) -> Self {
        Self {
            common: WidgetCommon::fixed(0, CURVE_SLOT_NAV_WIDTH, CURVE_SLOT_ROW_HEIGHT)
                .with_keyboard_focus()
                .without_default_chrome(),
            direction,
        }
    }
}

impl Widget for CurveSlotNavigationWidget {
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
                ..
            } if bounds.contains(position) => {
                self.common.state.focused = true;
                self.common.state.hovered = true;
                self.common.state.pressed = true;
                Some(CurveSlotMessage::Navigate {
                    delta: self.direction * CURVE_SLOT_VISIBLE_COUNT as i8,
                })
            }
            WidgetInput::PointerRelease { .. } | WidgetInput::PointerDrop { .. } => {
                self.common.state.pressed = false;
                None
            }
            WidgetInput::Wheel { delta, .. } => Some(CurveSlotMessage::Navigate {
                delta: if delta.y < 0.0 { 1 } else { -1 },
            }),
            WidgetInput::FocusChanged(focused) => {
                self.common.state.focused = focused;
                None
            }
            WidgetInput::KeyPress(key) if self.common.state.focused => match key {
                WidgetKey::ArrowLeft | WidgetKey::Home => {
                    Some(CurveSlotMessage::Navigate { delta: -1 })
                }
                WidgetKey::ArrowRight | WidgetKey::End => {
                    Some(CurveSlotMessage::Navigate { delta: 1 })
                }
                _ => None,
            },
            _ => None,
        }?;
        Some(WidgetOutput::typed(message))
    }

    fn accepts_wheel_input(&self) -> bool {
        true
    }

    fn synchronize_from_previous(&mut self, previous: &dyn Widget) {
        let Some(previous) = previous.as_any().downcast_ref::<Self>() else {
            return;
        };
        self.common.state.hovered = previous.common.state.hovered;
        self.common.state.pressed = previous.common.state.pressed;
        self.common.state.focused = previous.common.state.focused;
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        let fill = if self.common.state.pressed {
            CURVE_SLOT_CORAL_SOFT
        } else if self.common.state.hovered || self.common.state.focused {
            theme.bg_secondary
        } else {
            theme.bg_primary
        };
        let outline = if self.common.state.hovered
            || self.common.state.focused
            || self.common.state.pressed
        {
            CURVE_SLOT_CORAL
        } else {
            theme.border
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
        let center = Point::new(
            bounds.min.x + bounds.width() * 0.5,
            bounds.min.y + bounds.height() * 0.5,
        );
        let direction = self.direction as f32;
        let base_x = center.x - direction * 3.0;
        let tip_x = center.x + direction * 4.0;
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: Arc::from([
                Point::new(base_x, center.y - 5.0),
                Point::new(tip_x, center.y),
                Point::new(base_x, center.y + 5.0),
            ]),
            color: CURVE_SLOT_CORAL,
            width: 1.5,
        }));
    }
}

#[derive(Clone)]
struct GainReductionMeterWidget {
    common: WidgetCommon,
    reduction_db: f32,
}

impl GainReductionMeterWidget {
    fn new(reduction_db: f32) -> Self {
        Self {
            common: WidgetCommon::fixed(0, GAIN_REDUCTION_METER_WIDTH, CURVE_PREVIEW_HEIGHT)
                .without_default_chrome(),
            reduction_db: reduction_db.clamp(0.0, crate::gui_status::GAIN_REDUCTION_METER_MAX_DB),
        }
    }
}

impl Widget for GainReductionMeterWidget {
    fn common(&self) -> &WidgetCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        &mut self.common
    }

    fn handle_input(&mut self, _bounds: Rect, _input: WidgetInput) -> Option<WidgetOutput> {
        None
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        let title_height = 14.0;
        let value_height = 14.0;
        let bar_top = bounds.min.y + title_height;
        let bar_height = (bounds.height() - title_height - value_height).max(1.0);
        let bar_left = bounds.min.x + (bounds.width() - GAIN_REDUCTION_METER_BAR_WIDTH) * 0.5;
        let bar = Rect::from_xy_size(
            bar_left,
            bar_top,
            GAIN_REDUCTION_METER_BAR_WIDTH,
            bar_height,
        );
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.common.id,
            rect: bar,
            color: theme.border_emphasis,
            width: 1.0,
        }));
        let fraction = crate::gui_status::gain_reduction_meter_fraction(self.reduction_db);
        let fill_height = (bar.height() - 2.0).max(0.0) * fraction;
        if fill_height > 0.0 {
            primitives.push(PaintPrimitive::FillRect(PaintFillRect {
                widget_id: self.common.id,
                rect: Rect::from_xy_size(
                    bar.min.x + 1.0,
                    bar.min.y + 1.0,
                    (bar.width() - 2.0).max(0.0),
                    fill_height,
                ),
                color: theme.accent_warning,
            }));
        }
        for step in 1..3 {
            let y = bar.min.y + bar.height() * step as f32 / 3.0;
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.common.id,
                points: Arc::from([
                    Point::new(bar.min.x - 2.0, y),
                    Point::new(bar.max.x + 2.0, y),
                ]),
                color: theme.border_emphasis,
                width: 1.0,
            }));
        }
        for (text_value, rect) in [
            (
                "GR dB".to_string(),
                Rect::from_xy_size(bounds.min.x, bounds.min.y, bounds.width(), title_height),
            ),
            (
                format!("{:.1}", self.reduction_db),
                Rect::from_xy_size(
                    bounds.min.x,
                    bounds.max.y - value_height,
                    bounds.width(),
                    value_height,
                ),
            ),
        ] {
            primitives.push(PaintPrimitive::Text(PaintTextRun {
                widget_id: self.common.id,
                text: PaintText::from(text_value),
                rect,
                font_size: 9.0,
                baseline: None,
                color: theme.text_muted,
                align: PaintTextAlign::Center,
                wrap: TextWrap::None,
            }));
        }
    }
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
    command_hover_held: bool,
    shift_hover_held: bool,
    active_segment_move: bool,
    active_curve_offset_start_x: Option<f32>,
    playhead_phase: Option<f32>,
    incoming_waveform: Option<IncomingWaveformSnapshot>,
    sync_division: usize,
    depth_db: f32,
    floor_db: f32,
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
            command_hover_held: false,
            shift_hover_held: false,
            active_segment_move: false,
            active_curve_offset_start_x: None,
            playhead_phase: None,
            incoming_waveform: None,
            sync_division: crate::params::DEFAULT_SYNC_DIVISION_INDEX,
            depth_db: crate::params::DEFAULT_DEPTH_DB,
            floor_db: crate::params::DEFAULT_FLOOR_DB,
        }
    }

    fn with_playhead_phase(mut self, playhead_phase: Option<f32>) -> Self {
        self.playhead_phase = playhead_phase.map(|phase| phase.rem_euclid(1.0));
        self
    }

    fn with_command_hover_held(mut self, command_hover_held: bool) -> Self {
        self.command_hover_held = command_hover_held;
        self
    }

    fn with_shift_hover_held(mut self, shift_hover_held: bool) -> Self {
        self.shift_hover_held = shift_hover_held;
        self
    }

    fn with_active_segment_move(mut self, active_segment_move: bool) -> Self {
        self.active_segment_move = active_segment_move;
        self
    }

    fn with_active_curve_offset(mut self, start_pointer_x: Option<f32>) -> Self {
        self.active_curve_offset_start_x = start_pointer_x;
        self
    }

    fn with_incoming_waveform(
        mut self,
        incoming_waveform: Option<IncomingWaveformSnapshot>,
    ) -> Self {
        self.incoming_waveform = incoming_waveform;
        self
    }

    fn with_sync_division(mut self, sync_division: usize) -> Self {
        self.sync_division = sync_division;
        self
    }

    fn with_gain_mapping(mut self, depth_db: f32, floor_db: f32) -> Self {
        self.depth_db = depth_db;
        self.floor_db = floor_db;
        self
    }

    fn curve_bounds(bounds: Rect) -> Rect {
        let gutter_width = curve_reference_gutter_width(bounds.width());
        Rect::from_xy_size(
            bounds.min.x + gutter_width,
            bounds.min.y,
            curve_viewport_width(bounds.width()),
            bounds.height(),
        )
    }

    fn reference_gutter_bounds(bounds: Rect) -> Rect {
        let curve_bounds = Self::curve_bounds(bounds);
        Rect::from_xy_size(
            bounds.min.x,
            bounds.min.y,
            (curve_bounds.min.x - bounds.min.x).max(0.0),
            bounds.height(),
        )
    }

    fn curve_point(bounds: Rect, node: CurveNode) -> Point {
        let curve_bounds = Self::curve_bounds(bounds);
        let width = curve_bounds.width().max(1.0) - 1.0;
        let height = curve_bounds.height().max(1.0) - 1.0;
        Point::new(
            curve_bounds.min.x + node.x.clamp(0.0, 1.0) * width,
            curve_bounds.min.y + (1.0 - node.y.clamp(0.0, 1.0)) * height,
        )
    }

    fn node_from_point(bounds: Rect, position: Point) -> CurveNode {
        let curve_bounds = Self::curve_bounds(bounds);
        let width = (curve_bounds.width().max(1.0) - 1.0).max(1.0);
        let height = (curve_bounds.height().max(1.0) - 1.0).max(1.0);
        CurveNode {
            x: ((position.x - curve_bounds.min.x) / width).clamp(0.0, 1.0),
            y: (1.0 - ((position.y - curve_bounds.min.y) / height)).clamp(0.0, 1.0),
        }
    }

    fn offset_pointer_x(bounds: Rect, position: Point) -> f32 {
        let curve_bounds = Self::curve_bounds(bounds);
        let width = (curve_bounds.width().max(1.0) - 1.0).max(1.0);
        (position.x - curve_bounds.min.x) / width
    }

    fn hit_node(&self, bounds: Rect, position: Point) -> Option<usize> {
        self.hit_node_within(bounds, position, CURVE_NODE_HIT_RADIUS)
    }

    fn hit_node_within(&self, bounds: Rect, position: Point, radius: f32) -> Option<usize> {
        if !Self::curve_bounds(bounds).contains(position) {
            return None;
        }
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
        if !Self::curve_bounds(bounds).contains(position)
            || self.curve.nodes.len() < 2
            || self.curve.nodes.len() >= MAX_EDITABLE_NODES
        {
            return None;
        }

        Some(Self::node_from_point(bounds, position))
    }

    fn hover_at(&self, bounds: Rect, position: Point) -> CurveHoverState {
        let curve_bounds = Self::curve_bounds(bounds);
        let node = if curve_bounds.contains(position) {
            self.hit_node(bounds, position)
        } else {
            None
        };
        let segment = if node.is_none() && curve_bounds.contains(position) {
            self.hit_segment(bounds, position, CURVE_SEGMENT_HOVER_RADIUS)
        } else {
            None
        };
        let preview_node =
            if (self.option_hover_held || self.command_hover_held) && segment.is_some() {
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
        if !Self::curve_bounds(bounds).contains(position)
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
        let curve_bounds = Self::curve_bounds(bounds);
        let grid = super::curve_beat_grid(self.sync_division, curve_bounds.width());
        for (positions, color) in [
            (grid.minor.as_slice(), theme.grid_soft),
            (grid.major.as_slice(), theme.grid_strong),
        ] {
            for position in positions {
                let x = curve_bounds.min.x + (curve_bounds.width().max(1.0) - 1.0) * position;
                primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                    widget_id: self.common.id,
                    points: Arc::from([
                        Point::new(x, curve_bounds.min.y),
                        Point::new(x, curve_bounds.max.y),
                    ]),
                    color,
                    width: 1.0,
                }));
            }
        }
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: Arc::from([
                Point::new(curve_bounds.min.x, curve_bounds.min.y),
                Point::new(curve_bounds.min.x, curve_bounds.max.y),
            ]),
            color: theme.border_emphasis,
            width: 1.0,
        }));
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.common.id,
            rect: bounds,
            color: theme.border_emphasis,
            width: 1.0,
        }));
    }

    fn push_gain_references(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        theme: &ThemeTokens,
    ) {
        let curve_bounds = Self::curve_bounds(bounds);
        let gutter_bounds = Self::reference_gutter_bounds(bounds);
        let label_width = (gutter_bounds.width() - 8.0).max(1.0);
        let label_height = CURVE_REFERENCE_LABEL_HEIGHT.min(bounds.height().max(1.0));
        let label_left = gutter_bounds.min.x + 4.0;
        let min_top = bounds.min.y;
        let max_top = (bounds.max.y - label_height).max(min_top);

        for reference in super::curve_gain_references_for_mapping(self.depth_db, self.floor_db) {
            let y = Self::curve_point(
                bounds,
                CurveNode {
                    x: 0.0,
                    y: reference.gain,
                },
            )
            .y;
            let label_top = (y - label_height * 0.5).clamp(min_top, max_top);
            let label_rect = Rect::from_xy_size(label_left, label_top, label_width, label_height);
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.common.id,
                points: Arc::from([
                    Point::new(curve_bounds.min.x, y),
                    Point::new(curve_bounds.max.x, y),
                ]),
                color: theme.text_muted.with_alpha(72),
                width: 1.0,
            }));
            primitives.push(PaintPrimitive::Text(PaintTextRun {
                widget_id: self.common.id,
                text: PaintText::from(super::curve_gain_reference_text(reference, false)),
                rect: label_rect,
                font_size: CURVE_REFERENCE_FONT_SIZE,
                baseline: None,
                color: theme.text_muted,
                align: PaintTextAlign::Right,
                wrap: TextWrap::None,
            }));
        }
    }

    fn push_curve(&self, primitives: &mut Vec<PaintPrimitive>, bounds: Rect, theme: &ThemeTokens) {
        let curve_bounds = Self::curve_bounds(bounds);
        let points = self.sample_curve_points(bounds);
        let gradient = PaintLinearGradient::vertical(
            curve_bounds,
            theme.accent_mint.with_alpha(CURVE_FILL_TOP_ALPHA),
            theme.accent_mint.with_alpha(CURVE_FILL_BOTTOM_ALPHA),
        );
        push_sampled_curve_area_fill(
            primitives,
            SampledCurveAreaFillParts::new(
                self.common.id,
                curve_bounds,
                CURVE_SAMPLE_COUNT,
                SampledCurveAreaBaseline::Bottom,
                PaintBrush::linear_gradient(gradient),
            ),
            |phase| {
                Some(Self::curve_point(
                    bounds,
                    CurveNode {
                        x: phase,
                        y: sample_editable_curve(&self.curve, phase),
                    },
                ))
            },
        );
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: Arc::from(points),
            color: theme.accent_mint,
            width: 2.0,
        }));

        let move_segment = (self.active_segment_move && self.active_segment.is_some())
            .then_some(self.active_segment)
            .flatten()
            .or_else(|| {
                self.command_hover_held
                    .then_some(self.hover_segment)
                    .flatten()
            });
        let tension_segment = (!self.active_segment_move)
            .then_some(self.active_segment)
            .flatten()
            .or_else(|| {
                (!self.command_hover_held && self.option_hover_held)
                    .then_some(self.hover_segment)
                    .flatten()
            });
        if let Some((segment, color)) = move_segment
            .map(|segment| (segment, CURVE_SEGMENT_MOVE_COLOR))
            .or_else(|| tension_segment.map(|segment| (segment, theme.accent_warning)))
        {
            let points = self.sample_segment_points(bounds, segment);
            if points.len() > 1 {
                primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                    widget_id: self.common.id,
                    points: Arc::from(points),
                    color,
                    width: 3.5,
                }));
            }
        }
    }

    fn push_incoming_waveform(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        theme: &ThemeTokens,
    ) {
        let Some(waveform) = self.incoming_waveform.as_ref() else {
            return;
        };
        let curve_bounds = Self::curve_bounds(bounds);
        let center_y = curve_bounds.min.y + curve_bounds.height() * 0.5;
        let amplitude_scale = curve_bounds.height() * 0.43;
        let points_for = |upper: bool| -> Arc<[Point]> {
            Arc::from(
                waveform
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(index, amplitude)| {
                        let phase = index as f32 / (waveform.len() - 1) as f32;
                        let x = curve_bounds.min.x + phase * curve_bounds.width();
                        let offset = amplitude.clamp(0.0, 1.0) * amplitude_scale;
                        Point::new(x, center_y + if upper { -offset } else { offset })
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let color = theme.text_muted.with_alpha(88);
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: points_for(true),
            color,
            width: 1.0,
        }));
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: points_for(false),
            color,
            width: 1.0,
        }));
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

    fn push_playhead(&self, primitives: &mut Vec<PaintPrimitive>, bounds: Rect) {
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
            color: CURVE_PLAYHEAD_GLOW_COLOR,
        }));
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.common.id,
            rect: Rect::from_xy_size(
                center.x - core_radius,
                center.y - core_radius,
                CURVE_PLAYHEAD_MARKER_SIZE,
                CURVE_PLAYHEAD_MARKER_SIZE,
            ),
            color: CURVE_PLAYHEAD_CORE_COLOR,
        }));
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.common.id,
            rect: Rect::from_xy_size(
                center.x - core_radius,
                center.y - core_radius,
                CURVE_PLAYHEAD_MARKER_SIZE,
                CURVE_PLAYHEAD_MARKER_SIZE,
            ),
            color: CURVE_PLAYHEAD_STROKE_COLOR,
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
                let command_held = self.command_hover_held || modifiers.command;
                let option_held = self.option_hover_held || modifiers.alt;
                if let Some(index) = self.hit_node(bounds, position) {
                    Some(CurvePreviewMessage::PressNode {
                        index,
                        pointer: Self::node_from_point(bounds, position),
                        shift_held: self.shift_hover_held || modifiers.shift,
                        option_held,
                        command_held,
                    })
                } else {
                    let hover = self.hover_at(bounds, position);
                    if modifiers.command
                        && modifiers.shift
                        && !option_held
                        && hover.segment.is_none()
                        && Self::curve_bounds(bounds).contains(position)
                    {
                        Some(CurvePreviewMessage::PressCurveOffset {
                            pointer_x: Self::offset_pointer_x(bounds, position),
                        })
                    } else {
                        match (command_held, option_held, hover.segment) {
                            (true, _, Some(index)) => {
                                Some(CurvePreviewMessage::PressSegmentMove { index, position })
                            }
                            (false, true, Some(index)) => {
                                Some(CurvePreviewMessage::PressSegment { index, position })
                            }
                            (false, true, None) => None,
                            _ => hover
                                .preview_node
                                .or_else(|| self.insert_node_at(bounds, position))
                                .map(|node| CurvePreviewMessage::InsertNode {
                                    node: if command_held {
                                        CurveNode {
                                            x: snap_curve_time_to_beat_grid(
                                                self.sync_division,
                                                Self::curve_bounds(bounds).width(),
                                                node.x,
                                            ),
                                            ..node
                                        }
                                    } else {
                                        node
                                    },
                                    command_held,
                                }),
                        }
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
                        push_through_threshold_x: curve_node_push_through_threshold_x(
                            bounds.width(),
                        ),
                    })
                } else if let Some(start_x) = self.active_curve_offset_start_x {
                    Some(CurvePreviewMessage::DragCurveOffset {
                        delta: Self::offset_pointer_x(bounds, position) - start_x,
                    })
                } else if let Some(index) = self.active_segment {
                    Some(CurvePreviewMessage::DragSegment {
                        index,
                        position,
                        curve_size: Vector2::new(
                            Self::curve_bounds(bounds).width(),
                            Self::curve_bounds(bounds).height(),
                        ),
                    })
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
                != self.option_hover_held
                || modifiers.command != self.command_hover_held
                || modifiers.shift != self.shift_hover_held)
                .then_some(CurvePreviewMessage::ModifiersChanged {
                    option_held: modifiers.alt,
                    command_held: modifiers.command,
                    shift_held: modifiers.shift,
                }),
            WidgetInput::PointerRelease {
                position,
                button: PointerButton::Primary,
                modifiers,
            }
            | WidgetInput::PointerDrop {
                position,
                button: PointerButton::Primary,
                modifiers,
            } => {
                if let Some(index) = self.active_node {
                    Some(CurvePreviewMessage::ReleaseNode {
                        index,
                        node: Self::node_from_point(bounds, position),
                        push_through_threshold_x: curve_node_push_through_threshold_x(
                            bounds.width(),
                        ),
                        shift_held: modifiers.shift,
                        option_held: modifiers.alt,
                        command_held: modifiers.command,
                    })
                } else if let Some(start_x) = self.active_curve_offset_start_x {
                    Some(CurvePreviewMessage::ReleaseCurveOffset {
                        delta: Self::offset_pointer_x(bounds, position) - start_x,
                    })
                } else {
                    self.active_segment
                        .map(|index| CurvePreviewMessage::ReleaseSegment {
                            index,
                            position,
                            curve_size: Vector2::new(
                                Self::curve_bounds(bounds).width(),
                                Self::curve_bounds(bounds).height(),
                            ),
                        })
                }
            }
            WidgetInput::FocusChanged(false) => (self.active_node.is_some()
                || self.active_segment.is_some()
                || self.active_curve_offset_start_x.is_some()
                || self.hover_node.is_some()
                || self.preview_node.is_some()
                || self.hover_segment.is_some()
                || self.option_hover_held
                || self.command_hover_held
                || self.shift_hover_held)
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
        self.push_incoming_waveform(primitives, bounds, theme);
        self.push_gain_references(primitives, bounds, theme);
        self.push_curve(primitives, bounds, theme);
        self.push_nodes(primitives, bounds, theme);
        self.push_playhead(primitives, bounds);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CurvePreviewMessage {
    Hover {
        node: Option<usize>,
        preview_node: Option<CurveNode>,
        segment: Option<usize>,
    },
    ModifiersChanged {
        option_held: bool,
        command_held: bool,
        shift_held: bool,
    },
    PressNode {
        index: usize,
        pointer: CurveNode,
        shift_held: bool,
        option_held: bool,
        command_held: bool,
    },
    PressCurveOffset {
        pointer_x: f32,
    },
    InsertNode {
        node: CurveNode,
        command_held: bool,
    },
    DeleteNode {
        index: usize,
    },
    DragNode {
        index: usize,
        node: CurveNode,
        push_through_threshold_x: f32,
    },
    DragCurveOffset {
        delta: f32,
    },
    ReleaseNode {
        index: usize,
        node: CurveNode,
        push_through_threshold_x: f32,
        shift_held: bool,
        option_held: bool,
        command_held: bool,
    },
    ReleaseCurveOffset {
        delta: f32,
    },
    PressSegment {
        index: usize,
        position: Point,
    },
    PressSegmentMove {
        index: usize,
        position: Point,
    },
    DragSegment {
        index: usize,
        position: Point,
        curve_size: Vector2,
    },
    ReleaseSegment {
        index: usize,
        position: Point,
        curve_size: Vector2,
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

    fn test_curve_push_through_threshold_x() -> f32 {
        curve_node_push_through_threshold_x(300.0)
    }

    fn unconstrained_press(index: usize) -> CurvePreviewMessage {
        CurvePreviewMessage::PressNode {
            index,
            pointer: CurveNode { x: 0.0, y: 0.0 },
            shift_held: false,
            option_held: false,
            command_held: false,
        }
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
        widget.handle_input(
            bounds,
            WidgetInput::PointerRelease {
                position: Point::new(10.0, 10.0),
                button: PointerButton::Primary,
                modifiers: PointerModifiers::default(),
            },
        );
        assert!(!widget.common.state.pressed);
    }

    #[test]
    fn curve_slot_widget_wheel_and_keyboard_navigation_emit_bounded_moves() {
        let bounds = Rect::from_xy_size(0.0, 0.0, 48.0, CURVE_SLOT_ROW_HEIGHT);
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurveSlotWidget::new(3, Some(curve), false, false);

        let wheel = widget
            .handle_input(
                bounds,
                WidgetInput::Wheel {
                    position: Point::new(10.0, 10.0),
                    delta: Vector2::new(0.0, -1.0),
                    modifiers: PointerModifiers::default(),
                },
            )
            .expect("slot wheel should navigate the carousel");
        assert_eq!(
            wheel.typed_copied(),
            Some(CurveSlotMessage::Navigate { delta: 1 })
        );

        widget.handle_input(bounds, WidgetInput::FocusChanged(true));
        let key = widget
            .handle_input(bounds, WidgetInput::KeyPress(WidgetKey::ArrowLeft))
            .expect("focused slot should accept horizontal keyboard navigation");
        assert_eq!(
            key.typed_copied(),
            Some(CurveSlotMessage::Navigate { delta: -1 })
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
    fn curve_slot_navigation_pages_and_reverses_the_visible_window() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(params);

        assert_eq!(state.curve_slot_scroll_offset, 0);
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::CurveSlot(CurveSlotMessage::Navigate {
                delta: CURVE_SLOT_VISIBLE_COUNT as i8,
            }),
        );
        assert_eq!(state.curve_slot_scroll_offset, curve_slot_scroll_max());

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::CurveSlot(CurveSlotMessage::Navigate { delta: -1 }),
        );
        assert_eq!(state.curve_slot_scroll_offset, curve_slot_scroll_max() - 1);
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
            RadiantEditorMessage::Curve(unconstrained_press(1)),
        );
        assert_eq!(state.active_curve_node, Some(1));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.2, y: 0.25 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
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
                push_through_threshold_x: test_curve_push_through_threshold_x(),
                shift_held: false,
                option_held: false,
                command_held: false,
            }),
        );
        let curve = params.editable_curve_snapshot();
        assert!((curve.nodes[1].x - 0.24).abs() < f32::EPSILON);
        assert!((curve.nodes[1].y - 0.3).abs() < f32::EPSILON);
        assert_eq!(state.active_curve_node, None);
    }

    #[test]
    fn radiant_editor_shift_drag_from_start_locks_gain_through_vertical_drift() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
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

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let origin = curve.nodes[2];
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressNode {
                index: 2,
                pointer: origin,
                shift_held: true,
                option_held: false,
                command_held: false,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.95, y: 0.05 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes.len(), curve.nodes.len() - 1);
        assert!(!dragged.nodes.contains(&curve.nodes[3]));
        assert!((dragged.nodes[2].x - 0.95).abs() < 1.0e-6);
        assert!((dragged.nodes[2].y - origin.y).abs() < 1.0e-6);
        assert!(state.shift_hover_held);
        assert_eq!(
            state
                .active_curve_node_drag
                .as_ref()
                .and_then(|drag| drag.horizontal_gain_anchor),
            Some(origin.y)
        );
    }

    #[test]
    fn radiant_editor_shift_mid_drag_engages_and_releases_without_gain_jump() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot().nodes[1];
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressNode {
                index: 1,
                pointer: origin,
                shift_held: false,
                option_held: false,
                command_held: false,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.42, y: 0.6 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );
        let engaged_gain = params.editable_curve_snapshot().nodes[1].y;

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: false,
                shift_held: true,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.5, y: 0.05 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );
        assert!((params.editable_curve_snapshot().nodes[1].y - engaged_gain).abs() < 1.0e-6);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: false,
                shift_held: false,
            }),
        );
        let released_gain = params.editable_curve_snapshot().nodes[1].y;
        assert!((released_gain - engaged_gain).abs() < 1.0e-6);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.55, y: 0.05 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );
        assert!((params.editable_curve_snapshot().nodes[1].y - engaged_gain).abs() < 1.0e-6);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.58, y: 0.15 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );
        assert!(
            (params.editable_curve_snapshot().nodes[1].y - (engaged_gain + 0.1)).abs() < 1.0e-6
        );
    }

    #[test]
    fn radiant_editor_shift_drag_preserves_wrapped_endpoint_gain() {
        let params = Arc::new(PumpParams::new());
        let curve = params.editable_curve_snapshot();
        let origin = curve.nodes[0];
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressNode {
                index: 0,
                pointer: origin,
                shift_held: true,
                option_held: false,
                command_held: false,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 0,
                node: CurveNode { x: 0.8, y: 0.2 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        let dragged = params.editable_curve_snapshot();
        let last = dragged.nodes.len() - 1;
        assert_eq!(dragged.nodes[0].x, 0.0);
        assert_eq!(dragged.nodes[last].x, 1.0);
        assert!((dragged.nodes[0].y - origin.y).abs() < 1.0e-6);
        assert!((dragged.nodes[last].y - origin.y).abs() < 1.0e-6);
    }

    #[test]
    fn radiant_editor_shift_anchor_does_not_leak_into_consecutive_gesture() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot().nodes[1];
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressNode {
                index: 1,
                pointer: origin,
                shift_held: true,
                option_held: false,
                command_held: false,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleaseNode {
                index: 1,
                node: CurveNode { x: 0.4, y: 0.1 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
                shift_held: true,
                option_held: false,
                command_held: false,
            }),
        );
        assert!(state.active_curve_node_drag.is_none());

        let second_origin = params.editable_curve_snapshot().nodes[1];
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressNode {
                index: 1,
                pointer: second_origin,
                shift_held: false,
                option_held: false,
                command_held: false,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.45, y: 0.25 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        assert!((params.editable_curve_snapshot().nodes[1].y - 0.25).abs() < 1.0e-6);
        assert!(!state.shift_hover_held);
    }

    #[test]
    fn radiant_editor_shift_option_drag_from_start_locks_time_while_gain_moves() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.8 },
                CurveNode { x: 0.25, y: 0.6 },
                CurveNode { x: 0.53, y: 0.3 },
                CurveNode { x: 0.75, y: 0.5 },
                CurveNode { x: 1.0, y: 0.8 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let origin = curve.nodes[2];
        let mut state = editor_state(Arc::clone(&params));

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 2,
                pointer: origin,
                shift_held: true,
                option_held: true,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.98, y: 0.05 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes.len(), curve.nodes.len());
        assert!((dragged.nodes[2].x - origin.x).abs() < 1.0e-6);
        assert!((dragged.nodes[2].y - 0.05).abs() < 1.0e-6);
        assert_eq!(
            state
                .active_curve_node_drag
                .as_ref()
                .and_then(|drag| drag.vertical_time_anchor),
            Some(origin.x)
        );
        assert!(state
            .active_curve_node_drag
            .as_ref()
            .is_some_and(|drag| drag.horizontal_gain_anchor.is_none()));
    }

    #[test]
    fn radiant_editor_shift_option_mid_drag_engages_and_releases_without_jump() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot().nodes[1];
        let mut state = editor_state(Arc::clone(&params));

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 1,
                pointer: origin,
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.42, y: 0.6 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let engaged = params.editable_curve_snapshot().nodes[1];

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: true,
                command_held: false,
                shift_held: true,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.9, y: 0.2 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let constrained = params.editable_curve_snapshot().nodes[1];
        assert!((constrained.x - engaged.x).abs() < 1.0e-6);
        assert!((constrained.y - 0.2).abs() < 1.0e-6);

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: false,
                shift_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.9, y: 0.2 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let released = params.editable_curve_snapshot().nodes[1];
        assert!((released.x - constrained.x).abs() < 1.0e-6);
        assert!((released.y - constrained.y).abs() < 1.0e-6);

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.94, y: 0.3 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let resumed = params.editable_curve_snapshot().nodes[1];
        assert!((resumed.x - (released.x + 0.04)).abs() < 1.0e-6);
        assert!((resumed.y - 0.3).abs() < 1.0e-6);
    }

    #[test]
    fn radiant_editor_option_release_transitions_smoothly_to_shift_only() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.8 },
                CurveNode { x: 0.25, y: 0.6 },
                CurveNode { x: 0.53, y: 0.3 },
                CurveNode { x: 0.75, y: 0.5 },
                CurveNode { x: 1.0, y: 0.8 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let origin = curve.nodes[2];
        let mut state = editor_state(Arc::clone(&params));

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 2,
                pointer: origin,
                shift_held: true,
                option_held: true,
                command_held: true,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.9, y: 0.1 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let vertical = params.editable_curve_snapshot().nodes[2];
        assert!((vertical.x - origin.x).abs() < 1.0e-6);
        assert!((vertical.y - 0.1).abs() < 1.0e-6);

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: true,
                shift_held: true,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.9, y: 0.1 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let handoff = params.editable_curve_snapshot().nodes[2];
        assert!((handoff.x - vertical.x).abs() < 1.0e-6);
        assert!((handoff.y - vertical.y).abs() < 1.0e-6);

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: false,
                shift_held: true,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.94, y: 0.9 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let horizontal = params.editable_curve_snapshot().nodes[2];
        assert!((horizontal.x - (handoff.x + 0.04)).abs() < 1.0e-6);
        assert!((horizontal.y - handoff.y).abs() < 1.0e-6);
    }

    #[test]
    fn radiant_editor_shift_option_precedes_command_and_preserves_boundaries() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.8 },
                CurveNode { x: 0.25, y: 0.6 },
                CurveNode { x: 0.53, y: 0.3 },
                CurveNode { x: 0.75, y: 0.5 },
                CurveNode { x: 1.0, y: 0.8 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let origin = curve.nodes[2];
        let mut state = editor_state(Arc::clone(&params));

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 2,
                pointer: origin,
                shift_held: true,
                option_held: true,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: true,
                command_held: true,
                shift_held: true,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 1.5, y: 1.5 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let high = params.editable_curve_snapshot();
        assert_eq!(high.nodes.len(), curve.nodes.len());
        assert!((high.nodes[2].x - origin.x).abs() < 1.0e-6);
        assert!((high.nodes[2].y - 1.0).abs() < 1.0e-6);

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: true,
                command_held: false,
                shift_held: true,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: -0.5, y: -0.5 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let low = params.editable_curve_snapshot();
        assert_eq!(low.nodes.len(), curve.nodes.len());
        assert!((low.nodes[2].x - origin.x).abs() < 1.0e-6);
        assert!(low.nodes[2].y.abs() < 1.0e-6);
    }

    #[test]
    fn radiant_editor_vertical_anchor_clears_on_cancel_and_consecutive_gesture() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot().nodes[1];
        let mut state = editor_state(Arc::clone(&params));

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 1,
                pointer: origin,
                shift_held: true,
                option_held: true,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.8, y: 0.2 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        reduce_curve_message(&mut state, CurvePreviewMessage::Cancel);
        assert!(state.active_curve_node_drag.is_none());
        assert!(!state.shift_hover_held);
        assert!(!state.option_hover_held);

        let second_origin = params.editable_curve_snapshot().nodes[1];
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 1,
                pointer: second_origin,
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.4, y: 0.4 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let second = params.editable_curve_snapshot().nodes[1];
        assert!((second.x - 0.4).abs() < 1.0e-6);
        assert!((second.y - 0.4).abs() < 1.0e-6);
    }

    #[test]
    fn radiant_editor_curve_drag_sticks_before_neighbor_boundary() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
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

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));
        let preview_width = 300.0;
        let threshold_x = curve_node_push_through_threshold_x(preview_width);
        let visible_margin_px =
            threshold_x * (curve_viewport_width(preview_width).max(1.0) - 1.0).max(1.0);
        assert!((visible_margin_px - CURVE_NODE_PUSH_THROUGH_MARGIN_PX).abs() < f32::EPSILON);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(unconstrained_press(2)),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode {
                    x: curve.nodes[3].x + threshold_x - 1.0e-3,
                    y: 0.4,
                },
                push_through_threshold_x: threshold_x,
            }),
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes.len(), curve.nodes.len());
        assert_eq!(dragged.nodes[3], curve.nodes[3]);
        assert!(dragged.nodes[2].x <= curve.nodes[3].x - CURVE_NODE_MIN_SPACING_X);
        assert_eq!(state.active_curve_node, Some(2));
    }

    #[test]
    fn radiant_editor_curve_drag_removes_crossed_neighbor() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
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

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(unconstrained_press(2)),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.95, y: 0.4 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes.len(), curve.nodes.len() - 1);
        assert!(!dragged.nodes.contains(&curve.nodes[3]));
        assert_eq!(dragged.segments.len(), dragged.nodes.len() - 1);
        assert_eq!(state.active_curve_node, Some(2));
        assert_eq!(state.hover_curve_node, Some(2));
    }

    #[test]
    fn radiant_editor_curve_drag_removes_multiple_crossed_neighbors() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.2, y: 0.6 },
                CurveNode { x: 0.4, y: 0.3 },
                CurveNode { x: 0.6, y: 0.7 },
                CurveNode { x: 0.8, y: 0.2 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
            ],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(unconstrained_press(1)),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.98, y: 0.45 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes.len(), 3);
        assert_eq!(dragged.nodes[0].x, 0.0);
        assert!((dragged.nodes[1].x - 0.98).abs() < 1.0e-6);
        assert_eq!(dragged.nodes[2].x, 1.0);
        assert_eq!(state.active_curve_node, Some(1));
    }

    #[test]
    fn radiant_editor_curve_drag_reverse_restores_buffered_neighbors() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.25, y: 0.6 },
                CurveNode { x: 0.5, y: 0.3 },
                CurveNode { x: 0.75, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![
                CurveSegment { tension: 0.15 },
                CurveSegment { tension: -0.25 },
                CurveSegment { tension: 0.35 },
                CurveSegment { tension: -0.05 },
            ],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(unconstrained_press(2)),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.95, y: 0.4 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );
        assert_eq!(
            params.editable_curve_snapshot().nodes.len(),
            curve.nodes.len() - 1
        );

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.55, y: 0.4 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        let restored = params.editable_curve_snapshot();
        assert_eq!(restored.nodes.len(), curve.nodes.len());
        assert_eq!(restored.nodes[3], curve.nodes[3]);
        assert_eq!(restored.segments, curve.segments);
        assert_eq!(state.active_curve_node, Some(2));
    }

    #[test]
    fn radiant_editor_curve_drag_release_commits_visible_crossings() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
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

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(unconstrained_press(2)),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleaseNode {
                index: 2,
                node: CurveNode { x: 0.95, y: 0.4 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
                shift_held: false,
                option_held: false,
                command_held: false,
            }),
        );

        let released = params.editable_curve_snapshot();
        assert_eq!(released.nodes.len(), curve.nodes.len() - 1);
        assert!(!released.nodes.contains(&curve.nodes[3]));
        assert_eq!(state.active_curve_node, None);
        assert!(state.active_curve_node_drag.is_none());
    }

    #[test]
    fn radiant_editor_curve_drag_keeps_endpoints_anchored() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.5, y: 0.25 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }, CurveSegment { tension: 0.0 }],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(unconstrained_press(0)),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 0,
                node: CurveNode { x: 0.9, y: 0.31 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        let dragged = params.editable_curve_snapshot();
        let last_index = dragged.nodes.len() - 1;
        assert_eq!(dragged.nodes.len(), curve.nodes.len());
        assert_eq!(dragged.nodes[0].x, 0.0);
        assert_eq!(dragged.nodes[last_index].x, 1.0);
        assert!((dragged.nodes[0].y - 0.31).abs() < 1.0e-6);
        assert!((dragged.nodes[last_index].y - 0.31).abs() < 1.0e-6);
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
            RadiantEditorMessage::Curve(CurvePreviewMessage::InsertNode {
                node,
                command_held: false,
            }),
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
            RadiantEditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
                option_held: true,
                command_held: false,
                shift_held: false,
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

            ..EditableCurve::default()
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
                curve_size: Vector2::new(320.0, CURVE_PREVIEW_HEIGHT),
            }),
        );
        let upward_midpoint = sample_editable_curve(&params.editable_curve_snapshot(), 0.25);
        assert!(upward_midpoint > baseline_midpoint);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragSegment {
                index: 0,
                position: Point::new(start.x, start.y + 24.0),
                curve_size: Vector2::new(320.0, CURVE_PREVIEW_HEIGHT),
            }),
        );
        let downward_midpoint = sample_editable_curve(&params.editable_curve_snapshot(), 0.25);
        assert!(downward_midpoint < baseline_midpoint);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleaseSegment {
                index: 0,
                position: Point::new(start.x, start.y + 24.0),
                curve_size: Vector2::new(320.0, CURVE_PREVIEW_HEIGHT),
            }),
        );
        assert!(state.active_curve_segment.is_none());
    }

    #[test]
    fn radiant_editor_command_segment_drag_translates_pair_without_changing_slope() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.2 },
                CurveNode { x: 0.25, y: 0.3 },
                CurveNode { x: 0.55, y: 0.7 },
                CurveNode { x: 0.8, y: 0.4 },
                CurveNode { x: 1.0, y: 0.2 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));
        state.command_hover_held = true;
        let start = Point::new(140.0, 40.0);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressSegmentMove {
                index: 1,
                position: start,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragSegment {
                index: 1,
                position: Point::new(start.x + 24.0, start.y - 12.0),
                curve_size: Vector2::new(320.0, CURVE_PREVIEW_HEIGHT),
            }),
        );

        let moved = params.editable_curve_snapshot();
        let left_delta = (
            moved.nodes[1].x - curve.nodes[1].x,
            moved.nodes[1].y - curve.nodes[1].y,
        );
        let right_delta = (
            moved.nodes[2].x - curve.nodes[2].x,
            moved.nodes[2].y - curve.nodes[2].y,
        );
        assert!((left_delta.0 - right_delta.0).abs() < 1.0e-6);
        assert!((left_delta.1 - right_delta.1).abs() < 1.0e-6);
        assert!(
            ((moved.nodes[2].y - moved.nodes[1].y) - (curve.nodes[2].y - curve.nodes[1].y)).abs()
                < 1.0e-6
        );
        assert!(state
            .active_curve_segment
            .as_ref()
            .is_some_and(|drag| { drag.mode == CurveSegmentDragMode::MovePair }));
    }

    #[test]
    fn radiant_editor_command_release_clears_segment_move_without_mutation() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));
        let before = params.editable_curve_snapshot();
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressSegmentMove {
                index: 1,
                position: Point::new(120.0, 40.0),
            }),
        );
        assert!(state.command_hover_held);
        assert!(state
            .active_curve_segment
            .as_ref()
            .is_some_and(|drag| drag.mode == CurveSegmentDragMode::MovePair));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: false,
                shift_held: false,
            }),
        );

        assert_eq!(params.editable_curve_snapshot(), before);
        assert!(state.active_curve_segment.is_none());
        assert_eq!(state.hover_curve_segment, None);
        assert!(!state.command_hover_held);
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
            Some(CurvePreviewMessage::PressNode {
                index: 1,
                pointer: CurvePreviewWidget::node_from_point(bounds, position),
                shift_held: false,
                option_held: false,
                command_held: false,
            })
        );
    }

    #[test]
    fn curve_preview_widget_forwards_shift_option_on_point_press_and_release() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);
        let modifiers = PointerModifiers {
            shift: true,
            alt: true,
            command: true,
        };
        let mut press_widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);

        let press = press_widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers,
                },
            )
            .and_then(|output| output.typed_copied::<CurvePreviewMessage>());
        assert_eq!(
            press,
            Some(CurvePreviewMessage::PressNode {
                index: 1,
                pointer: CurvePreviewWidget::node_from_point(bounds, position),
                shift_held: true,
                option_held: true,
                command_held: true,
            })
        );

        let mut release_widget =
            CurvePreviewWidget::new(curve, Some(1), None, None, None, None, false);
        let release = release_widget
            .handle_input(
                bounds,
                WidgetInput::PointerRelease {
                    position,
                    button: PointerButton::Primary,
                    modifiers,
                },
            )
            .and_then(|output| output.typed_copied::<CurvePreviewMessage>());
        assert!(matches!(
            release,
            Some(CurvePreviewMessage::ReleaseNode {
                index: 1,
                shift_held: true,
                option_held: true,
                command_held: true,
                ..
            })
        ));
    }

    #[test]
    fn curve_preview_widget_reserves_noninteractive_reference_gutter() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let curve_bounds = CurvePreviewWidget::curve_bounds(bounds);
        let gutter_position = Point::new(bounds.min.x + 10.0, bounds.min.y + 30.0);

        assert!(!curve_bounds.contains(gutter_position));
        assert_eq!(widget.hit_node(bounds, gutter_position), None);
        assert_eq!(widget.insert_node_at(bounds, gutter_position), None);
        assert!(widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: gutter_position,
                    button: PointerButton::Primary,
                    modifiers: Default::default(),
                },
            )
            .is_none());
        assert!(
            (CurvePreviewWidget::curve_point(bounds, curve.nodes[0]).x - curve_bounds.min.x).abs()
                < 1.0e-6
        );
        assert!(
            (CurvePreviewWidget::curve_point(bounds, *curve.nodes.last().unwrap()).x
                - (curve_bounds.max.x - 1.0))
                .abs()
                < 1.0e-6
        );
    }

    #[test]
    fn curve_preview_widget_push_through_threshold_tracks_actual_bounds() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, Some(1), None, None, None, None, false);
        let mut thresholds = Vec::new();

        for preview_width in [220.0, 396.0] {
            let bounds = Rect::from_xy_size(0.0, 0.0, preview_width, CURVE_PREVIEW_HEIGHT);
            let position = Point::new(bounds.max.x - 20.0, bounds.min.y + 30.0);
            let curve_pixel_width =
                (CurvePreviewWidget::curve_bounds(bounds).width().max(1.0) - 1.0).max(1.0);

            let drag = widget
                .handle_input(bounds, WidgetInput::PointerMove { position })
                .expect("active node move should emit a drag message");
            let drag_threshold = match drag.typed_copied() {
                Some(CurvePreviewMessage::DragNode {
                    push_through_threshold_x,
                    ..
                }) => push_through_threshold_x,
                other => panic!("unexpected drag output: {other:?}"),
            };
            assert!(
                (drag_threshold * curve_pixel_width - CURVE_NODE_PUSH_THROUGH_MARGIN_PX).abs()
                    < 1.0e-5
            );

            let release = widget
                .handle_input(
                    bounds,
                    WidgetInput::PointerRelease {
                        position,
                        button: PointerButton::Primary,
                        modifiers: Default::default(),
                    },
                )
                .expect("active node release should emit a release message");
            let release_threshold = match release.typed_copied() {
                Some(CurvePreviewMessage::ReleaseNode {
                    push_through_threshold_x,
                    ..
                }) => push_through_threshold_x,
                other => panic!("unexpected release output: {other:?}"),
            };
            assert!((release_threshold - drag_threshold).abs() < f32::EPSILON);
            thresholds.push(drag_threshold);
        }

        assert!(thresholds[0] > thresholds[1]);
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
            Some(CurvePreviewMessage::InsertNode { node, .. }) => {
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
    fn curve_preview_widget_option_press_on_empty_canvas_is_no_op() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.72, y: 0.18 });

        assert!(widget
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
            .is_none());
    }

    #[test]
    fn curve_preview_widget_cmd_shift_press_on_empty_canvas_starts_offset() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.72, y: 0.18 });

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        command: true,
                        shift: true,
                        ..PointerModifiers::default()
                    },
                },
            )
            .expect("Cmd+Shift empty-canvas press should start an offset");
        assert!(matches!(
            output.typed_copied(),
            Some(CurvePreviewMessage::PressCurveOffset { .. })
        ));
    }

    #[test]
    fn curve_preview_widget_cmd_only_press_does_not_reuse_stale_shift_hover() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_command_hover_held(true)
            .with_shift_hover_held(true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.72, y: 0.18 });

        let output = widget.handle_input(
            bounds,
            WidgetInput::PointerPress {
                position,
                button: PointerButton::Primary,
                modifiers: PointerModifiers {
                    command: true,
                    ..PointerModifiers::default()
                },
            },
        );
        assert!(!matches!(
            output.and_then(|output| output.typed_copied()),
            Some(CurvePreviewMessage::PressCurveOffset { .. })
        ));
    }

    #[test]
    fn curve_preview_widget_shift_only_press_does_not_reuse_stale_command_hover() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_command_hover_held(true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.72, y: 0.18 });

        let output = widget.handle_input(
            bounds,
            WidgetInput::PointerPress {
                position,
                button: PointerButton::Primary,
                modifiers: PointerModifiers {
                    shift: true,
                    ..PointerModifiers::default()
                },
            },
        );
        assert!(!matches!(
            output.and_then(|output| output.typed_copied()),
            Some(CurvePreviewMessage::PressCurveOffset { .. })
        ));
    }

    #[test]
    fn radiant_editor_cmd_shift_offset_moves_only_phase_and_commits_on_release() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressCurveOffset { pointer_x: 0.4 }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragCurveOffset { delta: 0.25 }),
        );
        let moved = state
            .preview_curve_offset
            .clone()
            .expect("offset drag should expose a live preview");
        assert!(state.active_curve_offset.is_some());
        assert_eq!(params.editable_curve_snapshot(), origin);
        assert!(moved.nodes.iter().any(|node| {
            !origin
                .nodes
                .iter()
                .any(|other| (other.x - node.x).abs() < 1.0e-5 && (other.y - node.y).abs() < 1.0e-5)
        }));
        for index in 0..=100 {
            let phase = index as f32 / 100.0;
            let expected = sample_editable_curve(&origin, phase - 0.25);
            let actual = sample_editable_curve(&moved, phase);
            assert!((actual - expected).abs() < 0.025);
        }

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleaseCurveOffset { delta: 0.25 }),
        );
        assert!(state.active_curve_offset.is_none());
        assert!(state.preview_curve_offset.is_none());
        assert_ne!(params.editable_curve_snapshot(), origin);
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
            Some(CurvePreviewMessage::InsertNode { node, .. }) => {
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
                command_held: false,
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
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );
        let curve = params.editable_curve_snapshot();
        assert!((curve.nodes[inserted].x - 0.62).abs() < 1.0e-6);
        assert!((curve.nodes[inserted].y - 0.42).abs() < 1.0e-6);
    }

    #[test]
    fn curve_preview_widget_command_insert_snaps_time_and_preserves_gain() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_sync_division(6);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let raw = CurveNode { x: 0.34, y: 0.08 };
        let position = CurvePreviewWidget::curve_point(bounds, raw);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        command: true,
                        ..PointerModifiers::default()
                    },
                },
            )
            .expect("command empty-canvas press should insert a snapped point");

        let Some(CurvePreviewMessage::InsertNode {
            node,
            command_held: true,
        }) = output.typed_copied()
        else {
            panic!(
                "unexpected command insertion output: {:?}",
                output.typed_copied::<CurvePreviewMessage>()
            );
        };
        let width = CurvePreviewWidget::curve_bounds(bounds).width();
        assert!((node.x - snap_curve_time_to_beat_grid(6, width, raw.x)).abs() < 1.0e-6);
        assert!((node.y - raw.y).abs() < 1.0e-2);
    }

    #[test]
    fn radiant_editor_command_press_and_release_update_point_snap_mid_drag() {
        let params = Arc::new(PumpParams::new());
        params.set_sync_division(6.0);
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.8 },
                CurveNode { x: 0.25, y: 0.4 },
                CurveNode { x: 0.75, y: 0.6 },
                CurveNode { x: 1.0, y: 0.8 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 3],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 1,
                pointer: curve.nodes[1],
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: true,
                shift_held: false,
            },
        );

        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let raw = CurveNode { x: 0.34, y: 0.7 };
        let position = CurvePreviewWidget::curve_point(bounds, raw);
        let mut snapped_widget = CurvePreviewWidget::new(
            params.editable_curve_snapshot(),
            state.active_curve_node,
            None,
            None,
            None,
            None,
            false,
        )
        .with_command_hover_held(true)
        .with_sync_division(6);
        let snapped_message = snapped_widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .and_then(|output| output.typed_copied::<CurvePreviewMessage>())
            .expect("active command drag should emit a snapped node move");
        reduce_curve_message(&mut state, snapped_message);

        let width = CurvePreviewWidget::curve_bounds(bounds).width();
        let snapped = params.editable_curve_snapshot().nodes[1];
        assert!((snapped.x - snap_curve_time_to_beat_grid(6, width, raw.x)).abs() < 1.0e-6);

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: false,
                shift_held: false,
            },
        );
        let mut plain_widget = CurvePreviewWidget::new(
            params.editable_curve_snapshot(),
            state.active_curve_node,
            None,
            None,
            None,
            None,
            false,
        )
        .with_sync_division(6);
        let plain_message = plain_widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .and_then(|output| output.typed_copied::<CurvePreviewMessage>())
            .expect("active plain drag should emit a continuous node move");
        reduce_curve_message(&mut state, plain_message);

        let continuous = params.editable_curve_snapshot().nodes[1];
        assert!((continuous.x - raw.x).abs() < 1.0e-2);
        assert!((continuous.y - snapped.y).abs() < 1.0e-2);
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
            Some(CurvePreviewMessage::ModifiersChanged {
                option_held: true,
                command_held: false,
                shift_held: false,
            })
        );
    }

    #[test]
    fn curve_preview_widget_command_press_prefers_point_then_segment_then_empty_canvas() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let modifiers = PointerModifiers {
            command: true,
            ..PointerModifiers::default()
        };

        let mut point_widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let point = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);
        let point_output = point_widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: point,
                    button: PointerButton::Primary,
                    modifiers,
                },
            )
            .expect("command point press should remain a point gesture");
        assert_eq!(
            point_output.typed_copied(),
            Some(CurvePreviewMessage::PressNode {
                index: 1,
                pointer: CurvePreviewWidget::node_from_point(bounds, point),
                shift_held: false,
                option_held: false,
                command_held: true,
            })
        );

        let mut segment_widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let segment = CurvePreviewWidget::curve_point(
            bounds,
            CurveNode {
                x: 0.2,
                y: sample_editable_curve(&curve, 0.2),
            },
        );
        let segment_output = segment_widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: segment,
                    button: PointerButton::Primary,
                    modifiers,
                },
            )
            .expect("command line press should start grouped movement");
        assert!(matches!(
            segment_output.typed_copied(),
            Some(CurvePreviewMessage::PressSegmentMove { index: 1, .. })
        ));

        let mut empty_widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false);
        let empty = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.72, y: 0.18 });
        let empty_output = empty_widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: empty,
                    button: PointerButton::Primary,
                    modifiers,
                },
            )
            .expect("command empty-canvas press should retain insertion behavior");
        assert!(matches!(
            empty_output.typed_copied(),
            Some(CurvePreviewMessage::InsertNode { .. })
        ));
    }

    #[test]
    fn curve_preview_widget_shift_and_command_compose_on_point_press() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        command: true,
                        shift: true,
                        alt: false,
                    },
                },
            )
            .expect("modified point press should remain a point gesture");

        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::PressNode {
                index: 1,
                pointer: CurvePreviewWidget::node_from_point(bounds, position),
                shift_held: true,
                option_held: false,
                command_held: true,
            })
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
    fn curve_preview_widget_paints_command_hovered_segment_in_dedicated_color() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, Some(1), false)
            .with_command_hover_held(true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        assert_ne!(CURVE_SEGMENT_MOVE_COLOR, theme.accent_warning);
        assert_ne!(CURVE_SEGMENT_MOVE_COLOR, theme.accent_mint);
        assert_ne!(CURVE_SEGMENT_MOVE_COLOR, CURVE_PLAYHEAD_CORE_COLOR);
        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokePolyline(polyline)
                    if polyline.color == CURVE_SEGMENT_MOVE_COLOR
                        && (polyline.width - 3.5).abs() < 1.0e-6
                        && polyline.points.len() > 2
            )
        }));
    }

    #[test]
    fn curve_preview_widget_paints_subtle_fill_beneath_curve() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let fill = primitives
            .iter()
            .find_map(|primitive| match primitive {
                PaintPrimitive::FillPath(fill) => Some(fill),
                _ => None,
            })
            .expect("curve preview should emit one gradient attenuation fill path");
        assert_eq!(fill.path.commands().len(), CURVE_SAMPLE_COUNT + 4);
        assert_eq!(
            fill.brush,
            PaintBrush::linear_gradient(PaintLinearGradient::vertical(
                CurvePreviewWidget::curve_bounds(bounds),
                theme.accent_mint.with_alpha(CURVE_FILL_TOP_ALPHA),
                theme.accent_mint.with_alpha(CURVE_FILL_BOTTOM_ALPHA),
            ))
        );
    }

    #[test]
    fn curve_preview_widget_grid_tracks_sync_length_and_resize_geometry() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let theme = ThemeTokens::default();
        let grid_x_positions = |width: f32| {
            let widget =
                CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false)
                    .with_sync_division(7);
            let bounds = Rect::from_xy_size(0.0, 0.0, width, CURVE_PREVIEW_HEIGHT);
            let mut primitives = Vec::new();
            widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);
            let curve_bounds = CurvePreviewWidget::curve_bounds(bounds);
            let positions = primitives
                .iter()
                .filter_map(|primitive| match primitive {
                    PaintPrimitive::StrokePolyline(stroke)
                        if stroke.color == theme.grid_strong
                            && stroke.points.len() == 2
                            && (stroke.points[0].x - stroke.points[1].x).abs() < 1.0e-6 =>
                    {
                        Some(stroke.points[0].x)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            (curve_bounds, positions)
        };

        let (wide_bounds, wide) = grid_x_positions(396.0);
        let (narrow_bounds, narrow) = grid_x_positions(198.0);
        assert_eq!(wide.len(), 7);
        assert_eq!(narrow.len(), 7);
        for (wide_x, narrow_x) in wide.iter().zip(narrow.iter()) {
            let wide_normalized = (*wide_x - wide_bounds.min.x) / (wide_bounds.width() - 1.0);
            let narrow_normalized =
                (*narrow_x - narrow_bounds.min.x) / (narrow_bounds.width() - 1.0);
            assert!((wide_normalized - narrow_normalized).abs() < 1.0e-4);
        }
    }

    #[test]
    fn curve_preview_widget_gain_references_track_curve_mapping_and_stay_labeled() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let theme = ThemeTokens::default();

        for bounds in [
            Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT),
            Rect::from_xy_size(0.0, 0.0, 270.0, 51.0),
        ] {
            let widget =
                CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
            let mut primitives = Vec::new();
            widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

            let guides: Vec<_> = primitives
                .iter()
                .filter_map(|primitive| match primitive {
                    PaintPrimitive::StrokePolyline(stroke)
                        if stroke.color == theme.text_muted.with_alpha(72)
                            && stroke.points.len() == 2
                            && (stroke.points[0].y - stroke.points[1].y).abs() < 1.0e-6 =>
                    {
                        Some((stroke.points[0].y, stroke.points[0].x, stroke.points[1].x))
                    }
                    _ => None,
                })
                .collect();
            let guide_y_positions: Vec<_> = guides.iter().map(|(y, _, _)| *y).collect();
            let curve_bounds = CurvePreviewWidget::curve_bounds(bounds);
            let expected_y_positions: Vec<_> = super::super::curve_gain_references()
                .iter()
                .map(|reference| {
                    CurvePreviewWidget::curve_point(
                        bounds,
                        CurveNode {
                            x: 0.0,
                            y: reference.gain,
                        },
                    )
                    .y
                })
                .collect();
            let labels: Vec<_> = primitives
                .iter()
                .filter_map(|primitive| match primitive {
                    PaintPrimitive::Text(text)
                        if ["0 dB", "−6 dB", "−12 dB", "−∞"].contains(&text.text.as_str()) =>
                    {
                        assert!(text.rect.min.x >= bounds.min.x);
                        assert!(text.rect.min.y >= bounds.min.y);
                        assert!(text.rect.max.x <= bounds.max.x);
                        assert!(text.rect.max.y <= bounds.max.y);
                        assert!(text.rect.max.x <= curve_bounds.min.x);
                        Some(text.text.as_str())
                    }
                    _ => None,
                })
                .collect();

            assert_eq!(guide_y_positions, expected_y_positions);
            assert!(guides.iter().all(|(_, start_x, end_x)| {
                (*start_x - curve_bounds.min.x).abs() < 1.0e-6
                    && (*end_x - curve_bounds.max.x).abs() < 1.0e-6
            }));
            assert_eq!(labels, ["0 dB", "−6 dB", "−12 dB", "−∞"]);
            assert!((guide_y_positions[0] - bounds.min.y).abs() < 1.0e-6);
            assert!((guide_y_positions[3] - (bounds.max.y - 1.0)).abs() < 1.0e-6);
        }
    }

    #[test]
    fn curve_preview_widget_paints_incoming_waveform_before_curve_and_nodes() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut waveform = [0.0; crate::incoming_waveform::INCOMING_WAVEFORM_BIN_COUNT];
        waveform[crate::incoming_waveform::INCOMING_WAVEFORM_BIN_COUNT / 2] = 1.0;
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_incoming_waveform(Some(waveform));
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let waveform_color = theme.text_muted.with_alpha(88);
        let waveform_indices: Vec<_> = primitives
            .iter()
            .enumerate()
            .filter_map(|(index, primitive)| match primitive {
                PaintPrimitive::StrokePolyline(stroke) if stroke.color == waveform_color => {
                    Some(index)
                }
                _ => None,
            })
            .collect();
        assert_eq!(waveform_indices.len(), 2);
        let curve_index = primitives
            .iter()
            .position(|primitive| {
                matches!(
                    primitive,
                    PaintPrimitive::StrokePolyline(stroke)
                        if stroke.color == theme.accent_mint && (stroke.width - 2.0).abs() < 1.0e-6
                )
            })
            .expect("editable curve stroke should be present");
        let node_index = primitives
            .iter()
            .position(|primitive| {
                matches!(
                    primitive,
                    PaintPrimitive::FillRect(fill) if fill.color == theme.surface_raised
                )
            })
            .expect("curve nodes should be present");
        assert!(waveform_indices.iter().all(|index| *index < curve_index));
        assert!(waveform_indices.iter().all(|index| *index < node_index));
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
    fn gain_reduction_meter_paints_labeled_top_down_db_fill() {
        let bounds = Rect::from_xy_size(0.0, 0.0, GAIN_REDUCTION_METER_WIDTH, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut unity_primitives = Vec::new();
        GainReductionMeterWidget::new(0.0).append_paint(
            &mut unity_primitives,
            bounds,
            &LayoutOutput::default(),
            &theme,
        );
        assert!(!unity_primitives.iter().any(|primitive| {
            matches!(primitive, PaintPrimitive::FillRect(fill) if fill.color == theme.accent_warning)
        }));

        let reduction_db = crate::gui_status::GAIN_REDUCTION_METER_MAX_DB * 0.5;
        let mut reduced_primitives = Vec::new();
        GainReductionMeterWidget::new(reduction_db).append_paint(
            &mut reduced_primitives,
            bounds,
            &LayoutOutput::default(),
            &theme,
        );
        let fill = reduced_primitives
            .iter()
            .find_map(|primitive| match primitive {
                PaintPrimitive::FillRect(fill) if fill.color == theme.accent_warning => Some(fill),
                _ => None,
            })
            .expect("reduction should paint one meter fill");
        assert!((fill.rect.min.y - 15.0).abs() < 1.0e-6);
        assert!((fill.rect.height() - 19.0).abs() < 1.0e-6);
        for expected in ["GR dB", "18.0"] {
            assert!(reduced_primitives.iter().any(|primitive| {
                matches!(primitive, PaintPrimitive::Text(text) if text.text.as_str() == expected)
            }));
        }
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
                    if fill.color == CURVE_PLAYHEAD_CORE_COLOR
                        && (fill.rect.width() - CURVE_PLAYHEAD_MARKER_SIZE).abs() < 1.0e-4
                        && (fill.rect.height() - CURVE_PLAYHEAD_MARKER_SIZE).abs() < 1.0e-4
                        && ((fill.rect.min.x + fill.rect.width() * 0.5) - expected_center.x).abs()
                            < 1.0e-4
                        && ((fill.rect.min.y + fill.rect.height() * 0.5) - expected_center.y).abs()
                            < 1.0e-4
            )
        }));
    }

    #[test]
    fn curve_preview_widget_paints_playhead_above_overlapping_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_playhead_phase(Some(0.0));
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let endpoint_node_index = primitives
            .iter()
            .enumerate()
            .filter_map(|(index, primitive)| match primitive {
                PaintPrimitive::StrokeRect(stroke)
                    if stroke.color == theme.accent_copper
                        && (stroke.rect.width() - CURVE_NODE_SIZE).abs() < 1.0e-6
                        && (stroke.rect.height() - CURVE_NODE_SIZE).abs() < 1.0e-6 =>
                {
                    Some(index)
                }
                _ => None,
            })
            .next()
            .expect("default curve should paint its endpoint node");
        let playhead_core_index = primitives
            .iter()
            .position(|primitive| {
                matches!(
                    primitive,
                    PaintPrimitive::FillRect(fill)
                        if fill.color == CURVE_PLAYHEAD_CORE_COLOR
                            && (fill.rect.width() - CURVE_PLAYHEAD_MARKER_SIZE).abs() < 1.0e-6
                            && (fill.rect.height() - CURVE_PLAYHEAD_MARKER_SIZE).abs() < 1.0e-6
                )
            })
            .expect("phase-zero playhead should paint over the endpoint node");

        assert!(
            playhead_core_index > endpoint_node_index,
            "playhead must be appended after overlapping node primitives"
        );
    }

    #[test]
    fn curve_playhead_palette_does_not_reuse_editable_node_states() {
        let theme = ThemeTokens::default();
        let editable_node_colors = [
            theme.surface_raised,
            theme.accent_warning,
            theme.accent_mint,
            theme.accent_copper,
        ];

        for editable_color in editable_node_colors {
            assert_ne!(CURVE_PLAYHEAD_CORE_COLOR, editable_color);
            assert_ne!(CURVE_PLAYHEAD_STROKE_COLOR, editable_color);
        }
        assert_ne!(CURVE_PLAYHEAD_CORE_COLOR, CURVE_PLAYHEAD_STROKE_COLOR);
    }

    #[cfg(feature = "vst3")]
    #[test]
    fn radiant_editor_consumes_inactive_meter_clear_repaint() {
        let params = Arc::new(PumpParams::new());
        let status = Arc::new(GuiStatus::default());
        let mut editor = RadiantPumpEditor::new(
            params,
            Arc::clone(&status),
            Arc::new(AutomationQueue::default()),
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        );
        status.update(
            0.0,
            0.25,
            GuiTransportTelemetry {
                is_playing: true,
                transport_is_playing: true,
                has_host_beats_timeline: false,
                beat_phase: 0.0,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        assert!(status.gain_reduction_db() > 0.0);
        status.update_transport(
            0.0,
            GuiTransportTelemetry {
                is_playing: false,
                transport_is_playing: false,
                has_host_beats_timeline: false,
                beat_phase: 0.0,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        status.mark_gain_reduction_inactive();

        assert!(editor.needs_realtime_redraw());
        let _ = editor.paint_plan();
        assert!(
            !editor.needs_realtime_redraw(),
            "painting zero must consume the one-shot clear repaint"
        );
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
                transport_is_playing: true,
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
                transport_is_playing: true,
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
        plan.primitives
            .iter()
            .find_map(|primitive| match primitive {
                PaintPrimitive::FillRect(fill)
                    if fill.color == CURVE_PLAYHEAD_CORE_COLOR
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
