//! Backend-neutral Pump editor state, host automation, and curve gestures.
//!
//! This module deliberately contains no renderer or native-window code. GPUI
//! owns composition and input dispatch in `gui_gpui`; this state machine keeps
//! the established Pump parameter, history, slot, and curve semantics intact.

use std::sync::Arc;

use toybox::clack_extensions::params::HostParams;
use toybox::clack_plugin::prelude::HostSharedHandle;
use toybox::clack_plugin::utils::ClapId;
use toybox::clap::automation::AutomationConfig;

use crate::automation_queue::PumpAutomationQueue;
use crate::curve::{
    CurveNode, CurveSegment, EditableCurve, MAX_EDITABLE_NODES, MAX_SEGMENT_TENSION,
    MIN_SEGMENT_TENSION,
};
use crate::params::{
    clamp_delay_beats, format_plain_value_text, parse_plain_value_text,
    plain_from_normalized_value, PumpParams, PumpSoundState, SoundSide, BYPASS_ACTIVE_VALUE,
    BYPASS_BYPASSED_VALUE, DEFAULT_FREE_RATE_HZ, MAX_DELAY_BEATS, MAX_OUTPUT_GAIN_DB,
    MAX_SYNC_DIVISION, MIN_DELAY_BEATS, MIN_OUTPUT_GAIN_DB, PARAM_BYPASS_ID, PARAM_DELAY_ID,
    PARAM_FREE_RATE_ID, PARAM_MIX_ID, PARAM_OUTPUT_GAIN_ID, PARAM_PHASE_OFFSET_ID, PARAM_SMOOTH_ID,
    PARAM_SOUND_ID, PARAM_SWING_ID, PARAM_SYNC_DIVISION_ID, PARAM_TIMING_MODE_ID, TIMING_MODE_FREE,
    TIMING_MODE_SYNC,
};
use crate::GuiStatus;

use super::curve_paint::{
    reconstruct_paint, PaintCommitOutcome, PaintRun, RectBounds, RectPoint, StrokeRecorder,
};
use super::{snap_curve_time_to_beat_grid_with_swing, WINDOW_WIDTH};

/// Minimal point used by the renderer-independent curve interaction model.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Point {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

impl Point {
    pub(crate) const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Minimal vector used by segment gesture normalization.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Vector2 {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

impl Vector2 {
    pub(crate) const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

const SURFACE_PADDING: f32 = 10.2;
const CURVE_REFERENCE_GUTTER_WIDTH: f32 = 40.8;
const CURVE_DISPLAY_SEAM_EPSILON: f32 = 1.0e-5;
const CURVE_STRUCTURAL_ENDPOINT_RAW_EPSILON: f32 = 1.0e-3;
const CURVE_SEAM_OWNER_RAW_EPSILON: f32 = 1.0e-6;
const CURVE_NODE_MIN_SPACING_X: f32 = 1.0e-3;
const CURVE_NODE_PUSH_THROUGH_MARGIN_PX: f32 = 10.0;
const CURVE_SEGMENT_TENSION_PIXEL_SCALE: f32 = 120.0;

struct CurveGeometry;

impl CurveGeometry {
    fn display_phase(raw_x: f32, phase_offset: f32) -> f32 {
        let shifted = raw_x - phase_offset;
        let wrapped = shifted.rem_euclid(1.0);
        if shifted > CURVE_DISPLAY_SEAM_EPSILON && wrapped <= CURVE_DISPLAY_SEAM_EPSILON {
            1.0
        } else {
            wrapped
        }
    }
}

/// Main-thread CLAP parameter flush callback retained by the hosted editor.
#[derive(Clone, Copy)]
pub(crate) struct HostParamFlushRequester {
    host: HostSharedHandle<'static>,
    params: HostParams,
}

impl HostParamFlushRequester {
    pub(crate) fn new(host: HostSharedHandle<'_>) -> Option<Self> {
        let params = host.get_extension::<HostParams>()?;
        let host =
            unsafe { std::mem::transmute::<HostSharedHandle<'_>, HostSharedHandle<'static>>(host) };
        Some(Self { host, params })
    }

    pub(crate) fn request_flush(self) {
        self.params.request_flush(&self.host);
    }
}

/// Format-neutral sink for one complete UI-originated host parameter edit.
pub(crate) trait HostParamEditSink: Send + Sync {
    fn edit(&self, config: &AutomationConfig, param_id: ClapId, value: f64) -> bool;
    fn gesture_started(&self, config: &AutomationConfig, param_id: ClapId) -> bool;
    fn gesture_value(&self, config: &AutomationConfig, param_id: ClapId, value: f64) -> bool;
    fn gesture_ended(&self, config: &AutomationConfig, param_id: ClapId) -> bool;
}

struct ClapHostParamEditSink {
    queue: Arc<PumpAutomationQueue>,
    requester: Option<HostParamFlushRequester>,
}

impl HostParamEditSink for ClapHostParamEditSink {
    fn edit(&self, config: &AutomationConfig, param_id: ClapId, value: f64) -> bool {
        let complete = self.queue.push_gesture_edit(config, param_id, value);
        if complete {
            if let Some(requester) = self.requester {
                requester.request_flush();
            }
        }
        complete
    }

    fn gesture_started(&self, config: &AutomationConfig, param_id: ClapId) -> bool {
        let accepted = self.queue.push_gesture_begin(config, param_id);
        if accepted {
            if let Some(requester) = self.requester {
                requester.request_flush();
            }
        }
        accepted
    }

    fn gesture_value(&self, config: &AutomationConfig, param_id: ClapId, value: f64) -> bool {
        let accepted = self.queue.push_gesture_value(config, param_id, value);
        if accepted {
            if let Some(requester) = self.requester {
                requester.request_flush();
            }
        }
        accepted
    }

    fn gesture_ended(&self, config: &AutomationConfig, param_id: ClapId) -> bool {
        let complete = self.queue.push_gesture_end(config, param_id);
        if complete {
            if let Some(requester) = self.requester {
                requester.request_flush();
            }
        }
        complete
    }
}

pub(crate) fn clap_edit_sink(
    queue: Arc<PumpAutomationQueue>,
    requester: Option<HostParamFlushRequester>,
) -> Arc<dyn HostParamEditSink> {
    Arc::new(ClapHostParamEditSink { queue, requester })
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CurveSegmentHitZone {
    OnLine,
    OuterProximity,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CurveSegmentHit {
    index: usize,
    zone: CurveSegmentHitZone,
    distance_squared: f32,
}

fn point_to_segment_distance_squared(point: Point, start: Point, end: Point) -> f32 {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length_squared = dx * dx + dy * dy;
    if !length_squared.is_finite() || length_squared <= f32::EPSILON {
        return (point.x - start.x).powi(2) + (point.y - start.y).powi(2);
    }

    let projection =
        (((point.x - start.x) * dx + (point.y - start.y) * dy) / length_squared).clamp(0.0, 1.0);
    let closest = Point::new(start.x + projection * dx, start.y + projection * dy);
    (point.x - closest.x).powi(2) + (point.y - closest.y).powi(2)
}

fn point_to_polyline_distance_squared(point: Point, points: &[Point]) -> Option<f32> {
    let first = points.first().copied()?;
    if points.len() == 1 {
        return Some((point.x - first.x).powi(2) + (point.y - first.y).powi(2));
    }

    points
        .windows(2)
        .map(|pair| point_to_segment_distance_squared(point, pair[0], pair[1]))
        .min_by(f32::total_cmp)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CanonicalSeamOwner {
    Endpoints,
    Interior(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CanonicalSeamDragKind {
    ExistingOwner,
    Takeover,
}

#[derive(Clone)]
struct CanonicalSeamDrag {
    kind: CanonicalSeamDragKind,
    owner: CanonicalSeamOwner,
    template_curve: EditableCurve,
}

fn seam_raw(phase_offset: f32) -> f32 {
    if phase_offset.is_finite() {
        crate::dsp::authored_curve_phase(0.0, phase_offset)
    } else {
        0.0
    }
}

fn seam_uses_wrapped_endpoints(phase_offset: f32) -> bool {
    let raw = seam_raw(phase_offset);
    raw <= CURVE_STRUCTURAL_ENDPOINT_RAW_EPSILON
        || raw >= 1.0 - CURVE_STRUCTURAL_ENDPOINT_RAW_EPSILON
}

fn canonical_seam_owner(curve: &EditableCurve, phase_offset: f32) -> Option<CanonicalSeamOwner> {
    if curve.nodes.len() < 2 {
        return None;
    }
    if seam_uses_wrapped_endpoints(phase_offset) {
        return Some(CanonicalSeamOwner::Endpoints);
    }

    let seam = seam_raw(phase_offset);
    curve
        .nodes
        .iter()
        .enumerate()
        .skip(1)
        .take(curve.nodes.len().saturating_sub(2))
        .filter(|(_, node)| (node.x - seam).abs() <= CURVE_SEAM_OWNER_RAW_EPSILON)
        .min_by(|(left_index, left), (right_index, right)| {
            (left.x - seam)
                .abs()
                .total_cmp(&(right.x - seam).abs())
                .then_with(|| left_index.cmp(right_index))
        })
        .map(|(index, _)| CanonicalSeamOwner::Interior(index))
}

fn seam_owner_indices(curve: &EditableCurve, owner: CanonicalSeamOwner) -> Vec<usize> {
    match owner {
        CanonicalSeamOwner::Endpoints => {
            let last = curve.nodes.len().saturating_sub(1);
            if last == 0 {
                Vec::new()
            } else {
                vec![0, last]
            }
        }
        CanonicalSeamOwner::Interior(index) => vec![index],
    }
}

fn seam_owner_contains_index(
    curve: &EditableCurve,
    owner: CanonicalSeamOwner,
    index: usize,
) -> bool {
    seam_owner_indices(curve, owner).contains(&index)
}

fn display_x_is_in_edge_zone(raw_x: f32, phase_offset: f32, threshold_x: f32) -> bool {
    let display_x = CurveGeometry::display_phase(raw_x, phase_offset);
    let threshold_x = threshold_x.max(0.0);
    display_x <= threshold_x || display_x >= 1.0 - threshold_x
}

fn display_x_is_at_viewport_boundary(raw_x: f32, phase_offset: f32) -> bool {
    let display_x = CurveGeometry::display_phase(raw_x, phase_offset);
    // raw_node_from_display_point offsets exact edge contacts by the seam
    // epsilon to preserve which display side was selected. Allow one float
    // ulp for that offset when recognizing the physical boundary.
    let epsilon = CURVE_DISPLAY_SEAM_EPSILON + f32::EPSILON;
    display_x <= epsilon || display_x >= 1.0 - epsilon
}

fn preferred_seam_active_index(
    curve: &EditableCurve,
    owner: CanonicalSeamOwner,
    target: CurveNode,
    phase_offset: f32,
) -> usize {
    match owner {
        CanonicalSeamOwner::Interior(index) => index,
        CanonicalSeamOwner::Endpoints => {
            let last = curve.nodes.len().saturating_sub(1);
            if CurveGeometry::display_phase(target.x, phase_offset) >= 0.5 {
                last
            } else {
                0
            }
        }
    }
}

fn interactive_curve_node_survivors(
    curve: &EditableCurve,
    phase_offset: f32,
    active_node: Option<usize>,
) -> Vec<usize> {
    if curve.nodes.is_empty() {
        return Vec::new();
    }

    let phase_offset = if phase_offset.is_finite() {
        phase_offset.rem_euclid(1.0)
    } else {
        0.0
    };
    let display_x: Vec<f32> = curve
        .nodes
        .iter()
        .map(|node| CurveGeometry::display_phase(node.x, phase_offset))
        .collect();
    let seam_owner = canonical_seam_owner(curve, phase_offset);
    let seam_indices = seam_owner
        .map(|owner| seam_owner_indices(curve, owner))
        .unwrap_or_default();
    let left_survivor = select_interactive_edge_survivor(&display_x, active_node, true)
        .or_else(|| seam_indices.first().copied());
    let right_survivor = select_interactive_edge_survivor(&display_x, active_node, false)
        .or_else(|| seam_indices.last().copied());
    let left_seam_replaced = seam_indices
        .first()
        .is_some_and(|index| Some(*index) != left_survivor);
    let right_seam_replaced = seam_indices
        .last()
        .is_some_and(|index| Some(*index) != right_survivor);

    (0..curve.nodes.len())
        .filter(|index| {
            let x = display_x[*index];
            let in_left_band = x <= CURVE_NODE_MIN_SPACING_X;
            let in_right_band = x >= 1.0 - CURVE_NODE_MIN_SPACING_X;
            let canonical_seam_survivor = seam_indices.contains(index)
                && !(left_seam_replaced && seam_indices.first() == Some(index))
                && !(right_seam_replaced && seam_indices.last() == Some(index));
            (!in_left_band && !in_right_band)
                || canonical_seam_survivor
                || Some(*index) == left_survivor
                || Some(*index) == right_survivor
        })
        .collect()
}

fn select_interactive_edge_survivor(
    display_x: &[f32],
    active_node: Option<usize>,
    left_side: bool,
) -> Option<usize> {
    let last_index = display_x.len().saturating_sub(1);
    let in_band = |x: f32| {
        if left_side {
            x <= CURVE_NODE_MIN_SPACING_X
        } else {
            x >= 1.0 - CURVE_NODE_MIN_SPACING_X
        }
    };

    if let Some(active) =
        active_node.filter(|index| display_x.get(*index).copied().is_some_and(&in_band))
    {
        return Some(active);
    }

    let structural = if left_side { 0 } else { last_index };
    if display_x.get(structural).copied().is_some_and(|x| {
        in_band(x)
            && if left_side {
                x <= CURVE_DISPLAY_SEAM_EPSILON
            } else {
                x >= 1.0 - CURVE_DISPLAY_SEAM_EPSILON
            }
    }) {
        return Some(structural);
    }

    display_x
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, x)| in_band(*x))
        .min_by(|(left_index, left_x), (right_index, right_x)| {
            let left_distance = if left_side { *left_x } else { 1.0 - *left_x };
            let right_distance = if left_side { *right_x } else { 1.0 - *right_x };
            left_distance
                .total_cmp(&right_distance)
                .then_with(|| left_index.cmp(right_index))
        })
        .map(|(index, _)| index)
}

fn feasible_delta_interval_including_zero(min_delta: f32, max_delta: f32) -> (f32, f32) {
    if min_delta <= max_delta && min_delta <= 0.0 && max_delta >= 0.0 {
        (min_delta, max_delta)
    } else {
        // Legacy topology can make the preferred spacing interval empty or
        // exclude the current group position. Keep zero movement available so
        // a vertical-only edit remains a valid gesture.
        (min_delta.min(0.0), max_delta.max(0.0))
    }
}

fn feasible_node_x_interval_including_origin(min_x: f32, max_x: f32, origin_x: f32) -> (f32, f32) {
    if min_x <= max_x && min_x <= origin_x && origin_x <= max_x {
        (min_x, max_x)
    } else {
        // Preserve the legacy node's current x when the newer interactive
        // spacing policy cannot contain it. This also guarantees clamp bounds
        // stay ordered when the preferred interval is contradictory.
        (min_x.min(origin_x), max_x.max(origin_x))
    }
}

fn resolve_curve_offset(
    sync_division: usize,
    width: f32,
    swing: f32,
    origin: f32,
    delta: f32,
    snap_to_grid: bool,
) -> f32 {
    let phase = (origin + delta).rem_euclid(1.0);
    if snap_to_grid {
        snap_curve_time_to_beat_grid_with_swing(sync_division, width, phase, swing)
    } else {
        phase
    }
}

#[derive(Clone)]
struct ActiveCurveNodeDrag {
    origin_index: usize,
    origin_curve: EditableCurve,
    /// Marquee selection retained for a grouped node drag. An empty list is
    /// the ordinary single-node gesture.
    selected_indices: Vec<usize>,
    seam_drag: Option<CanonicalSeamDrag>,
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
        swing: f32,
        current: CurveNode,
    ) -> CurveNode {
        self.last_pointer_x = target.x;
        self.last_pointer_y = target.y;
        if self
            .seam_drag
            .as_ref()
            .is_some_and(|drag| drag.kind == CanonicalSeamDragKind::ExistingOwner)
        {
            // A seam owner is one logical node even though it has two display
            // instances. Keep the raw phase selected at gesture start and use
            // only the pointer's vertical coordinate for every update.
            return CurveNode {
                x: current.x,
                y: target.y.clamp(0.0, 1.0),
            };
        }
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
            effective.x = snap_curve_time_to_beat_grid_with_swing(
                sync_division,
                curve_width,
                effective.x,
                swing,
            );
        }
        effective
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CurvePaintSample {
    pub(crate) node: CurveNode,
    pub(crate) display_position: RectPoint,
    pub(crate) outside: bool,
}

impl CurvePaintSample {
    fn raw_position(self) -> RectPoint {
        self.display_position
    }
}

#[derive(Clone)]
struct ActiveCurvePaint {
    origin_snapshot: HistorySnapshot,
    origin_curve: EditableCurve,
    phase_offset: f32,
    recorder: StrokeRecorder,
}

impl ActiveCurvePaint {
    fn new(origin_snapshot: HistorySnapshot, phase_offset: f32) -> Self {
        let origin_curve = origin_snapshot.curve.clone().normalized();
        Self {
            origin_snapshot,
            origin_curve,
            phase_offset,
            recorder: StrokeRecorder::new(RectBounds {
                min: RectPoint { x: 0.0, y: 0.0 },
                max: RectPoint { x: 1.0, y: 1.0 },
            }),
        }
    }

    fn push_sample(&mut self, sample: CurvePaintSample) {
        if sample.outside {
            self.recorder.observe_outside(sample.raw_position());
        } else {
            self.recorder.observe(sample.raw_position());
        }
    }

    fn push_boundary_sample(&mut self, sample: CurvePaintSample) {
        self.recorder.observe_outside(sample.raw_position());
    }

    fn preview_runs(&self) -> Vec<PaintRun> {
        self.recorder.runs().to_vec()
    }

    fn finished_curve(&self) -> PaintCommitOutcome {
        // curve_paint reconstructs authored coordinates from its legacy
        // display-minus-offset convention. Convert the positive audio
        // offset to that mapper's equivalent display coordinate here.
        reconstruct_paint(
            &self.origin_curve,
            (-self.phase_offset).rem_euclid(1.0),
            self.recorder.runs(),
        )
    }

    #[cfg(test)]
    fn preview_candidate(&self) -> EditableCurve {
        self.finished_curve().candidate().clone()
    }
}

#[derive(Clone)]
struct ActiveCurveSegmentDrag {
    index: usize,
    origin_curve: EditableCurve,
    start_pointer: Point,
    mode: CurveSegmentDragMode,
    source: CurveSegmentDragSource,
    history_origin: Option<HistorySnapshot>,
}

#[derive(Clone)]
struct ActiveCurveOffsetDrag {
    origin_phase_offset: f32,
    start_pointer_x: f32,
    raw_delta: f32,
    quantized: bool,
}

#[derive(Clone, Copy)]
struct ActiveCurveMarquee {
    start: CurveNode,
    current: CurveNode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CurveSegmentDragMode {
    AdjustTension { start_tension: f32 },
    MovePair,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CurveSegmentDragSource {
    OptionTension,
    Command,
    DirectProximity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumericEntryTarget {
    Mix,
    OutputGain,
    Smooth,
    Swing,
    FreeRate,
    Delay,
}

impl NumericEntryTarget {
    fn param_id(self) -> ClapId {
        match self {
            Self::Mix => PARAM_MIX_ID,
            Self::OutputGain => PARAM_OUTPUT_GAIN_ID,
            Self::Smooth => PARAM_SMOOTH_ID,
            Self::Swing => PARAM_SWING_ID,
            Self::FreeRate => PARAM_FREE_RATE_ID,
            Self::Delay => PARAM_DELAY_ID,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Mix => "Mix",
            Self::OutputGain => "Output",
            Self::Smooth => "Smooth",
            Self::Swing => "Swing",
            Self::FreeRate => "Free Rate",
            Self::Delay => "Delay",
        }
    }

    pub(crate) fn widget_key(self) -> &'static str {
        match self {
            Self::Mix => "numeric-entry-mix",
            Self::OutputGain => "numeric-entry-output",
            Self::Smooth => "numeric-entry-smooth",
            Self::Swing => "numeric-entry-swing",
            Self::FreeRate => "numeric-entry-free-rate",
            Self::Delay => "numeric-entry-delay",
        }
    }

    fn current_plain_value(self, params: &PumpParams) -> f64 {
        match self {
            Self::Mix => params.mix() as f64,
            Self::OutputGain => params.output_gain_db() as f64,
            Self::Smooth => params.smooth() as f64,
            Self::Swing => params.swing() as f64,
            Self::FreeRate => params.free_rate_hz() as f64,
            Self::Delay => params.delay_beats() as f64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FreeRateUnit {
    Milliseconds,
    Seconds,
    Hertz,
    Kilohertz,
}

impl FreeRateUnit {
    const ALL: [Self; 4] = [
        Self::Milliseconds,
        Self::Seconds,
        Self::Hertz,
        Self::Kilohertz,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Milliseconds => "ms",
            Self::Seconds => "s",
            Self::Hertz => "Hz",
            Self::Kilohertz => "kHz",
        }
    }

    fn value(self, rate_hz: f32) -> f32 {
        match self {
            Self::Milliseconds => 1_000.0 / rate_hz,
            Self::Seconds => 1.0 / rate_hz,
            Self::Hertz => rate_hz,
            Self::Kilohertz => rate_hz / 1_000.0,
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
pub(crate) enum NumericEntryMessage {
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
    Step {
        target: NumericEntryTarget,
        delta: i32,
    },
    Cancel {
        target: NumericEntryTarget,
    },
}

#[derive(Clone)]
pub(crate) struct PumpEditorState {
    params: Arc<PumpParams>,
    status: Arc<GuiStatus>,
    host_param_edit_sink: Arc<dyn HostParamEditSink>,
    automation_config: AutomationConfig,
    #[cfg(test)]
    automation_flush_count: usize,
    active_curve_node: Option<usize>,
    active_curve_node_drag: Option<ActiveCurveNodeDrag>,
    active_curve_paint: Option<ActiveCurvePaint>,
    active_curve_segment: Option<ActiveCurveSegmentDrag>,
    active_curve_offset: Option<ActiveCurveOffsetDrag>,
    active_curve_marquee: Option<ActiveCurveMarquee>,
    selected_curve_nodes: Vec<usize>,
    preview_curve_offset: Option<EditableCurve>,
    hover_curve_node: Option<usize>,
    preview_curve_node: Option<CurveNode>,
    hover_curve_segment: Option<usize>,
    hover_curve_segment_zone: Option<CurveSegmentHitZone>,
    option_hover_held: bool,
    command_hover_held: bool,
    shift_hover_held: bool,
    loaded_global_curve_slot: Option<usize>,
    numeric_entry: Option<NumericEntryState>,
    active_knob_gesture: Option<NumericEntryTarget>,
    timing_dropdown_open: bool,
    hotkey_help_open: bool,
    free_rate_unit: FreeRateUnit,
    ab_confirmation: Option<String>,
    undo_history: Vec<HistorySnapshot>,
    redo_history: Vec<HistorySnapshot>,
}

#[derive(Clone)]
pub(crate) struct HistorySnapshot {
    mix: f32,
    smooth: f32,
    swing: f32,
    depth_db: f32,
    floor_db: f32,
    phase_offset: f32,
    output_gain_db: f32,
    sync_division: usize,
    mode: usize,
    timing_mode: usize,
    free_rate_hz: f32,
    delay_beats: usize,
    curve: EditableCurve,
    active_sound: SoundSide,
    sound_states: [PumpSoundState; 2],
    stored_sound_states: [PumpSoundState; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum EditorMessage {
    Undo,
    Redo,
    ToggleTimingMode,
    ToggleTimingDropdown,
    ToggleWaveformMode,
    ToggleHotkeyHelp,
    Knob {
        target: NumericEntryTarget,
        message: KnobMessage,
    },
    SyncDivision(f32),
    FreeRateUnit(FreeRateUnit),
    SelectSound {
        side: SoundSide,
        copy: bool,
    },
    CopyAndSelectSound(SoundSide),
    ToggleBypass,
    Curve(CurvePreviewMessage),
    CurveSlot(CurveSlotMessage),
    NumericEntry(NumericEntryMessage),
}

/// GPUI-normalized knob events. The old widget emitted the same semantic
/// phases; keeping the phases here lets native pointer and keyboard controls
/// share one history/automation path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum KnobMessage {
    GestureStarted,
    ValueChanged {
        value: f32,
    },
    GestureEnded,
    Reset {
        value: f32,
    },
    /// One transactional keyboard or wheel adjustment.
    Discrete {
        value: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CurveSlotMessage {
    Load { index: usize },
    Store { index: usize },
}

impl PumpEditorState {
    pub(crate) fn new(
        params: Arc<PumpParams>,
        status: Arc<GuiStatus>,
        host_param_edit_sink: Arc<dyn HostParamEditSink>,
    ) -> Self {
        Self {
            params,
            status,
            host_param_edit_sink,
            automation_config: AutomationConfig::default(),
            #[cfg(test)]
            automation_flush_count: 0,
            active_curve_node: None,
            active_curve_node_drag: None,
            active_curve_paint: None,
            active_curve_segment: None,
            active_curve_offset: None,
            active_curve_marquee: None,
            selected_curve_nodes: Vec::new(),
            preview_curve_offset: None,
            hover_curve_node: None,
            preview_curve_node: None,
            hover_curve_segment: None,
            hover_curve_segment_zone: None,
            option_hover_held: false,
            command_hover_held: false,
            shift_hover_held: false,
            loaded_global_curve_slot: None,
            numeric_entry: None,
            active_knob_gesture: None,
            timing_dropdown_open: false,
            hotkey_help_open: false,
            free_rate_unit: FreeRateUnit::Hertz,
            ab_confirmation: None,
            undo_history: Vec::new(),
            redo_history: Vec::new(),
        }
    }

    pub(crate) fn snapshot(&self) -> HistorySnapshot {
        HistorySnapshot {
            mix: self.params.mix(),
            smooth: self.params.smooth(),
            swing: self.params.swing(),
            depth_db: self.params.depth_db(),
            floor_db: self.params.floor_db(),
            phase_offset: self.params.phase_offset(),
            output_gain_db: self.params.output_gain_db(),
            sync_division: self.params.sync_division(),
            mode: self.params.mode(),
            timing_mode: self.params.timing_mode(),
            free_rate_hz: self.params.free_rate_hz(),
            delay_beats: self.params.delay_beats(),
            curve: self.params.editable_curve_snapshot(),
            active_sound: self.params.active_sound(),
            sound_states: [
                self.params.sound_state_snapshot(SoundSide::A),
                self.params.sound_state_snapshot(SoundSide::B),
            ],
            stored_sound_states: [
                self.params.stored_sound_state_snapshot(SoundSide::A),
                self.params.stored_sound_state_snapshot(SoundSide::B),
            ],
        }
    }

    fn push_history(&mut self) {
        self.push_history_snapshot(self.snapshot());
    }

    fn push_history_snapshot(&mut self, snapshot: HistorySnapshot) {
        self.ab_confirmation = None;
        self.undo_history.push(snapshot);
        if self.undo_history.len() > 128 {
            self.undo_history.remove(0);
        }
        self.redo_history.clear();
    }

    fn restore(&self, snapshot: &HistorySnapshot) {
        self.params.set_mix(snapshot.mix);
        self.params.set_smooth(snapshot.smooth);
        self.params.set_swing(snapshot.swing);
        self.params.set_depth_db(snapshot.depth_db);
        self.params.set_floor_db(snapshot.floor_db);
        self.params.set_phase_offset(snapshot.phase_offset);
        self.params.set_output_gain_db(snapshot.output_gain_db);
        self.params.set_sync_division(snapshot.sync_division as f32);
        self.params.set_mode(snapshot.mode as f32);
        self.params.set_timing_mode(snapshot.timing_mode as f32);
        self.params.set_free_rate_hz(snapshot.free_rate_hz);
        self.params.set_delay_beats(snapshot.delay_beats as f32);
        self.params
            .set_editable_curve_preserving_phase(&snapshot.curve);
        self.params
            .set_sound_states_with_references_without_persistence(
                snapshot.active_sound,
                snapshot.sound_states.clone(),
                snapshot.stored_sound_states.clone(),
            );
    }

    fn undo(&mut self) {
        if let Some(snapshot) = self.undo_history.pop() {
            self.redo_history.push(self.snapshot());
            self.restore(&snapshot);
            self.clear_curve_selection();
        }
    }

    fn redo(&mut self) {
        if let Some(snapshot) = self.redo_history.pop() {
            self.undo_history.push(self.snapshot());
            self.restore(&snapshot);
            self.clear_curve_selection();
        }
    }

    fn clear_curve_selection(&mut self) {
        self.active_curve_marquee = None;
        self.selected_curve_nodes.clear();
    }

    fn clear_curve_segment_hover(&mut self) {
        self.hover_curve_segment = None;
        self.hover_curve_segment_zone = None;
    }
}

impl PumpEditorState {
    /// Return the shared parameter projection used by GPUI controls.
    pub(crate) fn params(&self) -> &Arc<PumpParams> {
        &self.params
    }

    /// Return the shared telemetry projection used by GPUI drawing.
    pub(crate) fn status(&self) -> &Arc<GuiStatus> {
        &self.status
    }

    /// Dispatch one semantic editor operation.
    pub(crate) fn dispatch(&mut self, message: EditorMessage) {
        reduce_editor_message(self, message);
    }

    /// Return the currently rendered curve, including any live offset preview.
    pub(crate) fn rendered_curve(&self) -> EditableCurve {
        self.preview_curve_offset
            .clone()
            .unwrap_or_else(|| self.params.editable_curve_snapshot())
    }

    /// Return whether a node is in the retained marquee selection.
    pub(crate) fn selected_node(&self, index: usize) -> bool {
        self.selected_curve_nodes.contains(&index)
    }

    pub(crate) fn active_node(&self) -> Option<usize> {
        self.active_curve_node
    }

    /// Return the loaded quick slot, if one is active.
    pub(crate) fn loaded_slot(&self) -> Option<usize> {
        self.loaded_global_curve_slot
    }

    /// Return whether a gesture is active and needs cancellation on focus loss.
    pub(crate) fn has_active_gesture(&self) -> bool {
        self.active_curve_node_drag.is_some()
            || self.active_curve_paint.is_some()
            || self.active_curve_segment.is_some()
            || self.active_curve_offset.is_some()
            || self.active_curve_marquee.is_some()
            || self.active_knob_gesture.is_some()
    }

    /// End every host gesture before the native child view is torn down.
    ///
    /// GPUI can be destroyed while a pointer is down. Curve cancellation
    /// restores any auditioned offset; knob gestures need an explicit host
    /// end because their lifetime is owned by the automation queue/component
    /// handler rather than the curve reducer.
    pub(crate) fn cancel_active_gestures(&mut self) {
        if let Some(target) = self.active_knob_gesture.take() {
            let _ = self
                .host_param_edit_sink
                .gesture_ended(&self.automation_config, target.param_id());
        }
        if self.active_curve_node_drag.is_some()
            || self.active_curve_paint.is_some()
            || self.active_curve_segment.is_some()
            || self.active_curve_offset.is_some()
            || self.active_curve_marquee.is_some()
        {
            reduce_editor_message(self, EditorMessage::Curve(CurvePreviewMessage::Cancel));
        }
    }

    /// Finish native-host teardown without undoing the last applied audio
    /// value. In particular, an offset drag's current phase is already the
    /// audible state and must be ended at that value rather than restored as
    /// it would be for an explicit Escape/cancel gesture.
    pub(crate) fn finish_for_teardown(&mut self) {
        if let Some(target) = self.active_knob_gesture.take() {
            let _ = self
                .host_param_edit_sink
                .gesture_ended(&self.automation_config, target.param_id());
        }
        if let Some(_drag) = self.active_curve_offset.take() {
            let _ = self
                .host_param_edit_sink
                .gesture_ended(&self.automation_config, PARAM_PHASE_OFFSET_ID);
            // `DragCurveOffset` only applies a new phase after its host value
            // was admitted, so the shared parameter already contains the
            // latest audible value. Teardown needs the terminal event only;
            // sending the same value twice would create an extra host event.
        }
        // Paint samples are only a preview until ReleasePaint; discard the
        // recorder so teardown cannot mutate the last applied audio curve.
        self.active_curve_paint.take();
        if let Some(segment) = self.active_curve_segment.take() {
            commit_direct_curve_segment_history_if_changed(self, &segment);
        }
        self.active_curve_node = None;
        self.active_curve_node_drag = None;
        self.active_curve_marquee = None;
        self.preview_curve_offset = None;
        self.hover_curve_node = None;
        self.preview_curve_node = None;
        self.clear_curve_segment_hover();
        self.option_hover_held = false;
        self.command_hover_held = false;
        self.shift_hover_held = false;
    }
}

#[allow(clippy::arc_with_non_send_sync)]

fn reduce_knob_message(
    state: &mut PumpEditorState,
    target: NumericEntryTarget,
    message: KnobMessage,
) {
    match message {
        KnobMessage::GestureStarted => {
            if state.active_knob_gesture.is_none()
                && state
                    .host_param_edit_sink
                    .gesture_started(&state.automation_config, target.param_id())
            {
                state.push_history();
                state.active_knob_gesture = Some(target);
            }
        }
        KnobMessage::ValueChanged { value } => {
            if state.active_knob_gesture == Some(target) {
                let (param_id, plain_value) = knob_plain_value(target, value);
                if state.host_param_edit_sink.gesture_value(
                    &state.automation_config,
                    param_id,
                    plain_value as f64,
                ) {
                    let _ = set_knob_param(state.params.as_ref(), target, value);
                }
            }
        }
        KnobMessage::GestureEnded => {
            if state.active_knob_gesture == Some(target) {
                let _ = state
                    .host_param_edit_sink
                    .gesture_ended(&state.automation_config, target.param_id());
                state.active_knob_gesture = None;
            }
        }
        KnobMessage::Reset { value } => {
            if state.active_knob_gesture == Some(target) {
                let _ = state
                    .host_param_edit_sink
                    .gesture_ended(&state.automation_config, target.param_id());
                state.active_knob_gesture = None;
            }
            let (param_id, plain_value) = knob_plain_value(target, value);
            if state.host_param_edit_sink.edit(
                &state.automation_config,
                param_id,
                plain_value as f64,
            ) {
                state.push_history();
                let _ = set_knob_param(state.params.as_ref(), target, value);
            }
        }
        KnobMessage::Discrete { value } => {
            if !state
                .host_param_edit_sink
                .gesture_started(&state.automation_config, target.param_id())
            {
                return;
            }
            state.push_history();
            let (param_id, plain_value) = knob_plain_value(target, value);
            if state.host_param_edit_sink.gesture_value(
                &state.automation_config,
                param_id,
                plain_value as f64,
            ) {
                let _ = set_knob_param(state.params.as_ref(), target, value);
            }
            let _ = state
                .host_param_edit_sink
                .gesture_ended(&state.automation_config, target.param_id());
        }
    }
}

fn knob_plain_value(target: NumericEntryTarget, value: f32) -> (ClapId, f32) {
    match target {
        NumericEntryTarget::Mix => (PARAM_MIX_ID, value),
        NumericEntryTarget::OutputGain => (PARAM_OUTPUT_GAIN_ID, denormalize_output_gain(value)),
        NumericEntryTarget::Smooth => (PARAM_SMOOTH_ID, value),
        NumericEntryTarget::Swing => (PARAM_SWING_ID, value),
        NumericEntryTarget::FreeRate => (
            PARAM_FREE_RATE_ID,
            plain_from_normalized_value(PARAM_FREE_RATE_ID, value as f64)
                .unwrap_or(DEFAULT_FREE_RATE_HZ as f64) as f32,
        ),
        NumericEntryTarget::Delay => (
            PARAM_DELAY_ID,
            plain_from_normalized_value(PARAM_DELAY_ID, value as f64).unwrap_or(0.0) as f32,
        ),
    }
}

fn set_knob_param(params: &PumpParams, target: NumericEntryTarget, value: f32) -> (ClapId, f32) {
    let (param_id, plain_value) = knob_plain_value(target, value);
    match target {
        NumericEntryTarget::Mix => params.set_mix(value),
        NumericEntryTarget::OutputGain => params.set_output_gain_db(plain_value),
        NumericEntryTarget::Smooth => params.set_smooth(value),
        NumericEntryTarget::Swing => params.set_swing(value),
        NumericEntryTarget::FreeRate => params.set_free_rate_hz(plain_value),
        NumericEntryTarget::Delay => params.set_delay_beats(plain_value),
    }
    (param_id, plain_value)
}

fn format_free_rate_for_unit(rate_hz: f32, unit: FreeRateUnit) -> String {
    let value = unit.value(rate_hz);
    match unit {
        FreeRateUnit::Milliseconds => format!("{value:.1} ms"),
        FreeRateUnit::Seconds => format!("{value:.3} s"),
        FreeRateUnit::Hertz => format!("{value:.2} Hz"),
        FreeRateUnit::Kilohertz => format!("{value:.3} kHz"),
    }
}

fn format_delay_beats_for_ui(delay: usize) -> String {
    match delay {
        1 => "1 beat".to_string(),
        delay => format!("{delay} beats"),
    }
}

fn reduce_editor_message(state: &mut PumpEditorState, message: EditorMessage) {
    match message {
        EditorMessage::Undo => {
            state.active_curve_paint = None;
            state.undo();
        }
        EditorMessage::Redo => {
            state.active_curve_paint = None;
            state.redo();
        }
        EditorMessage::ToggleTimingMode => {
            state.push_history();
            state.timing_dropdown_open = false;
            state.numeric_entry = None;
            let timing_mode = if state.params.timing_mode() == TIMING_MODE_FREE {
                TIMING_MODE_SYNC
            } else {
                TIMING_MODE_FREE
            };
            state.params.set_timing_mode(timing_mode as f32);
            push_param_update(state, PARAM_TIMING_MODE_ID, timing_mode as f64);
        }
        EditorMessage::ToggleTimingDropdown => {
            state.timing_dropdown_open = !state.timing_dropdown_open;
        }
        EditorMessage::ToggleWaveformMode => {
            state
                .status
                .set_waveform_live_mode(!state.status.waveform_live_mode());
        }
        EditorMessage::ToggleHotkeyHelp => {
            state.hotkey_help_open = !state.hotkey_help_open;
            if state.hotkey_help_open {
                state.timing_dropdown_open = false;
            }
        }
        EditorMessage::Knob { target, message } => {
            reduce_knob_message(state, target, message);
        }
        EditorMessage::SyncDivision(value) => {
            state.push_history();
            state.timing_dropdown_open = false;
            let value = (value.clamp(0.0, 1.0) * MAX_SYNC_DIVISION).round();
            state.params.set_sync_division(value);
            push_param_update(state, PARAM_SYNC_DIVISION_ID, value as f64);
        }
        EditorMessage::FreeRateUnit(unit) => {
            state.free_rate_unit = unit;
            state.timing_dropdown_open = false;
        }
        EditorMessage::SelectSound { side, copy } => {
            if copy {
                let active = state.params.active_sound();
                if side == active.other() {
                    let before = state.snapshot();
                    if state.params.copy_active_to_inactive() {
                        state.push_history_snapshot(before);
                        state.clear_curve_selection();
                        state.ab_confirmation =
                            Some(format!("Copied {} → {}", active.label(), side.label()));
                    }
                }
                return;
            }
            if state.params.active_sound() != side {
                state.push_history();
                if state.params.set_active_sound(side) {
                    state.clear_curve_selection();
                    state.ab_confirmation = Some(format!("Switched to sound {}", side.label()));
                    push_param_update(state, PARAM_SOUND_ID, side.index() as f64);
                }
            }
        }
        EditorMessage::CopyAndSelectSound(side) => {
            let active = state.params.active_sound();
            if active == side {
                return;
            }

            let before = state.snapshot();
            let copied = state.params.copy_active_to_inactive();
            if !state.params.set_active_sound(side) {
                return;
            }

            state.push_history_snapshot(before);
            state.clear_curve_selection();
            state.ab_confirmation = Some(if copied {
                format!(
                    "Copied {} → {}; switched to sound {}",
                    active.label(),
                    side.label(),
                    side.label()
                )
            } else {
                format!("Switched to sound {}", side.label())
            });
            push_param_update(state, PARAM_SOUND_ID, side.index() as f64);
        }
        EditorMessage::ToggleBypass => {
            if try_toggle_bypass(
                state.params.as_ref(),
                state.host_param_edit_sink.as_ref(),
                &state.automation_config,
            ) {
                #[cfg(test)]
                {
                    state.automation_flush_count += 1;
                }
            }
        }
        EditorMessage::Curve(message) => {
            if matches!(
                message,
                CurvePreviewMessage::PressNode { .. }
                    | CurvePreviewMessage::PressCurveOffset { .. }
                    | CurvePreviewMessage::ResetCurveOffset
                    | CurvePreviewMessage::InsertNode { .. }
                    | CurvePreviewMessage::DeleteNode { .. }
                    | CurvePreviewMessage::DeleteSelectedNodes
                    | CurvePreviewMessage::PressSegment { .. }
                    | CurvePreviewMessage::PressSegmentMove { .. }
            ) {
                state.push_history();
            }
            reduce_curve_message(state, message)
        }
        EditorMessage::CurveSlot(message) => reduce_curve_slot_message(state, message),
        EditorMessage::NumericEntry(message) => reduce_numeric_entry_message(state, message),
    }
}

fn reduce_curve_slot_message(state: &mut PumpEditorState, message: CurveSlotMessage) {
    match message {
        CurveSlotMessage::Load { index } => {
            let Some(curve) = state.params.global_curve_slot_curve(index) else {
                return;
            };
            state.params.set_editable_curve_preserving_phase(&curve);
            state.clear_curve_selection();
            state.active_curve_node = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
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

fn reduce_numeric_entry_message(state: &mut PumpEditorState, message: NumericEntryMessage) {
    match message {
        NumericEntryMessage::Begin { target } => {
            let value = target.current_plain_value(state.params.as_ref());
            let draft = if target == NumericEntryTarget::FreeRate {
                format_free_rate_for_unit(value as f32, state.free_rate_unit)
            } else {
                format_plain_value_text(target.param_id(), value)
                    .unwrap_or_else(|| value.to_string())
            };
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
                let current = target.current_plain_value(state.params.as_ref());
                let applied = if target == NumericEntryTarget::Delay {
                    clamp_delay_beats(value as f32) as f64
                } else {
                    value
                };
                if (applied - current).abs() > f64::EPSILON {
                    state.push_history();
                }
                apply_numeric_entry_value(state, target, applied);
                state.numeric_entry = None;
            }
        }
        NumericEntryMessage::Step { target, delta } => {
            if target != NumericEntryTarget::Delay
                || !state
                    .numeric_entry
                    .as_ref()
                    .is_some_and(|entry| entry.target == target)
            {
                return;
            }

            let current = state.params.delay_beats();
            let magnitude = delta.unsigned_abs() as usize;
            let next = if delta.is_negative() {
                current.saturating_sub(magnitude)
            } else {
                current.saturating_add(magnitude)
            }
            .clamp(MIN_DELAY_BEATS, MAX_DELAY_BEATS);
            if next == current {
                return;
            }

            state.push_history();
            apply_numeric_entry_value(state, target, next as f64);
            state.numeric_entry = Some(NumericEntryState {
                target,
                draft: format_delay_beats_for_ui(next),
                dirty: false,
            });
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

fn apply_numeric_entry_value(state: &mut PumpEditorState, target: NumericEntryTarget, value: f64) {
    let value = if target == NumericEntryTarget::Delay {
        clamp_delay_beats(value as f32) as f64
    } else {
        value
    };
    match target {
        NumericEntryTarget::Mix => state.params.set_mix(value as f32),
        NumericEntryTarget::OutputGain => state.params.set_output_gain_db(value as f32),
        NumericEntryTarget::Smooth => state.params.set_smooth(value as f32),
        NumericEntryTarget::Swing => state.params.set_swing(value as f32),
        NumericEntryTarget::FreeRate => state.params.set_free_rate_hz(value as f32),
        NumericEntryTarget::Delay => state.params.set_delay_beats(value as f32),
    }

    push_param_update(state, target.param_id(), value);
}

/// Queue one complete discrete gesture and flush it once.
///
/// Pointer drags currently arrive as discrete reducer messages, so each update
/// is represented by its own begin/value/end batch. A future pointer-stream
/// reducer can coalesce these into one host gesture without changing the queue
/// contract here.
fn push_param_update(state: &mut PumpEditorState, param_id: ClapId, value: f64) {
    if state
        .host_param_edit_sink
        .edit(&state.automation_config, param_id, value)
    {
        #[cfg(test)]
        {
            state.automation_flush_count += 1;
        }
    }
}

/// Ask the host to accept the next bypass value before changing shared state.
pub(crate) fn try_toggle_bypass(
    params: &PumpParams,
    sink: &dyn HostParamEditSink,
    config: &AutomationConfig,
) -> bool {
    let value = if params.bypassed() {
        BYPASS_ACTIVE_VALUE
    } else {
        BYPASS_BYPASSED_VALUE
    };
    if !sink.edit(config, PARAM_BYPASS_ID, value as f64) {
        return false;
    }
    params.set_bypass(value);
    true
}

fn reduce_curve_message(state: &mut PumpEditorState, message: CurvePreviewMessage) {
    match message {
        CurvePreviewMessage::Hover {
            node,
            preview_node,
            segment,
        } => {
            state.hover_curve_node = node;
            state.preview_curve_node = preview_node;
            state.hover_curve_segment = segment;
            state.hover_curve_segment_zone = None;
        }
        CurvePreviewMessage::HoverProximitySegment { index } => {
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = Some(index);
            state.hover_curve_segment_zone = Some(CurveSegmentHitZone::OuterProximity);
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
            if let Some(drag) = state.active_curve_offset.as_mut() {
                drag.quantized = option_held;
                let raw_delta = drag.raw_delta;
                let phase_offset = resolve_curve_offset(
                    state.params.sync_division(),
                    (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0),
                    state.params.swing(),
                    drag.origin_phase_offset,
                    raw_delta,
                    drag.quantized,
                );
                if state.host_param_edit_sink.gesture_value(
                    &state.automation_config,
                    PARAM_PHASE_OFFSET_ID,
                    phase_offset as f64,
                ) {
                    state.params.set_phase_offset(phase_offset);
                }
            }
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
                if state.active_curve_segment.as_ref().is_some_and(|drag| {
                    drag.mode == CurveSegmentDragMode::MovePair
                        && drag.source == CurveSegmentDragSource::Command
                }) {
                    state.active_curve_segment = None;
                }
                state.hover_curve_segment = None;
                state.hover_curve_segment_zone = None;
            }
        }
        CurvePreviewMessage::PressPaint { sample } => {
            let mut paint = ActiveCurvePaint::new(state.snapshot(), state.params.phase_offset());
            paint.push_sample(sample);
            state.clear_curve_selection();
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.active_curve_marquee = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
            state.active_curve_paint = Some(paint);
        }
        CurvePreviewMessage::DragPaint { sample } => {
            if let Some(paint) = state.active_curve_paint.as_mut() {
                paint.push_sample(sample);
            }
        }
        CurvePreviewMessage::DragPaintOutside { sample } => {
            if let Some(paint) = state.active_curve_paint.as_mut() {
                paint.push_boundary_sample(sample);
            }
        }
        CurvePreviewMessage::ReleasePaint { sample } => {
            if let Some(mut paint) = state.active_curve_paint.take() {
                if let Some(sample) = sample {
                    paint.push_sample(sample);
                }
                commit_active_curve_paint(state, paint);
            }
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.active_curve_marquee = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
        }
        CurvePreviewMessage::ReleasePaintOutside { sample } => {
            if let Some(mut paint) = state.active_curve_paint.take() {
                paint.push_boundary_sample(sample);
                commit_active_curve_paint(state, paint);
            }
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.active_curve_marquee = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
        }
        CurvePreviewMessage::PressNode {
            index,
            pointer,
            shift_held,
            option_held,
            command_held,
        } => {
            let curve = state.params.editable_curve_snapshot();
            let survivors = interactive_curve_node_survivors(
                &curve,
                state.params.phase_offset(),
                state.active_curve_node,
            );
            if !survivors.contains(&index) {
                state.clear_curve_selection();
                return;
            }
            let selected_indices = if state.selected_curve_nodes.contains(&index) {
                state
                    .selected_curve_nodes
                    .iter()
                    .copied()
                    .filter(|selected| survivors.contains(selected))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            if selected_indices.is_empty() {
                state.clear_curve_selection();
            }
            if let Some(mut drag) = start_curve_node_drag(
                &curve,
                index,
                pointer,
                shift_held,
                option_held,
                state.params.phase_offset(),
            ) {
                drag.selected_indices = selected_indices;
                if drag.seam_drag.is_none() {
                    if let Some(owner) = canonical_seam_owner(&curve, state.params.phase_offset()) {
                        let selected_seam = drag
                            .selected_indices
                            .iter()
                            .copied()
                            .any(|selected| seam_owner_contains_index(&curve, owner, selected));
                        if selected_seam {
                            drag.seam_drag = Some(CanonicalSeamDrag {
                                kind: CanonicalSeamDragKind::ExistingOwner,
                                owner,
                                template_curve: curve.clone().normalized(),
                            });
                        }
                    }
                }
                state.option_hover_held = option_held;
                state.command_hover_held = command_held;
                state.active_curve_node = Some(index);
                state.active_curve_node_drag = Some(drag);
                state.active_curve_paint = None;
                state.active_curve_segment = None;
                state.active_curve_offset = None;
                state.preview_curve_offset = None;
                state.shift_hover_held = shift_held;
                state.hover_curve_node = Some(index);
                state.preview_curve_node = None;
                state.clear_curve_segment_hover();
            }
        }
        CurvePreviewMessage::PressMarquee { start } => {
            state.active_curve_marquee = Some(ActiveCurveMarquee {
                start,
                current: start,
            });
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
        }
        CurvePreviewMessage::DragMarquee { current } => {
            if let Some(marquee) = state.active_curve_marquee.as_mut() {
                marquee.current = current;
            }
        }
        CurvePreviewMessage::ReleaseMarquee { current } => {
            let Some(marquee) = state.active_curve_marquee.take() else {
                return;
            };
            let min_y = marquee.start.y.min(current.y);
            let max_y = marquee.start.y.max(current.y);
            let curve = state.params.editable_curve_snapshot();
            let phase_offset = state.params.phase_offset();
            let survivors = interactive_curve_node_survivors(&curve, phase_offset, None);
            let start_x = CurveGeometry::display_phase(marquee.start.x, phase_offset);
            let current_x = CurveGeometry::display_phase(current.x, phase_offset);
            let min_display_x = start_x.min(current_x);
            let max_display_x = start_x.max(current_x);
            state.selected_curve_nodes = curve
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(index, node)| {
                    let display_x = CurveGeometry::display_phase(node.x, phase_offset);
                    (survivors.contains(&index)
                        && display_x >= min_display_x
                        && display_x <= max_display_x
                        && node.y >= min_y
                        && node.y <= max_y)
                        .then_some(index)
                })
                .collect();
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
        }
        CurvePreviewMessage::PressCurveOffset {
            pointer_x,
            quantized,
        } => {
            if !state
                .host_param_edit_sink
                .gesture_started(&state.automation_config, PARAM_PHASE_OFFSET_ID)
            {
                return;
            }
            state.clear_curve_selection();
            state.active_curve_offset = Some(ActiveCurveOffsetDrag {
                origin_phase_offset: state.params.phase_offset(),
                start_pointer_x: pointer_x,
                raw_delta: 0.0,
                quantized,
            });
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
            state.command_hover_held = true;
            state.shift_hover_held = true;
        }
        CurvePreviewMessage::ResetCurveOffset => {
            if state.params.phase_offset().abs() <= f32::EPSILON
                || !state
                    .host_param_edit_sink
                    .gesture_started(&state.automation_config, PARAM_PHASE_OFFSET_ID)
            {
                return;
            }
            if state.host_param_edit_sink.gesture_value(
                &state.automation_config,
                PARAM_PHASE_OFFSET_ID,
                0.0,
            ) {
                state.params.set_phase_offset(0.0);
            }
            let _ = state
                .host_param_edit_sink
                .gesture_ended(&state.automation_config, PARAM_PHASE_OFFSET_ID);
        }
        CurvePreviewMessage::InsertNode { node, command_held } => {
            let mut curve = state.params.editable_curve_snapshot();
            state.clear_curve_selection();
            state.command_hover_held = command_held;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
            if let Some(index) = insert_curve_node(&mut curve, node) {
                state.params.set_editable_curve(&curve);
                state.active_curve_node = Some(index);
                state.active_curve_node_drag = start_curve_node_drag(
                    &curve,
                    index,
                    node,
                    false,
                    false,
                    state.params.phase_offset(),
                );
                state.hover_curve_node = Some(index);
            }
        }
        CurvePreviewMessage::DeleteNode { index } => {
            let mut curve = state.params.editable_curve_snapshot();
            if !interactive_curve_node_survivors(&curve, state.params.phase_offset(), None)
                .contains(&index)
            {
                return;
            }
            state.clear_curve_selection();
            if delete_curve_node(&mut curve, index) {
                state.params.set_editable_curve(&curve);
            }
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
        }
        CurvePreviewMessage::DeleteSelectedNodes => {
            let mut curve = state.params.editable_curve_snapshot();
            let deleted = delete_selected_curve_nodes(
                &mut curve,
                &state.selected_curve_nodes,
                state.params.phase_offset(),
            );
            if deleted {
                state.params.set_editable_curve(&curve);
            }
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.active_curve_marquee = None;
            state.preview_curve_offset = None;
            state.selected_curve_nodes.clear();
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
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
            let (curve, moved_index, seam_latched) =
                if let Some(drag) = state.active_curve_node_drag.as_mut() {
                    let target = drag.target_for_pointer(
                        node,
                        CurvePointDragModifiers {
                            shift_held,
                            option_held,
                            command_held,
                        },
                        state.params.sync_division(),
                        curve_width_from_push_through_threshold_x(push_through_threshold_x),
                        state.params.swing(),
                        current,
                    );
                    curve_with_dragged_node(
                        drag,
                        node,
                        target,
                        push_through_threshold_x,
                        state.params.phase_offset(),
                    )
                } else {
                    let mut curve = current_curve;
                    let mut target = node;
                    if command_held {
                        target.x = snap_curve_time_to_beat_grid_with_swing(
                            state.params.sync_division(),
                            curve_width_from_push_through_threshold_x(push_through_threshold_x),
                            target.x,
                            state.params.swing(),
                        );
                    }
                    let moved_index = update_curve_node(&mut curve, index, target);
                    (curve, moved_index, false)
                };
            state.params.set_editable_curve(&curve);
            if seam_latched
                && state
                    .active_curve_node_drag
                    .as_ref()
                    .is_some_and(|drag| drag.selected_indices.is_empty())
            {
                state.clear_curve_selection();
            }
            state.active_curve_node = Some(moved_index);
            state.active_curve_segment = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = Some(moved_index);
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
        }
        CurvePreviewMessage::DragCurveOffset { delta } => {
            let Some(drag) = state.active_curve_offset.as_mut() else {
                return;
            };
            drag.raw_delta = delta;
            let phase_offset = resolve_curve_offset(
                state.params.sync_division(),
                (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0),
                state.params.swing(),
                drag.origin_phase_offset,
                delta,
                drag.quantized,
            );
            if state.host_param_edit_sink.gesture_value(
                &state.automation_config,
                PARAM_PHASE_OFFSET_ID,
                phase_offset as f64,
            ) {
                state.params.set_phase_offset(phase_offset);
            }
            state.active_curve_node = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
        }
        CurvePreviewMessage::ReleaseCurveOffset { delta, option_held } => {
            if let Some(mut drag) = state.active_curve_offset.take() {
                drag.raw_delta = delta;
                drag.quantized = option_held;
                let phase_offset = resolve_curve_offset(
                    state.params.sync_division(),
                    (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0),
                    state.params.swing(),
                    drag.origin_phase_offset,
                    delta,
                    drag.quantized,
                );
                if state.host_param_edit_sink.gesture_value(
                    &state.automation_config,
                    PARAM_PHASE_OFFSET_ID,
                    phase_offset as f64,
                ) {
                    state.params.set_phase_offset(phase_offset);
                }
                let _ = state
                    .host_param_edit_sink
                    .gesture_ended(&state.automation_config, PARAM_PHASE_OFFSET_ID);
            }
            state.preview_curve_offset = None;
            state.option_hover_held = option_held;
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
        }
        CurvePreviewMessage::ReleaseNode {
            index,
            node,
            push_through_threshold_x,
            shift_held,
            option_held,
            command_held,
        } => {
            let grouped_drag = state
                .active_curve_node_drag
                .as_ref()
                .is_some_and(|drag| !drag.selected_indices.is_empty());
            if !grouped_drag {
                state.clear_curve_selection();
            }
            let current_curve = state.params.editable_curve_snapshot();
            let current = state
                .active_curve_node
                .and_then(|active_index| current_curve.nodes.get(active_index))
                .copied()
                .unwrap_or(node);
            let (curve, moved_index, seam_latched) =
                if let Some(drag) = state.active_curve_node_drag.as_mut() {
                    let target = drag.target_for_pointer(
                        node,
                        CurvePointDragModifiers {
                            shift_held,
                            option_held,
                            command_held,
                        },
                        state.params.sync_division(),
                        curve_width_from_push_through_threshold_x(push_through_threshold_x),
                        state.params.swing(),
                        current,
                    );
                    curve_with_dragged_node(
                        drag,
                        node,
                        target,
                        push_through_threshold_x,
                        state.params.phase_offset(),
                    )
                } else {
                    let mut curve = current_curve;
                    let mut target = node;
                    if command_held {
                        target.x = snap_curve_time_to_beat_grid_with_swing(
                            state.params.sync_division(),
                            curve_width_from_push_through_threshold_x(push_through_threshold_x),
                            target.x,
                            state.params.swing(),
                        );
                    }
                    let moved_index = update_curve_node(&mut curve, index, target);
                    (curve, moved_index, false)
                };
            state.params.set_editable_curve(&curve);
            if seam_latched {
                state.clear_curve_selection();
            }
            state.option_hover_held = option_held;
            state.command_hover_held = command_held;
            state.shift_hover_held = shift_held;
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = Some(moved_index);
            state.preview_curve_node = None;
            state.clear_curve_segment_hover();
        }
        CurvePreviewMessage::PressSegment { index, position } => {
            let curve = state.params.editable_curve_snapshot();
            if let Some(drag) = start_curve_segment_tension_drag(&curve, index, position) {
                state.active_curve_node = None;
                state.active_curve_node_drag = None;
                state.active_curve_paint = None;
                state.active_curve_offset = None;
                state.preview_curve_offset = None;
                state.active_curve_segment = Some(drag);
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = Some(index);
                state.hover_curve_segment_zone = None;
            }
        }
        CurvePreviewMessage::PressSegmentMove { index, position } => {
            let curve = state.params.editable_curve_snapshot();
            if let Some(drag) = start_curve_segment_move_drag(&curve, index, position) {
                state.active_curve_node = None;
                state.active_curve_node_drag = None;
                state.active_curve_paint = None;
                state.active_curve_offset = None;
                state.preview_curve_offset = None;
                state.active_curve_segment = Some(drag);
                state.command_hover_held = true;
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = Some(index);
                state.hover_curve_segment_zone = None;
            }
        }
        CurvePreviewMessage::PressDirectProximitySegment { index, position } => {
            let curve = state.params.editable_curve_snapshot();
            if let Some(mut drag) =
                start_curve_segment_direct_proximity_drag(&curve, index, position)
            {
                drag.history_origin = Some(state.snapshot());
                state.active_curve_node = None;
                state.active_curve_node_drag = None;
                state.active_curve_paint = None;
                state.active_curve_offset = None;
                state.preview_curve_offset = None;
                state.active_curve_segment = Some(drag);
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = Some(index);
                state.hover_curve_segment_zone = None;
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
                state.hover_curve_segment_zone = None;
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
                commit_direct_curve_segment_history_if_changed(state, &drag);
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = match drag.source {
                    CurveSegmentDragSource::Command => {
                        state.command_hover_held.then_some(drag.index)
                    }
                    CurveSegmentDragSource::OptionTension => {
                        state.option_hover_held.then_some(drag.index)
                    }
                    CurveSegmentDragSource::DirectProximity => None,
                };
                state.hover_curve_segment_zone = None;
            }
        }
        CurvePreviewMessage::Cancel => {
            state.active_curve_paint = None;
            if let Some(drag) = state.active_curve_offset.take() {
                if state.params.phase_offset() != drag.origin_phase_offset
                    && state.host_param_edit_sink.gesture_value(
                        &state.automation_config,
                        PARAM_PHASE_OFFSET_ID,
                        drag.origin_phase_offset as f64,
                    )
                {
                    state.params.set_phase_offset(drag.origin_phase_offset);
                }
                let _ = state
                    .host_param_edit_sink
                    .gesture_ended(&state.automation_config, PARAM_PHASE_OFFSET_ID);
            }
            if let Some(drag) = state.active_curve_segment.take() {
                commit_direct_curve_segment_history_if_changed(state, &drag);
            }
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_marquee = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            state.hover_curve_segment_zone = None;
            state.option_hover_held = false;
            state.command_hover_held = false;
            state.shift_hover_held = false;
        }
    }
}

fn commit_active_curve_paint(state: &mut PumpEditorState, paint: ActiveCurvePaint) {
    let candidate = match paint.finished_curve() {
        PaintCommitOutcome::Applied { candidate } | PaintCommitOutcome::NoOp { candidate } => {
            candidate
        }
    };
    if candidate == paint.origin_curve {
        return;
    }

    state.params.set_editable_curve(&candidate);
    let mut origin_snapshot = paint.origin_snapshot;
    origin_snapshot.curve = paint.origin_curve;
    state.push_history_snapshot(origin_snapshot);
}

fn commit_direct_curve_segment_history_if_changed(
    state: &mut PumpEditorState,
    drag: &ActiveCurveSegmentDrag,
) {
    if drag.source != CurveSegmentDragSource::DirectProximity {
        return;
    }

    let applied_curve = state.params.editable_curve_snapshot();
    if applied_curve == drag.origin_curve {
        return;
    }

    let Some(mut origin_snapshot) = drag.history_origin.clone() else {
        return;
    };
    origin_snapshot.curve = drag.origin_curve.clone();
    state.push_history_snapshot(origin_snapshot);
}

fn start_curve_node_drag(
    curve: &EditableCurve,
    index: usize,
    pointer: CurveNode,
    shift_held: bool,
    option_held: bool,
    phase_offset: f32,
) -> Option<ActiveCurveNodeDrag> {
    let normalized = curve.clone().normalized();
    let origin = *normalized.nodes.get(index)?;
    let vertical_active = shift_held && option_held;
    let horizontal_active = shift_held && !vertical_active;
    Some(ActiveCurveNodeDrag {
        origin_index: index,
        origin_curve: normalized,
        selected_indices: Vec::new(),
        seam_drag: canonical_seam_owner(curve, phase_offset).and_then(|owner| {
            seam_owner_contains_index(curve, owner, index).then(|| CanonicalSeamDrag {
                kind: CanonicalSeamDragKind::ExistingOwner,
                owner,
                template_curve: curve.clone().normalized(),
            })
        }),
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
    drag: &mut ActiveCurveNodeDrag,
    pointer: CurveNode,
    target: CurveNode,
    push_through_threshold_x: f32,
    phase_offset: f32,
) -> (EditableCurve, usize, bool) {
    let single_selected_source = drag.selected_indices.len() == 1
        && drag.selected_indices.first().copied() == Some(drag.origin_index);
    if !drag.selected_indices.is_empty() && !single_selected_source {
        let (curve, index) = curve_with_dragged_selected_nodes(drag, target, phase_offset);
        return (curve, index, false);
    }

    if let Some(seam_drag) = drag.seam_drag.as_mut() {
        if seam_drag.kind == CanonicalSeamDragKind::Takeover
            && !display_x_is_at_viewport_boundary(pointer.x, phase_offset)
        {
            // A takeover remains active only at the exact viewport boundary.
            // Rebuild from the gesture origin as soon as the pointer moves
            // inward, even if it is still within the push-through edge zone,
            // so removed neighbours and every original segment tension return
            // before following the pointer position.
            drag.seam_drag = None;
        } else {
            let mut curve = seam_drag.template_curve.clone();
            let moved_index =
                preferred_seam_active_index(&curve, seam_drag.owner, target, phase_offset);
            set_canonical_seam_y(&mut curve, seam_drag.owner, target.y);
            curve.normalize_in_place();
            return (curve, moved_index, false);
        }
    }

    let mut curve = drag.origin_curve.clone();
    if drag.origin_index > 0
        && drag.origin_index + 1 < curve.nodes.len()
        && !display_x_is_in_edge_zone(
            curve.nodes[drag.origin_index].x,
            phase_offset,
            push_through_threshold_x,
        )
        && drag.vertical_time_anchor.is_none()
        && display_x_is_at_viewport_boundary(pointer.x, phase_offset)
    {
        let (curve, owner) = take_over_viewport_seam(
            &curve,
            drag.origin_index,
            target.y,
            phase_offset,
            push_through_threshold_x,
        );
        let moved_index = preferred_seam_active_index(&curve, owner, pointer, phase_offset);
        drag.seam_drag = Some(CanonicalSeamDrag {
            kind: CanonicalSeamDragKind::Takeover,
            owner,
            template_curve: curve.clone(),
        });
        if single_selected_source {
            // A selected source becomes one logical seam node after takeover;
            // the old index and marquee selection must not survive the remap.
            drag.selected_indices.clear();
        }
        return (curve, moved_index, true);
    }

    let moved_index = move_curve_node_with_push_through(
        &mut curve,
        drag.origin_index,
        target,
        push_through_threshold_x,
    );
    curve.normalize_in_place();
    (curve, moved_index, false)
}

fn set_canonical_seam_y(curve: &mut EditableCurve, owner: CanonicalSeamOwner, y: f32) {
    let y = y.clamp(0.0, 1.0);
    match owner {
        CanonicalSeamOwner::Endpoints => set_wrapped_curve_endpoint_y(curve, y),
        CanonicalSeamOwner::Interior(index) => {
            if let Some(node) = curve.nodes.get_mut(index) {
                node.y = y;
            }
        }
    }
}

fn take_over_viewport_seam(
    origin: &EditableCurve,
    source_index: usize,
    y: f32,
    phase_offset: f32,
    push_through_threshold_x: f32,
) -> (EditableCurve, CanonicalSeamOwner) {
    let mut curve = origin.clone().normalized();
    let seam = seam_raw(phase_offset);
    let source_left_tension = curve
        .segments
        .get(source_index.saturating_sub(1))
        .copied()
        .unwrap_or(CurveSegment { tension: 0.0 })
        .tension;
    let source_right_tension = curve
        .segments
        .get(source_index)
        .copied()
        .unwrap_or(CurveSegment { tension: 0.0 })
        .tension;
    let owner = if let Some(existing_owner) = canonical_seam_owner(&curve, phase_offset) {
        existing_owner
    } else {
        if let Some(node) = curve.nodes.get_mut(source_index) {
            node.x = seam;
            node.y = y.clamp(0.0, 1.0);
        }
        // Re-evaluate only after the source has been put at the exact raw
        // seam. Nearby interior nodes must not become owners by tolerance.
        canonical_seam_owner(&curve, phase_offset)
            .unwrap_or(CanonicalSeamOwner::Interior(source_index))
    };

    let owner_index = match owner {
        CanonicalSeamOwner::Interior(index) => Some(index),
        CanonicalSeamOwner::Endpoints => None,
    };
    let mut removals: Vec<usize> = curve
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            if index == 0 || index + 1 == curve.nodes.len() {
                return None;
            }
            if owner_index == Some(index) {
                return None;
            }
            let edge_competitor =
                display_x_is_in_edge_zone(node.x, phase_offset, push_through_threshold_x);
            (edge_competitor || index == source_index).then_some(index)
        })
        .collect();
    let removed_before_owner = owner_index
        .map(|index| removals.iter().filter(|remove| **remove < index).count())
        .unwrap_or_else(|| {
            removals
                .iter()
                .filter(|remove| **remove < source_index)
                .count()
        });
    removals.sort_unstable_by(|left, right| right.cmp(left));
    for remove_index in removals {
        remove_interior_curve_node(&mut curve, remove_index);
    }

    let owner = match owner {
        CanonicalSeamOwner::Endpoints => CanonicalSeamOwner::Endpoints,
        CanonicalSeamOwner::Interior(mut index) => {
            index = index.saturating_sub(removed_before_owner);
            CanonicalSeamOwner::Interior(index)
        }
    };

    match owner {
        CanonicalSeamOwner::Endpoints => {
            set_wrapped_curve_endpoint_y(&mut curve, y);
            if let Some(first) = curve.segments.first_mut() {
                first.tension = source_right_tension;
            }
            if let Some(last) = curve.segments.last_mut() {
                last.tension = source_left_tension;
            }
        }
        CanonicalSeamOwner::Interior(index) => {
            if let Some(node) = curve.nodes.get_mut(index) {
                node.x = seam;
                node.y = y.clamp(0.0, 1.0);
            }
            if let Some(left) = curve.segments.get_mut(index.saturating_sub(1)) {
                left.tension = source_left_tension;
            }
            if let Some(right) = curve.segments.get_mut(index) {
                right.tension = source_right_tension;
            }
        }
    }
    curve.normalize_in_place();
    (curve, owner)
}

/// Apply one pointer delta to all nodes in a retained marquee selection.
///
/// Unlike the single-node gesture, grouped movement never pushes through or
/// removes neighbours. The common x delta is clipped against every unselected
/// node (and the fixed endpoints), so the original ordering and minimum
/// spacing remain intact for the duration of the gesture.
fn curve_with_dragged_selected_nodes(
    drag: &ActiveCurveNodeDrag,
    target: CurveNode,
    phase_offset: f32,
) -> (EditableCurve, usize) {
    let mut curve = drag.origin_curve.clone();
    let Some(anchor) = drag.origin_curve.nodes.get(drag.origin_index).copied() else {
        return (curve, drag.origin_index);
    };
    let selected: std::collections::HashSet<usize> = drag
        .selected_indices
        .iter()
        .copied()
        .filter(|index| *index < drag.origin_curve.nodes.len())
        .collect();
    if selected.is_empty() {
        return (curve, drag.origin_index);
    }

    let mut min_delta = -1.0_f32;
    let mut max_delta = 1.0_f32;
    // The curve normalizer folds coordinates at or inside the structural edge
    // epsilon into the endpoint. Keep grouped interior nodes just outside
    // that band so an endpoint clamp cannot erase a selected node.
    let endpoint_spacing = CURVE_STRUCTURAL_ENDPOINT_RAW_EPSILON + CURVE_SEAM_OWNER_RAW_EPSILON;
    for (index, node) in drag.origin_curve.nodes.iter().enumerate() {
        if !selected.contains(&index) || index == 0 || index + 1 == drag.origin_curve.nodes.len() {
            continue;
        }
        min_delta = min_delta.max(-node.x + endpoint_spacing);
        max_delta = max_delta.min(1.0 - node.x - endpoint_spacing);
        for (other_index, other) in drag.origin_curve.nodes.iter().enumerate() {
            // Endpoints are always fixed in x, even when marquee-selected.
            let other_selected = selected.contains(&other_index)
                && other_index > 0
                && other_index + 1 < drag.origin_curve.nodes.len();
            if other_selected {
                continue;
            }
            let spacing = if other_index == 0 || other_index + 1 == drag.origin_curve.nodes.len() {
                endpoint_spacing
            } else {
                CURVE_NODE_MIN_SPACING_X
            };
            if other_index < index {
                min_delta = min_delta.max(other.x + spacing - node.x);
            } else if other_index > index {
                max_delta = max_delta.min(other.x - spacing - node.x);
            }
        }
    }
    let (min_delta, max_delta) = feasible_delta_interval_including_zero(min_delta, max_delta);
    let delta_x = if drag.seam_drag.is_some() {
        0.0
    } else {
        (target.x - anchor.x).clamp(min_delta, max_delta)
    };
    let delta_x = group_delta_preserving_canonical_seam(
        &drag.origin_curve,
        &selected,
        phase_offset,
        delta_x,
        min_delta,
        max_delta,
    );
    let delta_y = target.y - anchor.y;
    for index in selected.iter().copied() {
        let Some(origin) = drag.origin_curve.nodes.get(index).copied() else {
            continue;
        };
        let x = if index == 0 {
            0.0
        } else if index + 1 == drag.origin_curve.nodes.len() {
            1.0
        } else {
            (origin.x + delta_x).clamp(0.0, 1.0)
        };
        curve.nodes[index] = CurveNode {
            x,
            y: (origin.y + delta_y).clamp(0.0, 1.0),
        };
    }
    if selected.contains(&0) || selected.contains(&(curve.nodes.len().saturating_sub(1))) {
        let endpoint_y = selected
            .iter()
            .find_map(|index| {
                (*index == 0 || *index + 1 == curve.nodes.len()).then_some(curve.nodes[*index].y)
            })
            .unwrap_or(curve.nodes[0].y);
        set_wrapped_curve_endpoint_y(&mut curve, endpoint_y);
    } else {
        enforce_wrapped_curve_endpoints(&mut curve);
    }
    curve.normalize_in_place();
    (curve, drag.origin_index)
}

fn group_delta_preserving_canonical_seam(
    curve: &EditableCurve,
    selected: &std::collections::HashSet<usize>,
    phase_offset: f32,
    requested_delta: f32,
    min_delta: f32,
    max_delta: f32,
) -> f32 {
    let Some(owner) = canonical_seam_owner(curve, phase_offset) else {
        return requested_delta;
    };
    if seam_owner_indices(curve, owner)
        .iter()
        .any(|index| selected.contains(index))
    {
        return 0.0;
    }

    let seam = seam_raw(phase_offset);
    let can_use_delta = |delta: f32| {
        selected.iter().all(|index| {
            if *index == 0 || *index + 1 == curve.nodes.len() {
                return true;
            }
            (curve.nodes[*index].x + delta - seam).abs() > CURVE_SEAM_OWNER_RAW_EPSILON
        })
    };
    if can_use_delta(requested_delta) {
        return requested_delta;
    }

    // Keep the group gesture intact, but do not let a selected member land on
    // an already-owned seam. The spacing clamp normally makes this redundant;
    // the explicit guard also covers legacy/epsilon-close topology.
    let seam_gap = CURVE_SEAM_OWNER_RAW_EPSILON * 2.0;
    let mut candidates = vec![min_delta, max_delta];
    for index in selected.iter().copied() {
        if index == 0 || index + 1 == curve.nodes.len() {
            continue;
        }
        let origin_x = curve.nodes[index].x;
        candidates.push(seam - seam_gap - origin_x);
        candidates.push(seam + seam_gap - origin_x);
    }
    candidates
        .into_iter()
        .filter(|delta| {
            delta.is_finite() && *delta >= min_delta && *delta <= max_delta && can_use_delta(*delta)
        })
        .min_by(|left, right| {
            (left - requested_delta)
                .abs()
                .total_cmp(&(right - requested_delta).abs())
                .then_with(|| left.total_cmp(right))
        })
        .or_else(|| can_use_delta(0.0).then_some(0.0))
        .unwrap_or(requested_delta)
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

    if let Some(edge) = curve_edge_for_x(target.x) {
        remove_interior_curve_node_at_edge(curve, moved_index, edge);
        set_wrapped_curve_endpoint_y(curve, y);
        return edge.index(curve.nodes.len());
    }

    let origin_x = curve.nodes[moved_index].x;
    let preferred_min_x = curve.nodes[moved_index - 1].x + CURVE_NODE_MIN_SPACING_X;
    let preferred_max_x = curve.nodes[moved_index + 1].x - CURVE_NODE_MIN_SPACING_X;
    let (min_x, max_x) =
        feasible_node_x_interval_including_origin(preferred_min_x, preferred_max_x, origin_x);
    curve.nodes[moved_index] = CurveNode {
        x: target.x.clamp(min_x, max_x),
        y,
    };
    enforce_wrapped_curve_endpoints(curve);
    moved_index
}

fn remove_interior_curve_node_at_edge(
    curve: &mut EditableCurve,
    remove_index: usize,
    edge: CurveEdge,
) {
    let preserved_segment_index = match edge {
        CurveEdge::Left => remove_index,
        CurveEdge::Right => remove_index.saturating_sub(1),
    };
    let preserved_tension = curve
        .segments
        .get(preserved_segment_index)
        .copied()
        .map(|segment| segment.tension);
    remove_interior_curve_node_with_merge_tension(curve, remove_index, preserved_tension);
}

fn remove_interior_curve_node(curve: &mut EditableCurve, remove_index: usize) {
    remove_interior_curve_node_with_merge_tension(curve, remove_index, None);
}

fn remove_interior_curve_node_with_merge_tension(
    curve: &mut EditableCurve,
    remove_index: usize,
    preserved_tension: Option<f32>,
) {
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
    let merged_tension = preserved_tension.unwrap_or_else(|| {
        ((left_tension + right_tension) * 0.5).clamp(MIN_SEGMENT_TENSION, MAX_SEGMENT_TENSION)
    });

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CurveEdge {
    Left,
    Right,
}

impl CurveEdge {
    fn index(self, node_count: usize) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => node_count.saturating_sub(1),
        }
    }
}

fn curve_edge_for_x(x: f32) -> Option<CurveEdge> {
    if !x.is_finite() {
        return None;
    }
    if x <= CURVE_NODE_MIN_SPACING_X {
        Some(CurveEdge::Left)
    } else if x >= 1.0 - CURVE_NODE_MIN_SPACING_X {
        Some(CurveEdge::Right)
    } else {
        None
    }
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
        source: CurveSegmentDragSource::OptionTension,
        history_origin: None,
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
        source: CurveSegmentDragSource::Command,
        history_origin: None,
    })
}

fn start_curve_segment_direct_proximity_drag(
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
        source: CurveSegmentDragSource::DirectProximity,
        history_origin: None,
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
    if curve.nodes.len() < 2 {
        return None;
    }
    if let Some(edge) = curve_edge_for_x(node.x) {
        set_wrapped_curve_endpoint_y(curve, node.y);
        curve.normalize_in_place();
        return Some(edge.index(curve.nodes.len()));
    }
    if curve.nodes.len() >= MAX_EDITABLE_NODES {
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

fn delete_selected_curve_nodes(
    curve: &mut EditableCurve,
    selected: &[usize],
    phase_offset: f32,
) -> bool {
    let survivors = interactive_curve_node_survivors(curve, phase_offset, None);
    let mut indices: Vec<usize> = selected
        .iter()
        .copied()
        .filter(|index| survivors.contains(index) && *index > 0 && *index + 1 < curve.nodes.len())
        .collect();
    indices.sort_unstable();
    indices.dedup();
    let mut deleted = false;
    for index in indices.into_iter().rev() {
        deleted |= delete_curve_node(curve, index);
    }
    deleted
}

fn update_curve_node(curve: &mut EditableCurve, index: usize, node: CurveNode) -> usize {
    if curve.nodes.is_empty() || index >= curve.nodes.len() {
        return index;
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
        return index;
    }
    if let Some(edge) = curve_edge_for_x(node.x) {
        remove_interior_curve_node_at_edge(curve, index, edge);
        set_wrapped_curve_endpoint_y(curve, node.y);
        curve.normalize_in_place();
        return edge.index(curve.nodes.len());
    }

    let previous_x = curve.nodes[index - 1].x + CURVE_NODE_MIN_SPACING_X;
    let next_x = curve.nodes[index + 1].x - CURVE_NODE_MIN_SPACING_X;
    if previous_x >= next_x {
        return index;
    }
    curve.nodes[index] = CurveNode {
        x: node.x.clamp(previous_x, next_x),
        y: node.y.clamp(0.0, 1.0),
    };
    curve.normalize_in_place();
    index
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CurvePreviewMessage {
    Hover {
        node: Option<usize>,
        preview_node: Option<CurveNode>,
        segment: Option<usize>,
    },
    HoverProximitySegment {
        index: usize,
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
    PressPaint {
        sample: CurvePaintSample,
    },
    DragPaint {
        sample: CurvePaintSample,
    },
    DragPaintOutside {
        sample: CurvePaintSample,
    },
    ReleasePaint {
        sample: Option<CurvePaintSample>,
    },
    ReleasePaintOutside {
        sample: CurvePaintSample,
    },
    PressMarquee {
        start: CurveNode,
    },
    DragMarquee {
        current: CurveNode,
    },
    ReleaseMarquee {
        current: CurveNode,
    },
    PressCurveOffset {
        pointer_x: f32,
        quantized: bool,
    },
    ResetCurveOffset,
    InsertNode {
        node: CurveNode,
        command_held: bool,
    },
    DeleteNode {
        index: usize,
    },
    DeleteSelectedNodes,
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
        option_held: bool,
    },
    PressSegment {
        index: usize,
        position: Point,
    },
    PressSegmentMove {
        index: usize,
        position: Point,
    },
    PressDirectProximitySegment {
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
    segment_zone: Option<CurveSegmentHitZone>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum SinkEvent {
        Edit(ClapId, f64),
        Begin(ClapId),
        Value(ClapId, f64),
        End(ClapId),
    }

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<SinkEvent>>,
    }

    impl RecordingSink {
        fn events(&self) -> Vec<SinkEvent> {
            self.events.lock().expect("recording sink lock").clone()
        }
    }

    impl HostParamEditSink for RecordingSink {
        fn edit(&self, _: &AutomationConfig, param_id: ClapId, value: f64) -> bool {
            self.events
                .lock()
                .expect("recording sink lock")
                .push(SinkEvent::Edit(param_id, value));
            true
        }

        fn gesture_started(&self, _: &AutomationConfig, param_id: ClapId) -> bool {
            self.events
                .lock()
                .expect("recording sink lock")
                .push(SinkEvent::Begin(param_id));
            true
        }

        fn gesture_value(&self, _: &AutomationConfig, param_id: ClapId, value: f64) -> bool {
            self.events
                .lock()
                .expect("recording sink lock")
                .push(SinkEvent::Value(param_id, value));
            true
        }

        fn gesture_ended(&self, _: &AutomationConfig, param_id: ClapId) -> bool {
            self.events
                .lock()
                .expect("recording sink lock")
                .push(SinkEvent::End(param_id));
            true
        }
    }

    fn editor(sink: Arc<RecordingSink>) -> PumpEditorState {
        PumpEditorState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            sink,
        )
    }

    #[test]
    fn teardown_ends_knob_gesture_and_admits_following_gesture() {
        let sink = Arc::new(RecordingSink::default());
        let mut state = editor(Arc::clone(&sink));

        state.dispatch(EditorMessage::Knob {
            target: NumericEntryTarget::Mix,
            message: KnobMessage::GestureStarted,
        });
        state.dispatch(EditorMessage::Knob {
            target: NumericEntryTarget::Mix,
            message: KnobMessage::ValueChanged { value: 0.25 },
        });
        let applied = state.params().mix();
        state.finish_for_teardown();

        assert_eq!(state.params().mix(), applied);
        assert_eq!(
            sink.events(),
            vec![
                SinkEvent::Begin(PARAM_MIX_ID),
                SinkEvent::Value(PARAM_MIX_ID, 0.25),
                SinkEvent::End(PARAM_MIX_ID),
            ]
        );

        state.dispatch(EditorMessage::Knob {
            target: NumericEntryTarget::Mix,
            message: KnobMessage::GestureStarted,
        });
        assert!(state.has_active_gesture());
        assert_eq!(sink.events().last(), Some(&SinkEvent::Begin(PARAM_MIX_ID)));
    }

    #[test]
    fn teardown_finishes_offset_at_applied_phase_without_restoring_it() {
        let sink = Arc::new(RecordingSink::default());
        let mut state = editor(Arc::clone(&sink));
        state.dispatch(EditorMessage::Curve(
            CurvePreviewMessage::PressCurveOffset {
                pointer_x: 10.0,
                quantized: false,
            },
        ));
        state.dispatch(EditorMessage::Curve(CurvePreviewMessage::DragCurveOffset {
            delta: 0.24,
        }));
        let applied_phase = state.params().phase_offset();
        assert_ne!(applied_phase, 0.0);

        state.finish_for_teardown();

        assert_eq!(state.params().phase_offset(), applied_phase);
        assert_eq!(
            sink.events(),
            vec![
                SinkEvent::Begin(PARAM_PHASE_OFFSET_ID),
                SinkEvent::Value(PARAM_PHASE_OFFSET_ID, applied_phase as f64),
                SinkEvent::End(PARAM_PHASE_OFFSET_ID),
            ]
        );
        state.dispatch(EditorMessage::Curve(
            CurvePreviewMessage::PressCurveOffset {
                pointer_x: 10.0,
                quantized: false,
            },
        ));
        assert!(state.has_active_gesture());
    }

    #[test]
    fn discrete_knob_update_is_one_host_transaction() {
        let sink = Arc::new(RecordingSink::default());
        let mut state = editor(Arc::clone(&sink));
        state.dispatch(EditorMessage::Knob {
            target: NumericEntryTarget::Swing,
            message: KnobMessage::Discrete { value: 0.75 },
        });
        assert!((state.params().swing() - 0.75).abs() < f32::EPSILON);
        assert_eq!(
            sink.events(),
            vec![
                SinkEvent::Begin(PARAM_SWING_ID),
                SinkEvent::Value(PARAM_SWING_ID, 0.75),
                SinkEvent::End(PARAM_SWING_ID),
            ]
        );
        assert_eq!(state.undo_history.len(), 1);
    }
}

#[cfg(test)]
#[path = "model_regressions.rs"]
mod model_regressions;
