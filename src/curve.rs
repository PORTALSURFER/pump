//! Curve construction and sampling utilities for Pump.

/// Number of normalized samples stored in one pump cycle table.
pub const CURVE_TABLE_LEN: usize = 1024;
/// Maximum number of editable spline nodes.
pub const MAX_EDITABLE_NODES: usize = 64;
/// Minimum curvature amount for one segment.
pub const MIN_SEGMENT_TENSION: f32 = -1.0;
/// Maximum curvature amount for one segment.
pub const MAX_SEGMENT_TENSION: f32 = 1.0;

const NODE_X_EPSILON: f32 = 1.0e-4;
const LEGACY_RESTORE_NODE_COUNT: usize = 9;

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
    }
}

/// Build the default sidechain-style curve used on plugin initialization.
///
/// The shape performs a fast initial dip followed by a smooth recovery.
pub fn default_sidechain_curve() -> [f32; CURVE_TABLE_LEN] {
    let mut curve = [1.0_f32; CURVE_TABLE_LEN];
    let attack_end = 0.06_f32;
    let floor = 0.05_f32;

    for (index, sample) in curve.iter_mut().enumerate() {
        let phase = index as f32 / (CURVE_TABLE_LEN - 1) as f32;
        *sample = if phase < attack_end {
            let t = phase / attack_end;
            1.0 + (floor - 1.0) * t
        } else {
            let t = ((phase - attack_end) / (1.0 - attack_end)).clamp(0.0, 1.0);
            let eased = t.powf(1.8);
            floor + (1.0 - floor) * eased
        }
        .clamp(0.0, 1.0);
    }

    curve
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

    let left_slope = node_slope(curve, segment_index);
    let right_slope = node_slope(curve, segment_index + 1);

    hermite(
        left.y,
        right.y,
        left_slope * span,
        right_slope * span,
        shaped_local,
    )
    .clamp(0.0, 1.0)
}

fn node_slope(curve: &EditableCurve, index: usize) -> f32 {
    if curve.nodes.len() < 2 {
        return 0.0;
    }

    if index == 0 {
        let right = curve.nodes[1];
        let left = curve.nodes[0];
        return (right.y - left.y) / (right.x - left.x).max(1.0e-6);
    }

    if index >= curve.nodes.len() - 1 {
        let right = curve.nodes[curve.nodes.len() - 1];
        let left = curve.nodes[curve.nodes.len() - 2];
        return (right.y - left.y) / (right.x - left.x).max(1.0e-6);
    }

    let left = curve.nodes[index - 1];
    let right = curve.nodes[index + 1];
    (right.y - left.y) / (right.x - left.x).max(1.0e-6)
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

fn hermite(p0: f32, p1: f32, m0: f32, m1: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    (2.0 * t3 - 3.0 * t2 + 1.0) * p0
        + (t3 - 2.0 * t2 + t) * m0
        + (-2.0 * t3 + 3.0 * t2) * p1
        + (t3 - t2) * m1
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
        curve_table_to_editable, default_editable_curve, default_sidechain_curve,
        editable_curve_to_table, sample_curve, sample_editable_curve, CurveNode, CurveSegment,
        EditableCurve, CURVE_TABLE_LEN, MAX_EDITABLE_NODES,
    };

    #[test]
    fn default_curve_stays_bounded() {
        let curve = default_sidechain_curve();
        assert!(curve.iter().all(|value| (0.0..=1.0).contains(value)));
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
        let table = default_sidechain_curve();
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
        }
        .normalized();
        assert!(curve.nodes.len() <= MAX_EDITABLE_NODES);
    }
}
