//! Curve construction and sampling utilities for Pump.

/// Number of normalized samples stored in one pump cycle table.
pub const CURVE_TABLE_LEN: usize = 1024;
/// Maximum number of editable spline nodes.
/// Maximum editable nodes, including one spare slot for a cyclic seam anchor.
pub const MAX_EDITABLE_NODES: usize = 65;
/// Minimum curvature amount for one segment.
pub const MIN_SEGMENT_TENSION: f32 = -1.0;
/// Maximum curvature amount for one segment.
pub const MAX_SEGMENT_TENSION: f32 = 1.0;

const NODE_X_EPSILON: f32 = 1.0e-4;
const LEGACY_RESTORE_NODE_COUNT: usize = 9;
#[allow(dead_code)]
const OFFSET_SAMPLE_EPSILON: f32 = 1.0e-5;

/// One spline control node inside the cycle editor.
///
/// `x` is normalized cycle phase in `[0, 1]`.
/// `y` is normalized gain shape where `0` is max duck and `1` is unity.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct CurveNode {
    /// Normalized cycle phase position.
    pub x: f32,
    /// Normalized gain shape value.
    pub y: f32,
}

/// One editable spline segment between adjacent nodes.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct CurveSegment {
    /// Curvature amount for this segment.
    ///
    /// `0.0` keeps neutral curvature, positive values bias later recovery,
    /// negative values bias earlier recovery.
    pub tension: f32,
}

/// Editable node/segment model used by the GUI spline editor.
#[derive(Debug, Clone, PartialEq)]
pub struct EditableCurve {
    /// Ordered control nodes from phase `0` to phase `1`.
    pub nodes: Vec<CurveNode>,
    /// Segment settings where `segments[i]` connects `nodes[i] -> nodes[i + 1]`.
    pub segments: Vec<CurveSegment>,
    /// Optional unwrapped source used for exact cyclic phase translation.
    #[doc(hidden)]
    pub phase_source: Option<Box<EditableCurve>>,
    /// Phase offset applied to `phase_source`, in normalized cycles.
    #[doc(hidden)]
    pub phase_offset: f32,
}

impl Default for EditableCurve {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            segments: Vec::new(),
            phase_source: None,
            phase_offset: 0.0,
        }
    }
}

impl EditableCurve {
    /// Return a normalized copy with clamped values and valid topology.
    pub fn normalized(mut self) -> Self {
        self.normalize_in_place();
        self
    }

    /// Clamp and repair this curve in-place so it is valid for sampling.
    pub fn normalize_in_place(&mut self) {
        self.nodes = normalize_nodes(&self.nodes);
        self.segments = normalize_segments(&self.segments, self.nodes.len().saturating_sub(1));
        if let Some(source) = self.phase_source.as_mut() {
            source.normalize_in_place();
        }
        self.phase_offset = if self.phase_source.is_some() {
            self.phase_offset.rem_euclid(1.0)
        } else {
            0.0
        };
    }
}

/// Cyclically translate an editable curve by a normalized phase delta.
///
/// Positive deltas move features to the right. The curve is rebuilt from the
/// original drag origin, so a complete cycle returns the original point and
/// segment representation without accumulating floating-point error.
#[allow(dead_code)]
pub fn cyclically_offset_editable_curve(curve: &EditableCurve, delta: f32) -> EditableCurve {
    let origin = curve.clone().normalized();
    if origin.nodes.len() < 2 || !delta.is_finite() {
        return origin;
    }

    let delta = delta.rem_euclid(1.0);
    if delta <= OFFSET_SAMPLE_EPSILON || (1.0 - delta) <= OFFSET_SAMPLE_EPSILON {
        return origin;
    }

    let last_index = origin.nodes.len() - 1;
    let mut boundaries = vec![0.0_f32];
    for node in origin.nodes[..last_index].iter().copied() {
        let shifted = (node.x + delta).rem_euclid(1.0);
        if shifted > OFFSET_SAMPLE_EPSILON && shifted < 1.0 - OFFSET_SAMPLE_EPSILON {
            boundaries.push(shifted);
        }
    }
    boundaries.sort_by(f32::total_cmp);
    boundaries.dedup_by(|left, right| (*left - *right).abs() <= OFFSET_SAMPLE_EPSILON);
    boundaries.push(1.0);

    let split_count = boundaries
        .windows(2)
        .filter(|window| {
            !offset_interval_matches_origin_segment(&origin, delta, window[0], window[1])
        })
        .count();
    let split_subdivision_count =
        (MAX_EDITABLE_NODES.saturating_sub(boundaries.len()) / split_count.max(1) + 1).max(1);
    let mut positions = Vec::with_capacity(
        boundaries.len() + split_count * split_subdivision_count.saturating_sub(1),
    );
    for window in boundaries.windows(2) {
        let left = window[0];
        let right = window[1];
        positions.push(left);
        if !offset_interval_matches_origin_segment(&origin, delta, left, right) {
            let span = right - left;
            for subdivision in 1..split_subdivision_count {
                positions.push(left + span * subdivision as f32 / split_subdivision_count as f32);
            }
        }
    }
    positions.push(1.0);

    let nodes = positions
        .iter()
        .copied()
        .enumerate()
        .map(|(index, x)| CurveNode {
            x,
            y: if index == positions.len() - 1 {
                sample_editable_curve(&origin, -delta)
            } else {
                sample_editable_curve(&origin, x - delta)
            },
        })
        .collect::<Vec<_>>();

    let mut offset = EditableCurve {
        nodes,
        segments: Vec::new(),
        phase_source: Some(Box::new(exact_phase_source(&origin))),
        phase_offset: exact_phase_offset(&origin, delta),
    };
    for window in offset.nodes.windows(2) {
        let left = window[0].x;
        let right = window[1].x;
        offset.segments.push(CurveSegment {
            tension: offset_segment_tension(&origin, delta, left, right),
        });
    }
    offset.normalize_in_place();
    offset
}

#[allow(dead_code)]
fn offset_interval_source_range(delta: f32, left: f32, right: f32) -> (f32, f32) {
    let mut source_left = left - delta;
    while source_left < 0.0 {
        source_left += 1.0;
    }
    while source_left >= 1.0 {
        source_left -= 1.0;
    }
    (source_left, source_left + (right - left))
}

#[allow(dead_code)]
fn origin_segment_for_phase(curve: &EditableCurve, phase: f32) -> usize {
    let phase = phase.rem_euclid(1.0);
    if phase <= curve.nodes[0].x {
        return 0;
    }
    if phase >= curve.nodes[curve.nodes.len() - 1].x {
        return curve.nodes.len().saturating_sub(2);
    }
    let mut index = 0;
    while index + 1 < curve.nodes.len() && phase > curve.nodes[index + 1].x {
        index += 1;
    }
    index.min(curve.nodes.len().saturating_sub(2))
}

#[allow(dead_code)]
fn offset_interval_matches_origin_segment(
    curve: &EditableCurve,
    delta: f32,
    left: f32,
    right: f32,
) -> bool {
    let (source_left, source_right) = offset_interval_source_range(delta, left, right);
    let segment = origin_segment_for_phase(curve, source_left + (source_right - source_left) * 0.5);
    let expected_left = curve.nodes[segment].x;
    let expected_right = curve.nodes[segment + 1].x;
    (source_left - expected_left).abs() <= OFFSET_SAMPLE_EPSILON
        && (source_right - expected_right).abs() <= OFFSET_SAMPLE_EPSILON
}

#[allow(dead_code)]
fn offset_segment_tension(curve: &EditableCurve, delta: f32, left: f32, right: f32) -> f32 {
    let (source_left, source_right) = offset_interval_source_range(delta, left, right);
    let source_mid = source_left + (source_right - source_left) * 0.5;
    let segment = origin_segment_for_phase(curve, source_mid);
    if offset_interval_matches_origin_segment(curve, delta, left, right) {
        return curve
            .segments
            .get(segment)
            .copied()
            .unwrap_or(CurveSegment { tension: 0.0 })
            .tension;
    }

    let left_y = sample_editable_curve(curve, source_left);
    let right_y = sample_editable_curve(curve, source_right);
    let span = right_y - left_y;
    if span.abs() <= OFFSET_SAMPLE_EPSILON {
        return curve
            .segments
            .get(segment)
            .copied()
            .unwrap_or(CurveSegment { tension: 0.0 })
            .tension;
    }
    let midpoint = ((sample_editable_curve(curve, source_mid) - left_y) / span).clamp(0.0, 1.0);
    let exponent = if midpoint <= 0.5 {
        (-midpoint.max(OFFSET_SAMPLE_EPSILON).ln() / 2.0_f32.ln()).clamp(1.0, 4.0)
    } else {
        (-(1.0 - midpoint).max(OFFSET_SAMPLE_EPSILON).ln() / 2.0_f32.ln()).clamp(1.0, 4.0)
    };
    if midpoint <= 0.5 {
        ((exponent - 1.0) / 3.0).clamp(MIN_SEGMENT_TENSION, MAX_SEGMENT_TENSION)
    } else {
        -((exponent - 1.0) / 3.0).clamp(0.0, MAX_SEGMENT_TENSION)
    }
}

fn exact_phase_source(curve: &EditableCurve) -> EditableCurve {
    curve
        .phase_source
        .as_deref()
        .cloned()
        .unwrap_or_else(|| EditableCurve {
            nodes: curve.nodes.clone(),
            segments: curve.segments.clone(),
            ..EditableCurve::default()
        })
}

fn exact_phase_offset(curve: &EditableCurve, delta: f32) -> f32 {
    (if curve.phase_source.is_some() {
        curve.phase_offset + delta
    } else {
        delta
    })
    .rem_euclid(1.0)
}

/// Build the default editable curve used by the spline GUI.
pub fn default_editable_curve() -> EditableCurve {
    EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 1.0 },
            CurveNode { x: 0.08, y: 0.08 },
            CurveNode { x: 0.32, y: 0.52 },
            CurveNode { x: 1.0, y: 1.0 },
        ],
        segments: vec![
            CurveSegment { tension: -0.35 },
            CurveSegment { tension: 0.45 },
            CurveSegment { tension: -0.1 },
        ],
        ..EditableCurve::default()
    }
    .normalized()
}

/// Sample the curve table with linear interpolation at a normalized phase.
pub fn sample_curve(curve: &[f32; CURVE_TABLE_LEN], phase: f32) -> f32 {
    let wrapped = phase.rem_euclid(1.0);
    let scaled = wrapped * (CURVE_TABLE_LEN as f32 - 1.0);
    let left_index = scaled.floor() as usize;
    let right_index = (left_index + 1).min(CURVE_TABLE_LEN - 1);
    let frac = scaled - left_index as f32;
    lerp(curve[left_index], curve[right_index], frac).clamp(0.0, 1.0)
}

/// Sample the editable spline curve at normalized phase.
pub fn sample_editable_curve(curve: &EditableCurve, phase: f32) -> f32 {
    if let Some(source) = curve.phase_source.as_deref() {
        let relative = (phase - curve.phase_offset).rem_euclid(1.0);
        let relative =
            if relative <= OFFSET_SAMPLE_EPSILON || 1.0 - relative <= OFFSET_SAMPLE_EPSILON {
                0.0
            } else {
                relative
            };
        return sample_editable_curve(source, relative);
    }
    if curve.nodes.len() < 2 {
        return 1.0;
    }

    let wrapped = phase.rem_euclid(1.0);
    if wrapped <= curve.nodes[0].x {
        return curve.nodes[0].y.clamp(0.0, 1.0);
    }
    if wrapped >= curve.nodes[curve.nodes.len() - 1].x {
        return curve.nodes[curve.nodes.len() - 1].y.clamp(0.0, 1.0);
    }

    let mut segment_index = 0usize;
    while segment_index + 1 < curve.nodes.len() && wrapped > curve.nodes[segment_index + 1].x {
        segment_index += 1;
    }

    sample_segment(curve, segment_index, wrapped)
}

/// Convert an editable node/segment curve into a fixed-size table.
pub fn editable_curve_to_table(editable: &EditableCurve) -> [f32; CURVE_TABLE_LEN] {
    let normalized = editable.clone().normalized();
    let mut curve = [1.0_f32; CURVE_TABLE_LEN];
    for (index, sample) in curve.iter_mut().enumerate() {
        let phase = index as f32 / (CURVE_TABLE_LEN - 1) as f32;
        *sample = sample_editable_curve(&normalized, phase).clamp(0.0, 1.0);
    }
    curve
}

/// Convert a legacy fixed table into a best-effort editable curve.
pub fn curve_table_to_editable(curve: &[f32; CURVE_TABLE_LEN]) -> EditableCurve {
    let mut nodes = Vec::with_capacity(LEGACY_RESTORE_NODE_COUNT);
    for index in 0..LEGACY_RESTORE_NODE_COUNT {
        let x = index as f32 / (LEGACY_RESTORE_NODE_COUNT - 1) as f32;
        nodes.push(CurveNode {
            x,
            y: sample_curve(curve, x).clamp(0.0, 1.0),
        });
    }

    EditableCurve {
        segments: vec![CurveSegment { tension: 0.0 }; LEGACY_RESTORE_NODE_COUNT - 1],
        nodes,
        ..EditableCurve::default()
    }
    .normalized()
}

fn sample_segment(curve: &EditableCurve, segment_index: usize, x: f32) -> f32 {
    let left = curve.nodes[segment_index];
    let right = curve.nodes[(segment_index + 1).min(curve.nodes.len() - 1)];
    let span = (right.x - left.x).max(1.0e-6);
    let local = ((x - left.x) / span).clamp(0.0, 1.0);

    let shaped_local = shape_with_tension(
        local,
        curve
            .segments
            .get(segment_index)
            .copied()
            .unwrap_or(CurveSegment { tension: 0.0 })
            .tension,
    );

    lerp(left.y, right.y, shaped_local).clamp(0.0, 1.0)
}

fn shape_with_tension(value: f32, tension: f32) -> f32 {
    let v = value.clamp(0.0, 1.0);
    let t = tension.clamp(MIN_SEGMENT_TENSION, MAX_SEGMENT_TENSION);
    if t >= 0.0 {
        v.powf(1.0 + t * 3.0)
    } else {
        1.0 - (1.0 - v).powf(1.0 + (-t) * 3.0)
    }
}

fn normalize_nodes(nodes: &[CurveNode]) -> Vec<CurveNode> {
    let mut normalized: Vec<CurveNode> = nodes
        .iter()
        .copied()
        .filter(|node| node.x.is_finite() && node.y.is_finite())
        .map(|node| CurveNode {
            x: node.x.clamp(0.0, 1.0),
            y: node.y.clamp(0.0, 1.0),
        })
        .collect();

    if normalized.is_empty() {
        return vec![CurveNode { x: 0.0, y: 1.0 }, CurveNode { x: 1.0, y: 1.0 }];
    }

    normalized.sort_by(|a, b| a.x.total_cmp(&b.x));

    let mut deduped: Vec<CurveNode> = Vec::with_capacity(normalized.len());
    for node in normalized {
        if let Some(last) = deduped.last_mut() {
            if (node.x - last.x).abs() < NODE_X_EPSILON {
                last.y = node.y;
                continue;
            }
        }
        deduped.push(node);
    }

    if deduped.len() == 1 {
        let only = deduped[0];
        deduped.push(CurveNode {
            x: if only.x < 0.5 { 1.0 } else { 0.0 },
            y: only.y,
        });
        deduped.sort_by(|a, b| a.x.total_cmp(&b.x));
    }

    if deduped.len() > MAX_EDITABLE_NODES {
        deduped = limit_nodes(&deduped, MAX_EDITABLE_NODES);
    }

    deduped[0].x = 0.0;
    let last = deduped.len() - 1;
    deduped[last].x = 1.0;

    for index in 1..=last {
        let min_x = deduped[index - 1].x + NODE_X_EPSILON;
        if deduped[index].x < min_x {
            deduped[index].x = min_x;
        }
    }
    deduped[last].x = 1.0;
    for index in (0..last).rev() {
        let max_x = deduped[index + 1].x - NODE_X_EPSILON;
        if deduped[index].x > max_x {
            deduped[index].x = max_x;
        }
    }
    deduped[0].x = 0.0;

    // Looping continuity: start and end nodes represent the same wrapped point.
    deduped[last].y = deduped[0].y;

    deduped
}

fn limit_nodes(nodes: &[CurveNode], max_nodes: usize) -> Vec<CurveNode> {
    if nodes.len() <= max_nodes || max_nodes < 2 {
        return nodes.to_vec();
    }

    let mut limited = Vec::with_capacity(max_nodes);
    limited.push(nodes[0]);
    for slot in 1..(max_nodes - 1) {
        let ratio = slot as f32 / (max_nodes - 1) as f32;
        let index = (ratio * (nodes.len() - 1) as f32).round() as usize;
        limited.push(nodes[index.min(nodes.len() - 2)]);
    }
    limited.push(nodes[nodes.len() - 1]);
    limited
}

fn normalize_segments(segments: &[CurveSegment], target_len: usize) -> Vec<CurveSegment> {
    let mut normalized = Vec::with_capacity(target_len);
    for index in 0..target_len {
        let tension = segments
            .get(index)
            .copied()
            .unwrap_or(CurveSegment { tension: 0.0 })
            .tension
            .clamp(MIN_SEGMENT_TENSION, MAX_SEGMENT_TENSION);
        normalized.push(CurveSegment { tension });
    }
    normalized
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::{
        curve_table_to_editable, cyclically_offset_editable_curve, default_editable_curve,
        editable_curve_to_table, sample_curve, sample_editable_curve, CurveNode, CurveSegment,
        EditableCurve, CURVE_TABLE_LEN, MAX_EDITABLE_NODES,
    };

    #[test]
    fn default_curve_stays_bounded() {
        let curve = editable_curve_to_table(&default_editable_curve());
        assert!(curve.iter().all(|value| (0.0..=1.0).contains(value)));
    }

    #[test]
    fn cyclic_offset_preserves_shape_and_wrapped_endpoint() {
        let origin = default_editable_curve();
        let delta = 0.23;
        let offset = cyclically_offset_editable_curve(&origin, delta);

        assert_eq!(offset.nodes.first().map(|node| node.x), Some(0.0));
        assert_eq!(offset.nodes.last().map(|node| node.x), Some(1.0));
        assert!((offset.nodes.first().unwrap().y - offset.nodes.last().unwrap().y).abs() < 1.0e-6);
        assert!(offset
            .nodes
            .windows(2)
            .all(|window| window[1].x - window[0].x >= 1.0e-4));

        for index in 0..=200 {
            let phase = index as f32 / 200.0;
            let expected = sample_editable_curve(&origin, phase - delta);
            let actual = sample_editable_curve(&offset, phase);
            assert!(
                (actual - expected).abs() < 1.0e-6,
                "phase {phase}: {actual} != {expected}"
            );
        }
    }

    #[test]
    fn cyclic_offset_full_cycle_returns_origin_without_drift() {
        let origin = default_editable_curve();
        assert_eq!(cyclically_offset_editable_curve(&origin, 1.0), origin);
        assert_eq!(cyclically_offset_editable_curve(&origin, 2.0), origin);
        assert_eq!(cyclically_offset_editable_curve(&origin, -1.0), origin);
    }

    #[test]
    fn cyclic_offset_keeps_nonlinear_seam_stable_across_inverse_gestures() {
        let mut origin = default_editable_curve();
        origin.segments = vec![
            CurveSegment { tension: 0.9 },
            CurveSegment { tension: -0.9 },
            CurveSegment { tension: 0.8 },
        ];
        let origin = origin.normalized();
        let delta = 0.37;
        let offset = cyclically_offset_editable_curve(&origin, delta);
        let restored = cyclically_offset_editable_curve(&offset, -delta);
        let mut repeated = origin.clone();
        for _ in 0..8 {
            repeated = cyclically_offset_editable_curve(&repeated, delta);
        }

        for index in 0..=400 {
            let phase = index as f32 / 400.0;
            let expected = sample_editable_curve(&origin, phase);
            let actual = sample_editable_curve(&restored, phase);
            assert!(
                (actual - expected).abs() < 1.0e-6,
                "phase {phase}: restored {actual} != origin {expected}"
            );
            let repeated_expected = sample_editable_curve(&origin, phase - delta * 8.0);
            let repeated_actual = sample_editable_curve(&repeated, phase);
            assert!(
                (repeated_actual - repeated_expected).abs() < 5.0e-6,
                "phase {phase}: repeated {repeated_actual} != expected {repeated_expected}"
            );
        }
    }

    #[test]
    fn cyclic_offset_preserves_a_curve_with_one_spare_seam_slot() {
        let mut origin = EditableCurve {
            nodes: Vec::with_capacity(MAX_EDITABLE_NODES),
            segments: Vec::with_capacity(MAX_EDITABLE_NODES - 1),

            ..EditableCurve::default()
        };
        for index in 0..MAX_EDITABLE_NODES {
            origin.nodes.push(CurveNode {
                x: index as f32 / (MAX_EDITABLE_NODES - 1) as f32,
                y: if index % 2 == 0 { 0.1 } else { 0.9 },
            });
            if index + 1 < MAX_EDITABLE_NODES {
                origin.segments.push(CurveSegment { tension: 0.0 });
            }
        }
        let origin = origin.normalized();
        let offset = cyclically_offset_editable_curve(&origin, 0.013);
        assert!(offset.nodes.len() <= MAX_EDITABLE_NODES);
        assert!(offset.phase_source.is_some());
        assert!(offset
            .nodes
            .windows(2)
            .all(|window| window[1].x - window[0].x >= 1.0e-4));
        for index in 0..=100 {
            let phase = index as f32 / 100.0;
            assert!(
                (sample_editable_curve(&offset, phase)
                    - sample_editable_curve(&origin, phase - 0.013))
                .abs()
                    < 1.0e-6
            );
        }
    }

    #[test]
    fn sampled_curve_interpolates_between_neighbors() {
        let mut curve = [0.0_f32; CURVE_TABLE_LEN];
        curve[0] = 0.0;
        curve[1] = 1.0;
        let sample = sample_curve(&curve, 0.0006);
        assert!(sample > 0.0);
        assert!(sample < 1.0);
    }

    #[test]
    fn editable_curve_normalization_repairs_invalid_input() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.8, y: 2.0 },
                CurveNode { x: 0.2, y: -1.0 },
                CurveNode { x: 0.2, y: 0.4 },
            ],
            segments: vec![CurveSegment { tension: 99.0 }],

            ..EditableCurve::default()
        }
        .normalized();

        assert!(curve.nodes.len() >= 2);
        assert_eq!(curve.nodes[0].x, 0.0);
        assert_eq!(curve.nodes[curve.nodes.len() - 1].x, 1.0);
        assert_eq!(curve.segments.len(), curve.nodes.len() - 1);
        assert!(curve
            .nodes
            .iter()
            .all(|node| (0.0..=1.0).contains(&node.y) && node.x.is_finite()));
    }

    #[test]
    fn editable_curve_to_table_stays_finite() {
        let curve = default_editable_curve();
        let table = editable_curve_to_table(&curve);
        assert!(table.iter().all(|sample| sample.is_finite()));
        assert!(table.iter().all(|sample| (0.0..=1.0).contains(sample)));
    }

    #[test]
    fn default_editable_curve_uses_simple_node_count() {
        let curve = default_editable_curve();
        assert_eq!(curve.nodes.len(), 4);
        assert_eq!(curve.segments.len(), 3);
    }

    #[test]
    fn legacy_curve_roundtrips_into_editable_domain() {
        let table = editable_curve_to_table(&default_editable_curve());
        let editable = curve_table_to_editable(&table);
        let rebuilt = editable_curve_to_table(&editable);
        assert!(rebuilt.iter().all(|sample| sample.is_finite()));
        assert!(rebuilt.iter().all(|sample| (0.0..=1.0).contains(sample)));
    }

    #[test]
    fn segment_tension_changes_midpoint() {
        let mut flat = default_editable_curve();
        flat.nodes = vec![CurveNode { x: 0.0, y: 1.0 }, CurveNode { x: 1.0, y: 0.0 }];
        flat.segments = vec![CurveSegment { tension: -1.0 }];
        let early = sample_editable_curve(&flat, 0.25);
        flat.segments[0].tension = 1.0;
        let late = sample_editable_curve(&flat, 0.25);
        assert!(late > early);
    }

    #[test]
    fn node_count_is_capped() {
        let mut nodes = Vec::new();
        for index in 0..(MAX_EDITABLE_NODES + 20) {
            let x = index as f32 / (MAX_EDITABLE_NODES + 19) as f32;
            nodes.push(CurveNode { x, y: 0.5 });
        }
        let curve = EditableCurve {
            nodes,
            segments: Vec::new(),

            ..EditableCurve::default()
        }
        .normalized();
        assert!(curve.nodes.len() <= MAX_EDITABLE_NODES);
    }

    #[test]
    fn normalized_curve_couples_wrapped_endpoints() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.9 },
                CurveNode { x: 0.5, y: 0.2 },
                CurveNode { x: 1.0, y: 0.1 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }, CurveSegment { tension: 0.0 }],

            ..EditableCurve::default()
        }
        .normalized();

        let last_index = curve.nodes.len() - 1;
        assert!((curve.nodes[last_index].y - curve.nodes[0].y).abs() <= f32::EPSILON);
    }
}
