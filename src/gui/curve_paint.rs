//! Ordered, geometry-only capture for curve painting.
//!
//! The pointer stream is deliberately kept in observation order.  Sorting
//! samples by x at capture time loses loops, reversals, and the distinction
//! between a boundary crossing and a clamped point.  The curve reducer can
//! choose how to reconstruct a curve from these runs later, but it must not
//! have to recreate the pointer topology first.

use std::cmp::Ordering;

use crate::curve::{
    sample_curve_segment, sample_editable_curve, CurveNode, CurveSegment, EditableCurve,
    MAX_EDITABLE_NODES, MAX_SEGMENT_TENSION, MIN_SEGMENT_TENSION,
};

const GEOMETRY_EPSILON: f32 = 1.0e-6;
const RAW_X_MERGE_EPSILON: f32 = 1.0e-6;
const RAW_NODE_ORDER_EPSILON: f32 = 1.0e-4;
const GENERATED_NODE_MERGE_X: f32 = 0.004;
const GENERATED_NODE_MERGE_Y: f32 = 0.01;
const SEAM_CROSSING_EPSILON: f32 = 4.0e-6;
const PAINT_FIT_TOLERANCE: f32 = 0.018;
const PAINT_FIT_RESAMPLE_STEPS: usize = 16;
const PAINT_FIT_TENSION_STEPS: usize = 64;
const PAINT_FIT_TENSION_REFINEMENT_STEPS: usize = 8;
const PAINT_FIT_EPSILON: f32 = 1.0e-6;
const MAX_RAW_PATCHES: usize = 64;
const MAX_RAW_CONSTRAINTS: usize = 64;
const MAX_OPTIONAL_FIT_ANCHORS: usize = 128;
const MAX_FIT_SAMPLES_PER_SEGMENT: usize = 64;
// Pointer samples carry a small amount of hand and device jitter. Treat only
// visibly meaningful vertical turns as hard anchors; smooth curvature is
// represented by fitted segment tension instead of consuming node capacity.
const PAINT_TURN_TOLERANCE: f32 = 0.01;
// A 0.004 normalized horizontal movement is about 1.6 px in the active
// curve viewport. Smaller backsteps are pointer jitter, not display turns.
const DISPLAY_HORIZONTAL_REVERSAL_TOLERANCE: f32 = 0.004;
// Pointer events can arrive much faster than the curve can be fitted. Keep
// the recorder's input bounded before any preview/release reconstruction.
const DISPLAY_SIMPLIFICATION_TOLERANCE: f32 = 0.0025;
const MAX_CAPTURED_POINTS_PER_RUN: usize = 128;
const MAX_CAPTURED_RUNS: usize = 16;
const MAX_CAPTURED_POINTS_PER_GESTURE: usize = MAX_CAPTURED_POINTS_PER_RUN * MAX_CAPTURED_RUNS;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RectPoint {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RectBounds {
    pub(crate) min: RectPoint,
    pub(crate) max: RectPoint,
}

impl RectBounds {
    fn is_valid(self) -> bool {
        self.min.x.is_finite()
            && self.min.y.is_finite()
            && self.max.x.is_finite()
            && self.max.y.is_finite()
            && self.min.x <= self.max.x
            && self.min.y <= self.max.y
    }

    fn contains(self, point: RectPoint) -> bool {
        point.x.is_finite()
            && point.y.is_finite()
            && point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }

    fn clamp(self, point: RectPoint) -> RectPoint {
        RectPoint {
            x: point.x.clamp(self.min.x, self.max.x),
            y: point.y.clamp(self.min.y, self.max.y),
        }
    }
}

/// Clockwise perimeter edges.  The parameter runs in the direction shown:
/// top left-to-right, right top-to-bottom, bottom right-to-left, and left
/// bottom-to-top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoundaryEdge {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoundaryCorner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

/// A normalized position along one perimeter edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EdgeParameter {
    pub(crate) edge: BoundaryEdge,
    pub(crate) parameter: f32,
}

impl EdgeParameter {
    fn new(edge: BoundaryEdge, parameter: f32) -> Option<Self> {
        (parameter.is_finite() && (-GEOMETRY_EPSILON..=1.0 + GEOMETRY_EPSILON).contains(&parameter))
            .then_some(Self {
                edge,
                parameter: parameter.clamp(0.0, 1.0),
            })
    }
}

/// The exact contact made by a captured point with the rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum BoundaryContact {
    Interior,
    Edge(EdgeParameter),
    Corner(BoundaryCorner),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaintPoint {
    pub(crate) position: RectPoint,
    pub(crate) contact: BoundaryContact,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PaintRun {
    points: Vec<PaintPoint>,
}

impl PaintRun {
    fn from_point(point: PaintPoint) -> Self {
        Self {
            points: vec![point],
        }
    }

    pub(crate) fn points(&self) -> &[PaintPoint] {
        &self.points
    }
}

#[derive(Clone, Copy, Debug)]
struct LastObservation {
    raw: RectPoint,
    point: PaintPoint,
    outside: bool,
}

/// Records a raw pointer stream as ordered interior/perimeter runs.
#[derive(Clone, Debug)]
pub(crate) struct StrokeRecorder {
    bounds: RectBounds,
    runs: Vec<PaintRun>,
    point_count: usize,
    truncated: bool,
    last: Option<LastObservation>,
}

impl StrokeRecorder {
    pub(crate) fn new(bounds: RectBounds) -> Self {
        Self {
            bounds,
            runs: Vec::new(),
            point_count: 0,
            truncated: false,
            last: None,
        }
    }

    /// The recorder never exposes more than the fixed capture budget.  This
    /// keeps preview cloning and reconstruction bounded by construction.
    pub(crate) fn runs(&self) -> &[PaintRun] {
        &self.runs
    }

    #[cfg(test)]
    pub(crate) fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// Record one raw observation.  Non-finite observations are ignored and do
    /// not disturb the previous valid point in the stream.
    pub(crate) fn observe(&mut self, raw: RectPoint) {
        self.observe_with_outside_hint(raw, false);
    }

    /// Record an observation known by the input router to have occurred beyond
    /// the viewport.  The hint matters when a backend reports the exact edge
    /// coordinate for a pointer that is already outside the widget.
    pub(crate) fn observe_outside(&mut self, raw: RectPoint) {
        self.observe_with_outside_hint(raw, true);
    }

    fn observe_with_outside_hint(&mut self, raw: RectPoint, outside_hint: bool) {
        let Some(point) = classify_point(self.bounds, raw) else {
            return;
        };
        let outside = outside_hint || !self.bounds.contains(raw);

        let Some(previous) = self.last else {
            self.start_run(point);
            self.last = Some(LastObservation {
                raw,
                point,
                outside,
            });
            return;
        };

        match (previous.outside, outside) {
            (false, false) => self.append_to_current(point),
            (false, true) => self.capture_exit(previous, raw, point),
            (true, false) => self.capture_reentry(previous, raw, point),
            (true, true) => {
                if !self.capture_perimeter_step(previous.point, point) {
                    // A sparse jump between non-adjacent perimeter locations
                    // has no uniquely recoverable path.
                    self.start_run(point);
                }
            }
        }

        self.last = Some(LastObservation {
            raw,
            point,
            outside,
        });
    }

    fn capture_exit(&mut self, previous: LastObservation, raw: RectPoint, point: PaintPoint) {
        let Some(intersection) =
            segment_boundary_intersection(self.bounds, previous.raw, raw, IntersectionOrder::First)
        else {
            self.start_run(point);
            return;
        };

        self.append_to_current(intersection);
        if !self.capture_perimeter_step(intersection, point) {
            // A sparse jump to a non-adjacent perimeter location is
            // ambiguous.  Preserve the endpoint, but never invent a chord
            // across unseen perimeter or interior geometry.
            self.start_run(point);
        }
    }

    fn capture_reentry(&mut self, previous: LastObservation, raw: RectPoint, point: PaintPoint) {
        let intersection =
            segment_boundary_intersection(self.bounds, previous.raw, raw, IntersectionOrder::Last);

        // Re-entry is always a run break.  Starting the new run at the exact
        // intersection keeps the visible stroke faithful without joining an
        // outside perimeter trace to a new interior chord.
        if self.start_run(intersection.unwrap_or(point)) {
            self.append_to_current(point);
        }
    }

    fn capture_perimeter_step(&mut self, previous: PaintPoint, point: PaintPoint) -> bool {
        let bounds = self.bounds;
        let before = self
            .runs
            .last()
            .map(|run| run.points.len())
            .unwrap_or_default();
        let compatible =
            append_boundary_transition(self.current_run_mut(), previous, point, bounds);
        let after = self
            .runs
            .last()
            .map(|run| run.points.len())
            .unwrap_or_default();
        self.point_count = self
            .point_count
            .saturating_add(after.saturating_sub(before));
        self.compact_current_run();
        compatible
    }

    fn start_run(&mut self, point: PaintPoint) -> bool {
        if self.runs.len() >= MAX_CAPTURED_RUNS
            || self.point_count >= MAX_CAPTURED_POINTS_PER_GESTURE
        {
            // Capture degradation is recoverable at release.  Evict one
            // lower-value retained run rather than joining this run to an
            // unrelated run and fabricating a chord between them.
            self.truncated = true;
            self.evict_run_for_new_run();
        }
        self.runs.push(PaintRun::from_point(point));
        self.point_count += 1;
        true
    }

    fn evict_run_for_new_run(&mut self) {
        if self.runs.is_empty() {
            self.point_count = 0;
            return;
        }

        let index = if self.runs.len() == 1 {
            0
        } else {
            (1..self.runs.len())
                .min_by(|left, right| {
                    run_capture_value(&self.runs[*left])
                        .cmp(&run_capture_value(&self.runs[*right]))
                        .then_with(|| right.cmp(left))
                })
                .unwrap_or(1)
        };
        let removed = self.runs.remove(index);
        self.point_count = self.point_count.saturating_sub(removed.points.len());
    }

    fn current_run_mut(&mut self) -> &mut PaintRun {
        self.runs
            .last_mut()
            .expect("a stroke recorder always has a run before appending")
    }

    fn append_to_current(&mut self, point: PaintPoint) {
        {
            let run = self.current_run_mut();
            if run
                .points
                .last()
                .is_some_and(|last| same_position(last.position, point.position))
            {
                return;
            }
            run.points.push(point);
        }
        self.point_count = self.point_count.saturating_add(1);
        self.compact_current_run();
    }

    fn compact_current_run(&mut self) -> bool {
        let Some(run) = self.runs.last_mut() else {
            return true;
        };
        let before = run.points.len();
        if before <= MAX_CAPTURED_POINTS_PER_RUN {
            return true;
        }
        let protected_count = protected_point_indices(&run.points)
            .into_iter()
            .filter(|is_protected| *is_protected)
            .count();
        if protected_count > MAX_CAPTURED_POINTS_PER_RUN {
            self.truncated = true;
        }
        simplify_paint_run(&mut run.points);
        let after = run.points.len();
        self.point_count = self
            .point_count
            .saturating_sub(before)
            .saturating_add(after);
        debug_assert!(after <= MAX_CAPTURED_POINTS_PER_RUN);
        true
    }
}

fn run_capture_value(run: &PaintRun) -> (usize, usize, usize) {
    let corner_count = run
        .points
        .iter()
        .filter(|point| matches!(point.contact, BoundaryContact::Corner(_)))
        .count();
    let transition_count = run
        .points
        .windows(2)
        .filter(|pair| paint_contact_kind(pair[0].contact) != paint_contact_kind(pair[1].contact))
        .count();
    let effect_span = run
        .points
        .first()
        .zip(run.points.last())
        .map(|(first, last)| {
            ((last.position.x - first.position.x).abs() * 10_000.0) as usize
                + ((last.position.y - first.position.y).abs() * 10_000.0) as usize
        })
        .unwrap_or_default();
    (
        corner_count + transition_count,
        effect_span,
        run.points.len(),
    )
}

#[derive(Clone, Copy)]
enum IntersectionOrder {
    First,
    Last,
}

fn classify_point(bounds: RectBounds, raw: RectPoint) -> Option<PaintPoint> {
    if !bounds.is_valid() || !raw.x.is_finite() || !raw.y.is_finite() {
        return None;
    }

    let position = bounds.clamp(raw);
    let contact = match (
        position.x == bounds.min.x,
        position.x == bounds.max.x,
        position.y == bounds.min.y,
        position.y == bounds.max.y,
    ) {
        (true, false, true, false) => BoundaryContact::Corner(BoundaryCorner::TopLeft),
        (false, true, true, false) => BoundaryContact::Corner(BoundaryCorner::TopRight),
        (true, false, false, true) => BoundaryContact::Corner(BoundaryCorner::BottomLeft),
        (false, true, false, true) => BoundaryContact::Corner(BoundaryCorner::BottomRight),
        (true, false, false, false) => BoundaryContact::Edge(EdgeParameter::new(
            BoundaryEdge::Left,
            (position.y - bounds.min.y) / (bounds.max.y - bounds.min.y).max(GEOMETRY_EPSILON),
        )?),
        (false, true, false, false) => BoundaryContact::Edge(EdgeParameter::new(
            BoundaryEdge::Right,
            (position.y - bounds.min.y) / (bounds.max.y - bounds.min.y).max(GEOMETRY_EPSILON),
        )?),
        (false, false, true, false) => BoundaryContact::Edge(EdgeParameter::new(
            BoundaryEdge::Top,
            (position.x - bounds.min.x) / (bounds.max.x - bounds.min.x).max(GEOMETRY_EPSILON),
        )?),
        (false, false, false, true) => BoundaryContact::Edge(EdgeParameter::new(
            BoundaryEdge::Bottom,
            (bounds.max.x - position.x) / (bounds.max.x - bounds.min.x).max(GEOMETRY_EPSILON),
        )?),
        _ => BoundaryContact::Interior,
    };

    Some(PaintPoint { position, contact })
}

fn append_boundary_transition(
    run: &mut PaintRun,
    previous: PaintPoint,
    current: PaintPoint,
    bounds: RectBounds,
) -> bool {
    let Some(corner) = transition_corner(previous.contact, current.contact) else {
        let compatible = match (previous.contact, current.contact) {
            (BoundaryContact::Edge(left), BoundaryContact::Edge(right)) => left.edge == right.edge,
            (BoundaryContact::Corner(left), BoundaryContact::Corner(right)) => left == right,
            (BoundaryContact::Corner(corner), BoundaryContact::Edge(edge))
            | (BoundaryContact::Edge(edge), BoundaryContact::Corner(corner)) => {
                corner_on_edge(corner, edge.edge)
            }
            _ => false,
        };
        if compatible {
            append_unique(run, current);
        }
        return compatible;
    };

    append_unique(run, corner_point(bounds, corner));
    append_unique(run, current);
    true
}

fn transition_corner(
    previous: BoundaryContact,
    current: BoundaryContact,
) -> Option<BoundaryCorner> {
    match (previous, current) {
        (BoundaryContact::Edge(left), BoundaryContact::Edge(right)) if left.edge != right.edge => {
            shared_corner(left.edge, right.edge)
        }
        _ => None,
    }
}

fn append_unique(run: &mut PaintRun, point: PaintPoint) {
    if run
        .points
        .last()
        .is_some_and(|last| same_position(last.position, point.position))
    {
        return;
    }
    run.points.push(point);
}

fn same_position(left: RectPoint, right: RectPoint) -> bool {
    (left.x - right.x).abs() <= GEOMETRY_EPSILON && (left.y - right.y).abs() <= GEOMETRY_EPSILON
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PaintContactKind {
    Interior,
    Edge(BoundaryEdge),
    Corner(BoundaryCorner),
}

fn paint_contact_kind(contact: BoundaryContact) -> PaintContactKind {
    match contact {
        BoundaryContact::Interior => PaintContactKind::Interior,
        BoundaryContact::Edge(parameter) => PaintContactKind::Edge(parameter.edge),
        BoundaryContact::Corner(corner) => PaintContactKind::Corner(corner),
    }
}

fn meaningful_horizontal_reversal(
    direction: i8,
    monotonic_extreme_x: f32,
    sample_x: f32,
) -> Option<i8> {
    match direction {
        1 if sample_x < monotonic_extreme_x - DISPLAY_HORIZONTAL_REVERSAL_TOLERANCE => Some(-1),
        -1 if sample_x > monotonic_extreme_x + DISPLAY_HORIZONTAL_REVERSAL_TOLERANCE => Some(1),
        _ => None,
    }
}

/// Return the points that must survive display-space simplification.  Run
/// endpoints and boundary transitions are hard anchors; turns and extrema are
/// retained so `collect_display_geometry` sees the same topology after a
/// dense pointer stream has been coalesced.
fn protected_point_indices(points: &[PaintPoint]) -> Vec<bool> {
    let mut protected = vec![false; points.len()];
    if points.is_empty() {
        return protected;
    }
    protected[0] = true;
    protected[points.len() - 1] = true;

    for (index, point) in points.iter().enumerate() {
        if matches!(point.contact, BoundaryContact::Corner(_)) {
            protected[index] = true;
        }
        if index > 0
            && paint_contact_kind(points[index - 1].contact) != paint_contact_kind(point.contact)
        {
            protected[index - 1] = true;
            protected[index] = true;
        }
        if index + 1 < points.len()
            && paint_contact_kind(point.contact) != paint_contact_kind(points[index + 1].contact)
        {
            protected[index] = true;
            protected[index + 1] = true;
        }
    }

    let mut horizontal_direction = 0i8;
    let mut monotonic_extreme_index = 0usize;
    let mut monotonic_extreme_x = points[0].position.x;
    for (index, point) in points.iter().enumerate().skip(1) {
        let sample_x = point.position.x;
        match horizontal_direction {
            0 => {
                let delta = sample_x - monotonic_extreme_x;
                if delta > GEOMETRY_EPSILON {
                    horizontal_direction = 1;
                    monotonic_extreme_index = index;
                    monotonic_extreme_x = sample_x;
                } else if delta < -GEOMETRY_EPSILON {
                    horizontal_direction = -1;
                    monotonic_extreme_index = index;
                    monotonic_extreme_x = sample_x;
                }
            }
            1 => {
                if sample_x > monotonic_extreme_x {
                    monotonic_extreme_index = index;
                    monotonic_extreme_x = sample_x;
                } else if let Some(next_direction) = meaningful_horizontal_reversal(
                    horizontal_direction,
                    monotonic_extreme_x,
                    sample_x,
                ) {
                    protected[monotonic_extreme_index] = true;
                    horizontal_direction = next_direction;
                    monotonic_extreme_index = index;
                    monotonic_extreme_x = sample_x;
                }
            }
            -1 => {
                if sample_x < monotonic_extreme_x {
                    monotonic_extreme_index = index;
                    monotonic_extreme_x = sample_x;
                } else if let Some(next_direction) = meaningful_horizontal_reversal(
                    horizontal_direction,
                    monotonic_extreme_x,
                    sample_x,
                ) {
                    protected[monotonic_extreme_index] = true;
                    horizontal_direction = next_direction;
                    monotonic_extreme_index = index;
                    monotonic_extreme_x = sample_x;
                }
            }
            _ => unreachable!("horizontal direction is normalized to -1, 0, or 1"),
        }
    }

    for index in 1..points.len().saturating_sub(1) {
        let before_y = points[index].position.y - points[index - 1].position.y;
        let after_y = points[index + 1].position.y - points[index].position.y;
        if (before_y > PAINT_TURN_TOLERANCE && after_y < -PAINT_TURN_TOLERANCE)
            || (before_y < -PAINT_TURN_TOLERANCE && after_y > PAINT_TURN_TOLERANCE)
        {
            protected[index] = true;
        }
    }

    // Preserve the first and latest observation at a same-x run.  The latter
    // is the chronological last-write-wins value used by raw_geometry.
    let mut group_start = 0;
    while group_start < points.len() {
        let mut group_end = group_start + 1;
        while group_end < points.len()
            && (points[group_end].position.x - points[group_start].position.x).abs()
                <= GEOMETRY_EPSILON
        {
            group_end += 1;
        }
        if group_end - group_start > 1 {
            protected[group_start] = true;
            protected[group_end - 1] = true;
        }
        group_start = group_end;
    }

    protected
}

fn display_point_line_distance(point: RectPoint, left: RectPoint, right: RectPoint) -> f32 {
    let dx = right.x - left.x;
    let dy = right.y - left.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= GEOMETRY_EPSILON * GEOMETRY_EPSILON {
        return (point.x - left.x).hypot(point.y - left.y);
    }
    let projection = ((point.x - left.x) * dx + (point.y - left.y) * dy) / length_squared;
    let projection = projection.clamp(0.0, 1.0);
    let projected = RectPoint {
        x: left.x + dx * projection,
        y: left.y + dy * projection,
    };
    (point.x - projected.x).hypot(point.y - projected.y)
}

fn mark_display_simplification_interval(
    points: &[PaintPoint],
    start: usize,
    end: usize,
    keep: &mut [bool],
) {
    if end <= start + 1 {
        return;
    }

    let left = points[start].position;
    let right = points[end].position;
    let (farthest, distance) = (start + 1..end)
        .map(|index| {
            (
                index,
                display_point_line_distance(points[index].position, left, right),
            )
        })
        .max_by(
            |(left_index, left_distance), (right_index, right_distance)| {
                left_distance
                    .total_cmp(right_distance)
                    .then_with(|| right_index.cmp(left_index))
            },
        )
        .unwrap_or((start, 0.0));

    if distance > DISPLAY_SIMPLIFICATION_TOLERANCE {
        keep[farthest] = true;
        mark_display_simplification_interval(points, start, farthest, keep);
        mark_display_simplification_interval(points, farthest, end, keep);
    }
}

fn display_point_importance(points: &[PaintPoint], index: usize) -> f32 {
    if index == 0 || index + 1 == points.len() {
        return f32::INFINITY;
    }
    display_point_line_distance(
        points[index].position,
        points[index - 1].position,
        points[index + 1].position,
    )
}

fn bound_simplified_indices(
    points: &[PaintPoint],
    selected: &[usize],
    protected: &[bool],
) -> Vec<usize> {
    if selected.len() <= MAX_CAPTURED_POINTS_PER_RUN {
        return selected.to_vec();
    }

    let mut ranked = selected
        .iter()
        .copied()
        .map(|index| {
            (
                index,
                protected[index],
                point_protection_rank(points, index),
                display_point_importance(points, index),
            )
        })
        .collect::<Vec<_>>();
    ranked.sort_by(
        |(left_index, left_protected, left_rank, left_importance),
         (right_index, right_protected, right_rank, right_importance)| {
            right_protected
                .cmp(left_protected)
                .then_with(|| right_rank.cmp(left_rank))
                .then_with(|| right_importance.total_cmp(left_importance))
                .then_with(|| right_index.cmp(left_index))
        },
    );
    let mut bounded = ranked
        .into_iter()
        .take(MAX_CAPTURED_POINTS_PER_RUN)
        .map(|(index, _, _, _)| index)
        .collect::<Vec<_>>();
    for endpoint in [0, points.len().saturating_sub(1)] {
        if endpoint < points.len() && !bounded.contains(&endpoint) {
            if let Some(replace) = bounded
                .iter()
                .enumerate()
                .filter(|(_, index)| **index != 0 && **index + 1 != points.len())
                .min_by_key(|(_, index)| **index)
                .map(|(position, _)| position)
            {
                bounded[replace] = endpoint;
            }
        }
    }
    bounded.sort_unstable();
    bounded.dedup();
    bounded
}

fn point_protection_rank(points: &[PaintPoint], index: usize) -> u8 {
    if index == 0 || index + 1 == points.len() {
        return 4;
    }
    if matches!(points[index].contact, BoundaryContact::Corner(_)) {
        return 3;
    }
    if (index > 0
        && paint_contact_kind(points[index - 1].contact)
            != paint_contact_kind(points[index].contact))
        || (index + 1 < points.len()
            && paint_contact_kind(points[index].contact)
                != paint_contact_kind(points[index + 1].contact))
    {
        return 2;
    }
    1
}

/// Simplify a run in normalized display space once it reaches the hard cap.
/// RDP-style coalescing removes near-collinear samples, while protected
/// anchors preserve the pointer topology used by the reducer.  If the
/// topology itself cannot fit in the hard cap, the bounded selection retains
/// the highest-value anchors and marks the capture as degraded; it still
/// publishes the retained geometry for release reconstruction.
fn simplify_paint_run(points: &mut Vec<PaintPoint>) {
    if points.len() <= MAX_CAPTURED_POINTS_PER_RUN {
        return;
    }

    let protected = protected_point_indices(points);
    let anchors = protected
        .iter()
        .enumerate()
        .filter_map(|(index, is_protected)| (*is_protected).then_some(index))
        .collect::<Vec<_>>();
    let mut keep = protected.clone();
    for pair in anchors.windows(2) {
        mark_display_simplification_interval(points, pair[0], pair[1], &mut keep);
    }

    let selected = keep
        .iter()
        .enumerate()
        .filter_map(|(index, should_keep)| (*should_keep).then_some(index))
        .collect::<Vec<_>>();
    let bounded = bound_simplified_indices(points, &selected, &protected);
    let simplified = bounded
        .into_iter()
        .map(|index| points[index])
        .collect::<Vec<_>>();
    points.clear();
    points.extend(simplified);
    debug_assert!(points.len() <= MAX_CAPTURED_POINTS_PER_RUN);
}

fn shared_corner(left: BoundaryEdge, right: BoundaryEdge) -> Option<BoundaryCorner> {
    match (left, right) {
        (BoundaryEdge::Top, BoundaryEdge::Right) | (BoundaryEdge::Right, BoundaryEdge::Top) => {
            Some(BoundaryCorner::TopRight)
        }
        (BoundaryEdge::Right, BoundaryEdge::Bottom)
        | (BoundaryEdge::Bottom, BoundaryEdge::Right) => Some(BoundaryCorner::BottomRight),
        (BoundaryEdge::Bottom, BoundaryEdge::Left) | (BoundaryEdge::Left, BoundaryEdge::Bottom) => {
            Some(BoundaryCorner::BottomLeft)
        }
        (BoundaryEdge::Left, BoundaryEdge::Top) | (BoundaryEdge::Top, BoundaryEdge::Left) => {
            Some(BoundaryCorner::TopLeft)
        }
        _ => None,
    }
}

fn corner_on_edge(corner: BoundaryCorner, edge: BoundaryEdge) -> bool {
    matches!(
        (corner, edge),
        (
            BoundaryCorner::TopLeft,
            BoundaryEdge::Top | BoundaryEdge::Left
        ) | (
            BoundaryCorner::TopRight,
            BoundaryEdge::Top | BoundaryEdge::Right
        ) | (
            BoundaryCorner::BottomRight,
            BoundaryEdge::Bottom | BoundaryEdge::Right
        ) | (
            BoundaryCorner::BottomLeft,
            BoundaryEdge::Bottom | BoundaryEdge::Left
        )
    )
}

fn corner_point(bounds: RectBounds, corner: BoundaryCorner) -> PaintPoint {
    let position = match corner {
        BoundaryCorner::TopLeft => bounds.min,
        BoundaryCorner::TopRight => RectPoint {
            x: bounds.max.x,
            y: bounds.min.y,
        },
        BoundaryCorner::BottomRight => bounds.max,
        BoundaryCorner::BottomLeft => RectPoint {
            x: bounds.min.x,
            y: bounds.max.y,
        },
    };
    PaintPoint {
        position,
        contact: BoundaryContact::Corner(corner),
    }
}

fn segment_boundary_intersection(
    bounds: RectBounds,
    from: RectPoint,
    to: RectPoint,
    order: IntersectionOrder,
) -> Option<PaintPoint> {
    if !bounds.is_valid() || ![from.x, from.y, to.x, to.y].into_iter().all(f32::is_finite) {
        return None;
    }

    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let mut candidates = Vec::with_capacity(4);
    let mut add_candidate = |t: f32| {
        if !(-GEOMETRY_EPSILON..=1.0 + GEOMETRY_EPSILON).contains(&t) {
            return;
        }
        let t = t.clamp(0.0, 1.0);
        let position = RectPoint {
            x: from.x + dx * t,
            y: from.y + dy * t,
        };
        if position.x < bounds.min.x - GEOMETRY_EPSILON
            || position.x > bounds.max.x + GEOMETRY_EPSILON
            || position.y < bounds.min.y - GEOMETRY_EPSILON
            || position.y > bounds.max.y + GEOMETRY_EPSILON
        {
            return;
        }
        let Some(point) = classify_point(bounds, position) else {
            return;
        };
        if candidates
            .iter()
            .any(|(existing_t, _): &(f32, PaintPoint)| (existing_t - t).abs() <= GEOMETRY_EPSILON)
        {
            return;
        }
        candidates.push((t, point));
    };

    if dx.abs() > GEOMETRY_EPSILON {
        add_candidate((bounds.min.x - from.x) / dx);
        add_candidate((bounds.max.x - from.x) / dx);
    }
    if dy.abs() > GEOMETRY_EPSILON {
        add_candidate((bounds.min.y - from.y) / dy);
        add_candidate((bounds.max.y - from.y) / dy);
    }

    candidates.sort_by(|left, right| left.0.total_cmp(&right.0));
    match order {
        IntersectionOrder::First => candidates.first().map(|(_, point)| *point),
        IntersectionOrder::Last => candidates.last().map(|(_, point)| *point),
    }
}

/// The result of reconstructing one complete secondary-button paint gesture.
///
/// Every variant carries the candidate selected for release handling.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PaintCommitOutcome {
    Applied { candidate: EditableCurve },
    NoOp { candidate: EditableCurve },
}

impl PaintCommitOutcome {
    #[cfg(test)]
    pub(crate) fn candidate(&self) -> &EditableCurve {
        match self {
            Self::Applied { candidate } | Self::NoOp { candidate } => candidate,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PaintPriority {
    chronology: usize,
    tie: usize,
}

#[derive(Clone, Copy, Debug)]
struct DisplayGraphPoint {
    x: f32,
    y: f32,
    contact: BoundaryContact,
    chronology: usize,
}

#[derive(Clone, Debug)]
struct DisplayFragment {
    points: Vec<RawSample>,
    priority: PaintPriority,
}

#[derive(Clone, Copy, Debug)]
struct DisplayConstraint {
    display_x: f32,
    y: f32,
    priority: PaintPriority,
}

#[derive(Clone, Copy, Debug)]
struct RawSample {
    x: f32,
    y: f32,
    mandatory: bool,
    chronology: usize,
}

#[derive(Clone, Debug)]
struct RawPatch {
    samples: Vec<RawSample>,
    priority: PaintPriority,
}

#[derive(Clone, Copy, Debug)]
struct RawConstraint {
    x: f32,
    y: f32,
    seam: bool,
    priority: PaintPriority,
}

#[derive(Clone, Copy, Debug)]
struct AnchorX {
    x: f32,
    protected: bool,
    rank: u8,
    priority: PaintPriority,
}

#[derive(Clone, Copy, Debug)]
struct SegmentFit {
    tension: f32,
    max_error: f32,
}

#[derive(Clone, Debug)]
struct FittedCandidate {
    curve: EditableCurve,
    max_error: f32,
}

#[derive(Clone, Copy)]
struct FitContext<'a> {
    origin: &'a EditableCurve,
    seam_y: f32,
    patches: &'a [RawPatch],
    constraints: &'a [RawConstraint],
}

impl FitContext<'_> {
    fn target_y_at(self, x: f32) -> f32 {
        target_y_at(self.origin, x, self.seam_y, self.patches, self.constraints)
    }
}

#[derive(Clone, Copy, Debug)]
struct FitSample {
    x: f32,
    target_y: f32,
}

#[derive(Clone, Copy, Debug)]
struct FitSampleCandidate {
    x: f32,
    protected: bool,
    rank: u8,
    priority: PaintPriority,
    chronology: usize,
}

/// Reconstruct an ordered paint gesture over one immutable origin curve.
///
/// The gesture is first segmented in observation order.  Only then are the
/// monotonic fragments mapped into raw phase, which keeps horizontal reversals,
/// run boundaries, perimeter travel, and chronological overlap observable to
/// the compositor.  The origin is never used as an accumulator.
pub(crate) fn reconstruct_paint(
    origin_curve: &EditableCurve,
    phase_offset: f32,
    runs: &[PaintRun],
) -> PaintCommitOutcome {
    let origin = origin_curve.clone().normalized();
    let phase_offset = if phase_offset.is_finite() {
        phase_offset.rem_euclid(1.0)
    } else {
        0.0
    };
    let (display_fragments, display_constraints) = collect_display_geometry(runs);
    if !paint_geometry_has_motion(
        &origin,
        phase_offset,
        &display_fragments,
        &display_constraints,
    ) {
        return PaintCommitOutcome::NoOp { candidate: origin };
    }

    let (patches, constraints) =
        raw_geometry(&display_fragments, &display_constraints, phase_offset);
    if patches.is_empty() && constraints.is_empty() {
        return PaintCommitOutcome::NoOp { candidate: origin };
    }

    let seam_y = raw_seam_y(&origin, &patches, &constraints);
    let fit_context = FitContext {
        origin: &origin,
        seam_y,
        patches: &patches,
        constraints: &constraints,
    };
    let mandatory_xs = mandatory_anchor_xs(&origin, &patches, &constraints);
    let optional_xs = optional_anchor_xs(&mandatory_xs, &patches);
    // Coalescing can discard the just-selected x, so selection progress must
    // not depend on the mutable selected set retaining it.
    let mut attempted_optional_xs = vec![false; optional_xs.len()];
    let mut selected_xs = mandatory_xs.clone();
    let mut fitted = fit_candidate(&origin, &selected_xs, seam_y, &patches, &constraints);
    let mut best = candidate_is_valid(&fitted.curve)
        .then(|| fitted.clone())
        .filter(|candidate| candidate.curve != origin);
    while selected_xs.len() < MAX_EDITABLE_NODES && fitted.max_error > PAINT_FIT_TOLERANCE {
        let Some((optional_index, _)) = optional_xs
            .iter()
            .enumerate()
            .filter(|(index, x)| {
                !attempted_optional_xs[*index]
                    && !selected_xs
                        .iter()
                        .any(|selected| same_raw_x(**x, *selected))
            })
            .filter_map(|(index, x)| {
                let error = candidate_error_at(&fitted.curve, *x, fit_context);
                (error > PAINT_FIT_EPSILON).then_some((index, error))
            })
            .max_by(|(left_index, left_error), (right_index, right_error)| {
                left_error
                    .total_cmp(right_error)
                    .then_with(|| optional_xs[*right_index].total_cmp(&optional_xs[*left_index]))
            })
        else {
            break;
        };
        attempted_optional_xs[optional_index] = true;
        selected_xs.push(optional_xs[optional_index]);
        selected_xs.sort_by(f32::total_cmp);
        selected_xs = coalesce_selected_xs(
            &origin,
            &selected_xs,
            &mandatory_xs,
            seam_y,
            &patches,
            &constraints,
        );
        fitted = fit_candidate(&origin, &selected_xs, seam_y, &patches, &constraints);
        if candidate_is_better(&origin, &fitted, best.as_ref()) {
            best = Some(fitted.clone());
        }
    }

    fitted = simplify_selected_anchors(
        &origin,
        &selected_xs,
        &mandatory_xs,
        seam_y,
        &patches,
        &constraints,
        fitted,
    );

    if candidate_is_better(&origin, &fitted, best.as_ref()) {
        best = Some(fitted);
    }
    let candidate = best.map(|fitted| fitted.curve).unwrap_or(origin.clone());
    if candidate == origin {
        PaintCommitOutcome::NoOp { candidate }
    } else {
        PaintCommitOutcome::Applied { candidate }
    }
}

#[cfg(test)]
fn paint_runs_within_capture_budget(runs: &[PaintRun]) -> bool {
    if runs.len() > MAX_CAPTURED_RUNS {
        return false;
    }
    let mut point_count = 0usize;
    for run in runs {
        if run.points.len() > MAX_CAPTURED_POINTS_PER_RUN {
            return false;
        }
        point_count = point_count.saturating_add(run.points.len());
        if point_count > MAX_CAPTURED_POINTS_PER_GESTURE {
            return false;
        }
    }
    true
}

fn collect_display_geometry(runs: &[PaintRun]) -> (Vec<DisplayFragment>, Vec<DisplayConstraint>) {
    let mut fragments = Vec::new();
    let mut constraints = Vec::new();
    let mut chronology = 0usize;

    for run in runs {
        let mut current = Vec::new();
        let mut direction = 0i8;
        let mut monotonic_extreme_x = None;
        for point in run.points().iter().copied() {
            if !valid_paint_point(point) {
                continue;
            }
            chronology = chronology.saturating_add(1);
            let sample = DisplayGraphPoint {
                x: point.position.x.clamp(0.0, 1.0),
                y: point.position.y.clamp(0.0, 1.0),
                contact: point.contact,
                chronology,
            };

            if let Some(display_x) = boundary_constraint_x(point.contact) {
                if let Some(fragment) = finish_display_fragment(&mut current) {
                    fragments.push(fragment);
                }
                direction = 0;
                monotonic_extreme_x = None;
                constraints.push(DisplayConstraint {
                    display_x,
                    y: sample.y,
                    priority: PaintPriority { chronology, tie: 0 },
                });
                continue;
            }

            if current.is_empty() {
                current.push(sample);
                monotonic_extreme_x = Some(sample.x);
                continue;
            }

            let previous = *current.last().expect("current graph point exists");
            let dx = sample.x - previous.x;
            let dy = sample.y - previous.y;
            if dx.abs() <= GEOMETRY_EPSILON {
                if dy.abs() > GEOMETRY_EPSILON {
                    if let Some(fragment) = finish_display_fragment(&mut current) {
                        fragments.push(fragment);
                    }
                    constraints.push(DisplayConstraint {
                        display_x: sample.x,
                        y: sample.y,
                        priority: PaintPriority { chronology, tie: 0 },
                    });
                    direction = 0;
                    current.push(sample);
                    monotonic_extreme_x = Some(sample.x);
                } else if let Some(last) = current.last_mut() {
                    if sample.chronology >= last.chronology {
                        *last = sample;
                    }
                }
                continue;
            }

            let extreme_x = monotonic_extreme_x.unwrap_or(previous.x);
            match direction {
                0 => {
                    current.push(sample);
                    direction = if dx.is_sign_positive() { 1 } else { -1 };
                    monotonic_extreme_x = Some(sample.x);
                }
                1 => {
                    if sample.x > extreme_x {
                        current.push(sample);
                        monotonic_extreme_x = Some(sample.x);
                    } else if let Some(next_direction) =
                        meaningful_horizontal_reversal(direction, extreme_x, sample.x)
                    {
                        if let Some(fragment) = finish_display_fragment(&mut current) {
                            fragments.push(fragment);
                        }
                        // Keep the turning point in both fragments. The
                        // previous fragment is finalized before the reversing
                        // sample is admitted, so each fragment remains
                        // monotonic in display space and raw_geometry only
                        // sees real phase seams.
                        current.push(previous);
                        current.push(sample);
                        direction = next_direction;
                        monotonic_extreme_x = Some(sample.x);
                    } else {
                        // Keep a sub-visible backstep at the current extreme.
                        // Replacing the extreme also prevents repeated jitter
                        // from accumulating into a false reversal.
                        let mut coalesced = sample;
                        coalesced.x = extreme_x;
                        *current.last_mut().expect("current graph point exists") = coalesced;
                    }
                }
                -1 => {
                    if sample.x < extreme_x {
                        current.push(sample);
                        monotonic_extreme_x = Some(sample.x);
                    } else if let Some(next_direction) =
                        meaningful_horizontal_reversal(direction, extreme_x, sample.x)
                    {
                        if let Some(fragment) = finish_display_fragment(&mut current) {
                            fragments.push(fragment);
                        }
                        current.push(previous);
                        current.push(sample);
                        direction = next_direction;
                        monotonic_extreme_x = Some(sample.x);
                    } else {
                        let mut coalesced = sample;
                        coalesced.x = extreme_x;
                        *current.last_mut().expect("current graph point exists") = coalesced;
                    }
                }
                _ => unreachable!("display direction is normalized to -1, 0, or 1"),
            }
        }
        if let Some(fragment) = finish_display_fragment(&mut current) {
            fragments.push(fragment);
        }
    }

    (fragments, constraints)
}

fn valid_paint_point(point: PaintPoint) -> bool {
    point.position.x.is_finite()
        && point.position.y.is_finite()
        && (0.0..=1.0).contains(&point.position.x)
        && (0.0..=1.0).contains(&point.position.y)
}

fn boundary_constraint_x(contact: BoundaryContact) -> Option<f32> {
    match contact {
        BoundaryContact::Edge(parameter) => match parameter.edge {
            BoundaryEdge::Left => Some(0.0),
            BoundaryEdge::Right => Some(1.0),
            BoundaryEdge::Top | BoundaryEdge::Bottom => None,
        },
        BoundaryContact::Corner(corner) => match corner {
            BoundaryCorner::TopLeft | BoundaryCorner::BottomLeft => Some(0.0),
            BoundaryCorner::TopRight | BoundaryCorner::BottomRight => Some(1.0),
        },
        BoundaryContact::Interior => None,
    }
}

fn finish_display_fragment(points: &mut Vec<DisplayGraphPoint>) -> Option<DisplayFragment> {
    if points.is_empty() {
        return None;
    }

    let mut ordered: Vec<RawSample> = Vec::with_capacity(points.len());
    for point in points.drain(..) {
        let sample = RawSample {
            x: point.x,
            y: point.y,
            mandatory: matches!(point.contact, BoundaryContact::Corner(_)),
            chronology: point.chronology,
        };
        if let Some(previous) = ordered.last_mut() {
            if (sample.x - previous.x).abs() <= GEOMETRY_EPSILON {
                if sample.chronology >= previous.chronology {
                    *previous = sample;
                }
                continue;
            }
        }
        ordered.push(sample);
    }
    if ordered.len() < 2 {
        return None;
    }
    if ordered[0].x > ordered[ordered.len() - 1].x {
        ordered.reverse();
    }

    let mut mandatory = vec![false; ordered.len()];
    mandatory[0] = true;
    mandatory[ordered.len() - 1] = true;
    for (index, point) in ordered.iter().enumerate() {
        if point.mandatory {
            mandatory[index] = true;
        }
        if index > 0 && index + 1 < ordered.len() {
            let before = point.y - ordered[index - 1].y;
            let after = ordered[index + 1].y - point.y;
            if (before > PAINT_TURN_TOLERANCE && after < -PAINT_TURN_TOLERANCE)
                || (before < -PAINT_TURN_TOLERANCE && after > PAINT_TURN_TOLERANCE)
            {
                mandatory[index] = true;
            }
        }
    }
    for (sample, required) in ordered.iter_mut().zip(mandatory.iter()) {
        if *required {
            sample.mandatory = true;
        }
    }

    let last_chronology = ordered
        .iter()
        .map(|point| point.chronology)
        .max()
        .unwrap_or_default();
    Some(DisplayFragment {
        points: ordered,
        priority: PaintPriority {
            chronology: last_chronology,
            tie: 0,
        },
    })
}

fn paint_geometry_has_motion(
    origin: &EditableCurve,
    phase_offset: f32,
    fragments: &[DisplayFragment],
    constraints: &[DisplayConstraint],
) -> bool {
    if fragments.iter().any(|fragment| {
        fragment
            .points
            .last()
            .zip(fragment.points.first())
            .is_some_and(|(last, first)| (last.x - first.x).abs() > GEOMETRY_EPSILON)
    }) {
        return true;
    }
    if constraints.len() > 1
        && constraints
            .windows(2)
            .any(|pair| (pair[0].y - pair[1].y).abs() > GEOMETRY_EPSILON)
    {
        return true;
    }
    constraints.iter().any(|constraint| {
        let raw_x = map_display_to_raw(constraint.display_x, phase_offset);
        let origin_y = sample_editable_curve(origin, raw_x);
        (origin_y - constraint.y).abs() > GEOMETRY_EPSILON
    })
}

fn raw_geometry(
    fragments: &[DisplayFragment],
    display_constraints: &[DisplayConstraint],
    phase_offset: f32,
) -> (Vec<RawPatch>, Vec<RawConstraint>) {
    let mut patches = Vec::new();
    for fragment in fragments {
        let mut display_points = fragment.points.clone();
        insert_display_seam_sample(&mut display_points, phase_offset);
        let mut current: Vec<RawSample> = Vec::new();
        let mut group_index = 0usize;
        for point in display_points {
            let raw_x = map_display_to_raw(point.x, phase_offset);
            if let Some(previous) = current.last().copied() {
                if raw_x + RAW_X_MERGE_EPSILON < previous.x {
                    append_raw_sample(
                        &mut current,
                        RawSample {
                            x: 1.0,
                            y: point.y,
                            mandatory: true,
                            chronology: point.chronology,
                        },
                    );
                    if let Some(patch) =
                        finish_raw_patch(&mut current, fragment.priority, group_index)
                    {
                        patches.push(patch);
                        group_index = group_index.saturating_add(1);
                    }
                    current.clear();
                    append_raw_sample(
                        &mut current,
                        RawSample {
                            x: 0.0,
                            y: point.y,
                            mandatory: true,
                            chronology: point.chronology,
                        },
                    );
                } else {
                    append_raw_sample(
                        &mut current,
                        RawSample {
                            x: canonical_raw_x(raw_x),
                            y: point.y,
                            mandatory: point.mandatory,
                            chronology: point.chronology,
                        },
                    );
                }
            } else {
                append_raw_sample(
                    &mut current,
                    RawSample {
                        x: canonical_raw_x(raw_x),
                        y: point.y,
                        mandatory: point.mandatory,
                        chronology: point.chronology,
                    },
                );
            }
        }
        if let Some(patch) = finish_raw_patch(&mut current, fragment.priority, group_index) {
            patches.push(patch);
        }
    }

    let constraints = display_constraints
        .iter()
        .copied()
        .map(|constraint| {
            let x = canonical_raw_x(map_display_to_raw(constraint.display_x, phase_offset));
            RawConstraint {
                x,
                y: constraint.y.clamp(0.0, 1.0),
                seam: is_raw_seam(x),
                priority: constraint.priority,
            }
        })
        .collect::<Vec<_>>();
    (
        retain_raw_patches(patches),
        retain_raw_constraints(constraints),
    )
}

fn insert_display_seam_sample(points: &mut Vec<RawSample>, phase_offset: f32) {
    if phase_offset <= GEOMETRY_EPSILON || points.len() < 2 {
        return;
    }
    for index in 0..points.len() - 1 {
        let left = points[index];
        let right = points[index + 1];
        if left.x < phase_offset
            && right.x > phase_offset
            && right.x - left.x <= SEAM_CROSSING_EPSILON
        {
            points[index].x = phase_offset;
            points[index].mandatory = true;
            points[index + 1].x = phase_offset;
            points[index + 1].mandatory = true;
            return;
        }
        if (left.x - phase_offset).abs() <= GEOMETRY_EPSILON {
            return;
        }
        if (right.x - phase_offset).abs() <= GEOMETRY_EPSILON {
            return;
        }
        if left.x < phase_offset && right.x > phase_offset {
            let fraction = (phase_offset - left.x) / (right.x - left.x);
            points.insert(
                index + 1,
                RawSample {
                    x: phase_offset,
                    y: left.y + (right.y - left.y) * fraction,
                    mandatory: false,
                    chronology: left.chronology.max(right.chronology),
                },
            );
            return;
        }
    }
}

fn finish_raw_patch(
    samples: &mut [RawSample],
    fragment_priority: PaintPriority,
    group_index: usize,
) -> Option<RawPatch> {
    if samples.is_empty() {
        return None;
    }
    if samples.len() == 1 && !is_raw_seam(samples[0].x) {
        return None;
    }
    samples.sort_by(|left, right| left.x.total_cmp(&right.x));
    Some(RawPatch {
        samples: samples.to_vec(),
        priority: PaintPriority {
            chronology: fragment_priority.chronology,
            tie: group_index,
        },
    })
}

fn append_raw_sample(samples: &mut Vec<RawSample>, sample: RawSample) {
    if let Some(previous) = samples.last_mut() {
        if same_raw_x(previous.x, sample.x) {
            if sample.chronology >= previous.chronology {
                previous.y = sample.y;
                previous.chronology = sample.chronology;
            }
            previous.mandatory |= sample.mandatory;
            return;
        }
    }
    samples.push(sample);
}

fn retain_raw_patches(mut patches: Vec<RawPatch>) -> Vec<RawPatch> {
    if patches.len() <= MAX_RAW_PATCHES {
        return patches;
    }

    let mut ranked = patches.drain(..).enumerate().collect::<Vec<_>>();
    ranked.sort_by(|(left_index, left), (right_index, right)| {
        patch_has_seam(right)
            .cmp(&patch_has_seam(left))
            .then_with(|| {
                patch_has_protected_geometry(right).cmp(&patch_has_protected_geometry(left))
            })
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| patch_latest_chronology(right).cmp(&patch_latest_chronology(left)))
            .then_with(|| right_index.cmp(left_index))
    });
    ranked.truncate(MAX_RAW_PATCHES);
    ranked.sort_by_key(|(index, _)| *index);
    ranked.into_iter().map(|(_, patch)| patch).collect()
}

fn retain_raw_constraints(mut constraints: Vec<RawConstraint>) -> Vec<RawConstraint> {
    if constraints.len() <= MAX_RAW_CONSTRAINTS {
        return constraints;
    }

    let mut ranked = constraints.drain(..).enumerate().collect::<Vec<_>>();
    ranked.sort_by(|(left_index, left), (right_index, right)| {
        right
            .seam
            .cmp(&left.seam)
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| right_index.cmp(left_index))
    });
    ranked.truncate(MAX_RAW_CONSTRAINTS);
    ranked.sort_by_key(|(index, _)| *index);
    ranked
        .into_iter()
        .map(|(_, constraint)| constraint)
        .collect()
}

fn patch_has_seam(patch: &RawPatch) -> bool {
    patch.samples.iter().any(|sample| is_raw_seam(sample.x))
}

fn patch_has_protected_geometry(patch: &RawPatch) -> bool {
    patch.samples.iter().any(|sample| sample.mandatory)
}

fn patch_latest_chronology(patch: &RawPatch) -> usize {
    patch
        .samples
        .iter()
        .map(|sample| sample.chronology)
        .max()
        .unwrap_or_default()
}

fn map_display_to_raw(display_x: f32, phase_offset: f32) -> f32 {
    let display_x = display_x.clamp(0.0, 1.0);
    if phase_offset <= GEOMETRY_EPSILON {
        if (display_x - 1.0).abs() <= GEOMETRY_EPSILON {
            1.0
        } else if display_x.abs() <= GEOMETRY_EPSILON {
            0.0
        } else {
            display_x
        }
    } else if (display_x - phase_offset).abs() <= GEOMETRY_EPSILON {
        0.0
    } else if display_x < phase_offset {
        1.0 - phase_offset + display_x
    } else {
        display_x - phase_offset
    }
}

fn canonical_raw_x(x: f32) -> f32 {
    if x.abs() <= RAW_X_MERGE_EPSILON {
        0.0
    } else if (x - 1.0).abs() <= RAW_X_MERGE_EPSILON {
        1.0
    } else {
        x.clamp(0.0, 1.0)
    }
}

fn same_raw_x(left: f32, right: f32) -> bool {
    (left - right).abs() <= RAW_X_MERGE_EPSILON
}

fn is_raw_seam(x: f32) -> bool {
    x.abs() <= RAW_X_MERGE_EPSILON || (x - 1.0).abs() <= RAW_X_MERGE_EPSILON
}

fn raw_seam_y(origin: &EditableCurve, patches: &[RawPatch], constraints: &[RawConstraint]) -> f32 {
    let mut best_priority = PaintPriority {
        chronology: 0,
        tie: 0,
    };
    let mut best_y = origin.nodes.first().map(|node| node.y).unwrap_or(1.0);
    for patch in patches {
        for sample in &patch.samples {
            if is_raw_seam(sample.x) && patch.priority >= best_priority {
                best_priority = patch.priority;
                best_y = sample.y;
            }
        }
    }
    for constraint in constraints.iter().filter(|constraint| constraint.seam) {
        if constraint.priority >= best_priority {
            best_priority = constraint.priority;
            best_y = constraint.y;
        }
    }
    best_y.clamp(0.0, 1.0)
}

fn mandatory_anchor_candidates(
    origin: &EditableCurve,
    patches: &[RawPatch],
    constraints: &[RawConstraint],
) -> Vec<AnchorX> {
    let mut anchors = Vec::new();
    anchors.push(AnchorX {
        x: 0.0,
        protected: true,
        rank: 5,
        priority: PaintPriority {
            chronology: 0,
            tie: 0,
        },
    });
    anchors.push(AnchorX {
        x: 1.0,
        protected: true,
        rank: 5,
        priority: PaintPriority {
            chronology: 0,
            tie: 1,
        },
    });
    for node in origin
        .nodes
        .iter()
        .copied()
        .skip(1)
        .take(origin.nodes.len().saturating_sub(2))
    {
        if !painted_at(node.x, patches, constraints) {
            anchors.push(AnchorX {
                x: node.x,
                protected: false,
                rank: 0,
                priority: PaintPriority {
                    chronology: 0,
                    tie: 0,
                },
            });
        }
    }
    for patch in patches {
        for sample in &patch.samples {
            if sample.mandatory {
                anchors.push(AnchorX {
                    x: canonical_raw_x(sample.x),
                    protected: true,
                    rank: 4,
                    priority: PaintPriority {
                        chronology: sample.chronology,
                        tie: patch.priority.tie,
                    },
                });
            }
        }
    }
    for constraint in constraints {
        anchors.push(AnchorX {
            x: canonical_raw_x(constraint.x),
            protected: true,
            rank: 4,
            priority: constraint.priority,
        });
    }
    let anchors = unique_anchor_candidates(anchors);
    reduce_anchor_candidates(anchors, origin, patches, constraints)
}

fn mandatory_anchor_xs(
    origin: &EditableCurve,
    patches: &[RawPatch],
    constraints: &[RawConstraint],
) -> Vec<f32> {
    mandatory_anchor_candidates(origin, patches, constraints)
        .into_iter()
        .map(|anchor| anchor.x)
        .collect()
}

fn optional_anchor_xs(mandatory: &[f32], patches: &[RawPatch]) -> Vec<f32> {
    let mut optional = Vec::new();
    for patch in patches {
        let Some(first) = patch.samples.first() else {
            continue;
        };
        let Some(last) = patch.samples.last() else {
            continue;
        };
        let span = last.x - first.x;
        if span <= RAW_X_MERGE_EPSILON {
            continue;
        }
        for step in 1..PAINT_FIT_RESAMPLE_STEPS {
            let x = first.x + span * step as f32 / PAINT_FIT_RESAMPLE_STEPS as f32;
            if !mandatory.iter().any(|anchor| same_raw_x(*anchor, x)) {
                optional.push(AnchorX {
                    x,
                    protected: false,
                    rank: 1,
                    priority: patch.priority,
                });
            }
        }
    }
    retain_anchor_candidates(unique_anchor_candidates(optional), MAX_OPTIONAL_FIT_ANCHORS)
        .into_iter()
        .map(|anchor| anchor.x)
        .collect()
}

fn unique_anchor_candidates(mut anchors: Vec<AnchorX>) -> Vec<AnchorX> {
    anchors.retain(|anchor| anchor.x.is_finite() && (0.0..=1.0).contains(&anchor.x));
    anchors.sort_by(|left, right| left.x.total_cmp(&right.x));
    let mut unique: Vec<AnchorX> = Vec::with_capacity(anchors.len());
    for anchor in anchors {
        if let Some(previous) = unique.last_mut() {
            if same_raw_x(previous.x, anchor.x) {
                if anchor_is_better(anchor, *previous) {
                    previous.x = anchor.x;
                    previous.priority = anchor.priority;
                    previous.rank = anchor.rank;
                }
                previous.protected |= anchor.protected;
                previous.rank = previous.rank.max(anchor.rank);
                previous.priority = previous.priority.max(anchor.priority);
                continue;
            }
        }
        unique.push(AnchorX {
            x: canonical_raw_x(anchor.x),
            ..anchor
        });
    }
    unique
}

fn anchor_is_better(left: AnchorX, right: AnchorX) -> bool {
    left.protected
        .cmp(&right.protected)
        .then_with(|| left.rank.cmp(&right.rank))
        .then_with(|| left.priority.cmp(&right.priority))
        .then_with(|| right.x.total_cmp(&left.x))
        == Ordering::Greater
}

fn reduce_anchor_candidates(
    anchors: Vec<AnchorX>,
    origin: &EditableCurve,
    patches: &[RawPatch],
    constraints: &[RawConstraint],
) -> Vec<AnchorX> {
    let seam_y = raw_seam_y(origin, patches, constraints);
    let mut anchors = coalesce_anchor_candidates(anchors, origin, seam_y, patches, constraints);
    anchors = retain_anchor_candidates(anchors, MAX_EDITABLE_NODES);
    coalesce_anchor_candidates(anchors, origin, seam_y, patches, constraints)
}

fn retain_anchor_candidates(mut anchors: Vec<AnchorX>, limit: usize) -> Vec<AnchorX> {
    if anchors.len() <= limit {
        return anchors;
    }

    anchors.sort_by(|left, right| {
        right
            .protected
            .cmp(&left.protected)
            .then_with(|| right.rank.cmp(&left.rank))
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| left.x.total_cmp(&right.x))
    });
    anchors.truncate(limit);
    anchors.sort_by(|left, right| left.x.total_cmp(&right.x));
    anchors
}

fn coalesce_anchor_candidates(
    mut anchors: Vec<AnchorX>,
    origin: &EditableCurve,
    seam_y: f32,
    patches: &[RawPatch],
    constraints: &[RawConstraint],
) -> Vec<AnchorX> {
    anchors.sort_by(|left, right| left.x.total_cmp(&right.x));
    let mut index = 0;
    while index + 1 < anchors.len() {
        let left = anchors[index];
        let right = anchors[index + 1];
        let dx = right.x - left.x;
        let left_y = target_y_at(origin, left.x, seam_y, patches, constraints);
        let right_y = target_y_at(origin, right.x, seam_y, patches, constraints);
        let close_2d =
            dx <= GENERATED_NODE_MERGE_X && (left_y - right_y).abs() <= GENERATED_NODE_MERGE_Y;
        if dx < RAW_NODE_ORDER_EPSILON || (close_2d && !(left.protected && right.protected)) {
            if left.protected && right.protected && dx >= RAW_NODE_ORDER_EPSILON {
                index += 1;
                continue;
            }
            if anchor_is_better(left, right) {
                anchors.remove(index + 1);
            } else {
                anchors.remove(index);
            }
            index = index.saturating_sub(1);
            continue;
        }
        index += 1;
    }
    anchors
}

fn coalesce_selected_xs(
    origin: &EditableCurve,
    selected: &[f32],
    mandatory: &[f32],
    seam_y: f32,
    patches: &[RawPatch],
    constraints: &[RawConstraint],
) -> Vec<f32> {
    let candidates = selected
        .iter()
        .copied()
        .map(|x| AnchorX {
            x,
            protected: mandatory.iter().any(|mandatory| same_raw_x(*mandatory, x)),
            rank: if mandatory.iter().any(|mandatory| same_raw_x(*mandatory, x)) {
                4
            } else {
                1
            },
            priority: PaintPriority {
                chronology: 0,
                tie: 0,
            },
        })
        .collect();
    coalesce_anchor_candidates(candidates, origin, seam_y, patches, constraints)
        .into_iter()
        .map(|anchor| anchor.x)
        .collect()
}

fn painted_at(x: f32, patches: &[RawPatch], constraints: &[RawConstraint]) -> bool {
    patches.iter().any(|patch| patch_covers(patch, x))
        || constraints
            .iter()
            .any(|constraint| same_raw_x(constraint.x, x))
}

fn patch_covers(patch: &RawPatch, x: f32) -> bool {
    let Some(first) = patch.samples.first() else {
        return false;
    };
    let Some(last) = patch.samples.last() else {
        return false;
    };
    x + RAW_X_MERGE_EPSILON >= first.x && x <= last.x + RAW_X_MERGE_EPSILON
}

fn patch_value(patch: &RawPatch, x: f32) -> Option<f32> {
    if !patch_covers(patch, x) {
        return None;
    }
    if let Some(sample) = patch.samples.iter().find(|sample| same_raw_x(sample.x, x)) {
        return Some(sample.y);
    }
    for pair in patch.samples.windows(2) {
        if x < pair[0].x - RAW_X_MERGE_EPSILON || x > pair[1].x + RAW_X_MERGE_EPSILON {
            continue;
        }
        let span = (pair[1].x - pair[0].x).max(RAW_X_MERGE_EPSILON);
        let fraction = ((x - pair[0].x) / span).clamp(0.0, 1.0);
        return Some(pair[0].y + (pair[1].y - pair[0].y) * fraction);
    }
    patch.samples.last().map(|sample| sample.y)
}

fn constraint_value(
    x: f32,
    seam_y: f32,
    constraints: &[RawConstraint],
) -> Option<(PaintPriority, f32)> {
    let mut best = None;
    for constraint in constraints {
        if (constraint.seam && is_raw_seam(x)) || (!constraint.seam && same_raw_x(constraint.x, x))
        {
            let candidate = (
                constraint.priority,
                if constraint.seam {
                    seam_y
                } else {
                    constraint.y
                },
            );
            if best.is_none_or(|current: (PaintPriority, f32)| candidate.0 >= current.0) {
                best = Some(candidate);
            }
        }
    }
    best
}

fn target_y_at(
    origin: &EditableCurve,
    x: f32,
    seam_y: f32,
    patches: &[RawPatch],
    constraints: &[RawConstraint],
) -> f32 {
    if is_raw_seam(x) {
        return seam_y;
    }
    let mut best = None;
    for patch in patches {
        if let Some(y) = patch_value(patch, x) {
            if best.is_none_or(|(priority, _): (PaintPriority, f32)| patch.priority >= priority) {
                best = Some((patch.priority, y));
            }
        }
    }
    if let Some((priority, y)) = constraint_value(x, seam_y, constraints) {
        if best.is_none_or(|(patch_priority, _): (PaintPriority, f32)| priority >= patch_priority) {
            best = Some((priority, y));
        }
    }
    best.map(|(_, y)| y)
        .unwrap_or_else(|| sample_editable_curve(origin, x))
        .clamp(0.0, 1.0)
}

fn candidate_error_at(curve: &EditableCurve, x: f32, context: FitContext<'_>) -> f32 {
    let target = target_y_at(
        curve,
        x,
        context.seam_y,
        context.patches,
        context.constraints,
    );
    let predicted = sample_editable_curve(curve, x);
    (predicted - target).abs()
}

fn painted_fit_samples(context: FitContext<'_>, left: f32, right: f32) -> Vec<FitSample> {
    let mut candidates = Vec::new();
    let mut add =
        |x: f32, protected: bool, rank: u8, priority: PaintPriority, chronology: usize| {
            if x.is_finite() && x >= left - RAW_X_MERGE_EPSILON && x <= right + RAW_X_MERGE_EPSILON
            {
                let x = x.clamp(left, right);
                let is_seam = is_raw_seam(x);
                candidates.push(FitSampleCandidate {
                    x,
                    protected: protected || is_seam,
                    rank: rank.max(if is_seam { 5 } else { 0 }),
                    priority,
                    chronology,
                });
            }
        };

    for patch in context.patches {
        let Some(first) = patch.samples.first() else {
            continue;
        };
        let Some(last) = patch.samples.last() else {
            continue;
        };
        let patch_start = first.x;
        let patch_end = last.x;
        let sample_start = left.max(patch_start);
        let sample_end = right.min(patch_end);
        if sample_start > sample_end + RAW_X_MERGE_EPSILON {
            continue;
        }
        let patch_latest_chronology = patch_latest_chronology(patch);

        // Always measure the painted interval boundaries, including when a
        // selected anchor was removed and a merged segment now crosses one.
        add(
            sample_start,
            true,
            3,
            patch.priority,
            patch_latest_chronology,
        );
        add(sample_end, true, 3, patch.priority, patch_latest_chronology);
        let overlap_span = sample_end - sample_start;
        if overlap_span > RAW_X_MERGE_EPSILON {
            for step in 1..PAINT_FIT_RESAMPLE_STEPS {
                add(
                    sample_start + overlap_span * step as f32 / PAINT_FIT_RESAMPLE_STEPS as f32,
                    false,
                    1,
                    patch.priority,
                    patch_latest_chronology,
                );
            }
        }

        // Keep the reducer sensitive to the captured patch shape and to the
        // same optional positions considered by the anchor-selection pass.
        let patch_span = patch_end - patch_start;
        if patch_span > RAW_X_MERGE_EPSILON {
            for step in 0..=PAINT_FIT_RESAMPLE_STEPS {
                add(
                    patch_start + patch_span * step as f32 / PAINT_FIT_RESAMPLE_STEPS as f32,
                    false,
                    2,
                    patch.priority,
                    patch_latest_chronology,
                );
            }
        }
        for sample in &patch.samples {
            add(
                sample.x,
                sample.mandatory,
                if sample.mandatory { 4 } else { 3 },
                patch.priority,
                sample.chronology,
            );
        }
    }

    for constraint in context.constraints {
        add(
            constraint.x,
            true,
            if constraint.seam { 5 } else { 4 },
            constraint.priority,
            constraint.priority.chronology,
        );
    }

    retain_fit_sample_candidates(candidates)
        .into_iter()
        .map(|candidate| FitSample {
            x: candidate.x,
            target_y: context.target_y_at(candidate.x),
        })
        .collect()
}

fn retain_fit_sample_candidates(
    mut candidates: Vec<FitSampleCandidate>,
) -> Vec<FitSampleCandidate> {
    candidates.retain(|candidate| candidate.x.is_finite());
    candidates.sort_by(|left, right| left.x.total_cmp(&right.x));
    let mut unique: Vec<FitSampleCandidate> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if let Some(previous) = unique.last_mut() {
            if same_raw_x(previous.x, candidate.x) {
                let previous_value = *previous;
                let mut merged = if fit_sample_candidate_is_better(candidate, previous_value) {
                    candidate
                } else {
                    previous_value
                };
                merged.protected |= previous_value.protected;
                merged.rank = merged.rank.max(previous_value.rank);
                merged.priority = merged.priority.max(previous_value.priority);
                merged.chronology = merged.chronology.max(previous_value.chronology);
                *previous = merged;
                continue;
            }
        }
        unique.push(candidate);
    }

    if unique.len() > MAX_FIT_SAMPLES_PER_SEGMENT {
        unique.sort_by(|left, right| {
            right
                .protected
                .cmp(&left.protected)
                .then_with(|| right.rank.cmp(&left.rank))
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| right.chronology.cmp(&left.chronology))
                .then_with(|| left.x.total_cmp(&right.x))
        });
        unique.truncate(MAX_FIT_SAMPLES_PER_SEGMENT);
        unique.sort_by(|left, right| left.x.total_cmp(&right.x));
    }
    unique
}

fn fit_sample_candidate_is_better(left: FitSampleCandidate, right: FitSampleCandidate) -> bool {
    left.protected
        .cmp(&right.protected)
        .then_with(|| left.rank.cmp(&right.rank))
        .then_with(|| left.priority.cmp(&right.priority))
        .then_with(|| left.chronology.cmp(&right.chronology))
        .then_with(|| right.x.total_cmp(&left.x))
        == Ordering::Greater
}

fn fit_candidate(
    origin: &EditableCurve,
    xs: &[f32],
    seam_y: f32,
    patches: &[RawPatch],
    constraints: &[RawConstraint],
) -> FittedCandidate {
    let nodes = xs
        .iter()
        .copied()
        .map(|x| CurveNode {
            x,
            y: target_y_at(origin, x, seam_y, patches, constraints),
        })
        .collect::<Vec<_>>();
    let mut segments = Vec::with_capacity(nodes.len().saturating_sub(1));
    let mut max_error: f32 = 0.0;
    for pair in nodes.windows(2) {
        let fit = if let Some(tension) =
            untouched_origin_tension(origin, pair[0], pair[1], patches, constraints)
        {
            SegmentFit {
                tension,
                max_error: 0.0,
            }
        } else {
            fit_segment(origin, pair[0], pair[1], seam_y, patches, constraints)
        };
        max_error = max_error.max(fit.max_error);
        segments.push(CurveSegment {
            tension: fit.tension.clamp(MIN_SEGMENT_TENSION, MAX_SEGMENT_TENSION),
        });
    }
    FittedCandidate {
        curve: EditableCurve {
            nodes,
            segments,
            ..EditableCurve::default()
        },
        max_error,
    }
}

fn untouched_origin_tension(
    origin: &EditableCurve,
    left: CurveNode,
    right: CurveNode,
    patches: &[RawPatch],
    constraints: &[RawConstraint],
) -> Option<f32> {
    let left_index = origin
        .nodes
        .iter()
        .position(|node| same_raw_x(node.x, left.x))?;
    if origin.nodes.get(left_index + 1).copied()? != right
        || painted_at(left.x, patches, constraints)
        || painted_at(right.x, patches, constraints)
        || patches.iter().any(|patch| {
            let first = patch.samples.first().map(|sample| sample.x).unwrap_or(1.0);
            let last = patch.samples.last().map(|sample| sample.x).unwrap_or(0.0);
            first < right.x - RAW_X_MERGE_EPSILON && last > left.x + RAW_X_MERGE_EPSILON
        })
        || constraints.iter().any(|constraint| {
            !constraint.seam
                && constraint.x > left.x + RAW_X_MERGE_EPSILON
                && constraint.x < right.x - RAW_X_MERGE_EPSILON
        })
    {
        return None;
    }
    origin
        .segments
        .get(left_index)
        .map(|segment| segment.tension)
}

fn fit_segment(
    origin: &EditableCurve,
    left: CurveNode,
    right: CurveNode,
    seam_y: f32,
    patches: &[RawPatch],
    constraints: &[RawConstraint],
) -> SegmentFit {
    let baseline = origin
        .nodes
        .windows(2)
        .enumerate()
        .find(|(_, pair)| {
            let midpoint = (left.x + right.x) * 0.5;
            midpoint >= pair[0].x && midpoint <= pair[1].x
        })
        .and_then(|(index, _)| origin.segments.get(index))
        .map(|segment| segment.tension)
        .unwrap_or(0.0);
    let context = FitContext {
        origin,
        seam_y,
        patches,
        constraints,
    };
    let samples = painted_fit_samples(context, left.x, right.x);
    if samples.is_empty() {
        return SegmentFit {
            tension: baseline,
            max_error: 0.0,
        };
    }
    let mut best = SegmentFit {
        tension: 0.0,
        max_error: f32::INFINITY,
    };
    for step in 0..=PAINT_FIT_TENSION_STEPS {
        let tension = MIN_SEGMENT_TENSION
            + (MAX_SEGMENT_TENSION - MIN_SEGMENT_TENSION) * step as f32
                / PAINT_FIT_TENSION_STEPS as f32;
        let candidate = evaluate_segment_tension(context, left, right, tension, &samples);
        if segment_fit_is_better(candidate, best, baseline) {
            best = candidate;
        }
    }
    let mut refinement_span =
        (MAX_SEGMENT_TENSION - MIN_SEGMENT_TENSION) / PAINT_FIT_TENSION_STEPS as f32;
    for _ in 0..2 {
        let start = (best.tension - refinement_span).max(MIN_SEGMENT_TENSION);
        let end = (best.tension + refinement_span).min(MAX_SEGMENT_TENSION);
        for step in 0..=PAINT_FIT_TENSION_REFINEMENT_STEPS {
            let tension =
                start + (end - start) * step as f32 / PAINT_FIT_TENSION_REFINEMENT_STEPS as f32;
            let candidate = evaluate_segment_tension(context, left, right, tension, &samples);
            if segment_fit_is_better(candidate, best, baseline) {
                best = candidate;
            }
        }
        refinement_span /= PAINT_FIT_TENSION_REFINEMENT_STEPS as f32;
    }
    best
}

fn evaluate_segment_tension(
    _context: FitContext<'_>,
    left: CurveNode,
    right: CurveNode,
    tension: f32,
    samples: &[FitSample],
) -> SegmentFit {
    let mut max_error: f32 = 0.0;
    for sample in samples {
        let predicted = sample_curve_segment(left, right, tension, sample.x);
        max_error = max_error.max((predicted - sample.target_y).abs());
    }
    SegmentFit { tension, max_error }
}

fn simplify_selected_anchors(
    origin: &EditableCurve,
    selected_xs: &[f32],
    mandatory_xs: &[f32],
    seam_y: f32,
    patches: &[RawPatch],
    constraints: &[RawConstraint],
    mut fitted: FittedCandidate,
) -> FittedCandidate {
    let mut selected = selected_xs.to_vec();
    loop {
        let mut removed = false;
        for index in 1..selected.len().saturating_sub(1) {
            if mandatory_xs
                .iter()
                .any(|mandatory| same_raw_x(*mandatory, selected[index]))
            {
                continue;
            }

            let mut merged = selected.clone();
            merged.remove(index);
            let candidate = fit_candidate(origin, &merged, seam_y, patches, constraints);
            if candidate.max_error <= PAINT_FIT_TOLERANCE {
                selected = merged;
                fitted = candidate;
                removed = true;
                break;
            }
        }
        if !removed {
            return fitted;
        }
    }
}

fn candidate_is_better(
    origin: &EditableCurve,
    candidate: &FittedCandidate,
    current: Option<&FittedCandidate>,
) -> bool {
    if !candidate_is_valid(&candidate.curve) || candidate.curve == *origin {
        return false;
    }
    let Some(current) = current else {
        return true;
    };
    let candidate_within_tolerance = candidate.max_error <= PAINT_FIT_TOLERANCE;
    let current_within_tolerance = current.max_error <= PAINT_FIT_TOLERANCE;
    match (candidate_within_tolerance, current_within_tolerance) {
        (true, false) => true,
        (false, true) => false,
        (true, true) => {
            candidate
                .curve
                .nodes
                .len()
                .cmp(&current.curve.nodes.len())
                .then_with(|| candidate.max_error.total_cmp(&current.max_error))
                .then_with(|| compare_candidate_tension_and_topology(candidate, current))
                == Ordering::Less
        }
        (false, false) => {
            candidate
                .max_error
                .total_cmp(&current.max_error)
                .then_with(|| candidate.curve.nodes.len().cmp(&current.curve.nodes.len()))
                .then_with(|| compare_candidate_tension_and_topology(candidate, current))
                == Ordering::Less
        }
    }
}

fn compare_candidate_tension_and_topology(
    left: &FittedCandidate,
    right: &FittedCandidate,
) -> Ordering {
    let left_total_tension = left
        .curve
        .segments
        .iter()
        .map(|segment| segment.tension.abs())
        .sum::<f32>();
    let right_total_tension = right
        .curve
        .segments
        .iter()
        .map(|segment| segment.tension.abs())
        .sum::<f32>();
    left_total_tension
        .total_cmp(&right_total_tension)
        .then_with(|| {
            left.curve
                .segments
                .iter()
                .map(|segment| segment.tension)
                .zip(right.curve.segments.iter().map(|segment| segment.tension))
                .map(|(left, right)| left.total_cmp(&right))
                .find(|ordering| *ordering != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| {
            left.curve
                .nodes
                .iter()
                .zip(right.curve.nodes.iter())
                .flat_map(|(left, right)| [left.x.total_cmp(&right.x), left.y.total_cmp(&right.y)])
                .find(|ordering| *ordering != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        })
}

fn segment_fit_is_better(candidate: SegmentFit, current: SegmentFit, baseline: f32) -> bool {
    if candidate.max_error < current.max_error - PAINT_FIT_EPSILON {
        return true;
    }
    if (candidate.max_error - current.max_error).abs() > PAINT_FIT_EPSILON {
        return false;
    }
    let candidate_distance = (candidate.tension - baseline).abs();
    let current_distance = (current.tension - baseline).abs();
    if candidate_distance < current_distance - PAINT_FIT_EPSILON {
        return true;
    }
    if (candidate_distance - current_distance).abs() <= PAINT_FIT_EPSILON {
        return candidate.tension.total_cmp(&current.tension) == Ordering::Less;
    }
    false
}

fn candidate_is_valid(curve: &EditableCurve) -> bool {
    curve.nodes.len() >= 2
        && curve.nodes.len() <= MAX_EDITABLE_NODES
        && curve.segments.len() == curve.nodes.len() - 1
        && curve.nodes.first().is_some_and(|node| node.x == 0.0)
        && curve.nodes.last().is_some_and(|node| node.x == 1.0)
        && curve.nodes.first().is_some_and(|node| {
            curve
                .nodes
                .last()
                .is_some_and(|last| (node.y - last.y).abs() <= RAW_X_MERGE_EPSILON)
        })
        && curve.nodes.iter().all(|node| {
            node.x.is_finite()
                && node.y.is_finite()
                && (0.0..=1.0).contains(&node.x)
                && (0.0..=1.0).contains(&node.y)
        })
        && curve
            .nodes
            .windows(2)
            .all(|pair| pair[1].x - pair[0].x >= RAW_NODE_ORDER_EPSILON)
        && curve.segments.iter().all(|segment| {
            segment.tension.is_finite()
                && (MIN_SEGMENT_TENSION..=MAX_SEGMENT_TENSION).contains(&segment.tension)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> RectBounds {
        RectBounds {
            min: RectPoint { x: 0.0, y: 0.0 },
            max: RectPoint { x: 1.0, y: 1.0 },
        }
    }

    fn point(x: f32, y: f32) -> RectPoint {
        RectPoint { x, y }
    }

    fn outside_on(edge: BoundaryEdge, parameter: f32) -> RectPoint {
        let epsilon = 0.25;
        match edge {
            BoundaryEdge::Top => point(parameter, -epsilon),
            BoundaryEdge::Right => point(1.0 + epsilon, parameter),
            BoundaryEdge::Bottom => point(1.0 - parameter, 1.0 + epsilon),
            BoundaryEdge::Left => point(-epsilon, 1.0 - parameter),
        }
    }

    fn positions(run: &PaintRun) -> Vec<RectPoint> {
        run.points.iter().map(|point| point.position).collect()
    }

    fn flat_curve(y: f32) -> EditableCurve {
        EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y }, CurveNode { x: 1.0, y }],
            segments: vec![CurveSegment { tension: 0.0 }],
            ..EditableCurve::default()
        }
        .normalized()
    }

    fn interior_run(points: impl IntoIterator<Item = (f32, f32)>) -> PaintRun {
        PaintRun {
            points: points
                .into_iter()
                .map(|(x, y)| PaintPoint {
                    position: point(x, y),
                    contact: BoundaryContact::Interior,
                })
                .collect(),
        }
    }

    #[test]
    fn classify_point_reports_typed_edge_parameters_and_corners() {
        let top = classify_point(bounds(), point(0.25, -0.2)).unwrap();
        assert_eq!(
            top.contact,
            BoundaryContact::Edge(EdgeParameter {
                edge: BoundaryEdge::Top,
                parameter: 0.25,
            })
        );
        assert_eq!(
            classify_point(bounds(), point(-0.2, -0.2)).unwrap().contact,
            BoundaryContact::Corner(BoundaryCorner::TopLeft)
        );
        assert_eq!(
            classify_point(bounds(), point(0.5, 0.5)).unwrap().contact,
            BoundaryContact::Interior
        );
    }

    #[test]
    fn interior_to_outside_uses_the_first_segment_intersection() {
        let mut recorder = StrokeRecorder::new(bounds());
        recorder.observe(point(0.25, 0.5));
        recorder.observe(point(1.5, 0.75));

        assert_eq!(recorder.runs().len(), 1);
        assert_eq!(
            positions(&recorder.runs()[0]),
            vec![point(0.25, 0.5), point(1.0, 0.65), point(1.0, 0.75)]
        );
        assert!(matches!(
            recorder.runs()[0].points().last().unwrap().contact,
            BoundaryContact::Edge(EdgeParameter {
                edge: BoundaryEdge::Right,
                ..
            })
        ));
    }

    #[test]
    fn all_directed_adjacent_edge_transitions_insert_the_exact_corner() {
        let transitions = [
            (BoundaryEdge::Top, BoundaryEdge::Right, point(1.0, 0.0)),
            (BoundaryEdge::Right, BoundaryEdge::Top, point(1.0, 0.0)),
            (BoundaryEdge::Right, BoundaryEdge::Bottom, point(1.0, 1.0)),
            (BoundaryEdge::Bottom, BoundaryEdge::Right, point(1.0, 1.0)),
            (BoundaryEdge::Bottom, BoundaryEdge::Left, point(0.0, 1.0)),
            (BoundaryEdge::Left, BoundaryEdge::Bottom, point(0.0, 1.0)),
            (BoundaryEdge::Left, BoundaryEdge::Top, point(0.0, 0.0)),
            (BoundaryEdge::Top, BoundaryEdge::Left, point(0.0, 0.0)),
        ];

        for (from, to, corner) in transitions {
            let mut recorder = StrokeRecorder::new(bounds());
            recorder.observe_outside(outside_on(from, 0.25));
            recorder.observe_outside(outside_on(to, 0.75));
            assert_eq!(recorder.runs().len(), 1, "{from:?} -> {to:?}");
            let run = &recorder.runs()[0];
            assert!(
                run.points().iter().any(|point| point.position == corner),
                "{from:?} -> {to:?} omitted {corner:?}: {:?}",
                positions(run)
            );
        }
    }

    #[test]
    fn every_corner_is_preserved_as_a_typed_contact() {
        let corners = [
            (point(-0.1, -0.1), BoundaryCorner::TopLeft),
            (point(1.1, -0.1), BoundaryCorner::TopRight),
            (point(1.1, 1.1), BoundaryCorner::BottomRight),
            (point(-0.1, 1.1), BoundaryCorner::BottomLeft),
        ];
        for (raw, expected) in corners {
            assert_eq!(
                classify_point(bounds(), raw).unwrap().contact,
                BoundaryContact::Corner(expected)
            );
        }
    }

    #[test]
    fn same_edge_motion_tracks_ordered_perimeter_positions() {
        let mut recorder = StrokeRecorder::new(bounds());
        recorder.observe_outside(outside_on(BoundaryEdge::Top, 0.2));
        recorder.observe_outside(outside_on(BoundaryEdge::Top, 0.8));
        recorder.observe_outside(outside_on(BoundaryEdge::Top, 0.4));

        assert_eq!(recorder.runs().len(), 1);
        assert_eq!(
            positions(&recorder.runs()[0]),
            vec![point(0.2, 0.0), point(0.8, 0.0), point(0.4, 0.0)]
        );
    }

    #[test]
    fn nonadjacent_sparse_jumps_break_runs_without_a_fabricated_chord() {
        let mut recorder = StrokeRecorder::new(bounds());
        recorder.observe_outside(outside_on(BoundaryEdge::Top, 0.2));
        recorder.observe_outside(outside_on(BoundaryEdge::Bottom, 0.8));

        assert_eq!(recorder.runs().len(), 2);
        assert_eq!(positions(&recorder.runs()[0]), vec![point(0.2, 0.0)]);
        let bottom = positions(&recorder.runs()[1]);
        assert_eq!(bottom.len(), 1);
        assert!((bottom[0].x - 0.2).abs() <= GEOMETRY_EPSILON);
        assert!((bottom[0].y - 1.0).abs() <= GEOMETRY_EPSILON);
    }

    #[test]
    fn duplicate_events_do_not_duplicate_points_or_corners() {
        let mut recorder = StrokeRecorder::new(bounds());
        let interior = point(0.2, 0.3);
        recorder.observe(interior);
        recorder.observe(interior);
        recorder.observe_outside(outside_on(BoundaryEdge::Top, 0.2));
        recorder.observe_outside(outside_on(BoundaryEdge::Top, 0.2));
        recorder.observe_outside(outside_on(BoundaryEdge::Right, 0.8));
        recorder.observe_outside(outside_on(BoundaryEdge::Right, 0.8));

        assert_eq!(recorder.runs().len(), 1);
        assert_eq!(
            positions(&recorder.runs()[0]),
            vec![interior, point(0.2, 0.0), point(1.0, 0.0), point(1.0, 0.8)]
        );
    }

    #[test]
    fn reentry_starts_a_new_run_at_the_last_boundary_intersection() {
        let mut recorder = StrokeRecorder::new(bounds());
        recorder.observe(point(0.5, 0.5));
        recorder.observe_outside(point(0.5, -0.5));
        recorder.observe(point(0.75, 0.5));

        assert_eq!(recorder.runs().len(), 2);
        assert_eq!(
            positions(&recorder.runs()[0]),
            vec![point(0.5, 0.5), point(0.5, 0.0)]
        );
        assert_eq!(
            positions(&recorder.runs()[1]),
            vec![point(0.625, 0.0), point(0.75, 0.5)]
        );
    }

    #[test]
    fn nonfinite_observations_are_ignored_without_poisoning_the_stream() {
        let mut recorder = StrokeRecorder::new(bounds());
        recorder.observe(point(0.25, 0.25));
        recorder.observe(point(f32::NAN, 0.5));
        recorder.observe(point(0.5, f32::INFINITY));
        recorder.observe(point(0.75, 0.75));

        assert_eq!(recorder.runs().len(), 1);
        assert_eq!(
            positions(&recorder.runs()[0]),
            vec![point(0.25, 0.25), point(0.75, 0.75)]
        );
        assert!(classify_point(bounds(), point(f32::NEG_INFINITY, 0.5)).is_none());
    }

    #[test]
    fn invalid_bounds_reject_all_observations() {
        let mut recorder = StrokeRecorder::new(RectBounds {
            min: point(1.0, 0.0),
            max: point(0.0, 1.0),
        });
        recorder.observe(point(0.5, 0.5));
        assert!(recorder.runs().is_empty());
    }

    #[test]
    fn dense_stroke_stays_within_capture_bound_and_reconstructs_valid_candidate() {
        let mut recorder = StrokeRecorder::new(bounds());
        for index in 0..20_000 {
            let progress = index as f32 / 19_999.0;
            recorder.observe(point(0.1 + progress * 0.8, 0.25 + progress * 0.5));
        }

        assert!(!recorder.is_truncated());
        assert_eq!(recorder.runs().len(), 1);
        assert!(recorder
            .runs()
            .iter()
            .all(|run| { run.points().len() <= MAX_CAPTURED_POINTS_PER_RUN }));
        assert!(paint_runs_within_capture_budget(recorder.runs()));
        assert_eq!(
            recorder.point_count,
            recorder
                .runs()
                .iter()
                .map(|run| run.points().len())
                .sum::<usize>()
        );
        assert_eq!(
            recorder.runs()[0].points().first().unwrap().position,
            point(0.1, 0.25)
        );
        let dense_endpoint = recorder.runs()[0].points().last().unwrap().position;
        assert!((dense_endpoint.x - 0.9).abs() <= GEOMETRY_EPSILON);
        assert!((dense_endpoint.y - 0.75).abs() <= GEOMETRY_EPSILON);

        let outcome = reconstruct_paint(&flat_curve(0.5), 0.0, recorder.runs());
        assert!(candidate_is_valid(outcome.candidate()));
    }

    #[test]
    fn dense_straight_stroke_matches_sparse_endpoint_and_shape_result() {
        let origin = flat_curve(0.5);
        let mut dense = StrokeRecorder::new(bounds());
        for index in 0..10_000 {
            let progress = index as f32 / 9_999.0;
            dense.observe(point(0.1 + progress * 0.8, 0.2 + progress * 0.6));
        }
        let sparse = interior_run([(0.1, 0.2), (0.9, 0.8)]);

        assert!(dense.runs()[0].points().len() < 128);
        assert_eq!(
            dense.runs()[0].points().first().unwrap().position,
            point(0.1, 0.2)
        );
        let dense_endpoint = dense.runs()[0].points().last().unwrap().position;
        assert!((dense_endpoint.x - 0.9).abs() <= GEOMETRY_EPSILON);
        assert!((dense_endpoint.y - 0.8).abs() <= GEOMETRY_EPSILON);
        let dense_candidate = reconstruct_paint(&origin, 0.0, dense.runs());
        let sparse_candidate = reconstruct_paint(&origin, 0.0, &[sparse]);
        let dense_curve = dense_candidate.candidate();
        let sparse_curve = sparse_candidate.candidate();
        assert_eq!(dense_curve.nodes.len(), sparse_curve.nodes.len());
        assert_eq!(dense_curve.segments.len(), sparse_curve.segments.len());
        for (dense, sparse) in dense_curve.nodes.iter().zip(&sparse_curve.nodes) {
            assert!((dense.x - sparse.x).abs() <= 1.0e-5);
            assert!((dense.y - sparse.y).abs() <= 1.0e-5);
        }
        for (dense, sparse) in dense_curve.segments.iter().zip(&sparse_curve.segments) {
            assert!((dense.tension - sparse.tension).abs() <= 1.0e-5);
        }
    }

    #[test]
    fn capture_run_count_is_hard_bounded_without_joining_runs() {
        let mut recorder = StrokeRecorder::new(bounds());
        for _ in 0..(MAX_CAPTURED_RUNS + 4) {
            recorder.observe_outside(outside_on(BoundaryEdge::Top, 0.2));
            recorder.observe_outside(outside_on(BoundaryEdge::Bottom, 0.8));
        }

        assert!(recorder.is_truncated());
        assert!(recorder.runs().len() <= MAX_CAPTURED_RUNS);
        assert!(recorder
            .runs()
            .iter()
            .all(|run| { run.points().len() <= MAX_CAPTURED_POINTS_PER_RUN }));
        assert!(paint_runs_within_capture_budget(recorder.runs()));
        assert_eq!(
            recorder.point_count,
            recorder
                .runs()
                .iter()
                .map(|run| run.points().len())
                .sum::<usize>()
        );
    }

    #[test]
    fn dense_x_reversal_retains_the_turning_point_after_coalescing() {
        let mut recorder = StrokeRecorder::new(bounds());
        for index in 0..1_000 {
            let progress = index as f32 / 999.0;
            recorder.observe(point(0.1 + progress * 0.8, 0.2 + progress * 0.3));
        }
        for index in 1..1_000 {
            let progress = index as f32 / 999.0;
            recorder.observe(point(0.9 - progress * 0.7, 0.5 - progress * 0.2));
        }

        let run = &recorder.runs()[0];
        assert!(run.points().len() <= MAX_CAPTURED_POINTS_PER_RUN);
        assert!(run
            .points()
            .iter()
            .any(|point| (point.position.x - 0.9).abs() <= GEOMETRY_EPSILON));
        let (fragments, constraints) = collect_display_geometry(recorder.runs());
        assert!(constraints.is_empty());
        assert_eq!(fragments.len(), 2);
    }

    #[test]
    fn protected_anchor_overflow_keeps_a_bounded_best_effort_candidate() {
        let samples = (0..=MAX_CAPTURED_POINTS_PER_RUN)
            .map(|index| {
                if index % 2 == 0 {
                    point(0.1, 0.2)
                } else {
                    point(0.9, 0.8)
                }
            })
            .collect::<Vec<_>>();
        let protected_count = samples
            .iter()
            .map(|sample| classify_point(bounds(), *sample).expect("sample is in bounds"))
            .collect::<Vec<_>>();
        assert!(
            protected_point_indices(&protected_count)
                .into_iter()
                .filter(|is_protected| *is_protected)
                .count()
                > MAX_CAPTURED_POINTS_PER_RUN
        );

        let mut recorder = StrokeRecorder::new(bounds());
        for sample in samples {
            recorder.observe(sample);
        }

        assert!(recorder.is_truncated());
        assert_eq!(recorder.runs().len(), 1);
        assert!(recorder
            .runs()
            .iter()
            .all(|run| run.points().len() <= MAX_CAPTURED_POINTS_PER_RUN));
        assert_eq!(recorder.point_count, MAX_CAPTURED_POINTS_PER_RUN);
        assert!(paint_runs_within_capture_budget(recorder.runs()));

        let origin = flat_curve(0.5);
        let outcome = reconstruct_paint(&origin, 0.0, recorder.runs());
        assert!(candidate_is_valid(outcome.candidate()));
        assert_ne!(outcome.candidate(), &origin);
    }

    #[test]
    fn protected_anchor_overflow_keeps_corner_transition_best_effort() {
        let mut recorder = StrokeRecorder::new(bounds());
        for index in 0..127 {
            let sample = if index % 2 == 0 {
                point(0.1, 0.2)
            } else {
                point(0.9, 0.8)
            };
            recorder.observe(sample);
        }
        recorder.observe_outside(point(0.5, -1.0));
        assert_eq!(
            recorder.runs()[0].points().len(),
            MAX_CAPTURED_POINTS_PER_RUN
        );

        recorder.observe_outside(point(1.25, 0.5));

        assert!(recorder.is_truncated());
        assert_eq!(recorder.runs().len(), 1);
        assert_eq!(
            recorder.runs()[0].points().len(),
            MAX_CAPTURED_POINTS_PER_RUN
        );
        assert_eq!(recorder.point_count, MAX_CAPTURED_POINTS_PER_RUN);
        assert!(recorder.runs()[0]
            .points()
            .iter()
            .any(|point| matches!(point.contact, BoundaryContact::Corner(_))));
        assert!(paint_runs_within_capture_budget(recorder.runs()));
    }

    #[test]
    fn reversing_display_x_splits_fragments_before_raw_mapping() {
        let origin = flat_curve(0.5);
        let run = interior_run([(0.20, 0.20), (0.80, 0.80), (0.30, 0.10)]);

        let (fragments, constraints) = collect_display_geometry(std::slice::from_ref(&run));
        assert!(constraints.is_empty());
        assert_eq!(fragments.len(), 2);
        assert!(fragments.iter().all(|fragment| {
            fragment
                .points
                .windows(2)
                .all(|pair| pair[1].x >= pair[0].x)
        }));
        assert_eq!(
            fragments[0]
                .points
                .iter()
                .map(|point| (point.x, point.y))
                .collect::<Vec<_>>(),
            vec![(0.20, 0.20), (0.80, 0.80)]
        );
        assert_eq!(
            fragments[1]
                .points
                .iter()
                .map(|point| (point.x, point.y))
                .collect::<Vec<_>>(),
            vec![(0.30, 0.10), (0.80, 0.80)]
        );

        let (patches, constraints) = raw_geometry(&fragments, &constraints, 0.0);
        assert_eq!(patches.len(), 2);
        assert!(patches.iter().all(|patch| {
            let first = patch.samples.first().unwrap().x;
            let last = patch.samples.last().unwrap().x;
            first > 0.0 && last < 1.0
        }));
        assert!(patches.iter().any(|patch| {
            (patch.samples.first().unwrap().x - 0.20).abs() <= GEOMETRY_EPSILON
                && (patch.samples.last().unwrap().x - 0.80).abs() <= GEOMETRY_EPSILON
        }));
        assert!(patches.iter().any(|patch| {
            (patch.samples.first().unwrap().x - 0.30).abs() <= GEOMETRY_EPSILON
                && (patch.samples.last().unwrap().x - 0.80).abs() <= GEOMETRY_EPSILON
        }));
        let seam_y = raw_seam_y(&origin, &patches, &constraints);
        assert!(
            (target_y_at(&origin, 0.25, seam_y, &patches, &constraints) - 0.25).abs()
                <= GEOMETRY_EPSILON
        );
        assert!(
            (target_y_at(&origin, 0.40, seam_y, &patches, &constraints) - 0.24).abs()
                <= GEOMETRY_EPSILON
        );
        assert!(
            (target_y_at(&origin, 0.75, seam_y, &patches, &constraints) - 0.73).abs()
                <= GEOMETRY_EPSILON
        );
        assert!(
            (target_y_at(&origin, 0.90, seam_y, &patches, &constraints) - 0.5).abs()
                <= GEOMETRY_EPSILON
        );

        let (fragments, constraints) = collect_display_geometry(std::slice::from_ref(&run));
        let (patches, constraints) = raw_geometry(&fragments, &constraints, 0.6);
        assert_eq!(patches.len(), 4);
        let count_range = |start: f32, end: f32| {
            patches
                .iter()
                .filter(|patch| {
                    (patch.samples.first().unwrap().x - start).abs() <= GEOMETRY_EPSILON
                        && (patch.samples.last().unwrap().x - end).abs() <= GEOMETRY_EPSILON
                })
                .count()
        };
        assert_eq!(count_range(0.0, 0.2), 2);
        assert_eq!(count_range(0.6, 1.0), 1);
        assert_eq!(count_range(0.7, 1.0), 1);
        let seam_y = raw_seam_y(&origin, &patches, &constraints);
        assert!(
            (target_y_at(&origin, 0.65, seam_y, &patches, &constraints) - 0.25).abs()
                <= GEOMETRY_EPSILON
        );
        assert!(
            (target_y_at(&origin, 0.75, seam_y, &patches, &constraints) - 0.17).abs()
                <= GEOMETRY_EPSILON
        );
        assert!(
            (target_y_at(&origin, 0.30, seam_y, &patches, &constraints) - 0.5).abs()
                <= GEOMETRY_EPSILON
        );
    }

    #[test]
    fn reconstruct_paint_reports_applied_noop_and_capacity_best_effort() {
        let origin = flat_curve(0.5);
        let applied = reconstruct_paint(&origin, 0.0, &[interior_run([(0.2, 0.2), (0.8, 0.8)])]);
        assert!(matches!(&applied, PaintCommitOutcome::Applied { .. }));
        assert_ne!(applied.candidate(), &origin);

        let no_op = reconstruct_paint(&origin, 0.0, &[interior_run([(0.4, 0.2)])]);
        assert!(matches!(&no_op, PaintCommitOutcome::NoOp { .. }));
        assert_eq!(no_op.candidate(), &origin);

        let best_effort = reconstruct_paint(
            &origin,
            0.0,
            &[interior_run((0..70).map(|index| {
                let x = 0.1 + index as f32 * 0.8 / 69.0;
                let y = if index % 2 == 0 { 0.1 } else { 0.9 };
                (x, y)
            }))],
        );
        assert!(matches!(&best_effort, PaintCommitOutcome::Applied { .. }));
        assert!(candidate_is_valid(best_effort.candidate()));
        assert_ne!(best_effort.candidate(), &origin);
    }

    #[test]
    fn release_fit_terminates_when_coalescing_removes_an_optional_anchor() {
        let origin = flat_curve(0.5);
        // This stroke makes the highest-error optional anchor coalesce away.
        // Before release tracked attempted anchors, the same candidate stayed
        // eligible forever because optional_xs is immutable.
        let run = interior_run([
            (0.5484174, 0.90177536),
            (0.555112, 0.12531574),
            (0.5618067, 0.17531161),
            (0.5685013, 0.94877195),
        ]);

        let first = reconstruct_paint(&origin, 0.0, std::slice::from_ref(&run));
        let second = reconstruct_paint(&origin, 0.0, &[run]);

        assert_eq!(first, second);
        assert!(matches!(&first, PaintCommitOutcome::Applied { .. }));
        assert!(candidate_is_valid(first.candidate()));
        assert_ne!(first.candidate(), &origin);
    }

    #[test]
    fn phase_seam_micro_crossing_becomes_one_changed_seam_candidate() {
        let origin = flat_curve(0.5);
        let run = interior_run([(0.4999992, 0.2), (0.5000012, 0.8)]);
        let (fragments, display_constraints) = collect_display_geometry(std::slice::from_ref(&run));
        let (patches, _) = raw_geometry(&fragments, &display_constraints, 0.5);

        assert!(patches
            .iter()
            .any(|patch| { patch.samples.len() == 1 && is_raw_seam(patch.samples[0].x) }));
        let outcome = reconstruct_paint(&origin, 0.5, &[run]);
        assert!(matches!(&outcome, PaintCommitOutcome::Applied { .. }));
        assert!(candidate_is_valid(outcome.candidate()));
        assert_ne!(outcome.candidate(), &origin);
    }

    #[test]
    fn release_anchor_cleanup_merges_close_unprotected_nodes_in_two_dimensions() {
        let origin = flat_curve(0.5);
        let anchors = vec![
            AnchorX {
                x: 0.4,
                protected: false,
                rank: 1,
                priority: PaintPriority {
                    chronology: 1,
                    tie: 0,
                },
            },
            AnchorX {
                x: 0.402,
                protected: false,
                rank: 1,
                priority: PaintPriority {
                    chronology: 2,
                    tie: 0,
                },
            },
        ];
        let merged = coalesce_anchor_candidates(anchors, &origin, 0.5, &[], &[]);
        assert_eq!(merged.len(), 1);
        assert!((merged[0].x - 0.402).abs() <= GENERATED_NODE_MERGE_X);
    }

    #[test]
    fn release_anchor_cleanup_keeps_protected_pairs_and_protected_wins() {
        let origin = flat_curve(0.5);
        let protected_pair = vec![
            AnchorX {
                x: 0.4,
                protected: true,
                rank: 4,
                priority: PaintPriority {
                    chronology: 1,
                    tie: 0,
                },
            },
            AnchorX {
                x: 0.402,
                protected: true,
                rank: 4,
                priority: PaintPriority {
                    chronology: 2,
                    tie: 0,
                },
            },
        ];
        assert_eq!(
            coalesce_anchor_candidates(protected_pair, &origin, 0.5, &[], &[]).len(),
            2
        );

        let protected_wins = vec![
            AnchorX {
                x: 0.4,
                protected: false,
                rank: 0,
                priority: PaintPriority {
                    chronology: 2,
                    tie: 0,
                },
            },
            AnchorX {
                x: 0.402,
                protected: true,
                rank: 4,
                priority: PaintPriority {
                    chronology: 1,
                    tie: 0,
                },
            },
        ];
        let retained = coalesce_anchor_candidates(protected_wins, &origin, 0.5, &[], &[]);
        assert_eq!(retained.len(), 1);
        assert!(retained[0].protected);
    }

    #[test]
    fn full_capacity_origin_accepts_a_narrow_painted_interval() {
        let origin = EditableCurve {
            nodes: (0..MAX_EDITABLE_NODES)
                .map(|index| CurveNode {
                    x: index as f32 / (MAX_EDITABLE_NODES - 1) as f32,
                    y: 0.2 + 0.6 * (index as f32 / (MAX_EDITABLE_NODES - 1) as f32),
                })
                .collect(),
            segments: vec![CurveSegment { tension: 0.0 }; MAX_EDITABLE_NODES - 1],
            ..EditableCurve::default()
        }
        .normalized();
        let outcome =
            reconstruct_paint(&origin, 0.0, &[interior_run([(0.005, 0.85), (0.02, 0.15)])]);
        assert!(matches!(&outcome, PaintCommitOutcome::Applied { .. }));
        assert!(candidate_is_valid(outcome.candidate()));
        assert_ne!(outcome.candidate(), &origin);
    }

    #[test]
    fn truncated_capture_applies_the_same_changed_best_effort_candidate() {
        let origin = flat_curve(0.5);
        let mut recorder = StrokeRecorder::new(bounds());
        for index in 0..512 {
            recorder.observe(point(
                0.1 + (index % 2) as f32 * 0.8,
                if index % 2 == 0 { 0.1 } else { 0.9 },
            ));
        }
        assert!(recorder.is_truncated());
        let first = reconstruct_paint(&origin, 0.0, recorder.runs());
        let second = reconstruct_paint(&origin, 0.0, recorder.runs());
        assert!(matches!(&first, PaintCommitOutcome::Applied { .. }));
        assert!(candidate_is_valid(first.candidate()));
        assert_ne!(first.candidate(), &origin);
        assert_eq!(first, second);
    }

    #[test]
    fn maximum_run_reversal_reentry_release_is_bounded_and_deterministic() {
        let mut runs = Vec::with_capacity(MAX_CAPTURED_RUNS);
        for run_index in 0..MAX_CAPTURED_RUNS {
            let mut points = Vec::with_capacity(112);
            for sample_index in 0..96 {
                let x = if sample_index % 2 == 0 { 0.08 } else { 0.92 };
                let y = 0.1 + ((run_index * 19 + sample_index * 7) % 80) as f32 / 100.0;
                points.push(PaintPoint {
                    position: point(x, y),
                    contact: BoundaryContact::Interior,
                });
            }
            for boundary_index in 0..8 {
                let edge = if boundary_index % 2 == 0 {
                    BoundaryEdge::Left
                } else {
                    BoundaryEdge::Right
                };
                let parameter = 0.1 + boundary_index as f32 * 0.1;
                points.push(classify_point(bounds(), outside_on(edge, parameter)).unwrap());
                let reentry_x = if edge == BoundaryEdge::Left { 0.2 } else { 0.8 };
                points.push(PaintPoint {
                    position: point(reentry_x, 0.2 + boundary_index as f32 * 0.07),
                    contact: BoundaryContact::Interior,
                });
            }
            runs.push(PaintRun { points });
        }

        assert_eq!(runs.len(), MAX_CAPTURED_RUNS);
        assert!(paint_runs_within_capture_budget(&runs));

        let origin = flat_curve(0.5);
        let (fragments, display_constraints) = collect_display_geometry(&runs);
        let (patches, constraints) = raw_geometry(&fragments, &display_constraints, 0.37);
        assert!(patches.len() <= MAX_RAW_PATCHES);
        assert!(constraints.len() <= MAX_RAW_CONSTRAINTS);

        let seam_y = raw_seam_y(&origin, &patches, &constraints);
        let mandatory = mandatory_anchor_xs(&origin, &patches, &constraints);
        let optional = optional_anchor_xs(&mandatory, &patches);
        assert!(optional.len() <= MAX_OPTIONAL_FIT_ANCHORS);

        let fit_context = FitContext {
            origin: &origin,
            seam_y,
            patches: &patches,
            constraints: &constraints,
        };
        let first = reconstruct_paint(&origin, 0.37, &runs);
        let second = reconstruct_paint(&origin, 0.37, &runs);
        assert!(matches!(first, PaintCommitOutcome::Applied { .. }));
        assert_eq!(first, second);
        assert!(candidate_is_valid(first.candidate()));
        assert_ne!(first.candidate(), &origin);
        for pair in first.candidate().nodes.windows(2) {
            assert!(
                painted_fit_samples(fit_context, pair[0].x, pair[1].x).len()
                    <= MAX_FIT_SAMPLES_PER_SEGMENT
            );
        }
    }

    #[test]
    fn painted_fit_ignores_unpainted_endpoint_transitions() {
        let origin = flat_curve(0.5);
        let run = interior_run([(0.4, 0.1), (0.6, 0.9)]);
        let (fragments, display_constraints) = collect_display_geometry(std::slice::from_ref(&run));
        let (patches, constraints) = raw_geometry(&fragments, &display_constraints, 0.0);
        let seam_y = raw_seam_y(&origin, &patches, &constraints);
        let mandatory = mandatory_anchor_xs(&origin, &patches, &constraints);
        let fitted = fit_candidate(&origin, &mandatory, seam_y, &patches, &constraints);

        assert!(fitted.max_error <= PAINT_FIT_TOLERANCE);
    }

    #[test]
    fn monotonic_curved_stroke_uses_curved_segments() {
        let origin = flat_curve(0.5);
        let outcome = reconstruct_paint(
            &origin,
            0.0,
            &[interior_run([
                (0.1, 0.10),
                (0.3, 0.12),
                (0.5, 0.20),
                (0.7, 0.45),
                (0.9, 0.90),
            ])],
        );
        let candidate = outcome.candidate();
        assert!(matches!(&outcome, PaintCommitOutcome::Applied { .. }));
        assert!(candidate.nodes.len() < 12);
        assert!(candidate
            .nodes
            .windows(2)
            .zip(&candidate.segments)
            .any(|(nodes, segment)| {
                nodes[0].x >= 0.1 - RAW_NODE_ORDER_EPSILON
                    && nodes[1].x <= 0.9 + RAW_NODE_ORDER_EPSILON
                    && segment.tension.abs() > 0.05
            }));
    }

    #[test]
    fn linear_stroke_prefers_near_zero_tension_before_splitting() {
        let outcome = reconstruct_paint(
            &flat_curve(0.5),
            0.0,
            &[interior_run([
                (0.1, 0.2),
                (0.3, 0.35),
                (0.5, 0.5),
                (0.7, 0.65),
                (0.9, 0.8),
            ])],
        );
        let candidate = outcome.candidate();
        assert!(matches!(&outcome, PaintCommitOutcome::Applied { .. }));
        assert!(candidate
            .nodes
            .windows(2)
            .zip(&candidate.segments)
            .filter(|(nodes, _)| {
                nodes[0].x >= 0.1 - RAW_NODE_ORDER_EPSILON
                    && nodes[1].x <= 0.9 + RAW_NODE_ORDER_EPSILON
            })
            .all(|(_, segment)| segment.tension.abs() <= 0.05));
    }

    #[test]
    fn noisy_monotonic_stroke_does_not_silently_reject() {
        let origin = flat_curve(0.5);
        let outcome = reconstruct_paint(
            &origin,
            0.0,
            &[interior_run((0..100).map(|index| {
                let x = 0.1 + index as f32 * 0.8 / 99.0;
                let y =
                    0.2 + index as f32 * 0.6 / 99.0 + if index % 2 == 0 { 0.004 } else { -0.004 };
                (x, y)
            }))],
        );
        assert!(matches!(&outcome, PaintCommitOutcome::Applied { .. }));
        let candidate = outcome.candidate();
        assert!(candidate_is_valid(candidate));
        assert!(candidate.nodes.len() < 16);
    }

    #[test]
    fn small_horizontal_jitter_does_not_exhaust_capture_capacity() {
        let origin = flat_curve(0.5);
        let mut recorder = StrokeRecorder::new(bounds());
        recorder.observe(point(0.1, 0.2));
        for index in 0..20_000 {
            let progress = index as f32 / 19_999.0;
            let base_x = 0.12 + progress * 0.78;
            let jitter = if index % 2 == 0 { 0.0 } else { -0.0025 };
            let y = 0.2 + progress * 0.6 + if index % 2 == 0 { 0.0025 } else { -0.0025 };
            recorder.observe(point(base_x + jitter, y));
        }

        assert!(!recorder.is_truncated());
        assert!(paint_runs_within_capture_budget(recorder.runs()));
        assert!(recorder
            .runs()
            .iter()
            .all(|run| run.points().len() <= MAX_CAPTURED_POINTS_PER_RUN));

        let (fragments, constraints) = collect_display_geometry(recorder.runs());
        assert!(constraints.is_empty());
        assert_eq!(fragments.len(), 1);
        assert!(fragments[0]
            .points
            .windows(2)
            .all(|pair| pair[1].x >= pair[0].x));

        let outcome = reconstruct_paint(&origin, 0.0, recorder.runs());
        assert!(matches!(&outcome, PaintCommitOutcome::Applied { .. }));
        assert!(candidate_is_valid(outcome.candidate()));
    }
}
