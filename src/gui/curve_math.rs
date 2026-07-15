use super::*;

#[cfg(test)]
fn design_curve_size() -> Size {
    UiLayoutMetrics::design_space().curve_size
}

pub(super) const fn fixed_box(width: u32, height: u32) -> LayoutBox {
    LayoutBox::fixed(width, height).max(width, height)
}

pub(super) fn curve_scale_for_size(curve_size: Size) -> f32 {
    let width_scale = curve_size.width.max(1) as f32 / WINDOW_WIDTH as f32;
    let height_scale = curve_size.height.max(1) as f32 / WINDOW_HEIGHT as f32;
    width_scale.min(height_scale).clamp(0.2, 4.0)
}

pub(super) fn scaled_curve_i32(base: i32, curve_size: Size) -> i32 {
    (base as f32 * curve_scale_for_size(curve_size))
        .round()
        .max(1.0) as i32
}

pub(super) fn scaled_curve_u32(base: u32, curve_size: Size) -> u32 {
    (base as f32 * curve_scale_for_size(curve_size))
        .round()
        .max(1.0) as u32
}

pub(super) fn scaled_curve_tension_pixel_scale(curve_size: Size) -> f32 {
    CURVE_TENSION_PIXEL_SCALE * curve_scale_for_size(curve_size)
}

pub(super) fn node_hit_radius(curve_size: Size) -> i32 {
    scaled_curve_i32(NODE_HIT_RADIUS, curve_size)
}

pub(super) fn segment_near_hit_radius(curve_size: Size) -> i32 {
    scaled_curve_i32(SEGMENT_NEAR_HIT_RADIUS, curve_size)
}

pub(super) fn segment_direct_hit_radius(curve_size: Size) -> i32 {
    scaled_curve_i32(SEGMENT_DIRECT_HIT_RADIUS, curve_size)
}

pub(super) fn node_insert_guard_radius(curve_size: Size) -> i32 {
    scaled_curve_i32(NODE_INSERT_GUARD_RADIUS, curve_size)
}

pub(super) fn curve_drag_threshold_px(curve_size: Size) -> i32 {
    scaled_curve_i32(CURVE_DRAG_START_THRESHOLD_PX, curve_size)
}

pub(super) fn node_push_through_threshold_px(curve_size: Size) -> i32 {
    scaled_curve_i32(NODE_PUSH_THROUGH_PX, curve_size)
}

pub(super) fn curve_tension_pixel_scale(curve_size: Size) -> f32 {
    scaled_curve_tension_pixel_scale(curve_size)
}

pub(super) fn curve_editor_style(theme: PumpTheme) -> CurveEditorStyle {
    CurveEditorStyle {
        background: theme.curve_bg,
        border: theme.curve_border,
        grid_vertical: theme.curve_grid_vertical,
        grid_vertical_emphasis: theme.curve_grid_emphasis,
        grid_horizontal: theme.curve_grid_horizontal,
        line: theme.curve_line,
        line_highlight: theme.curve_line_highlight,
        node_fill: theme.node_fill,
        node_stroke: theme.node_stroke,
        node_hover_fill: theme.node_hover_fill,
        node_hover_stroke: theme.node_hover_stroke,
        node_selected_fill: theme.node_selected_fill,
        node_selected_stroke: theme.node_selected_stroke,
        preview_fill: theme.preview_fill,
        preview_stroke: theme.preview_stroke,
        playhead_core: theme.playhead_dot_core,
        playhead_stroke: theme.playhead_dot_stroke,
        highlight_mode: CurveHighlightMode::BrightCircle,
    }
}

pub(super) fn curve_editor_grid_config(grid_division: usize) -> CurveGridConfig {
    CurveGridConfig {
        emphasized_verticals: curve_beat_grid(grid_division, WINDOW_WIDTH as f32).major,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct CurveBeatGrid {
    pub(super) minor: Vec<f32>,
    pub(super) major: Vec<f32>,
}

/// Build normalized beat-grid positions for one synchronized curve cycle.
///
/// Full quarter-note beats are major divisions and sixteenth-note boundaries
/// are minor divisions. Minor divisions are thinned when their projected
/// spacing would become illegible. Sub-beat cycles longer than a sixteenth
/// keep a stable half-cycle emphasis; a sixteenth-note cycle has no internal
/// musical boundary and therefore deliberately renders no vertical line.
/// Unknown timing states also return an empty grid.
pub(super) fn curve_beat_grid(sync_division: usize, width: f32) -> CurveBeatGrid {
    const SIXTEENTH_NOTE_BEATS: f32 = 0.25;
    const MIN_MINOR_SPACING_PX: f32 = 8.0;
    const POSITION_EPSILON: f32 = 1.0e-5;

    let Some(division) = crate::params::SYNC_DIVISIONS.get(sync_division) else {
        return CurveBeatGrid::default();
    };
    let cycle_beats = division.beats;
    if !cycle_beats.is_finite() || cycle_beats <= 0.0 || !width.is_finite() || width <= 0.0 {
        return CurveBeatGrid::default();
    }
    if cycle_beats <= SIXTEENTH_NOTE_BEATS + POSITION_EPSILON {
        return CurveBeatGrid::default();
    }

    let mut major = Vec::new();
    let mut beat = 1.0;
    while beat < cycle_beats - POSITION_EPSILON {
        major.push(beat / cycle_beats);
        beat += 1.0;
    }
    if major.is_empty() {
        major.push(0.5);
    }

    let mut minor = Vec::new();
    let mut interval_beats = SIXTEENTH_NOTE_BEATS;
    while interval_beats < cycle_beats
        && width * interval_beats / cycle_beats < MIN_MINOR_SPACING_PX
    {
        interval_beats *= 2.0;
    }
    let mut subdivision = interval_beats;
    while subdivision < cycle_beats - POSITION_EPSILON {
        minor.push(subdivision / cycle_beats);
        subdivision += interval_beats;
    }
    minor.retain(|position| {
        !major
            .iter()
            .any(|major_position| (major_position - *position).abs() <= POSITION_EPSILON)
    });

    CurveBeatGrid { minor, major }
}

pub(super) fn curve_editor_interaction_options(
    curve_size: Size,
    grid_division: usize,
    snap_enabled: bool,
    command_snap_enabled: bool,
) -> CurveInteractionOptions {
    let command_snap_positions = if command_snap_enabled {
        curve_beat_grid_snap_positions(grid_division, curve_size.width as f32)
    } else {
        Vec::new()
    };
    CurveInteractionOptions {
        max_points: MAX_EDITABLE_NODES,
        min_point_spacing_x: NODE_X_MIN_SPACING,
        drag_start_threshold_px: curve_drag_threshold_px(curve_size),
        push_through_threshold_px: node_push_through_threshold_px(curve_size),
        endpoint_mode: EndpointMode::CoupledY,
        double_click_delete_interior: true,
        snap: CurveSnapConfig {
            enabled: snap_enabled || !command_snap_positions.is_empty(),
            vertical_positions: if command_snap_enabled {
                command_snap_positions
            } else {
                snap_vertical_positions_for_division(grid_division)
            },
            horizontal_positions: if snap_enabled {
                vec![0.0, 0.25, 0.5, 0.75, 1.0]
            } else {
                Vec::new()
            },
        },
    }
}

/// Return the exact normalized time targets represented by the visible beat grid.
///
/// Start and end anchors participate in snapping even when the current cycle has
/// no internal division. Unknown timing states deliberately return no targets.
pub(super) fn curve_beat_grid_snap_positions(sync_division: usize, width: f32) -> Vec<f32> {
    const POSITION_EPSILON: f32 = 1.0e-5;

    if crate::params::SYNC_DIVISIONS.get(sync_division).is_none()
        || !width.is_finite()
        || width <= 0.0
    {
        return Vec::new();
    }

    let grid = curve_beat_grid(sync_division, width);
    let mut positions = Vec::with_capacity(grid.minor.len() + grid.major.len() + 2);
    positions.push(0.0);
    positions.extend(grid.minor);
    positions.extend(grid.major);
    positions.push(1.0);
    positions.sort_by(f32::total_cmp);
    positions.dedup_by(|left, right| (*left - *right).abs() <= POSITION_EPSILON);
    positions
}

/// Snap one normalized time position to the nearest visible beat-grid target.
///
/// Exact ties resolve toward the earlier target for deterministic behavior.
pub(super) fn snap_curve_time_to_beat_grid(sync_division: usize, width: f32, time: f32) -> f32 {
    let time = time.clamp(0.0, 1.0);
    curve_beat_grid_snap_positions(sync_division, width)
        .into_iter()
        .min_by(|left, right| {
            (time - *left)
                .abs()
                .total_cmp(&(time - *right).abs())
                .then_with(|| left.total_cmp(right))
        })
        .unwrap_or(time)
}

pub(super) fn effective_grid_division(sync_division: usize, grid_override: Option<usize>) -> usize {
    grid_override
        .unwrap_or(sync_division)
        .min(MAX_SYNC_DIVISION as usize)
}

pub(super) fn grid_override_option_labels() -> Vec<String> {
    std::iter::once("Auto".to_string())
        .chain((0..=MAX_SYNC_DIVISION as usize).map(|index| sync_division_label(index).to_string()))
        .collect()
}

fn snap_vertical_positions_for_division(grid_division: usize) -> Vec<f32> {
    let mut positions = Vec::with_capacity(beat_grid_subdivision_count(grid_division) + 1);
    positions.push(0.0);
    positions.extend(vertical_grid_positions_for_division(grid_division));
    positions.push(1.0);
    positions
}

fn vertical_grid_positions_for_division(grid_division: usize) -> Vec<f32> {
    let beat_count = beat_grid_subdivision_count(grid_division).max(1);
    (1..beat_count)
        .map(|step| step as f32 / beat_count as f32)
        .collect()
}

fn beat_grid_subdivision_count(grid_division: usize) -> usize {
    let beats = crate::params::sync_division_beats(grid_division);
    let subdivisions = (4.0 / beats.max(1.0e-6)).round() as usize;
    subdivisions.max(1)
}

pub(super) fn curve_model_from_editable(editable_curve: &EditableCurve) -> CurveModel {
    CurveModel::new(
        editable_curve
            .nodes
            .iter()
            .copied()
            .map(|node| CurvePoint::new(node.x, node.y))
            .collect(),
        editable_curve
            .segments
            .iter()
            .copied()
            .map(|segment| CurveEditorSegment::new(segment.tension))
            .collect(),
    )
}

pub(super) fn editable_curve_from_model(model: &CurveModel) -> EditableCurve {
    EditableCurve {
        nodes: model
            .points
            .iter()
            .copied()
            .map(|point| CurveNode {
                x: point.x,
                y: point.y,
            })
            .collect(),
        segments: model
            .segments
            .iter()
            .copied()
            .map(|segment| CurveSegment {
                tension: segment.tension,
            })
            .collect(),
    }
    .normalized()
}

/// Return the internal tension-sign multiplier that produces visual upward bend.
///
/// Rising segments require negative tension for upward bend while falling
/// segments require positive tension, so drag logic must compensate by segment.
pub(super) fn segment_upward_tension_sign(curve: &EditableCurve, segment_index: usize) -> f32 {
    let left = curve.nodes.get(segment_index).copied();
    let right = curve.nodes.get(segment_index + 1).copied();
    match (left, right) {
        (Some(left_node), Some(right_node)) if right_node.y > left_node.y => -1.0,
        _ => 1.0,
    }
}

/// Convert vertical drag delta into internal segment tension delta.
///
/// Dragging upward (smaller `y`) always returns a positive visual bend amount.
pub(super) fn tension_delta_from_drag_for_segment(
    curve: &EditableCurve,
    segment_index: usize,
    start_pointer: Point,
    raw_local_pointer: Point,
    curve_size: Size,
) -> f32 {
    let drag_units =
        (start_pointer.y - raw_local_pointer.y) as f32 / curve_tension_pixel_scale(curve_size);
    drag_units * segment_upward_tension_sign(curve, segment_index)
}

#[cfg(test)]
pub(super) fn local_from_node(node: CurveNode) -> Point {
    local_from_node_for_size(node, design_curve_size())
}

pub(super) fn local_from_node_for_size(node: CurveNode, curve_size: Size) -> Point {
    let width = curve_size.width.max(1) as f32 - 1.0;
    let height = curve_size.height.max(1) as f32 - 1.0;
    let x = (node.x.clamp(0.0, 1.0) * width).round() as i32;
    let y = ((1.0 - node.y.clamp(0.0, 1.0)) * height).round() as i32;
    Point { x, y }
}

pub(super) fn node_from_local_for_size(local: Point, curve_size: Size) -> CurveNode {
    let width = (curve_size.width.max(1) as f32 - 1.0).max(1.0);
    let height = (curve_size.height.max(1) as f32 - 1.0).max(1.0);
    let x = (local.x as f32 / width).clamp(0.0, 1.0);
    let y = (1.0 - (local.y as f32 / height)).clamp(0.0, 1.0);
    CurveNode { x, y }
}

pub(super) fn scale_point_from_design(point: Point, curve_size: Size) -> Point {
    Point {
        x: point.x.clamp(0, curve_size.width.max(1) as i32 - 1),
        y: point.y.clamp(0, curve_size.height.max(1) as i32 - 1),
    }
}

pub(super) fn scale_point_to_design(point: Point, curve_size: Size) -> Point {
    scale_point_from_design(point, curve_size)
}

pub(super) fn find_node_hit_for_size(
    curve: &EditableCurve,
    local_pointer: Point,
    radius: i32,
    curve_size: Size,
) -> Option<usize> {
    find_node_hit_within_for_size(curve, local_pointer, radius, curve_size)
}

pub(super) fn find_node_hit_within_for_size(
    curve: &EditableCurve,
    local_pointer: Point,
    radius: i32,
    curve_size: Size,
) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    let radius_squared = radius.max(0) as i64 * radius.max(0) as i64;
    for (index, node) in curve.nodes.iter().copied().enumerate() {
        let center = local_from_node_for_size(node, curve_size);
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

#[cfg(test)]
pub(super) fn find_segment_line_hit_within(
    curve: &EditableCurve,
    local_pointer: Point,
    radius: i32,
) -> Option<usize> {
    find_segment_line_hit_within_for_size(curve, local_pointer, radius, design_curve_size())
}

pub(super) fn find_segment_line_hit_within_for_size(
    curve: &EditableCurve,
    local_pointer: Point,
    radius: i32,
    curve_size: Size,
) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    let radius_squared = (radius.max(0) * radius.max(0)) as f32;
    for index in 0..curve.segments.len() {
        let distance =
            segment_polyline_distance_squared_for_size(curve, index, local_pointer, curve_size);
        if distance <= radius_squared {
            match best {
                Some((_, best_distance)) if distance >= best_distance => {}
                _ => best = Some((index, distance)),
            }
        }
    }
    best.map(|(index, _)| index)
}

pub(super) fn insert_node_for_size(
    curve: &mut EditableCurve,
    node: CurveNode,
    curve_size: Size,
) -> usize {
    if curve.nodes.len() >= MAX_EDITABLE_NODES {
        return find_nearest_node_for_size(
            curve,
            local_from_node_for_size(node, curve_size),
            curve_size,
        )
        .unwrap_or(0);
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

#[cfg(test)]
pub(super) fn move_node_with_push_through(
    curve: &mut EditableCurve,
    index: usize,
    target: CurveNode,
    push_threshold_px: i32,
) -> usize {
    move_node_with_push_through_for_size(
        curve,
        index,
        target,
        push_threshold_px,
        design_curve_size(),
    )
}

/// Recompute move-node drag output from one drag-origin snapshot.
///
/// This makes push-through deletion reversible within the same drag gesture:
/// dragging back across previously crossed nodes rebuilds them from the origin
/// curve because each frame starts from `origin_curve`.
pub(super) fn recompute_move_node_from_origin_for_size(
    origin_curve: &EditableCurve,
    origin_index: usize,
    target: CurveNode,
    push_threshold_px: i32,
    curve_size: Size,
) -> (EditableCurve, usize) {
    let mut recomputed = origin_curve.clone();
    let moved_index = move_node_with_push_through_for_size(
        &mut recomputed,
        origin_index,
        target,
        push_threshold_px,
        curve_size,
    );
    (recomputed, moved_index)
}

pub(super) fn move_node_with_push_through_for_size(
    curve: &mut EditableCurve,
    index: usize,
    target: CurveNode,
    push_threshold_px: i32,
    curve_size: Size,
) -> usize {
    if index >= curve.nodes.len() {
        return index;
    }

    let y = target.y.clamp(0.0, 1.0);
    let last_index = curve.nodes.len() - 1;
    if index == 0 {
        set_wrapped_endpoint_y(curve, y);
        return 0;
    }
    if index == last_index {
        set_wrapped_endpoint_y(curve, y);
        return curve.nodes.len().saturating_sub(1);
    }

    let mut moved_index = index;
    let threshold_x = push_threshold_px.max(0) as f32 / (curve_size.width.max(2) - 1) as f32;
    while moved_index + 1 < curve.nodes.len().saturating_sub(1)
        && target.x > curve.nodes[moved_index + 1].x + threshold_x
    {
        remove_interior_node(curve, moved_index + 1);
    }
    while moved_index > 1 && target.x < curve.nodes[moved_index - 1].x - threshold_x {
        remove_interior_node(curve, moved_index - 1);
        moved_index = moved_index.saturating_sub(1);
    }

    let min_x = curve.nodes[moved_index - 1].x + NODE_X_MIN_SPACING;
    let max_x = curve.nodes[moved_index + 1].x - NODE_X_MIN_SPACING;
    curve.nodes[moved_index].x = target.x.clamp(min_x, max_x);
    curve.nodes[moved_index].y = y;
    enforce_wrapped_endpoints(curve);
    moved_index
}

pub(super) fn move_segment_translated(
    curve: &mut EditableCurve,
    segment_index: usize,
    start_left: (f32, f32),
    start_right: (f32, f32),
    delta: (f32, f32),
) {
    let (start_left_x, start_left_y) = start_left;
    let (start_right_x, start_right_y) = start_right;
    let (delta_x, delta_y) = delta;
    if curve.nodes.len() < 2 || segment_index >= curve.nodes.len() - 1 {
        return;
    }

    let right_index = segment_index + 1;
    let mut applied_dx = delta_x;
    if segment_index == 0 || right_index == curve.nodes.len() - 1 {
        applied_dx = 0.0;
    } else {
        let min_dx = curve.nodes[segment_index - 1].x + NODE_X_MIN_SPACING - start_left_x;
        let max_dx = curve.nodes[right_index + 1].x - NODE_X_MIN_SPACING - start_right_x;
        applied_dx = applied_dx.clamp(min_dx, max_dx);
    }
    let min_dy = -start_left_y.min(start_right_y);
    let max_dy = 1.0 - start_left_y.max(start_right_y);
    let applied_dy = delta_y.clamp(min_dy, max_dy);

    curve.nodes[segment_index].x = start_left_x + applied_dx;
    curve.nodes[right_index].x = start_right_x + applied_dx;
    curve.nodes[segment_index].y = start_left_y + applied_dy;
    curve.nodes[right_index].y = start_right_y + applied_dy;

    if segment_index == 0 {
        set_wrapped_endpoint_y(curve, curve.nodes[0].y);
    }
    if right_index == curve.nodes.len() - 1 {
        set_wrapped_endpoint_y(curve, curve.nodes[right_index].y);
    }
    enforce_wrapped_endpoints(curve);
}

pub(super) fn set_wrapped_endpoint_y(curve: &mut EditableCurve, y: f32) {
    if curve.nodes.len() < 2 {
        return;
    }
    let clamped = y.clamp(0.0, 1.0);
    let last_index = curve.nodes.len() - 1;
    curve.nodes[0].x = 0.0;
    curve.nodes[0].y = clamped;
    curve.nodes[last_index].x = 1.0;
    curve.nodes[last_index].y = clamped;
}

pub(super) fn enforce_wrapped_endpoints(curve: &mut EditableCurve) {
    if curve.nodes.len() < 2 {
        return;
    }
    let clamped = curve.nodes[0].y.clamp(0.0, 1.0);
    let last_index = curve.nodes.len() - 1;
    curve.nodes[0].x = 0.0;
    curve.nodes[0].y = clamped;
    curve.nodes[last_index].x = 1.0;
    curve.nodes[last_index].y = clamped;
}

pub(super) fn remove_interior_node(curve: &mut EditableCurve, remove_index: usize) {
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

pub(super) fn find_nearest_node_for_size(
    curve: &EditableCurve,
    local_pointer: Point,
    curve_size: Size,
) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    for (index, node) in curve.nodes.iter().copied().enumerate() {
        let distance = distance_squared(local_from_node_for_size(node, curve_size), local_pointer);
        match best {
            Some((_, best_distance)) if distance >= best_distance => {}
            _ => best = Some((index, distance)),
        }
    }
    best.map(|(index, _)| index)
}

pub(super) fn distance_squared(a: Point, b: Point) -> i64 {
    let dx = a.x as i64 - b.x as i64;
    let dy = a.y as i64 - b.y as i64;
    dx * dx + dy * dy
}

pub(super) fn segment_polyline_distance_squared_for_size(
    curve: &EditableCurve,
    index: usize,
    point: Point,
    curve_size: Size,
) -> f32 {
    let left = curve.nodes[index];
    let right = curve.nodes[(index + 1).min(curve.nodes.len() - 1)];
    let width = ((right.x - left.x).abs() * (curve_size.width.max(1) as f32 - 1.0))
        .round()
        .max(2.0) as i32;
    let steps = width.clamp(2, 96) as usize;
    let mut prev = local_from_node_for_size(
        CurveNode {
            x: left.x,
            y: sample_editable_curve(curve, left.x),
        },
        curve_size,
    );
    let mut best = f32::MAX;
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let x = left.x + (right.x - left.x) * t;
        let current = local_from_node_for_size(
            CurveNode {
                x,
                y: sample_editable_curve(curve, x),
            },
            curve_size,
        );
        let distance = point_to_segment_distance_squared(point, prev, current);
        if distance < best {
            best = distance;
        }
        prev = current;
    }
    best
}

pub(super) fn point_to_segment_distance_squared(point: Point, a: Point, b: Point) -> f32 {
    let px = point.x as f32;
    let py = point.y as f32;
    let ax = a.x as f32;
    let ay = a.y as f32;
    let bx = b.x as f32;
    let by = b.y as f32;
    let abx = bx - ax;
    let aby = by - ay;
    let ab_len2 = abx * abx + aby * aby;
    if ab_len2 <= f32::EPSILON {
        let dx = px - ax;
        let dy = py - ay;
        return dx * dx + dy * dy;
    }
    let apx = px - ax;
    let apy = py - ay;
    let t = ((apx * abx + apy * aby) / ab_len2).clamp(0.0, 1.0);
    let cx = ax + abx * t;
    let cy = ay + aby * t;
    let dx = px - cx;
    let dy = py - cy;
    dx * dx + dy * dy
}

#[cfg(test)]
pub(super) fn preview_node_on_curve(
    curve: &EditableCurve,
    local_pointer: Point,
) -> Option<CurveNode> {
    preview_node_on_curve_for_size(curve, local_pointer, design_curve_size())
}

pub(super) fn preview_node_on_curve_for_size(
    curve: &EditableCurve,
    local_pointer: Point,
    curve_size: Size,
) -> Option<CurveNode> {
    if curve.nodes.len() < 2 {
        return None;
    }
    let x = node_from_local_for_size(local_pointer, curve_size).x;
    Some(CurveNode {
        x,
        y: sample_editable_curve(curve, x).clamp(0.0, 1.0),
    })
}

pub(super) fn drag_threshold_crossed(
    start_pointer: Point,
    current_pointer: Point,
    threshold_px: i32,
) -> bool {
    let threshold = threshold_px.max(0) as i64;
    distance_squared(start_pointer, current_pointer) >= threshold * threshold
}

#[cfg(test)]
pub(super) fn find_deletable_node_hit(
    curve: &EditableCurve,
    local_pointer: Point,
) -> Option<usize> {
    find_deletable_node_hit_for_size(curve, local_pointer, design_curve_size())
}

pub(super) fn find_deletable_node_hit_for_size(
    curve: &EditableCurve,
    local_pointer: Point,
    curve_size: Size,
) -> Option<usize> {
    let index = find_node_hit_for_size(
        curve,
        local_pointer,
        node_hit_radius(curve_size),
        curve_size,
    )?;
    if index == 0 || index + 1 == curve.nodes.len() {
        return None;
    }
    Some(index)
}
