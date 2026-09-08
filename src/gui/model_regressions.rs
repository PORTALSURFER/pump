//! Renderer-independent regression coverage for the Pump editor model.
//!
//! These tests intentionally live in a child module so they can exercise the
//! model's private reducer state without introducing a renderer or widget test
//! dependency.

use super::super::curve_paint::{BoundaryContact, BoundaryEdge, EdgeParameter};
use super::*;

use std::sync::Arc;

use crate::automation_queue::PumpAutomationQueue;
use crate::curve::{
    sample_curve_segment, sample_editable_curve, CurveNode, CurveSegment, EditableCurve,
};
use crate::params::{normalized_from_plain_value, with_test_curve_slot_path, MAX_DELAY_BEATS};
use toybox::clack_plugin::events::event_types::{
    ParamGestureBeginEvent, ParamGestureEndEvent, ParamValueEvent,
};
use toybox::clack_plugin::events::io::EventBuffer;
use toybox::clack_plugin::events::spaces::CoreEventSpace;
use toybox::clack_plugin::events::Event;
use toybox::clap::automation::{AutomationDropPolicy, AutomationQueueConfig};

const CURVE_PAINT_ASSERT_EPSILON: f32 = 1.0e-4;
const CURVE_PREVIEW_HEIGHT: f32 = 153.0;

fn editor_state(params: Arc<PumpParams>) -> PumpEditorState {
    PumpEditorState::new(
        params,
        Arc::new(GuiStatus::default()),
        clap_edit_sink(Arc::new(PumpAutomationQueue::default()), None),
    )
}

fn editor_state_with_queue(
    params: Arc<PumpParams>,
    queue: Arc<PumpAutomationQueue>,
) -> PumpEditorState {
    PumpEditorState::new(
        params,
        Arc::new(GuiStatus::default()),
        clap_edit_sink(queue, None),
    )
}

fn paint_sample(x: f32, y: f32) -> CurvePaintSample {
    CurvePaintSample {
        node: CurveNode { x, y },
        display_position: RectPoint { x, y },
        outside: false,
    }
}

fn boundary_paint_sample(x: f32, y: f32) -> CurvePaintSample {
    CurvePaintSample {
        node: CurveNode { x, y },
        display_position: RectPoint { x, y },
        outside: true,
    }
}

fn recorded_run(points: impl IntoIterator<Item = RectPoint>) -> PaintRun {
    let mut recorder = StrokeRecorder::new(RectBounds {
        min: RectPoint { x: 0.0, y: 0.0 },
        max: RectPoint { x: 1.0, y: 1.0 },
    });
    for point in points {
        recorder.observe(point);
    }
    assert_eq!(recorder.runs().len(), 1);
    recorder
        .runs()
        .first()
        .cloned()
        .expect("recorded points should produce one paint run")
}

fn sampled_segment_run(left: CurveNode, right: CurveNode, tension: f32, steps: usize) -> PaintRun {
    recorded_run((0..=steps).map(|step| {
        let fraction = step as f32 / steps as f32;
        let x = left.x + (right.x - left.x) * fraction;
        RectPoint {
            x,
            y: sample_curve_segment(left, right, tension, x),
        }
    }))
}

fn assert_curve_paint_topology_is_bounded(curve: &EditableCurve) {
    assert!(curve.nodes.len() <= MAX_EDITABLE_NODES);
    assert_eq!(curve.segments.len(), curve.nodes.len().saturating_sub(1));
    assert_eq!(curve.nodes.first().map(|node| node.x), Some(0.0));
    assert_eq!(curve.nodes.last().map(|node| node.x), Some(1.0));
    assert!(curve.nodes.iter().all(|node| {
        node.x.is_finite()
            && node.y.is_finite()
            && (0.0..=1.0).contains(&node.x)
            && (0.0..=1.0).contains(&node.y)
    }));
    assert!(curve
        .nodes
        .windows(2)
        .all(|pair| pair[1].x - pair[0].x >= CURVE_PAINT_ASSERT_EPSILON));
}

fn test_curve_push_through_threshold_x() -> f32 {
    curve_node_push_through_threshold_x(300.0)
}

fn max_capacity_curve() -> EditableCurve {
    EditableCurve {
        nodes: (0..MAX_EDITABLE_NODES)
            .map(|index| CurveNode {
                x: index as f32 / (MAX_EDITABLE_NODES - 1) as f32,
                y: 0.5,
            })
            .collect(),
        segments: vec![CurveSegment { tension: 0.0 }; MAX_EDITABLE_NODES - 1],
        ..EditableCurve::default()
    }
    .normalized()
}

fn flat_segment_hit_curve() -> EditableCurve {
    EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.5 },
            CurveNode { x: 0.25, y: 0.5 },
            CurveNode { x: 0.5, y: 0.5 },
            CurveNode { x: 0.75, y: 0.5 },
            CurveNode { x: 1.0, y: 0.5 },
        ],
        segments: vec![CurveSegment { tension: 0.0 }; 4],
        ..EditableCurve::default()
    }
    .normalized()
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

fn legacy_edge_curve() -> EditableCurve {
    EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.75 },
            CurveNode { x: 0.0001, y: 0.2 },
            CurveNode { x: 0.5, y: 0.45 },
            CurveNode { x: 0.9999, y: 0.15 },
            CurveNode { x: 1.0, y: 0.75 },
        ],
        segments: vec![CurveSegment { tension: 0.0 }; 4],
        ..EditableCurve::default()
    }
    .normalized()
}

fn interior_seam_curve() -> EditableCurve {
    EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.8 },
            CurveNode { x: 0.2, y: 0.2 },
            CurveNode {
                x: seam_raw(0.25),
                y: 0.4,
            },
            CurveNode { x: 0.9, y: 0.6 },
            CurveNode { x: 1.0, y: 0.8 },
        ],
        segments: vec![
            CurveSegment { tension: 0.11 },
            CurveSegment { tension: 0.22 },
            CurveSegment { tension: 0.33 },
            CurveSegment { tension: 0.44 },
        ],
        ..EditableCurve::default()
    }
    .normalized()
}

fn interior_seam_curve_with_raw_x(raw_x: f32) -> EditableCurve {
    let mut curve = interior_seam_curve();
    curve.nodes[2].x = raw_x;
    curve.normalized()
}

fn occupied_interior_seam_curve(edge_competitor_raw_x: f32) -> EditableCurve {
    EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.8 },
            CurveNode { x: 0.2, y: 0.2 },
            CurveNode {
                x: seam_raw(0.25),
                y: 0.4,
            },
            CurveNode {
                x: edge_competitor_raw_x,
                y: 0.3,
            },
            CurveNode { x: 0.9, y: 0.6 },
            CurveNode { x: 1.0, y: 0.8 },
        ],
        segments: vec![
            CurveSegment { tension: 0.11 },
            CurveSegment { tension: 0.22 },
            CurveSegment { tension: 0.33 },
            CurveSegment { tension: 0.44 },
            CurveSegment { tension: 0.55 },
        ],
        ..EditableCurve::default()
    }
    .normalized()
}

fn direct_edge_drag_curve() -> EditableCurve {
    EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.9 },
            CurveNode { x: 0.2, y: 0.3 },
            CurveNode { x: 0.5, y: 0.6 },
            CurveNode { x: 0.8, y: 0.4 },
            CurveNode { x: 1.0, y: 0.9 },
        ],
        segments: vec![
            CurveSegment { tension: 0.13 },
            CurveSegment { tension: 0.62 },
            CurveSegment { tension: -0.74 },
            CurveSegment { tension: 0.31 },
        ],
        ..EditableCurve::default()
    }
    .normalized()
}

fn assert_direct_edge_drag_preserves_incident_tension(edge: CurveEdge) {
    let origin = direct_edge_drag_curve();
    let (index, target_x, target_y, expected_active, expected_segment, expected_tension) =
        match edge {
            CurveEdge::Left => (1, 0.0, 0.35, 0, 0, 0.62),
            CurveEdge::Right => (3, 1.0, 0.65, 3, 2, -0.74),
        };
    let params = Arc::new(PumpParams::new());
    params.set_editable_curve(&origin);
    params.set_phase_offset(0.25);
    let mut state = editor_state(Arc::clone(&params));

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::PressNode {
            index,
            pointer: origin.nodes[index],
            shift_held: false,
            option_held: false,
            command_held: false,
        },
    );
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragNode {
            index,
            node: CurveNode {
                x: target_x,
                y: target_y,
            },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        },
    );

    let dragged = params.editable_curve_snapshot();
    let last_index = dragged.nodes.len() - 1;
    assert_eq!(dragged.nodes.len(), origin.nodes.len() - 1);
    assert_eq!(dragged.segments.len(), dragged.nodes.len() - 1);
    assert_eq!(state.active_curve_node, Some(expected_active));
    assert_eq!(dragged.nodes[0].y, target_y);
    assert_eq!(dragged.nodes[last_index].y, target_y);
    assert_eq!(dragged.segments[expected_segment].tension, expected_tension);
}

fn assert_edge_insert_is_single_visible_node(node: CurveNode, edge: CurveEdge) {
    let params = Arc::new(PumpParams::new());
    let mut state = editor_state(Arc::clone(&params));
    let before = params.editable_curve_snapshot();
    state.preview_curve_node = Some(node);
    state.hover_curve_segment = Some(0);
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::InsertNode {
            node,
            command_held: false,
        },
    );
    let after = params.editable_curve_snapshot();
    assert_eq!(after.nodes.len(), before.nodes.len());
    assert_eq!(after.segments.len(), before.segments.len());
    assert_eq!(state.active_curve_node, Some(edge.index(after.nodes.len())));
    assert!(state.preview_curve_node.is_none());
    assert!(state.hover_curve_segment.is_none());
    let endpoint = edge.index(after.nodes.len());
    assert!((after.nodes[endpoint].y - node.y).abs() < 1.0e-6);
}

fn assert_marquee_selected_single_source_takes_over_occupied_edge(edge_x: f32) {
    let phase_offset = 0.25;
    let threshold = test_curve_push_through_threshold_x();
    let edge_competitor_raw_x = if edge_x <= 0.0 { 0.27 } else { 0.23 };
    let curve = occupied_interior_seam_curve(edge_competitor_raw_x);
    assert!(matches!(
        canonical_seam_owner(&curve, phase_offset),
        Some(CanonicalSeamOwner::Interior(_))
    ));
    assert!(display_x_is_in_edge_zone(
        edge_competitor_raw_x,
        phase_offset,
        threshold,
    ));

    let params = Arc::new(PumpParams::new());
    params.set_editable_curve(&curve);
    params.set_phase_offset(phase_offset);
    let mut state = editor_state(Arc::clone(&params));
    state.selected_curve_nodes = vec![1];

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressNode {
            index: 1,
            pointer: curve.nodes[1],
            shift_held: false,
            option_held: false,
            command_held: false,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
            index: 1,
            node: CurveNode {
                x: seam_raw(phase_offset),
                y: 0.55,
            },
            push_through_threshold_x: threshold,
        }),
    );

    let taken_over = params.editable_curve_snapshot();
    let seam_indices: Vec<_> = taken_over
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            ((node.x - seam_raw(phase_offset)).abs() <= CURVE_SEAM_OWNER_RAW_EPSILON)
                .then_some(index)
        })
        .collect();
    assert_eq!(taken_over.nodes.len(), curve.nodes.len() - 2);
    assert_eq!(seam_indices, vec![1]);
    assert_eq!(taken_over.nodes[1].x, seam_raw(phase_offset));
    assert!(!taken_over
        .nodes
        .iter()
        .any(|node| (node.x - curve.nodes[1].x).abs() <= CURVE_SEAM_OWNER_RAW_EPSILON));
    assert!(!taken_over
        .nodes
        .iter()
        .any(|node| (node.x - edge_competitor_raw_x).abs() <= CURVE_SEAM_OWNER_RAW_EPSILON));
    assert_eq!(state.active_curve_node, Some(1));
    assert!(state.selected_curve_nodes.is_empty());
    assert!(state
        .active_curve_node_drag
        .as_ref()
        .is_some_and(|drag| drag.selected_indices.is_empty() && drag.seam_drag.is_some()));
    assert!((taken_over.nodes[1].y - 0.55).abs() < 1.0e-6);
    assert!((taken_over.segments[0].tension - 0.11).abs() < 1.0e-6);
    assert!((taken_over.segments[1].tension - 0.22).abs() < 1.0e-6);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
            index: 1,
            node: CurveNode { x: 0.2, y: 0.65 },
            push_through_threshold_x: threshold,
        }),
    );
    let unsnapped = params.editable_curve_snapshot();
    assert_eq!(unsnapped.nodes.len(), curve.nodes.len());
    assert_eq!(unsnapped.nodes[1].x, curve.nodes[1].x);
    assert!((unsnapped.nodes[1].y - 0.65).abs() < 1.0e-6);
    assert_eq!(unsnapped.nodes[3], curve.nodes[3]);
    assert_eq!(unsnapped.segments, curve.segments);
    assert_eq!(state.active_curve_node, Some(1));
    assert!(state
        .active_curve_node_drag
        .as_ref()
        .is_some_and(|drag| drag.seam_drag.is_none()));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleaseNode {
            index: 1,
            node: CurveNode { x: 0.2, y: 0.7 },
            push_through_threshold_x: threshold,
            shift_held: false,
            option_held: false,
            command_held: false,
        }),
    );
    let released = params.editable_curve_snapshot();
    assert_eq!(released.nodes.len(), curve.nodes.len());
    assert_eq!(released.nodes[1].x, curve.nodes[1].x);
    assert!((released.nodes[1].y - 0.7).abs() < 1.0e-6);
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.selected_curve_nodes.is_empty());
}

#[test]
fn marquee_selected_single_source_takes_over_occupied_left_edge() {
    assert_marquee_selected_single_source_takes_over_occupied_edge(0.0);
}

#[test]
fn marquee_selected_single_source_takes_over_occupied_right_edge() {
    assert_marquee_selected_single_source_takes_over_occupied_edge(1.0);
}

#[test]
fn marquee_selected_single_source_clears_selection_on_direct_edge_release() {
    let phase_offset = 0.25;
    let curve = occupied_interior_seam_curve(0.27);
    let params = Arc::new(PumpParams::new());
    params.set_editable_curve(&curve);
    params.set_phase_offset(phase_offset);
    let mut state = editor_state(Arc::clone(&params));
    state.selected_curve_nodes = vec![1];

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressNode {
            index: 1,
            pointer: curve.nodes[1],
            shift_held: false,
            option_held: false,
            command_held: false,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleaseNode {
            index: 1,
            node: CurveNode {
                x: seam_raw(phase_offset),
                y: 0.55,
            },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
            shift_held: false,
            option_held: false,
            command_held: false,
        }),
    );

    let released = params.editable_curve_snapshot();
    assert_eq!(released.nodes[1].x, seam_raw(phase_offset));
    assert!(state.selected_curve_nodes.is_empty());
    assert_eq!(state.undo_history.len(), 1);
}

#[test]
fn canonical_seam_owner_uses_wrapped_endpoints_at_offset_zero_and_interior_at_offset_quarter() {
    let curve = interior_seam_curve();

    assert_eq!(
        canonical_seam_owner(&curve, 0.0),
        Some(CanonicalSeamOwner::Endpoints)
    );
    assert_eq!(
        canonical_seam_owner(&curve, 0.25),
        Some(CanonicalSeamOwner::Interior(2))
    );
    assert_eq!(
        canonical_seam_owner(&curve, 0.0005),
        Some(CanonicalSeamOwner::Endpoints)
    );
}

#[test]
fn near_interior_seam_nodes_are_ordinary_and_horizontally_movable() {
    for raw_x in [0.2495, 0.2505] {
        let curve = interior_seam_curve_with_raw_x(raw_x);
        assert_eq!(canonical_seam_owner(&curve, 0.25), None);

        let params = Arc::new(PumpParams::new());
        params.set_editable_curve(&curve);
        params.set_phase_offset(0.25);
        let mut state = editor_state(Arc::clone(&params));
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 2,
                pointer: curve.nodes[2],
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.6, y: 0.65 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );

        let dragged = params.editable_curve_snapshot();
        assert!((dragged.nodes[2].x - 0.6).abs() < 1.0e-6);
        assert!((dragged.nodes[2].y - 0.65).abs() < 1.0e-6);
    }
}

#[test]
fn exact_interior_seam_is_one_vertical_only_owner_from_both_display_sides() {
    let curve = interior_seam_curve();
    let seam_y = curve.nodes[2].y;
    assert!(seam_y.is_finite());

    for target_x in [0.1, 0.9] {
        let params = Arc::new(PumpParams::new());
        params.set_editable_curve(&curve);
        params.set_phase_offset(0.25);
        let mut state = editor_state(Arc::clone(&params));
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 2,
                pointer: curve.nodes[2],
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode {
                    x: target_x,
                    y: 0.65,
                },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes[2].x, seam_raw(0.25));
        assert!((dragged.nodes[2].y - 0.65).abs() < 1.0e-6);
    }
}

#[test]
fn existing_interior_seam_drag_ignores_horizontal_excursions() {
    let params = Arc::new(PumpParams::new());
    let curve = interior_seam_curve();
    params.set_editable_curve(&curve);
    params.set_phase_offset(0.25);
    let mut state = editor_state(Arc::clone(&params));

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::PressNode {
            index: 2,
            pointer: curve.nodes[2],
            shift_held: false,
            option_held: false,
            command_held: false,
        },
    );
    for target_x in [0.0, 1.0] {
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode {
                    x: target_x,
                    y: 0.65,
                },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
    }

    let dragged = params.editable_curve_snapshot();
    assert_eq!(dragged.nodes.len(), curve.nodes.len());
    assert!((dragged.nodes[2].x - seam_raw(0.25)).abs() < 1.0e-6);
    assert!((dragged.nodes[2].y - 0.65).abs() < 1.0e-6);
    assert_eq!(dragged.nodes[0], curve.nodes[0]);
    assert_eq!(dragged.nodes[1], curve.nodes[1]);
    assert_eq!(dragged.nodes[3], curve.nodes[3]);
    assert_eq!(dragged.nodes[4], curve.nodes[4]);
}

#[test]
fn incoming_node_waits_for_viewport_boundary_before_takeover() {
    let threshold = test_curve_push_through_threshold_x();
    let origin = EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.8 },
            CurveNode { x: 0.2, y: 0.3 },
            CurveNode { x: 0.5, y: 0.5 },
            CurveNode { x: 0.8, y: 0.4 },
            CurveNode { x: 1.0, y: 0.8 },
        ],
        segments: vec![CurveSegment { tension: 0.0 }; 4],
        ..EditableCurve::default()
    }
    .normalized();

    let params = Arc::new(PumpParams::new());
    params.set_editable_curve(&origin);
    let mut state = editor_state(Arc::clone(&params));
    reduce_curve_message(&mut state, unconstrained_press(1));
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragNode {
            index: 1,
            node: CurveNode {
                x: threshold,
                y: 0.35,
            },
            push_through_threshold_x: threshold,
        },
    );
    let ordinary = params.editable_curve_snapshot();
    assert_eq!(ordinary.nodes.len(), origin.nodes.len());
    assert_eq!(state.active_curve_node, Some(1));
    assert!((ordinary.nodes[0].y - origin.nodes[0].y).abs() < 1.0e-6);
    assert!((ordinary.nodes.last().unwrap().y - origin.nodes.last().unwrap().y).abs() < 1.0e-6);
    assert!(state
        .active_curve_node_drag
        .as_ref()
        .is_some_and(|drag| drag.seam_drag.is_none()));

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragNode {
            index: 1,
            node: CurveNode { x: 0.0, y: 0.35 },
            push_through_threshold_x: threshold,
        },
    );
    let taken_over = params.editable_curve_snapshot();
    assert_eq!(taken_over.nodes.len(), origin.nodes.len() - 1);
    assert!((taken_over.nodes[0].y - 0.35).abs() < 1.0e-6);
    assert!((taken_over.nodes.last().unwrap().y - 0.35).abs() < 1.0e-6);
    assert!(!taken_over
        .nodes
        .iter()
        .any(|node| (node.x - 0.2).abs() < 1.0e-6));
    assert_eq!(state.active_curve_node, Some(0));
}

#[test]
fn edge_takeover_unsnaps_inside_edge_zone_restores_origin_and_resnaps_at_boundary() {
    let threshold = 1.0e-4;
    let curve = interior_seam_curve_with_raw_x(0.2495);
    let phase_offset = 0.25;
    assert_eq!(canonical_seam_owner(&curve, phase_offset), None);

    let params = Arc::new(PumpParams::new());
    params.set_editable_curve(&curve);
    params.set_phase_offset(phase_offset);
    let mut state = editor_state(Arc::clone(&params));
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::PressNode {
            index: 2,
            pointer: curve.nodes[2],
            shift_held: false,
            option_held: false,
            command_held: false,
        },
    );
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragNode {
            index: 2,
            node: CurveNode {
                x: seam_raw(phase_offset),
                y: 0.55,
            },
            push_through_threshold_x: threshold,
        },
    );

    let taken_over = params.editable_curve_snapshot();
    assert_eq!(taken_over.nodes[2].x, 0.25);
    assert_eq!(taken_over.nodes[2].x, seam_raw(phase_offset));
    assert_eq!(
        canonical_seam_owner(&taken_over, phase_offset),
        Some(CanonicalSeamOwner::Interior(2))
    );

    let just_inside_edge_zone = seam_raw(phase_offset) - threshold * 0.5;
    assert!(display_x_is_in_edge_zone(
        just_inside_edge_zone,
        phase_offset,
        threshold
    ));
    assert!(!display_x_is_at_viewport_boundary(
        just_inside_edge_zone,
        phase_offset
    ));
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragNode {
            index: 2,
            node: CurveNode {
                x: just_inside_edge_zone,
                y: 0.65,
            },
            push_through_threshold_x: threshold,
        },
    );
    let restored = params.editable_curve_snapshot();
    assert_eq!(restored.nodes.len(), curve.nodes.len());
    assert!(restored.nodes[2].x > curve.nodes[1].x);
    assert!((restored.nodes[2].x - just_inside_edge_zone).abs() < 1.0e-6);
    assert!((restored.nodes[2].y - 0.65).abs() < 1.0e-6);
    assert_eq!(restored.nodes[3], curve.nodes[3]);
    assert_eq!(restored.segments, curve.segments);
    assert!(state
        .active_curve_node_drag
        .as_ref()
        .is_some_and(|drag| drag.seam_drag.is_none()));

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragNode {
            index: 2,
            node: CurveNode {
                x: seam_raw(phase_offset),
                y: 0.7,
            },
            push_through_threshold_x: threshold,
        },
    );
    let resnapped = params.editable_curve_snapshot();
    assert_eq!(resnapped.nodes[2].x, seam_raw(phase_offset));
    assert!((resnapped.nodes[2].y - 0.7).abs() < 1.0e-6);
    assert_eq!(
        canonical_seam_owner(&resnapped, phase_offset),
        Some(CanonicalSeamOwner::Interior(2))
    );

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::ReleaseNode {
            index: 2,
            node: CurveNode { x: 0.2, y: 0.8 },
            push_through_threshold_x: threshold,
            shift_held: false,
            option_held: false,
            command_held: false,
        },
    );
    let released = params.editable_curve_snapshot();
    assert_eq!(released.nodes.len(), curve.nodes.len());
    assert!(released.nodes[2].x > curve.nodes[1].x);
    assert!(released.nodes[2].x < curve.nodes[2].x);
    assert!((released.nodes[2].y - 0.8).abs() < 1.0e-6);
    assert_eq!(released.nodes[3], curve.nodes[3]);
    assert_eq!(released.segments, curve.segments);
    assert!(state.active_curve_node.is_none());
    assert!(state.active_curve_node_drag.is_none());
}

#[test]
fn seam_takeover_removes_competitors_remaps_active_index_and_inherits_incident_tension() {
    let params = Arc::new(PumpParams::new());
    let curve = EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.8 },
            CurveNode { x: 0.1, y: 0.2 },
            CurveNode { x: 0.24, y: 0.3 },
            CurveNode { x: 0.25, y: 0.5 },
            CurveNode { x: 0.9, y: 0.6 },
            CurveNode { x: 1.0, y: 0.8 },
        ],
        segments: vec![
            CurveSegment { tension: 0.11 },
            CurveSegment { tension: 0.22 },
            CurveSegment { tension: 0.33 },
            CurveSegment { tension: 0.44 },
            CurveSegment { tension: 0.55 },
        ],
        ..EditableCurve::default()
    }
    .normalized();
    params.set_editable_curve(&curve);
    params.set_phase_offset(0.25);
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
        CurvePreviewMessage::DragNode {
            index: 1,
            node: CurveNode { x: 0.25, y: 0.55 },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        },
    );

    let taken_over = params.editable_curve_snapshot();
    assert_eq!(taken_over.nodes.len(), 4);
    assert_eq!(taken_over.nodes[0].x, 0.0);
    assert!((taken_over.nodes[1].x - 0.25).abs() < 1.0e-6);
    assert_eq!(taken_over.nodes[3].x, 1.0);
    assert!(!taken_over
        .nodes
        .iter()
        .any(|node| (node.x - 0.1).abs() < 1.0e-6));
    assert!(!taken_over
        .nodes
        .iter()
        .any(|node| (node.x - 0.24).abs() < 1.0e-6));
    assert_eq!(state.active_curve_node, Some(1));
    assert!((taken_over.segments[0].tension - 0.11).abs() < 1.0e-6);
    assert!((taken_over.segments[1].tension - 0.22).abs() < 1.0e-6);
}

#[test]
fn group_containing_interior_seam_keeps_zero_horizontal_delta() {
    let params = Arc::new(PumpParams::new());
    let curve = interior_seam_curve();
    params.set_editable_curve(&curve);
    params.set_phase_offset(0.25);
    let mut state = editor_state(Arc::clone(&params));
    state.selected_curve_nodes = vec![2, 3];

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::PressNode {
            index: 2,
            pointer: curve.nodes[2],
            shift_held: false,
            option_held: false,
            command_held: false,
        },
    );
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragNode {
            index: 2,
            node: CurveNode { x: 0.1, y: 0.6 },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        },
    );

    let dragged = params.editable_curve_snapshot();
    assert!((dragged.nodes[2].x - curve.nodes[2].x).abs() < 1.0e-6);
    assert!((dragged.nodes[3].x - curve.nodes[3].x).abs() < 1.0e-6);
    assert!((dragged.nodes[2].y - 0.6).abs() < 1.0e-6);
    assert!((dragged.nodes[3].y - 0.8).abs() < 1.0e-6);
}

#[test]
fn multi_node_group_does_not_duplicate_an_occupied_interior_seam() {
    let params = Arc::new(PumpParams::new());
    let curve = interior_seam_curve();
    params.set_editable_curve(&curve);
    params.set_phase_offset(0.25);
    let mut state = editor_state(Arc::clone(&params));
    state.selected_curve_nodes = vec![1, 3];

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
        CurvePreviewMessage::DragNode {
            index: 1,
            node: CurveNode { x: 0.25, y: 0.7 },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        },
    );

    let dragged = params.editable_curve_snapshot();
    let seam_indices: Vec<_> = dragged
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            ((node.x - seam_raw(0.25)).abs() <= CURVE_SEAM_OWNER_RAW_EPSILON).then_some(index)
        })
        .collect();
    assert_eq!(seam_indices, vec![2]);
    assert_eq!(
        canonical_seam_owner(&dragged, 0.25),
        Some(CanonicalSeamOwner::Interior(2))
    );
    assert_eq!(dragged.nodes.len(), curve.nodes.len());
    assert_eq!(state.selected_curve_nodes, vec![1, 3]);
}

#[test]
fn seam_drag_has_one_undo_entry_and_roundtrips_through_redo() {
    let params = Arc::new(PumpParams::new());
    let origin = params.editable_curve_snapshot();
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressNode {
            index: 0,
            pointer: origin.nodes[0],
            shift_held: false,
            option_held: false,
            command_held: false,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
            index: 0,
            node: CurveNode { x: 0.8, y: 0.25 },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleaseNode {
            index: 0,
            node: CurveNode { x: 0.8, y: 0.25 },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
            shift_held: false,
            option_held: false,
            command_held: false,
        }),
    );

    let changed = params.editable_curve_snapshot();
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.redo_history.is_empty());
    assert_ne!(changed, origin);

    reduce_editor_message(&mut state, EditorMessage::Undo);
    assert_eq!(params.editable_curve_snapshot(), origin);
    assert_eq!(state.redo_history.len(), 1);

    reduce_editor_message(&mut state, EditorMessage::Redo);
    assert_eq!(params.editable_curve_snapshot(), changed);
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.redo_history.is_empty());
}

#[test]
fn legacy_edge_hidden_selection_is_not_deleted() {
    let params = Arc::new(PumpParams::new());
    let curve = legacy_edge_curve();
    params.set_editable_curve(&curve);
    let mut state = editor_state(Arc::clone(&params));
    state.selected_curve_nodes = (0..curve.nodes.len()).collect();

    reduce_curve_message(&mut state, CurvePreviewMessage::DeleteSelectedNodes);

    let remaining = params.editable_curve_snapshot();
    assert_eq!(remaining.nodes.len(), 2);
    assert_eq!(remaining.nodes[0].x, 0.0);
    assert_eq!(remaining.nodes[1].x, 1.0);
}

#[test]
fn legacy_edge_node_drag_preserves_origin_for_vertical_movement() {
    let params = Arc::new(PumpParams::new());
    let curve = legacy_edge_curve();
    params.set_editable_curve(&curve);
    params.set_phase_offset(0.0);
    let origin = curve.nodes[2];
    let mut state = editor_state(Arc::clone(&params));

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::PressNode {
            index: 2,
            pointer: origin,
            shift_held: false,
            option_held: false,
            command_held: false,
        },
    );
    assert_eq!(state.active_curve_node, Some(2));
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragNode {
            index: 2,
            node: CurveNode {
                x: origin.x,
                y: 0.35,
            },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        },
    );

    let dragged = params.editable_curve_snapshot();
    assert_eq!(dragged.nodes.len(), curve.nodes.len());
    assert_eq!(dragged.nodes[0].x, 0.0);
    assert_eq!(dragged.nodes[2].x, 1.0);
    assert!((dragged.nodes[0].y - 0.35).abs() < 1.0e-6);
    assert!((dragged.nodes[2].y - 0.35).abs() < 1.0e-6);
}

#[test]
fn legacy_edge_group_drag_keeps_zero_delta_feasible_for_vertical_movement() {
    let params = Arc::new(PumpParams::new());
    let curve = legacy_edge_curve();
    params.set_editable_curve(&curve);
    params.set_phase_offset(0.0);
    let origin = curve.nodes[2];
    let mut state = editor_state(Arc::clone(&params));
    state.selected_curve_nodes = vec![0, 2];

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::PressNode {
            index: 2,
            pointer: origin,
            shift_held: false,
            option_held: false,
            command_held: false,
        },
    );
    assert_eq!(
        state
            .active_curve_node_drag
            .as_ref()
            .map(|drag| drag.selected_indices.clone()),
        Some(vec![0, 2])
    );
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragNode {
            index: 2,
            node: CurveNode {
                x: origin.x,
                y: 0.35,
            },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        },
    );

    let dragged = params.editable_curve_snapshot();
    assert_eq!(dragged.nodes.len(), curve.nodes.len());
    assert!((dragged.nodes[2].x - origin.x).abs() < 1.0e-6);
    assert!((dragged.nodes[2].y - 0.35).abs() < 1.0e-6);
    assert!((dragged.nodes[0].y - 0.35).abs() < 1.0e-6);
    assert!((dragged.nodes[2].y - 0.35).abs() < 1.0e-6);
}

#[test]
fn ab_copy_and_switch_are_coherent_undo_redo_actions() {
    let params = Arc::new(PumpParams::new());
    params.set_mix(0.2);
    let mut state = editor_state(Arc::clone(&params));
    reduce_editor_message(
        &mut state,
        EditorMessage::SelectSound {
            side: SoundSide::B,
            copy: true,
        },
    );
    assert!(!params.sound_sides_differ());
    reduce_editor_message(&mut state, EditorMessage::Undo);
    assert!(params.sound_sides_differ());
    reduce_editor_message(&mut state, EditorMessage::Redo);
    assert!(!params.sound_sides_differ());
    reduce_editor_message(
        &mut state,
        EditorMessage::SelectSound {
            side: SoundSide::B,
            copy: false,
        },
    );
    assert_eq!(params.active_sound(), SoundSide::B);
    reduce_editor_message(&mut state, EditorMessage::Undo);
    assert_eq!(params.active_sound(), SoundSide::A);
    reduce_editor_message(&mut state, EditorMessage::Redo);
    assert_eq!(params.active_sound(), SoundSide::B);
}

#[test]
fn ab_command_copy_switches_to_the_copied_side_and_emits_sound_automation() {
    let params = Arc::new(PumpParams::new());
    params.set_mix(0.2);
    let queue = Arc::new(PumpAutomationQueue::default());
    let mut state = PumpEditorState::new(
        Arc::clone(&params),
        Arc::new(GuiStatus::default()),
        Arc::new(ClapHostParamEditSink {
            queue: Arc::clone(&queue),
            requester: None,
        }),
    );
    state.selected_curve_nodes.push(1);

    reduce_editor_message(&mut state, EditorMessage::CopyAndSelectSound(SoundSide::B));

    assert_eq!(params.active_sound(), SoundSide::B);
    assert!(!params.sound_sides_differ());
    assert!(state.selected_curve_nodes.is_empty());
    assert_eq!(state.undo_history.len(), 1);

    let mut buffer = EventBuffer::new();
    let mut output = buffer.as_output();
    let mut scratch = Vec::new();
    assert_eq!(
        queue.drain_to_output(&mut output, &mut scratch).attempted,
        3
    );
    let value =
        (0..buffer.len()).find_map(|index| match buffer.get(index as u32)?.as_core_event()? {
            CoreEventSpace::ParamValue(value) => Some((value.param_id(), value.value())),
            _ => None,
        });
    assert_eq!(value, Some((Some(PARAM_SOUND_ID), 1.0)));

    reduce_editor_message(&mut state, EditorMessage::Undo);
    assert_eq!(params.active_sound(), SoundSide::A);
    assert!(params.sound_sides_differ());
    reduce_editor_message(&mut state, EditorMessage::Redo);
    assert_eq!(params.active_sound(), SoundSide::B);
    assert!(!params.sound_sides_differ());
}

#[test]
fn ab_command_copy_switches_even_when_sides_are_equal() {
    let params = Arc::new(PumpParams::new());
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(&mut state, EditorMessage::CopyAndSelectSound(SoundSide::B));

    assert_eq!(params.active_sound(), SoundSide::B);
    assert!(!params.sound_sides_differ());
    assert_eq!(state.undo_history.len(), 1);
    reduce_editor_message(&mut state, EditorMessage::Undo);
    assert_eq!(params.active_sound(), SoundSide::A);
    reduce_editor_message(&mut state, EditorMessage::Redo);
    assert_eq!(params.active_sound(), SoundSide::B);
}

#[test]
fn equivalent_ab_copy_preserves_undo_and_redo_history() {
    let params = Arc::new(PumpParams::new());
    params.set_mix(0.2);
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::SelectSound {
            side: SoundSide::B,
            copy: true,
        },
    );
    params.set_mix(0.3);
    reduce_editor_message(
        &mut state,
        EditorMessage::SelectSound {
            side: SoundSide::B,
            copy: true,
        },
    );
    reduce_editor_message(&mut state, EditorMessage::Undo);
    params.set_mix(0.2);
    assert!(!params.sound_sides_differ());

    let undo_len = state.undo_history.len();
    let redo_len = state.redo_history.len();
    reduce_editor_message(
        &mut state,
        EditorMessage::SelectSound {
            side: SoundSide::B,
            copy: true,
        },
    );

    assert_eq!(state.undo_history.len(), undo_len);
    assert_eq!(state.redo_history.len(), redo_len);
}

#[test]
fn radiant_editor_curve_drag_updates_editable_curve() {
    let params = Arc::new(PumpParams::new());
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(&mut state, EditorMessage::Curve(unconstrained_press(1)));
    assert_eq!(state.active_curve_node, Some(1));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
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
        EditorMessage::Curve(CurvePreviewMessage::ReleaseNode {
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
fn radiant_editor_marquee_selects_node_centers_without_mutating_curve() {
    let params = Arc::new(PumpParams::new());
    let curve = EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.2 },
            CurveNode { x: 0.2, y: 0.8 },
            CurveNode { x: 0.5, y: 0.5 },
            CurveNode { x: 0.8, y: 0.2 },
            CurveNode { x: 1.0, y: 0.2 },
        ],
        segments: vec![CurveSegment { tension: 0.0 }; 4],
        ..EditableCurve::default()
    }
    .normalized();
    params.set_editable_curve(&curve);
    let before = params.editable_curve_snapshot();
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressMarquee {
            start: CurveNode { x: 0.15, y: 0.9 },
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragMarquee {
            current: CurveNode { x: 0.6, y: 0.1 },
        }),
    );
    assert!(state.active_curve_marquee.is_some());
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleaseMarquee {
            current: CurveNode { x: 0.6, y: 0.1 },
        }),
    );

    assert_eq!(state.selected_curve_nodes, vec![1, 2]);
    assert_eq!(params.editable_curve_snapshot(), before);
    assert!(state.active_curve_marquee.is_none());
    assert!(state.undo_history.is_empty());
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
        EditorMessage::Curve(CurvePreviewMessage::PressNode {
            index: 2,
            pointer: origin,
            shift_held: true,
            option_held: false,
            command_held: false,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
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
        EditorMessage::Curve(CurvePreviewMessage::PressNode {
            index: 1,
            pointer: origin,
            shift_held: false,
            option_held: false,
            command_held: false,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
            index: 1,
            node: CurveNode { x: 0.42, y: 0.6 },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        }),
    );
    let engaged_gain = params.editable_curve_snapshot().nodes[1].y;

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
            option_held: false,
            command_held: false,
            shift_held: true,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
            index: 1,
            node: CurveNode { x: 0.5, y: 0.05 },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        }),
    );
    assert!((params.editable_curve_snapshot().nodes[1].y - engaged_gain).abs() < 1.0e-6);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
            option_held: false,
            command_held: false,
            shift_held: false,
        }),
    );
    let released_gain = params.editable_curve_snapshot().nodes[1].y;
    assert!((released_gain - engaged_gain).abs() < 1.0e-6);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
            index: 1,
            node: CurveNode { x: 0.55, y: 0.05 },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        }),
    );
    assert!((params.editable_curve_snapshot().nodes[1].y - engaged_gain).abs() < 1.0e-6);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
            index: 1,
            node: CurveNode { x: 0.58, y: 0.15 },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        }),
    );
    assert!((params.editable_curve_snapshot().nodes[1].y - (engaged_gain + 0.1)).abs() < 1.0e-6);
}

#[test]
fn radiant_editor_shift_drag_preserves_gain_for_non_seam_node() {
    let params = Arc::new(PumpParams::new());
    let curve = params.editable_curve_snapshot();
    let origin = curve.nodes[1];
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressNode {
            index: 1,
            pointer: origin,
            shift_held: true,
            option_held: false,
            command_held: false,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
            index: 1,
            node: CurveNode { x: 0.2, y: 0.2 },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        }),
    );

    let dragged = params.editable_curve_snapshot();
    assert!((dragged.nodes[1].x - 0.2).abs() < 1.0e-6);
    assert!((dragged.nodes[1].y - origin.y).abs() < 1.0e-6);
}

#[test]
fn radiant_editor_canonical_seam_press_is_vertical_only_for_both_instances() {
    for index in [0, 3] {
        let params = Arc::new(PumpParams::new());
        let curve = params.editable_curve_snapshot();
        let origin = curve.nodes[index];
        let mut state = editor_state(Arc::clone(&params));

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index,
                pointer: origin,
                shift_held: true,
                option_held: false,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index,
                node: CurveNode {
                    x: if index == 0 { 0.8 } else { 0.2 },
                    y: 0.2,
                },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );

        let dragged = params.editable_curve_snapshot();
        let last = dragged.nodes.len() - 1;
        assert_eq!(dragged.nodes[0].x, 0.0);
        assert_eq!(dragged.nodes[last].x, 1.0);
        assert!((dragged.nodes[0].y - 0.2).abs() < 1.0e-6);
        assert!((dragged.nodes[last].y - 0.2).abs() < 1.0e-6);
    }
}

#[test]
fn radiant_editor_shift_anchor_does_not_leak_into_consecutive_gesture() {
    let params = Arc::new(PumpParams::new());
    let origin = params.editable_curve_snapshot().nodes[1];
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressNode {
            index: 1,
            pointer: origin,
            shift_held: true,
            option_held: false,
            command_held: false,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleaseNode {
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
        EditorMessage::Curve(CurvePreviewMessage::PressNode {
            index: 1,
            pointer: second_origin,
            shift_held: false,
            option_held: false,
            command_held: false,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
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
fn radiant_editor_shift_option_drag_does_not_take_over_viewport_boundaries() {
    let curve = EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.8 },
            CurveNode { x: 0.0001, y: 0.2 },
            CurveNode { x: 0.25, y: 0.6 },
            CurveNode { x: 0.53, y: 0.3 },
            CurveNode { x: 0.75, y: 0.5 },
            CurveNode { x: 0.9999, y: 0.15 },
            CurveNode { x: 1.0, y: 0.8 },
        ],
        segments: vec![
            CurveSegment { tension: 0.11 },
            CurveSegment { tension: 0.22 },
            CurveSegment { tension: 0.33 },
            CurveSegment { tension: 0.44 },
            CurveSegment { tension: 0.55 },
            CurveSegment { tension: 0.66 },
        ],
        ..EditableCurve::default()
    }
    .normalized();
    let origin = curve.nodes[3];

    let params = Arc::new(PumpParams::new());
    params.set_editable_curve(&curve);
    let mut state = editor_state(Arc::clone(&params));
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::PressNode {
            index: 3,
            pointer: origin,
            shift_held: true,
            option_held: true,
            command_held: false,
        },
    );

    for (boundary_x, y) in [(0.0, 0.05), (1.0, 0.15)] {
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 3,
                node: CurveNode { x: boundary_x, y },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes.len(), curve.nodes.len());
        assert_eq!(dragged.segments, curve.segments);
        for (dragged_node, origin_node) in dragged.nodes.iter().zip(&curve.nodes) {
            assert!((dragged_node.x - origin_node.x).abs() < 1.0e-6);
        }
        assert!((dragged.nodes[3].y - y).abs() < 1.0e-6);
        assert!(state
            .active_curve_node_drag
            .as_ref()
            .is_some_and(|drag| drag.seam_drag.is_none()));
    }
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
fn radiant_editor_direct_left_edge_drag_preserves_outgoing_tension() {
    assert_direct_edge_drag_preserves_incident_tension(CurveEdge::Left);
}

#[test]
fn radiant_editor_direct_right_edge_drag_preserves_incoming_tension() {
    assert_direct_edge_drag_preserves_incident_tension(CurveEdge::Right);
}

#[test]
fn radiant_editor_update_curve_node_preserves_edge_incident_tension() {
    for (index, target_x, target_y, expected_active, expected_segment, expected_tension) in
        [(1, 0.0, 0.35, 0, 0, 0.62), (3, 1.0, 0.65, 3, 2, -0.74)]
    {
        let origin = direct_edge_drag_curve();
        let mut updated = origin.clone();

        let moved_index = update_curve_node(
            &mut updated,
            index,
            CurveNode {
                x: target_x,
                y: target_y,
            },
        );

        assert_eq!(moved_index, expected_active);
        assert_eq!(updated.nodes.len(), origin.nodes.len() - 1);
        assert_eq!(updated.segments.len(), updated.nodes.len() - 1);
        let last_index = updated.nodes.len() - 1;
        assert_eq!(updated.nodes[0].y, target_y);
        assert_eq!(updated.nodes[last_index].y, target_y);
        assert_eq!(updated.segments[expected_segment].tension, expected_tension);
    }
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

    reduce_editor_message(&mut state, EditorMessage::Curve(unconstrained_press(2)));
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
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

    reduce_editor_message(&mut state, EditorMessage::Curve(unconstrained_press(2)));
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
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

    reduce_editor_message(&mut state, EditorMessage::Curve(unconstrained_press(1)));
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
            index: 1,
            node: CurveNode { x: 0.9, y: 0.45 },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        }),
    );

    let dragged = params.editable_curve_snapshot();
    assert_eq!(dragged.nodes.len(), 3);
    assert_eq!(dragged.nodes[0].x, 0.0);
    assert!((dragged.nodes[1].x - 0.9).abs() < 1.0e-6);
    assert_eq!(dragged.nodes[2].x, 1.0);
    assert_eq!(state.active_curve_node, Some(1));
}

#[test]
fn radiant_editor_selected_nodes_drag_as_group_with_spacing_clamp() {
    let params = Arc::new(PumpParams::new());
    let curve = EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.8 },
            CurveNode { x: 0.2, y: 0.2 },
            CurveNode { x: 0.4, y: 0.4 },
            CurveNode { x: 0.7, y: 0.6 },
            CurveNode { x: 1.0, y: 0.8 },
        ],
        segments: vec![CurveSegment { tension: 0.0 }; 4],
        ..EditableCurve::default()
    }
    .normalized();
    params.set_editable_curve(&curve);
    let mut state = editor_state(Arc::clone(&params));
    state.selected_curve_nodes = vec![1, 2];

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
    assert_eq!(state.selected_curve_nodes, vec![1, 2]);
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragNode {
            index: 1,
            node: CurveNode { x: 0.95, y: 0.8 },
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        },
    );

    let moved = params.editable_curve_snapshot();
    assert_eq!(moved.nodes.len(), curve.nodes.len());
    assert!((moved.nodes[1].x - (moved.nodes[2].x - 0.2)).abs() < 1.0e-6);
    assert!(moved.nodes[2].x <= moved.nodes[3].x - CURVE_NODE_MIN_SPACING_X);
    assert!((moved.nodes[1].y - 0.8).abs() < 1.0e-6);
    assert!((moved.nodes[2].y - 1.0).abs() < 1.0e-6);
    assert_eq!(moved.nodes[0].x, 0.0);
    assert_eq!(moved.nodes[4].x, 1.0);
    assert_eq!(state.selected_curve_nodes, vec![1, 2]);

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::ReleaseNode {
            index: 1,
            node: moved.nodes[1],
            push_through_threshold_x: test_curve_push_through_threshold_x(),
            shift_held: false,
            option_held: false,
            command_held: false,
        },
    );
    assert_eq!(state.selected_curve_nodes, vec![1, 2]);
}

#[test]
fn radiant_editor_pressing_unselected_node_clears_group_selection() {
    let params = Arc::new(PumpParams::new());
    let curve = params.editable_curve_snapshot();
    let mut state = editor_state(Arc::clone(&params));
    state.selected_curve_nodes = vec![1];

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::PressNode {
            index: 2,
            pointer: curve.nodes[2],
            shift_held: false,
            option_held: false,
            command_held: false,
        },
    );

    assert!(state.selected_curve_nodes.is_empty());
    assert!(state
        .active_curve_node_drag
        .as_ref()
        .is_some_and(|drag| drag.selected_indices.is_empty()));
}

#[test]
fn radiant_editor_delete_selected_nodes_preserves_endpoints_and_clears_state() {
    let params = Arc::new(PumpParams::new());
    let curve = EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.5 },
            CurveNode { x: 0.3, y: 0.2 },
            CurveNode { x: 0.6, y: 0.8 },
            CurveNode { x: 1.0, y: 0.5 },
        ],
        segments: vec![CurveSegment { tension: 0.0 }; 3],
        ..EditableCurve::default()
    }
    .normalized();
    params.set_editable_curve(&curve);
    let mut state = editor_state(Arc::clone(&params));
    state.selected_curve_nodes = vec![0, 1, 2, 3];
    state.active_curve_marquee = Some(ActiveCurveMarquee {
        start: curve.nodes[1],
        current: curve.nodes[2],
    });

    reduce_curve_message(&mut state, CurvePreviewMessage::DeleteSelectedNodes);

    let remaining = params.editable_curve_snapshot();
    assert_eq!(remaining.nodes.len(), 2);
    assert_eq!(remaining.segments.len(), 1);
    assert_eq!(remaining.nodes[0].x, 0.0);
    assert_eq!(remaining.nodes[1].x, 1.0);
    assert!(state.selected_curve_nodes.is_empty());
    assert!(state.active_curve_marquee.is_none());
    assert!(state.active_curve_node_drag.is_none());
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

    reduce_editor_message(&mut state, EditorMessage::Curve(unconstrained_press(2)));
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
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
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
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

    reduce_editor_message(&mut state, EditorMessage::Curve(unconstrained_press(2)));
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleaseNode {
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

    reduce_editor_message(&mut state, EditorMessage::Curve(unconstrained_press(0)));
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
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
        EditorMessage::Curve(CurvePreviewMessage::DeleteNode { index: 1 }),
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
        EditorMessage::Curve(CurvePreviewMessage::DeleteNode { index: 0 }),
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
        EditorMessage::Curve(CurvePreviewMessage::InsertNode {
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
    assert!(after.nodes.iter().any(
        |inserted| (inserted.x - node.x).abs() < 1.0e-6 && (inserted.y - node.y).abs() < 1.0e-6
    ));
}

#[test]
fn curve_preview_widget_inserts_left_edge_at_capacity() {
    let params = Arc::new(PumpParams::new());
    params.set_editable_curve(&max_capacity_curve());
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::InsertNode {
            node: CurveNode { x: 0.0005, y: 0.1 },
            command_held: true,
        }),
    );

    let after = params.editable_curve_snapshot();
    assert_eq!(after.nodes.len(), MAX_EDITABLE_NODES);
    assert_eq!(state.active_curve_node, Some(0));
    assert!((after.nodes[0].y - 0.1).abs() < 1.0e-6);
    assert_eq!(after.nodes[0].y, after.nodes[MAX_EDITABLE_NODES - 1].y);
}

#[test]
fn curve_preview_widget_inserts_right_edge_at_capacity() {
    let params = Arc::new(PumpParams::new());
    params.set_editable_curve(&max_capacity_curve());
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::InsertNode {
            node: CurveNode { x: 0.9995, y: 0.9 },
            command_held: true,
        }),
    );

    let after = params.editable_curve_snapshot();
    assert_eq!(after.nodes.len(), MAX_EDITABLE_NODES);
    assert_eq!(state.active_curve_node, Some(MAX_EDITABLE_NODES - 1));
    assert!((after.nodes[0].y - 0.9).abs() < 1.0e-6);
    assert_eq!(after.nodes[0].y, after.nodes[MAX_EDITABLE_NODES - 1].y);
}

#[test]
fn radiant_editor_canvas_insert_can_drag_inserted_node() {
    let params = Arc::new(PumpParams::new());
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::InsertNode {
            node: CurveNode { x: 0.72, y: 0.18 },
            command_held: false,
        }),
    );
    let inserted = state
        .active_curve_node
        .expect("inserted node should become active");

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragNode {
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
fn radiant_editor_edge_insertions_merge_to_visible_interactive_boundaries() {
    assert_edge_insert_is_single_visible_node(CurveNode { x: 0.0, y: 0.23 }, CurveEdge::Left);
    assert_edge_insert_is_single_visible_node(CurveNode { x: 1.0, y: 0.37 }, CurveEdge::Right);
}

#[test]
fn radiant_editor_option_hover_clears_insert_preview() {
    let params = Arc::new(PumpParams::new());
    let mut state = editor_state(params);
    state.preview_curve_node = Some(CurveNode { x: 0.2, y: 0.3 });
    state.hover_curve_segment = Some(1);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
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
        EditorMessage::Curve(CurvePreviewMessage::PressSegment {
            index: 0,
            position: start,
        }),
    );
    assert!(state.active_curve_segment.is_some());
    assert_eq!(state.preview_curve_node, None);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragSegment {
            index: 0,
            position: Point::new(start.x, start.y - 24.0),
            curve_size: Vector2::new(320.0, CURVE_PREVIEW_HEIGHT),
        }),
    );
    let upward_midpoint = sample_editable_curve(&params.editable_curve_snapshot(), 0.25);
    assert!(upward_midpoint > baseline_midpoint);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragSegment {
            index: 0,
            position: Point::new(start.x, start.y + 24.0),
            curve_size: Vector2::new(320.0, CURVE_PREVIEW_HEIGHT),
        }),
    );
    let downward_midpoint = sample_editable_curve(&params.editable_curve_snapshot(), 0.25);
    assert!(downward_midpoint < baseline_midpoint);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleaseSegment {
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
        EditorMessage::Curve(CurvePreviewMessage::PressSegmentMove {
            index: 1,
            position: start,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragSegment {
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
        EditorMessage::Curve(CurvePreviewMessage::PressSegmentMove {
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
        EditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
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
fn radiant_editor_direct_proximity_drag_is_transactional_and_locks_pair_origin() {
    let params = Arc::new(PumpParams::new());
    let curve = flat_segment_hit_curve();
    params.set_editable_curve(&curve);
    let mut state = editor_state(Arc::clone(&params));
    let curve_size = Vector2::new(320.0, 153.0);
    let start = Point::new(120.0, 48.0);
    let moved_pointer = Point::new(start.x + 24.0, start.y - 12.0);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressDirectProximitySegment {
            index: 1,
            position: start,
        }),
    );
    assert!(state.undo_history.is_empty());
    assert!(state
        .active_curve_segment
        .as_ref()
        .is_some_and(|drag| drag.history_origin.is_some()));
    assert!(state
        .active_curve_segment
        .as_ref()
        .is_some_and(|drag| drag.source == CurveSegmentDragSource::DirectProximity));

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragSegment {
            index: 0,
            position: moved_pointer,
            curve_size,
        },
    );

    let moved = params.editable_curve_snapshot();
    assert_eq!(moved.nodes.len(), curve.nodes.len());
    assert_eq!(moved.nodes[0], curve.nodes[0]);
    assert_eq!(moved.nodes[3], curve.nodes[3]);
    assert_eq!(moved.nodes[4], curve.nodes[4]);
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
    assert!(left_delta.0 > 0.0);
    assert!(left_delta.1 > 0.0);
    assert!((moved.nodes[2].y - moved.nodes[1].y).abs() < 1.0e-6);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleaseSegment {
            index: 0,
            position: moved_pointer,
            curve_size,
        }),
    );
    assert!(state.active_curve_segment.is_none());
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.redo_history.is_empty());

    reduce_editor_message(&mut state, EditorMessage::Undo);
    assert_eq!(params.editable_curve_snapshot(), curve);
    assert!(state.undo_history.is_empty());
    assert_eq!(state.redo_history.len(), 1);

    reduce_editor_message(&mut state, EditorMessage::Redo);
    assert_eq!(params.editable_curve_snapshot(), moved);
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.redo_history.is_empty());
}

#[test]
fn radiant_editor_direct_proximity_noop_release_and_cancel_preserve_redo() {
    let params = Arc::new(PumpParams::new());
    let curve = flat_segment_hit_curve();
    params.set_editable_curve(&curve);
    let mut state = editor_state(Arc::clone(&params));
    let curve_size = Vector2::new(320.0, 153.0);
    let start = Point::new(120.0, 48.0);
    let moved_pointer = Point::new(start.x + 24.0, start.y - 12.0);

    state.push_history();
    params.set_mix(0.25);
    reduce_editor_message(&mut state, EditorMessage::Undo);
    assert!(state.undo_history.is_empty());
    assert_eq!(state.redo_history.len(), 1);

    let press = EditorMessage::Curve(CurvePreviewMessage::PressDirectProximitySegment {
        index: 1,
        position: start,
    });
    reduce_editor_message(&mut state, press.clone());
    assert!(state.undo_history.is_empty());
    assert_eq!(state.redo_history.len(), 1);
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleaseSegment {
            index: 1,
            position: start,
            curve_size,
        }),
    );
    assert!(state.undo_history.is_empty());
    assert_eq!(state.redo_history.len(), 1);
    assert_eq!(params.editable_curve_snapshot(), curve);

    reduce_editor_message(&mut state, press);
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragSegment {
            index: 1,
            position: moved_pointer,
            curve_size,
        },
    );
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragSegment {
            index: 1,
            position: start,
            curve_size,
        },
    );
    assert_eq!(params.editable_curve_snapshot(), curve);
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::Cancel),
    );
    assert!(state.undo_history.is_empty());
    assert_eq!(state.redo_history.len(), 1);
    assert!(state.active_curve_segment.is_none());
}

#[test]
fn radiant_editor_direct_proximity_cleanup_does_not_cancel_on_command_release() {
    let params = Arc::new(PumpParams::new());
    params.set_editable_curve(&flat_segment_hit_curve());
    let mut state = editor_state(Arc::clone(&params));
    let start = Point::new(120.0, 48.0);
    let press = CurvePreviewMessage::PressDirectProximitySegment {
        index: 1,
        position: start,
    };

    reduce_curve_message(&mut state, press);
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::ModifiersChanged {
            option_held: false,
            command_held: true,
            shift_held: false,
        },
    );
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::ModifiersChanged {
            option_held: false,
            command_held: false,
            shift_held: false,
        },
    );
    assert!(state
        .active_curve_segment
        .as_ref()
        .is_some_and(|drag| drag.source == CurveSegmentDragSource::DirectProximity));
    assert_eq!(state.hover_curve_segment, None);
    assert_eq!(state.hover_curve_segment_zone, None);

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::ReleaseSegment {
            index: 0,
            position: start,
            curve_size: Vector2::new(320.0, 153.0),
        },
    );
    assert!(state.active_curve_segment.is_none());
    assert!(state.hover_curve_segment.is_none());
    assert!(state.hover_curve_segment_zone.is_none());

    reduce_curve_message(&mut state, press);
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::ModifiersChanged {
            option_held: false,
            command_held: true,
            shift_held: false,
        },
    );
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::ReleaseSegment {
            index: 1,
            position: start,
            curve_size: Vector2::new(320.0, 153.0),
        },
    );
    assert!(state.active_curve_segment.is_none());
    assert!(state.hover_curve_segment.is_none());
    assert!(state.hover_curve_segment_zone.is_none());

    reduce_curve_message(&mut state, press);
    state.hover_curve_segment = Some(1);
    state.hover_curve_segment_zone = Some(CurveSegmentHitZone::OuterProximity);
    reduce_curve_message(&mut state, CurvePreviewMessage::Cancel);
    assert!(state.active_curve_segment.is_none());
    assert!(state.hover_curve_segment.is_none());
    assert!(state.hover_curve_segment_zone.is_none());
}

#[test]
fn active_curve_paint_preserves_ordered_boundary_observations() {
    let origin_snapshot = editor_state(Arc::new(PumpParams::new())).snapshot();
    let mut paint = ActiveCurvePaint::new(origin_snapshot, 0.0);
    let start = paint_sample(0.35, 0.4);
    let first = boundary_paint_sample(0.0, 0.7);
    let replacement = boundary_paint_sample(0.0, 0.2);
    paint.push_sample(start);
    paint.push_boundary_sample(first);
    paint.push_boundary_sample(replacement);

    let runs = paint.preview_runs();
    assert_eq!(runs.len(), 1);
    let points = runs[0].points();
    assert_eq!(
        points.first().map(|point| point.position),
        Some(start.raw_position())
    );
    assert!(points.iter().any(|point| {
        point.position == first.raw_position()
            && matches!(
                point.contact,
                BoundaryContact::Edge(EdgeParameter {
                    edge: BoundaryEdge::Left,
                    ..
                })
            )
    }));
    assert_eq!(
        points.last().map(|point| point.position),
        Some(first.raw_position())
    );
}

#[test]
fn edge_paint_preview_keeps_authored_topology_until_release() {
    let params = Arc::new(PumpParams::new());
    let origin = interior_seam_curve();
    params.set_editable_curve(&origin);
    params.set_phase_offset(0.25);
    let mut state = editor_state(Arc::clone(&params));

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::PressPaint {
            sample: paint_sample(0.4, 0.35),
        },
    );
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragPaint {
            sample: boundary_paint_sample(0.0, 0.7),
        },
    );

    let paint = state
        .active_curve_paint
        .as_ref()
        .expect("edge paint remains active before release");
    let preview_runs = paint.preview_runs();
    let points = preview_runs[0].points();
    assert_eq!(
        points.last().map(|point| point.position),
        Some(RectPoint { x: 0.0, y: 0.7 })
    );
    assert!(matches!(
        points.last().map(|point| point.contact),
        Some(BoundaryContact::Edge(EdgeParameter {
            edge: BoundaryEdge::Left,
            ..
        }))
    ));
    assert_eq!(params.editable_curve_snapshot(), origin);
}

#[test]
fn released_edge_paint_uses_the_first_retained_crossing() {
    for first_right in [false, true] {
        for continued_outside in [false, true] {
            let params = Arc::new(PumpParams::new());
            let origin = EditableCurve {
                nodes: vec![
                    CurveNode { x: 0.0, y: 0.8 },
                    CurveNode { x: 0.2, y: 0.2 },
                    CurveNode { x: 0.5, y: 0.4 },
                    CurveNode { x: 1.0, y: 0.8 },
                ],
                segments: vec![
                    CurveSegment { tension: 0.1 },
                    CurveSegment { tension: 0.2 },
                    CurveSegment { tension: 0.3 },
                ],
                ..EditableCurve::default()
            }
            .normalized();
            params.set_editable_curve(&origin);
            params.set_phase_offset(0.25);
            let mut state = editor_state(Arc::clone(&params));

            reduce_editor_message(
                &mut state,
                EditorMessage::Curve(CurvePreviewMessage::PressPaint {
                    sample: paint_sample(0.4, 0.35),
                }),
            );
            let first_x = if first_right { 1.0 } else { 0.0 };
            let first_y = if first_right { 0.8 } else { 0.2 };
            reduce_editor_message(
                &mut state,
                EditorMessage::Curve(CurvePreviewMessage::DragPaint {
                    sample: boundary_paint_sample(first_x, first_y),
                }),
            );
            if continued_outside {
                reduce_editor_message(
                    &mut state,
                    EditorMessage::Curve(CurvePreviewMessage::DragPaint {
                        sample: boundary_paint_sample(first_x, if first_right { 0.2 } else { 0.8 }),
                    }),
                );
            }
            reduce_editor_message(
                &mut state,
                EditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
            );

            let committed = params.editable_curve_snapshot();
            let seam_nodes = committed
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| (node.x - 0.25).abs() <= CURVE_PAINT_ASSERT_EPSILON)
                .collect::<Vec<_>>();
            assert_eq!(seam_nodes.len(), 1);
            let (seam_index, seam) = seam_nodes[0];
            assert_eq!(
                canonical_seam_owner(&committed, 0.25),
                Some(CanonicalSeamOwner::Interior(seam_index))
            );
            assert!((seam.y - first_y).abs() <= CURVE_PAINT_ASSERT_EPSILON);
            let painted_segment_index = if first_right {
                seam_index.saturating_sub(1)
            } else {
                seam_index
            };
            assert!(
                committed.segments[painted_segment_index].tension.abs() <= 0.05,
                "painted seam incident tension leaked: nodes={:?}, segments={:?}",
                committed.nodes,
                committed.segments
            );
            assert_eq!(state.undo_history.len(), 1);
        }
    }
}

#[test]
fn straight_edge_paint_has_one_seam_owner_and_fits_the_retained_line() {
    let start_x = 0.4;
    let start_y = 0.35;
    for phase_offset in [0.0, 0.25, 0.73] {
        for first_right in [false, true] {
            let first_x = if first_right { 1.0 } else { 0.0 };
            let first_y = if first_right { 0.8 } else { 0.2 };
            let params = Arc::new(PumpParams::new());
            let origin = EditableCurve {
                nodes: vec![
                    CurveNode { x: 0.0, y: 0.8 },
                    CurveNode { x: 0.2, y: 0.2 },
                    CurveNode { x: 0.5, y: 0.4 },
                    CurveNode { x: 1.0, y: 0.8 },
                ],
                segments: vec![
                    CurveSegment { tension: 0.72 },
                    CurveSegment { tension: -0.63 },
                    CurveSegment { tension: 0.58 },
                ],
                ..EditableCurve::default()
            }
            .normalized();
            params.set_editable_curve(&origin);
            params.set_phase_offset(phase_offset);
            let mut state = editor_state(Arc::clone(&params));

            reduce_editor_message(
                &mut state,
                EditorMessage::Curve(CurvePreviewMessage::PressPaint {
                    sample: paint_sample(start_x, start_y),
                }),
            );
            reduce_editor_message(
                &mut state,
                EditorMessage::Curve(CurvePreviewMessage::DragPaint {
                    sample: boundary_paint_sample(first_x, first_y),
                }),
            );
            reduce_editor_message(
                &mut state,
                EditorMessage::Curve(CurvePreviewMessage::DragPaint {
                    sample: boundary_paint_sample(first_x, if first_right { 0.2 } else { 0.8 }),
                }),
            );
            reduce_editor_message(
                &mut state,
                EditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
            );

            let candidate = params.editable_curve_snapshot();
            assert_curve_paint_topology_is_bounded(&candidate);
            let seam = seam_raw(phase_offset);
            match canonical_seam_owner(&candidate, phase_offset) {
                Some(CanonicalSeamOwner::Endpoints) => {
                    assert!(seam_uses_wrapped_endpoints(phase_offset));
                    assert!((candidate.nodes[0].y - first_y).abs() <= CURVE_PAINT_ASSERT_EPSILON);
                    assert_eq!(candidate.nodes[0].y, candidate.nodes.last().unwrap().y);
                    assert!(candidate.nodes[1..candidate.nodes.len() - 1]
                        .iter()
                        .all(|node| node.x > CURVE_SEAM_OWNER_RAW_EPSILON
                            && node.x < 1.0 - CURVE_SEAM_OWNER_RAW_EPSILON));
                }
                Some(CanonicalSeamOwner::Interior(index)) => {
                    assert!(!seam_uses_wrapped_endpoints(phase_offset));
                    let seam_nodes = candidate
                        .nodes
                        .iter()
                        .enumerate()
                        .filter(|(_, node)| (node.x - seam).abs() <= CURVE_SEAM_OWNER_RAW_EPSILON)
                        .collect::<Vec<_>>();
                    assert_eq!(seam_nodes.len(), 1);
                    assert_eq!(seam_nodes[0].0, index);
                    assert!((seam_nodes[0].1.y - first_y).abs() <= CURVE_PAINT_ASSERT_EPSILON);
                }
                None => panic!("painted edge must establish a seam owner: {candidate:?}"),
            }

            let painted_segment_index = if seam_uses_wrapped_endpoints(phase_offset) {
                if first_right {
                    candidate.segments.len().saturating_sub(1)
                } else {
                    0
                }
            } else {
                let seam_index = candidate
                    .nodes
                    .iter()
                    .position(|node| (node.x - seam).abs() <= CURVE_SEAM_OWNER_RAW_EPSILON)
                    .expect("interior seam owner should be present");
                if first_right {
                    seam_index.saturating_sub(1)
                } else {
                    seam_index
                }
            };
            assert!(
                candidate.segments[painted_segment_index].tension.abs() <= 0.05,
                "straight edge paint inherited tension: offset={phase_offset}, right={first_right}, candidate={candidate:?}"
            );

            for fraction in [0.25, 0.5, 0.75] {
                let display_x = if first_right {
                    start_x + (1.0 - start_x) * fraction
                } else {
                    start_x * (1.0 - fraction)
                };
                let expected_y = start_y + (first_y - start_y) * fraction;
                let raw_x = (display_x + phase_offset).rem_euclid(1.0);
                let actual_y = sample_editable_curve(&candidate, raw_x);
                assert!(
                    (actual_y - expected_y).abs() <= 0.018 + CURVE_PAINT_ASSERT_EPSILON,
                    "straight edge paint drifted: offset={phase_offset}, right={first_right}, display_x={display_x}, expected={expected_y}, actual={actual_y}, candidate={candidate:?}"
                );
            }
            assert_eq!(state.undo_history.len(), 1);
        }
    }
}

#[test]
fn edge_paint_keeps_an_untouched_authored_tension_exact() {
    let origin = EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.8 },
            CurveNode { x: 0.2, y: 0.2 },
            CurveNode { x: 0.5, y: 0.4 },
            CurveNode { x: 1.0, y: 0.8 },
        ],
        segments: vec![
            CurveSegment { tension: 0.72 },
            CurveSegment { tension: -0.63 },
            CurveSegment { tension: 0.58 },
        ],
        ..EditableCurve::default()
    }
    .normalized();
    let run = recorded_run([RectPoint { x: 0.4, y: 0.35 }, RectPoint { x: 0.0, y: 0.2 }]);

    let candidate = reconstruct_paint(&origin, 0.25, &[run]).candidate().clone();
    let untouched_segment = candidate
        .nodes
        .windows(2)
        .zip(&candidate.segments)
        .find(|(nodes, _)| {
            (nodes[0].x - 0.2).abs() <= CURVE_PAINT_ASSERT_EPSILON
                && (nodes[1].x - 0.5).abs() <= CURVE_PAINT_ASSERT_EPSILON
        })
        .map(|(_, segment)| segment)
        .expect("untouched authored segment should remain present");
    assert_eq!(untouched_segment.tension, origin.segments[1].tension);
}

#[test]
fn curved_stroke_exiting_right_edge_keeps_nonzero_fitted_tension() {
    let origin = EditableCurve {
        nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
        segments: vec![CurveSegment { tension: 0.0 }],
        ..EditableCurve::default()
    }
    .normalized();
    let left = CurveNode { x: 0.2, y: 0.15 };
    let right = CurveNode { x: 1.0, y: 0.85 };
    let source_tension = 0.65;
    let run = sampled_segment_run(left, right, source_tension, 16);

    let outcome = reconstruct_paint(&origin, 0.0, &[run]);
    assert!(matches!(&outcome, PaintCommitOutcome::Applied { .. }));
    let candidate = outcome.candidate();
    let painted_segment = candidate
        .nodes
        .windows(2)
        .zip(&candidate.segments)
        .find(|(nodes, _)| {
            (nodes[0].x - left.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
                && (nodes[1].x - right.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
        })
        .map(|(_, segment)| segment)
        .expect("painted edge endpoints should remain adjacent");
    assert!(painted_segment.tension.abs() > 0.2);
    assert!((painted_segment.tension - source_tension).abs() <= 0.08);
}

#[test]
fn released_non_edge_paint_does_not_canonicalize_the_viewport_seam() {
    let params = Arc::new(PumpParams::new());
    let origin = EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.8 },
            CurveNode { x: 0.2, y: 0.2 },
            CurveNode { x: 0.5, y: 0.4 },
            CurveNode { x: 1.0, y: 0.8 },
        ],
        segments: vec![CurveSegment { tension: 0.0 }; 3],
        ..EditableCurve::default()
    }
    .normalized();
    params.set_editable_curve(&origin);
    params.set_phase_offset(0.25);
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint {
            sample: paint_sample(0.4, 0.25),
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaint {
            sample: paint_sample(0.6, 0.75),
        }),
    );
    assert!(state.active_curve_paint.as_ref().is_some_and(|paint| {
        paint.preview_runs().iter().all(|run| {
            run.points()
                .iter()
                .all(|point| matches!(point.contact, BoundaryContact::Interior))
        })
    }));
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
    );

    let committed = params.editable_curve_snapshot();
    assert!(!committed
        .nodes
        .iter()
        .any(|node| (node.x - 0.75).abs() <= CURVE_PAINT_ASSERT_EPSILON));
}

#[test]
fn truncated_active_curve_paint_preview_and_release_use_retained_geometry() {
    let params = Arc::new(PumpParams::new());
    let origin = params.editable_curve_snapshot();
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint {
            sample: paint_sample(0.1, 0.2),
        }),
    );
    for index in 1..=128 {
        let (x, y) = if index % 2 == 0 {
            (0.1, 0.2)
        } else {
            (0.9, 0.8)
        };
        reduce_editor_message(
            &mut state,
            EditorMessage::Curve(CurvePreviewMessage::DragPaint {
                sample: paint_sample(x, y),
            }),
        );
    }

    let paint = state
        .active_curve_paint
        .as_ref()
        .expect("truncated paint remains active until release");
    assert!(paint.recorder.is_truncated());
    assert_ne!(paint.preview_candidate(), origin);
    assert!(matches!(
        paint.finished_curve(),
        PaintCommitOutcome::Applied { .. }
    ));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint {
            sample: Some(paint_sample(0.1, 0.2)),
        }),
    );
    assert_ne!(params.editable_curve_snapshot(), origin);
    assert_eq!(state.undo_history.len(), 1);
}

#[test]
fn curve_paint_reconstructor_preserves_order_and_boundary_seams() {
    let origin = EditableCurve {
        nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
        segments: vec![CurveSegment { tension: 0.0 }],
        ..EditableCurve::default()
    }
    .normalized();
    let run = recorded_run([
        RectPoint { x: 0.0, y: 0.4 },
        RectPoint { x: 0.25, y: 0.2 },
        RectPoint { x: 0.5, y: 0.8 },
        RectPoint { x: 0.75, y: 0.2 },
        RectPoint { x: 1.0, y: 0.6 },
    ]);

    let outcome = reconstruct_paint(&origin, 0.0, &[run]);
    let candidate = match outcome {
        PaintCommitOutcome::Applied { candidate } => candidate,
        other => panic!("boundary paint should apply, got {other:?}"),
    };
    assert_curve_paint_topology_is_bounded(&candidate);
    assert!(candidate.nodes.iter().any(|node| {
        (node.x - 0.25).abs() <= CURVE_PAINT_ASSERT_EPSILON
            && (node.y - 0.2).abs() <= CURVE_PAINT_ASSERT_EPSILON
    }));
    assert!(candidate.nodes.iter().any(|node| {
        (node.x - 0.5).abs() <= CURVE_PAINT_ASSERT_EPSILON
            && (node.y - 0.8).abs() <= CURVE_PAINT_ASSERT_EPSILON
    }));
    assert!(candidate.nodes.iter().any(|node| {
        (node.x - 0.75).abs() <= CURVE_PAINT_ASSERT_EPSILON
            && (node.y - 0.2).abs() <= CURVE_PAINT_ASSERT_EPSILON
    }));
}

#[test]
fn curve_paint_prefers_one_curved_segment_over_an_overfit_sampled_stroke() {
    let origin = EditableCurve {
        nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
        segments: vec![CurveSegment { tension: 0.0 }],
        ..EditableCurve::default()
    }
    .normalized();
    let left = CurveNode { x: 0.1, y: 0.15 };
    let right = CurveNode { x: 0.9, y: 0.85 };
    let source_tension = 0.65;
    let run = sampled_segment_run(left, right, source_tension, 16);

    let outcome = reconstruct_paint(&origin, 0.0, &[run]);
    assert!(matches!(&outcome, PaintCommitOutcome::Applied { .. }));
    let candidate = outcome.candidate();
    let interval_nodes = candidate
        .nodes
        .iter()
        .filter(|node| {
            node.x >= left.x - CURVE_PAINT_ASSERT_EPSILON
                && node.x <= right.x + CURVE_PAINT_ASSERT_EPSILON
        })
        .collect::<Vec<_>>();
    assert_eq!(interval_nodes.len(), 2, "candidate: {candidate:?}");
    assert!((interval_nodes[0].x - left.x).abs() <= CURVE_PAINT_ASSERT_EPSILON);
    assert!((interval_nodes[1].x - right.x).abs() <= CURVE_PAINT_ASSERT_EPSILON);

    let painted_segment = candidate
        .nodes
        .windows(2)
        .zip(candidate.segments.iter())
        .find(|(nodes, _)| {
            (nodes[0].x - left.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
                && (nodes[1].x - right.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
        })
        .map(|(_, segment)| segment)
        .expect("painted endpoints should remain adjacent");
    assert!(painted_segment.tension.abs() > 0.2);
    assert!((painted_segment.tension - source_tension).abs() <= 0.08);
}

#[test]
fn curve_paint_keeps_a_sampled_linear_stroke_near_zero_tension() {
    let origin = EditableCurve {
        nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
        segments: vec![CurveSegment { tension: 0.0 }],
        ..EditableCurve::default()
    }
    .normalized();
    let left = CurveNode { x: 0.1, y: 0.15 };
    let right = CurveNode { x: 0.9, y: 0.85 };
    let run = sampled_segment_run(left, right, 0.0, 16);

    let outcome = reconstruct_paint(&origin, 0.0, &[run]);
    assert!(matches!(&outcome, PaintCommitOutcome::Applied { .. }));
    let candidate = outcome.candidate();
    let interval_nodes = candidate
        .nodes
        .iter()
        .filter(|node| {
            node.x >= left.x - CURVE_PAINT_ASSERT_EPSILON
                && node.x <= right.x + CURVE_PAINT_ASSERT_EPSILON
        })
        .collect::<Vec<_>>();
    assert_eq!(interval_nodes.len(), 2, "candidate: {candidate:?}");

    let painted_segment = candidate
        .nodes
        .windows(2)
        .zip(candidate.segments.iter())
        .find(|(nodes, _)| {
            (nodes[0].x - left.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
                && (nodes[1].x - right.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
        })
        .map(|(_, segment)| segment)
        .expect("painted endpoints should remain adjacent");
    assert!(painted_segment.tension.abs() <= 0.05);
}

#[test]
fn undo_discards_active_curve_paint_before_restoring_history() {
    let params = Arc::new(PumpParams::new());
    let origin = EditableCurve {
        nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
        segments: vec![CurveSegment { tension: 0.0 }],
        ..EditableCurve::default()
    }
    .normalized();
    params.set_editable_curve(&origin);
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint {
            sample: paint_sample(0.2, 0.2),
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaint {
            sample: paint_sample(0.8, 0.8),
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
    );
    let painted = params.editable_curve_snapshot();
    assert_ne!(painted, origin);
    assert_eq!(state.undo_history.len(), 1);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint {
            sample: paint_sample(0.3, 0.9),
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaint {
            sample: paint_sample(0.7, 0.1),
        }),
    );
    assert!(state.active_curve_paint.is_some());

    reduce_editor_message(&mut state, EditorMessage::Undo);
    assert!(state.active_curve_paint.is_none());
    assert_eq!(params.editable_curve_snapshot(), origin);
    assert!(state.undo_history.is_empty());
    assert_eq!(state.redo_history.len(), 1);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint {
            sample: Some(paint_sample(0.7, 0.1)),
        }),
    );
    assert_eq!(params.editable_curve_snapshot(), origin);
    assert!(state.undo_history.is_empty());
    assert_eq!(state.redo_history.len(), 1);
    assert_ne!(painted, origin);
}

#[test]
fn redo_discards_active_curve_paint_before_restoring_history() {
    let params = Arc::new(PumpParams::new());
    let origin = EditableCurve {
        nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
        segments: vec![CurveSegment { tension: 0.0 }],
        ..EditableCurve::default()
    }
    .normalized();
    params.set_editable_curve(&origin);
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint {
            sample: paint_sample(0.2, 0.2),
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaint {
            sample: paint_sample(0.8, 0.8),
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
    );
    let painted = params.editable_curve_snapshot();
    reduce_editor_message(&mut state, EditorMessage::Undo);
    assert_eq!(params.editable_curve_snapshot(), origin);
    assert_eq!(state.redo_history.len(), 1);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint {
            sample: paint_sample(0.3, 0.9),
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaint {
            sample: paint_sample(0.7, 0.1),
        }),
    );
    assert!(state.active_curve_paint.is_some());

    reduce_editor_message(&mut state, EditorMessage::Redo);
    assert!(state.active_curve_paint.is_none());
    assert_eq!(params.editable_curve_snapshot(), painted);
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.redo_history.is_empty());

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint {
            sample: Some(paint_sample(0.7, 0.1)),
        }),
    );
    assert_eq!(params.editable_curve_snapshot(), painted);
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.redo_history.is_empty());
}

#[test]
fn curve_paint_reentry_splits_preview_and_commit_without_an_unobserved_chord() {
    let params = Arc::new(PumpParams::new());
    let origin = EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 1.0 },
            CurveNode { x: 0.2, y: 0.2 },
            CurveNode { x: 0.5, y: 0.8 },
            CurveNode { x: 0.8, y: 0.3 },
            CurveNode { x: 1.0, y: 1.0 },
        ],
        segments: vec![CurveSegment { tension: 0.0 }; 4],
        ..EditableCurve::default()
    }
    .normalized();
    params.set_editable_curve(&origin);
    let mut state = editor_state(Arc::clone(&params));
    let first = paint_sample(0.2, 0.75);
    let second = paint_sample(0.3, 0.25);
    let outside = boundary_paint_sample(1.0, 0.55);
    let reentry = paint_sample(0.7, 0.65);
    let final_sample = paint_sample(0.8, 0.35);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: first }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaint { sample: second }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaintOutside { sample: outside }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaint { sample: reentry }),
    );
    let paint = state
        .active_curve_paint
        .as_ref()
        .expect("re-entry keeps painting active");
    let runs = paint.preview_runs();
    assert_eq!(runs.len(), 2);
    assert_eq!(
        runs[0].points().first().map(|point| point.position),
        Some(first.raw_position())
    );
    assert!(runs[0]
        .points()
        .iter()
        .any(|point| point.position == outside.raw_position()));
    assert_eq!(
        runs[1].points().first().map(|point| point.position),
        Some(outside.raw_position())
    );
    assert!(runs[1]
        .points()
        .iter()
        .any(|point| point.position == reentry.raw_position()));
    assert!(matches!(
        runs[1].points().first().map(|point| point.contact),
        Some(BoundaryContact::Edge(EdgeParameter {
            edge: BoundaryEdge::Right,
            ..
        }))
    ));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaint {
            sample: final_sample,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaintOutside { sample: outside }),
    );

    let painted = params.editable_curve_snapshot();
    assert_eq!(state.undo_history.len(), 1);
    assert!(painted.nodes.iter().any(|node| {
        (node.x - outside.node.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
            && (node.y - outside.node.y).abs() <= 0.08
    }));
    assert!(painted.nodes.iter().any(|node| {
        (node.x - reentry.node.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
            && (node.y - reentry.node.y).abs() <= 0.08
    }));
    assert_curve_paint_topology_is_bounded(&painted);
}

#[test]
fn curve_paint_full_capacity_boundary_extension_applies_boundary_candidate() {
    let params = Arc::new(PumpParams::new());
    let origin = EditableCurve {
        nodes: (0..MAX_EDITABLE_NODES)
            .map(|index| {
                let x = index as f32 / (MAX_EDITABLE_NODES - 1) as f32;
                CurveNode {
                    x,
                    y: 0.3 + 0.4 * x,
                }
            })
            .collect(),
        segments: vec![CurveSegment { tension: 0.0 }; MAX_EDITABLE_NODES - 1],
        ..EditableCurve::default()
    }
    .normalized();
    assert_eq!(origin.nodes.len(), MAX_EDITABLE_NODES);
    params.set_editable_curve(&origin);
    let mut state = editor_state(Arc::clone(&params));
    let in_bounds = paint_sample(0.005, 0.2);
    let boundary = boundary_paint_sample(0.0, 0.8);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: in_bounds }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaintOutside { sample: boundary }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaintOutside { sample: boundary }),
    );

    let painted = params.editable_curve_snapshot();
    assert_curve_paint_topology_is_bounded(&painted);
    assert_ne!(painted, origin);
    assert_eq!(
        painted.nodes.first().map(|node| node.y),
        Some(boundary.node.y)
    );
    assert_eq!(
        painted.nodes.last().map(|node| node.y),
        Some(boundary.node.y)
    );
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.redo_history.is_empty());
    assert!(state.active_curve_paint.is_none());
}

#[test]
fn curve_paint_effective_boundary_release_has_one_undo_and_cancel_has_none() {
    let params = Arc::new(PumpParams::new());
    let origin = params.editable_curve_snapshot();
    let mut state = editor_state(Arc::clone(&params));
    let start = paint_sample(0.35, 0.2);
    let outside = boundary_paint_sample(0.0, 0.75);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: start }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaintOutside { sample: outside }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
    );
    assert_ne!(params.editable_curve_snapshot(), origin);
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.redo_history.is_empty());

    let params = Arc::new(PumpParams::new());
    let origin = params.editable_curve_snapshot();
    let mut state = editor_state(Arc::clone(&params));
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: start }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaintOutside { sample: outside }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::Cancel),
    );
    assert_eq!(params.editable_curve_snapshot(), origin);
    assert!(state.undo_history.is_empty());
    assert!(state.redo_history.is_empty());
    assert!(state.active_curve_paint.is_none());
}

#[test]
fn radiant_editor_curve_paint_commits_one_localized_gesture() {
    let params = Arc::new(PumpParams::new());
    let origin = EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 1.0 },
            CurveNode { x: 0.25, y: 0.7 },
            CurveNode { x: 0.5, y: 0.3 },
            CurveNode { x: 0.75, y: 0.7 },
            CurveNode { x: 1.0, y: 1.0 },
        ],
        segments: vec![CurveSegment { tension: 0.0 }; 4],
        ..EditableCurve::default()
    }
    .normalized();
    params.set_editable_curve(&origin);
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint {
            sample: paint_sample(0.3, 0.15),
        }),
    );
    assert!(state.active_curve_paint.is_some());
    assert_eq!(params.editable_curve_snapshot(), origin);
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaint {
            sample: paint_sample(0.45, 0.85),
        }),
    );
    assert_eq!(params.editable_curve_snapshot(), origin);
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint {
            sample: Some(paint_sample(0.6, 0.2)),
        }),
    );

    let painted = params.editable_curve_snapshot();
    assert!(state.active_curve_paint.is_none());
    assert_eq!(state.undo_history.len(), 1);
    assert!(painted
        .nodes
        .iter()
        .any(|node| (node.x - 0.25).abs() < 1.0e-6));
    assert!(painted
        .nodes
        .iter()
        .any(|node| (node.x - 0.75).abs() < 1.0e-6));
    assert!(!painted
        .nodes
        .iter()
        .any(|node| (node.x - 0.5).abs() < 1.0e-6));
    assert!(painted
        .nodes
        .iter()
        .any(|node| (node.x - 0.3).abs() < 1.0e-6));
    assert!(painted
        .nodes
        .iter()
        .any(|node| (node.x - 0.6).abs() < 1.0e-6));
}

#[test]
fn radiant_editor_curve_paint_commits_prior_samples_when_release_is_outside() {
    let params = Arc::new(PumpParams::new());
    let origin = params.editable_curve_snapshot();
    let mut state = editor_state(Arc::clone(&params));
    let first = paint_sample(0.3, 0.15);
    let second = paint_sample(0.55, 0.8);
    let outside = boundary_paint_sample(1.0, 0.45);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: first }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaint { sample: second }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaintOutside { sample: outside }),
    );

    assert_ne!(params.editable_curve_snapshot(), origin);
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.redo_history.is_empty());
    assert!(state.active_curve_paint.is_none());
}

#[test]
fn radiant_editor_curve_paint_cancellation_and_noop_keep_history_unchanged() {
    let params = Arc::new(PumpParams::new());
    let origin = params.editable_curve_snapshot();
    let mut state = editor_state(Arc::clone(&params));
    let sample = paint_sample(0.4, 0.2);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint { sample }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
    );
    assert_eq!(params.editable_curve_snapshot(), origin);
    assert!(state.undo_history.is_empty());
    assert!(state.redo_history.is_empty());

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint { sample }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint {
            sample: Some(sample),
        }),
    );
    assert_eq!(params.editable_curve_snapshot(), origin);
    assert!(state.undo_history.is_empty());
    assert!(state.redo_history.is_empty());
}

#[test]
fn radiant_editor_curve_paint_noop_and_capacity_use_expected_history() {
    let params = Arc::new(PumpParams::new());
    let origin = EditableCurve {
        nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
        segments: vec![CurveSegment { tension: 0.0 }],
        ..EditableCurve::default()
    }
    .normalized();
    params.set_editable_curve(&origin);
    let mut state = editor_state(Arc::clone(&params));
    let seed_start = paint_sample(0.2, 0.2);
    let seed_end = paint_sample(0.8, 0.8);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: seed_start }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaint { sample: seed_end }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
    );
    reduce_editor_message(&mut state, EditorMessage::Undo);
    assert_eq!(params.editable_curve_snapshot(), origin);
    assert!(state.undo_history.is_empty());
    assert_eq!(state.redo_history.len(), 1);

    let no_op = paint_sample(0.4, 0.2);
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: no_op }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint {
            sample: Some(no_op),
        }),
    );
    assert_eq!(params.editable_curve_snapshot(), origin);
    assert!(state.undo_history.is_empty());
    assert_eq!(state.redo_history.len(), 1);

    let first = paint_sample(0.1, 0.1);
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: first }),
    );
    for index in 1..70 {
        let x = 0.1 + index as f32 * 0.8 / 69.0;
        let y = if index % 2 == 0 { 0.1 } else { 0.9 };
        reduce_editor_message(
            &mut state,
            EditorMessage::Curve(CurvePreviewMessage::DragPaint {
                sample: paint_sample(x, y),
            }),
        );
    }
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
    );
    assert_ne!(params.editable_curve_snapshot(), origin);
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.redo_history.is_empty());
    assert!(state.active_curve_paint.is_none());
}

#[test]
fn curve_paint_keeps_origin_until_release_and_commits_candidate_once() {
    let params = Arc::new(PumpParams::new());
    let origin = EditableCurve {
        nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
        segments: vec![CurveSegment { tension: 0.0 }],
        ..EditableCurve::default()
    }
    .normalized();
    params.set_editable_curve(&origin);
    let mut state = editor_state(Arc::clone(&params));
    let first = paint_sample(0.2, 0.2);
    let second = paint_sample(0.8, 0.8);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: first }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragPaint { sample: second }),
    );

    let paint = state
        .active_curve_paint
        .as_ref()
        .expect("paint remains active before release");
    let preview = paint.preview_candidate();
    let release = paint.finished_curve();
    assert!(matches!(&release, PaintCommitOutcome::Applied { .. }));
    assert_eq!(preview, *release.candidate());
    assert_ne!(preview, origin);
    assert_eq!(params.editable_curve_snapshot(), origin);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
    );
    assert_eq!(params.editable_curve_snapshot(), preview);
    assert_eq!(state.undo_history.len(), 1);

    reduce_editor_message(&mut state, EditorMessage::Undo);
    assert_eq!(params.editable_curve_snapshot(), origin);
    assert!(state.undo_history.is_empty());
    assert_eq!(state.redo_history.len(), 1);
}

#[test]
fn radiant_editor_cmd_shift_offset_moves_only_phase_and_commits_on_release() {
    let params = Arc::new(PumpParams::new());
    let origin = params.editable_curve_snapshot();
    let origin_table = params.curve_snapshot();
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressCurveOffset {
            pointer_x: 0.4,
            quantized: false,
        }),
    );
    assert_eq!(state.undo_history.len(), 1);
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragCurveOffset { delta: 0.25 }),
    );
    assert!(state.active_curve_offset.is_some());
    assert_eq!(params.editable_curve_snapshot(), origin);
    assert_eq!(params.curve_snapshot(), origin_table);
    assert!((params.phase_offset() - 0.25).abs() < 1.0e-6);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleaseCurveOffset {
            delta: 0.25,
            option_held: false,
        }),
    );
    assert!(state.active_curve_offset.is_none());
    assert_eq!(params.editable_curve_snapshot(), origin);
    assert!((params.phase_offset() - 0.25).abs() < 1.0e-6);
    assert_eq!(state.undo_history.len(), 1);
}

#[test]
fn radiant_editor_cmd_shift_offset_cancel_restores_auditioned_origin() {
    let params = Arc::new(PumpParams::new());
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressCurveOffset {
            pointer_x: 0.4,
            quantized: false,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragCurveOffset { delta: 0.25 }),
    );
    assert!((params.phase_offset() - 0.25).abs() < 1.0e-6);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::Cancel),
    );

    assert!(params.phase_offset().abs() < f32::EPSILON);
    assert!(state.active_curve_offset.is_none());
    assert!(state.preview_curve_offset.is_none());
}

#[test]
fn radiant_editor_cmd_shift_offset_snaps_immediately_when_option_is_pressed() {
    let params = Arc::new(PumpParams::new());
    let mut state = editor_state(Arc::clone(&params));
    let raw_delta = 0.17;
    let width = (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressCurveOffset {
            pointer_x: 0.4,
            quantized: false,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragCurveOffset { delta: raw_delta }),
    );
    assert!((params.phase_offset() - raw_delta).abs() < 1.0e-6);

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
            option_held: true,
            command_held: true,
            shift_held: true,
        }),
    );
    let snapped = resolve_curve_offset(
        state.params.sync_division(),
        width,
        state.params.swing(),
        0.0,
        raw_delta,
        true,
    );
    assert!(state
        .active_curve_offset
        .as_ref()
        .is_some_and(|drag| drag.quantized));
    assert!((params.phase_offset() - snapped).abs() < 1.0e-6);
}

#[test]
fn radiant_editor_cmd_shift_offset_reverses_to_free_mode_immediately() {
    let params = Arc::new(PumpParams::new());
    let mut state = editor_state(Arc::clone(&params));
    let raw_delta = 0.17;

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressCurveOffset {
            pointer_x: 0.4,
            quantized: true,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragCurveOffset { delta: raw_delta }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
            option_held: false,
            command_held: true,
            shift_held: true,
        }),
    );
    assert!(state
        .active_curve_offset
        .as_ref()
        .is_some_and(|drag| !drag.quantized));
    assert!((params.phase_offset() - raw_delta).abs() < 1.0e-6);
}

#[test]
fn radiant_editor_cmd_shift_offset_uses_option_state_at_release_without_new_move() {
    let params = Arc::new(PumpParams::new());
    let mut state = editor_state(Arc::clone(&params));
    let raw_delta = 0.17;
    let width = (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0);
    let snapped = resolve_curve_offset(
        state.params.sync_division(),
        width,
        state.params.swing(),
        0.0,
        raw_delta,
        true,
    );

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressCurveOffset {
            pointer_x: 0.4,
            quantized: false,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ReleaseCurveOffset {
            delta: raw_delta,
            option_held: true,
        }),
    );

    assert!((params.phase_offset() - snapped).abs() < 1.0e-6);
    assert!(state.active_curve_offset.is_none());
    assert!(state.preview_curve_offset.is_none());
}

#[test]
fn radiant_editor_option_offset_snap_uses_the_absolute_grid_position() {
    let params = Arc::new(PumpParams::new());
    params.set_phase_offset(0.18);
    let mut state = editor_state(Arc::clone(&params));
    let width = (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0);
    let delta = 0.12;
    let expected = resolve_curve_offset(
        state.params.sync_division(),
        width,
        state.params.swing(),
        0.18,
        delta,
        true,
    );

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::PressCurveOffset {
            pointer_x: 0.4,
            quantized: true,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::DragCurveOffset { delta }),
    );

    assert!((params.phase_offset() - expected).abs() < 1.0e-6);
}

#[test]
fn offset_handle_double_click_resets_the_automatable_phase_offset() {
    let params = Arc::new(PumpParams::new());
    params.set_phase_offset(0.43);
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::Curve(CurvePreviewMessage::ResetCurveOffset),
    );

    assert!(params.phase_offset().abs() < f32::EPSILON);
    assert!(state.active_curve_offset.is_none());
}

#[test]
fn hotkey_help_message_toggles_overlay_state() {
    let params = Arc::new(PumpParams::new());
    let mut state = editor_state(params);
    assert!(!state.hotkey_help_open);

    reduce_editor_message(&mut state, EditorMessage::ToggleHotkeyHelp);
    assert!(state.hotkey_help_open);
    reduce_editor_message(&mut state, EditorMessage::ToggleHotkeyHelp);
    assert!(!state.hotkey_help_open);
}

#[test]
fn timing_toggle_switches_modes_and_free_unit_selection_is_sound_neutral() {
    let params = Arc::new(PumpParams::new());
    params.set_free_rate_hz(8.0);
    let mut state = editor_state(params);

    reduce_editor_message(&mut state, EditorMessage::ToggleTimingMode);
    assert_eq!(state.params.timing_mode(), TIMING_MODE_FREE);
    assert_eq!(state.params.sync_division(), 4);
    assert_eq!(state.free_rate_unit, FreeRateUnit::Hertz);

    state.timing_dropdown_open = true;
    reduce_editor_message(
        &mut state,
        EditorMessage::FreeRateUnit(FreeRateUnit::Milliseconds),
    );
    assert_eq!(state.free_rate_unit, FreeRateUnit::Milliseconds);
    assert!(!state.timing_dropdown_open);
    assert!((state.params.free_rate_hz() - 8.0).abs() < f32::EPSILON);
}

#[test]
fn curve_slot_reducer_loads_and_command_stores_curves() {
    let path = std::env::temp_dir().join(format!(
        "pump-model-curve-slots-{}-{}.bin",
        std::process::id(),
        std::thread::current()
            .name()
            .unwrap_or("test")
            .replace(':', "_")
    ));
    let _ = std::fs::remove_file(&path);

    with_test_curve_slot_path(path.clone(), || {
        let params = Arc::new(PumpParams::new());
        let stored = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.2 },
                CurveNode { x: 0.3, y: 0.8 },
                CurveNode { x: 0.7, y: 0.1 },
                CurveNode { x: 1.0, y: 0.2 },
            ],
            segments: vec![CurveSegment { tension: 0.17 }; 3],
            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&stored);
        let mut state = editor_state(Arc::clone(&params));

        state.dispatch(EditorMessage::CurveSlot(CurveSlotMessage::Store {
            index: 3,
        }));
        assert_eq!(state.loaded_slot(), Some(3));
        assert_eq!(params.global_curve_slot_curve(3), Some(stored.clone()));

        let changed = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.9 }, CurveNode { x: 1.0, y: 0.9 }],
            segments: vec![CurveSegment { tension: 0.0 }],
            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&changed);
        state.dispatch(EditorMessage::CurveSlot(CurveSlotMessage::Load {
            index: 3,
        }));
        assert_eq!(state.loaded_slot(), Some(3));
        assert_eq!(params.editable_curve_snapshot(), stored);
    });

    let _ = std::fs::remove_file(path);
}

#[test]
fn host_sound_switch_clears_curve_selection_before_projecting_new_curve() {
    let params = Arc::new(PumpParams::new());
    params.copy_active_to_inactive();

    let curve_b = EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.1 },
            CurveNode { x: 0.2, y: 0.8 },
            CurveNode { x: 0.45, y: 0.3 },
            CurveNode { x: 0.7, y: 0.9 },
            CurveNode { x: 1.0, y: 0.2 },
        ],
        segments: vec![CurveSegment { tension: 0.0 }; 4],
        ..EditableCurve::default()
    }
    .normalized();
    params.set_active_sound(SoundSide::B);
    params.set_editable_curve(&curve_b);
    params.set_active_sound(SoundSide::A);

    let mut state = editor_state(Arc::clone(&params));
    state.selected_curve_nodes = vec![1];
    state.active_curve_marquee = Some(ActiveCurveMarquee {
        start: CurveNode { x: 0.1, y: 0.9 },
        current: CurveNode { x: 0.8, y: 0.1 },
    });

    reduce_editor_message(
        &mut state,
        EditorMessage::SelectSound {
            side: SoundSide::B,
            copy: false,
        },
    );

    assert!(state.selected_curve_nodes.is_empty());
    assert!(state.active_curve_marquee.is_none());
    assert_eq!(params.active_sound(), SoundSide::B);
    assert_eq!(params.editable_curve_snapshot(), curve_b);
}

#[test]
fn clap_sink_enqueues_each_continuous_event_as_it_arrives() {
    let queue = Arc::new(PumpAutomationQueue::default());
    let sink = ClapHostParamEditSink {
        queue: Arc::clone(&queue),
        requester: None,
    };
    let config = AutomationConfig::default();

    assert!(sink.gesture_started(&config, PARAM_FREE_RATE_ID));
    let mut buffer = EventBuffer::new();
    let mut output = buffer.as_output();
    let mut scratch = Vec::new();
    assert_eq!(
        queue.drain_to_output(&mut output, &mut scratch).attempted,
        1
    );

    assert!(sink.gesture_value(&config, PARAM_FREE_RATE_ID, 0.5));
    let mut buffer = EventBuffer::new();
    let mut output = buffer.as_output();
    let mut scratch = Vec::new();
    assert_eq!(
        queue.drain_to_output(&mut output, &mut scratch).attempted,
        1
    );

    assert!(sink.gesture_ended(&config, PARAM_FREE_RATE_ID));
    let mut buffer = EventBuffer::new();
    let mut output = buffer.as_output();
    let mut scratch = Vec::new();
    assert_eq!(
        queue.drain_to_output(&mut output, &mut scratch).attempted,
        1
    );
}

#[test]
fn clap_sink_preserves_lifecycle_order_for_each_deck_control() {
    let config = AutomationConfig::default();
    for param_id in [
        PARAM_SMOOTH_ID,
        PARAM_MIX_ID,
        PARAM_OUTPUT_GAIN_ID,
        PARAM_FREE_RATE_ID,
    ] {
        let queue = Arc::new(PumpAutomationQueue::default());
        let sink = ClapHostParamEditSink {
            queue: Arc::clone(&queue),
            requester: None,
        };
        assert!(sink.gesture_started(&config, param_id));
        assert!(sink.gesture_value(&config, param_id, 0.25));
        assert!(sink.gesture_ended(&config, param_id));

        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        assert_eq!(
            queue.drain_to_output(&mut output, &mut scratch).attempted,
            3,
            "each deck control should emit one complete lifecycle"
        );
        let kinds: Vec<_> = (0..buffer.len())
            .map(|index| {
                let event = buffer
                    .get(index as u32)
                    .expect("lifecycle event should be present");
                match event.header().type_id() {
                    ParamGestureBeginEvent::TYPE_ID => {
                        assert_eq!(
                            event
                                .as_event::<ParamGestureBeginEvent>()
                                .expect("gesture begin should decode")
                                .param_id(),
                            Some(param_id)
                        );
                        "begin"
                    }
                    ParamValueEvent::TYPE_ID => {
                        let CoreEventSpace::ParamValue(event) = event
                            .as_core_event()
                            .expect("value should decode as a core event")
                        else {
                            unreachable!()
                        };
                        assert_eq!(event.param_id(), Some(param_id));
                        "value"
                    }
                    ParamGestureEndEvent::TYPE_ID => {
                        assert_eq!(
                            event
                                .as_event::<ParamGestureEndEvent>()
                                .expect("gesture end should decode")
                                .param_id(),
                            Some(param_id)
                        );
                        "end"
                    }
                    _ => "other",
                }
            })
            .collect();
        assert_eq!(kinds, ["begin", "value", "end"]);
    }
}

#[test]
fn radiant_editor_command_release_uses_swung_snap() {
    let params = Arc::new(PumpParams::new());
    params.set_sync_division(6.0);
    params.set_swing(1.0);
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

    let raw = CurveNode { x: 0.34, y: 0.7 };
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::ReleaseNode {
            index: 1,
            node: raw,
            push_through_threshold_x: test_curve_push_through_threshold_x(),
            shift_held: false,
            option_held: false,
            command_held: true,
        },
    );

    let width = curve_width_from_push_through_threshold_x(test_curve_push_through_threshold_x());
    let released = params.editable_curve_snapshot().nodes[1];
    assert!(
        (released.x - snap_curve_time_to_beat_grid_with_swing(6, width, raw.x, 1.0)).abs() < 1.0e-6,
        "released {}, expected {} width {}",
        released.x,
        snap_curve_time_to_beat_grid_with_swing(6, width, raw.x, 1.0),
        width
    );
}

#[test]
fn radiant_editor_command_press_and_release_update_point_snap_mid_drag() {
    let params = Arc::new(PumpParams::new());
    params.set_sync_division(6.0);
    params.set_swing(1.0);
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
    reduce_curve_message(&mut state, unconstrained_press(1));

    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::ModifiersChanged {
            option_held: false,
            command_held: true,
            shift_held: false,
        },
    );
    let raw = CurveNode { x: 0.34, y: 0.7 };
    reduce_curve_message(
        &mut state,
        CurvePreviewMessage::DragNode {
            index: 1,
            node: raw,
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        },
    );
    let width = curve_width_from_push_through_threshold_x(test_curve_push_through_threshold_x());
    let snapped = params.editable_curve_snapshot().nodes[1];
    assert!(
        (snapped.x - snap_curve_time_to_beat_grid_with_swing(6, width, raw.x, 1.0)).abs() < 1.0e-6
    );

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
            node: raw,
            push_through_threshold_x: test_curve_push_through_threshold_x(),
        },
    );
    let continuous = params.editable_curve_snapshot().nodes[1];
    assert!((continuous.x - raw.x).abs() < 1.0e-2);
    assert!((continuous.y - snapped.y).abs() < 1.0e-2);
}

#[test]
fn curve_preview_widget_push_through_threshold_tracks_actual_bounds() {
    let mut thresholds = Vec::new();
    for preview_width in [220.0, 396.0] {
        let curve_width = curve_viewport_width(preview_width);
        let threshold = curve_node_push_through_threshold_x(preview_width);
        assert!(
            (threshold * (curve_width - 1.0) - CURVE_NODE_PUSH_THROUGH_MARGIN_PX).abs() < 1.0e-5
        );
        thresholds.push(threshold);
    }
    assert!(thresholds[0] > thresholds[1]);
}

#[test]
fn radiant_editor_reduces_slider_messages_to_params() {
    let params = Arc::new(PumpParams::new());
    let queue = Arc::new(PumpAutomationQueue::default());
    let mut state = PumpEditorState::new(
        Arc::clone(&params),
        Arc::new(GuiStatus::default()),
        Arc::new(ClapHostParamEditSink {
            queue: Arc::clone(&queue),
            requester: None,
        }),
    );

    for (target, value) in [
        (NumericEntryTarget::Mix, 0.25),
        (NumericEntryTarget::OutputGain, 0.5),
        (NumericEntryTarget::Smooth, 0.5),
        (NumericEntryTarget::Swing, 0.5),
        (NumericEntryTarget::FreeRate, 0.5),
    ] {
        reduce_editor_message(
            &mut state,
            EditorMessage::Knob {
                target,
                message: KnobMessage::Reset { value },
            },
        );
    }
    reduce_editor_message(&mut state, EditorMessage::SyncDivision(1.0));

    assert!((params.mix() - 0.25).abs() < f32::EPSILON);
    assert!((params.free_rate_hz() - 31.622_776).abs() < 1.0e-3);
    assert!((params.output_gain_db() + 6.0).abs() < f32::EPSILON);
    assert_eq!(params.sync_division(), MAX_SYNC_DIVISION as usize);

    let mut buffer = EventBuffer::new();
    let mut output = buffer.as_output();
    let mut scratch = Vec::new();
    let stats = queue.drain_to_output(&mut output, &mut scratch);
    assert_eq!(stats.attempted, 18);
    let value_ids: Vec<_> = (0..buffer.len())
        .filter_map(|index| match buffer.get(index as u32)?.as_core_event()? {
            CoreEventSpace::ParamValue(value) => value.param_id(),
            _ => None,
        })
        .collect();
    assert_eq!(
        value_ids,
        vec![
            PARAM_MIX_ID,
            PARAM_OUTPUT_GAIN_ID,
            PARAM_SMOOTH_ID,
            PARAM_SWING_ID,
            PARAM_FREE_RATE_ID,
            PARAM_SYNC_DIVISION_ID,
        ]
    );
}

#[test]
fn wheel_gesture_updates_mapped_param_with_one_ordered_transaction() {
    let params = Arc::new(PumpParams::new());
    let queue = Arc::new(PumpAutomationQueue::default());
    let mut state = editor_state_with_queue(Arc::clone(&params), Arc::clone(&queue));
    let final_value = 0.75;
    let expected_plain = knob_plain_value(NumericEntryTarget::FreeRate, final_value).1;

    reduce_editor_message(
        &mut state,
        EditorMessage::Knob {
            target: NumericEntryTarget::FreeRate,
            message: KnobMessage::Discrete { value: final_value },
        },
    );

    assert!((params.free_rate_hz() - expected_plain).abs() < 1.0e-3);
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.active_knob_gesture.is_none());

    let mut buffer = EventBuffer::new();
    let mut output = buffer.as_output();
    let mut scratch = Vec::new();
    assert_eq!(
        queue.drain_to_output(&mut output, &mut scratch).attempted,
        3
    );
    let kinds: Vec<_> = (0..buffer.len())
        .map(|index| {
            let event = buffer
                .get(index as u32)
                .expect("wheel lifecycle event should be present");
            match event.header().type_id() {
                ParamGestureBeginEvent::TYPE_ID => {
                    assert_eq!(
                        event
                            .as_event::<ParamGestureBeginEvent>()
                            .expect("wheel begin should decode")
                            .param_id(),
                        Some(PARAM_FREE_RATE_ID)
                    );
                    "begin"
                }
                ParamValueEvent::TYPE_ID => {
                    let CoreEventSpace::ParamValue(value) = event
                        .as_core_event()
                        .expect("wheel value should decode as a core event")
                    else {
                        unreachable!()
                    };
                    assert_eq!(value.param_id(), Some(PARAM_FREE_RATE_ID));
                    let expected_normalized =
                        normalized_from_plain_value(PARAM_FREE_RATE_ID, expected_plain as f64)
                            .expect("free-rate plain value should normalize");
                    assert!((value.value() - expected_normalized).abs() < f64::EPSILON);
                    "value"
                }
                ParamGestureEndEvent::TYPE_ID => {
                    assert_eq!(
                        event
                            .as_event::<ParamGestureEndEvent>()
                            .expect("wheel end should decode")
                            .param_id(),
                        Some(PARAM_FREE_RATE_ID)
                    );
                    "end"
                }
                _ => "other",
            }
        })
        .collect();
    assert_eq!(kinds, ["begin", "value", "end"]);
}

#[test]
fn wheel_gesture_ends_after_rejected_value_without_mutating_state() {
    let params = Arc::new(PumpParams::new());
    let initial_rate = params.free_rate_hz();
    let queue = Arc::new(PumpAutomationQueue::with_config(
        AutomationQueueConfig::new(2, AutomationDropPolicy::DropNewest),
    ));
    let mut state = editor_state_with_queue(Arc::clone(&params), Arc::clone(&queue));

    reduce_editor_message(
        &mut state,
        EditorMessage::Knob {
            target: NumericEntryTarget::FreeRate,
            message: KnobMessage::Discrete { value: 0.75 },
        },
    );

    assert!((params.free_rate_hz() - initial_rate).abs() < f32::EPSILON);
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.active_knob_gesture.is_none());
    let mut buffer = EventBuffer::new();
    let mut output = buffer.as_output();
    let mut scratch = Vec::new();
    assert_eq!(
        queue.drain_to_output(&mut output, &mut scratch).attempted,
        2
    );
    let kinds: Vec<_> = (0..buffer.len())
        .map(|index| {
            let event = buffer
                .get(index as u32)
                .expect("rejected wheel lifecycle event should be present");
            match event.header().type_id() {
                ParamGestureBeginEvent::TYPE_ID => "begin",
                ParamGestureEndEvent::TYPE_ID => "end",
                _ => "other",
            }
        })
        .collect();
    assert_eq!(kinds, ["begin", "end"]);
}

#[test]
fn bypass_reducer_emits_complete_clap_gesture_and_stays_out_of_undo_history() {
    let params = Arc::new(PumpParams::new());
    let queue = Arc::new(PumpAutomationQueue::default());
    let mut state = PumpEditorState::new(
        Arc::clone(&params),
        Arc::new(GuiStatus::default()),
        Arc::new(ClapHostParamEditSink {
            queue: Arc::clone(&queue),
            requester: None,
        }),
    );

    reduce_editor_message(&mut state, EditorMessage::ToggleBypass);
    assert!(params.bypassed());
    assert!(state.undo_history.is_empty());
    reduce_editor_message(&mut state, EditorMessage::Undo);
    assert!(params.bypassed(), "undo must never alter host bypass");

    let mut buffer = EventBuffer::new();
    let mut output = buffer.as_output();
    let mut scratch = Vec::new();
    let stats = queue.drain_to_output(&mut output, &mut scratch);
    assert_eq!(stats.attempted, 3);
    let value =
        (0..buffer.len()).find_map(|index| match buffer.get(index as u32)?.as_core_event()? {
            CoreEventSpace::ParamValue(value) => Some((value.param_id(), value.value())),
            _ => None,
        });
    assert_eq!(value, Some((Some(PARAM_BYPASS_ID), 1.0)));
}

#[test]
fn bypass_reducer_keeps_state_active_when_clap_gesture_cannot_fit() {
    let params = Arc::new(PumpParams::new());
    let queue = Arc::new(PumpAutomationQueue::with_config(
        AutomationQueueConfig::new(3, AutomationDropPolicy::DropNewest),
    ));
    let config = AutomationConfig::default();
    assert_eq!(
        queue.push_value(&config, PARAM_MIX_ID, 0.25),
        toybox::clap::automation::AutomationEnqueueStatus::Enqueued
    );
    let mut state = PumpEditorState::new(
        Arc::clone(&params),
        Arc::new(GuiStatus::default()),
        Arc::new(ClapHostParamEditSink {
            queue: Arc::clone(&queue),
            requester: None,
        }),
    );

    reduce_editor_message(&mut state, EditorMessage::ToggleBypass);

    assert!(!params.bypassed());
    assert_eq!(state.automation_flush_count, 0);
    let mut buffer = EventBuffer::new();
    let mut output = buffer.as_output();
    let mut scratch = Vec::with_capacity(3);
    let stats = queue.drain_to_output(&mut output, &mut scratch);
    assert_eq!(stats.attempted, 1);
    assert_eq!(buffer.len(), 1);
}

#[test]
fn swing_knob_transaction_updates_param_and_emits_complete_gesture() {
    let params = Arc::new(PumpParams::new());
    let queue = Arc::new(PumpAutomationQueue::default());
    let mut state = PumpEditorState::new(
        Arc::clone(&params),
        Arc::new(GuiStatus::default()),
        Arc::new(ClapHostParamEditSink {
            queue: Arc::clone(&queue),
            requester: None,
        }),
    );

    reduce_editor_message(
        &mut state,
        EditorMessage::Knob {
            target: NumericEntryTarget::Swing,
            message: KnobMessage::GestureStarted,
        },
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Knob {
            target: NumericEntryTarget::Swing,
            message: KnobMessage::ValueChanged { value: 0.5 },
        },
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::Knob {
            target: NumericEntryTarget::Swing,
            message: KnobMessage::GestureEnded,
        },
    );

    assert!((params.swing() - 0.5).abs() < f32::EPSILON);
    assert_eq!(state.undo_history.len(), 1);
    assert!(state.active_knob_gesture.is_none());

    let mut buffer = EventBuffer::new();
    let mut output = buffer.as_output();
    let mut scratch = Vec::new();
    assert_eq!(
        queue.drain_to_output(&mut output, &mut scratch).attempted,
        3
    );
    for index in 0..buffer.len() {
        let event = buffer
            .get(index as u32)
            .expect("queued swing gesture event should be readable");
        if let Some(CoreEventSpace::ParamValue(event)) = event.as_core_event() {
            assert_eq!(event.param_id(), Some(PARAM_SWING_ID));
            assert!((event.value() - 0.5).abs() < f64::EPSILON);
        }
    }
}

#[test]
fn radiant_numeric_entry_commit_valid_value_updates_param() {
    let params = Arc::new(PumpParams::new());
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::NumericEntry(NumericEntryMessage::Begin {
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
        EditorMessage::NumericEntry(NumericEntryMessage::DraftChanged {
            target: NumericEntryTarget::Mix,
            draft: "25".to_string(),
            dirty: true,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::NumericEntry(NumericEntryMessage::Commit {
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
        EditorMessage::NumericEntry(NumericEntryMessage::Begin {
            target: NumericEntryTarget::OutputGain,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::NumericEntry(NumericEntryMessage::Commit {
            target: NumericEntryTarget::OutputGain,
            draft: "not a number".to_string(),
        }),
    );

    assert!((params.output_gain_db() + 3.0).abs() < f32::EPSILON);
    assert!(state.numeric_entry.is_none());
}

#[test]
fn radiant_numeric_entry_cancel_leaves_prior_value() {
    let params = Arc::new(PumpParams::new());
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::NumericEntry(NumericEntryMessage::Begin {
            target: NumericEntryTarget::FreeRate,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::NumericEntry(NumericEntryMessage::DraftChanged {
            target: NumericEntryTarget::FreeRate,
            draft: "75".to_string(),
            dirty: true,
        }),
    );
    reduce_editor_message(
        &mut state,
        EditorMessage::NumericEntry(NumericEntryMessage::Cancel {
            target: NumericEntryTarget::FreeRate,
        }),
    );

    assert!((params.phase_offset() - 0.0).abs() < f32::EPSILON);
    assert!(state.numeric_entry.is_none());
}

#[test]
fn numeric_value_label_emits_delay_arrow_steps_with_shift_multiplier() {
    let params = Arc::new(PumpParams::new());
    params.set_delay_beats(2.0);
    let mut state = editor_state(Arc::clone(&params));

    reduce_editor_message(
        &mut state,
        EditorMessage::NumericEntry(NumericEntryMessage::Begin {
            target: NumericEntryTarget::Delay,
        }),
    );
    for (delta, expected) in [(1, 3), (-1, 2), (4, 6), (-4, 2)] {
        reduce_editor_message(
            &mut state,
            EditorMessage::NumericEntry(NumericEntryMessage::Step {
                target: NumericEntryTarget::Delay,
                delta,
            }),
        );
        assert_eq!(params.delay_beats(), expected);
    }

    reduce_editor_message(
        &mut state,
        EditorMessage::NumericEntry(NumericEntryMessage::Step {
            target: NumericEntryTarget::Delay,
            delta: -4,
        }),
    );
    assert_eq!(params.delay_beats(), MIN_DELAY_BEATS);
    let history_at_lower_bound = state.undo_history.len();
    let flushes_at_lower_bound = state.automation_flush_count;
    reduce_editor_message(
        &mut state,
        EditorMessage::NumericEntry(NumericEntryMessage::Step {
            target: NumericEntryTarget::Delay,
            delta: -1,
        }),
    );
    assert_eq!(params.delay_beats(), MIN_DELAY_BEATS);
    assert_eq!(state.undo_history.len(), history_at_lower_bound);
    assert_eq!(state.automation_flush_count, flushes_at_lower_bound);

    for _ in 0..8 {
        reduce_editor_message(
            &mut state,
            EditorMessage::NumericEntry(NumericEntryMessage::Step {
                target: NumericEntryTarget::Delay,
                delta: 4,
            }),
        );
    }
    assert_eq!(params.delay_beats(), MAX_DELAY_BEATS);
    let history_at_upper_bound = state.undo_history.len();
    let flushes_at_upper_bound = state.automation_flush_count;
    reduce_editor_message(
        &mut state,
        EditorMessage::NumericEntry(NumericEntryMessage::Step {
            target: NumericEntryTarget::Delay,
            delta: 4,
        }),
    );
    assert_eq!(params.delay_beats(), MAX_DELAY_BEATS);
    assert_eq!(state.undo_history.len(), history_at_upper_bound);
    assert_eq!(state.automation_flush_count, flushes_at_upper_bound);
}

#[test]
fn radiant_delay_numeric_entry_accepts_integer_beats_and_formats_units() {
    let params = Arc::new(PumpParams::new());
    let mut state = editor_state(Arc::clone(&params));

    for (raw, expected) in [
        ("-1", 0),
        ("0", 0),
        ("1 beat", 1),
        ("2 beats", 2),
        ("33 beats", MAX_DELAY_BEATS),
        ("32 beats", MAX_DELAY_BEATS),
    ] {
        reduce_editor_message(
            &mut state,
            EditorMessage::NumericEntry(NumericEntryMessage::Begin {
                target: NumericEntryTarget::Delay,
            }),
        );
        let expected_draft = format_delay_beats_for_ui(params.delay_beats());
        let prior_delay = params.delay_beats();
        let history_before_commit = state.undo_history.len();
        assert_eq!(
            state
                .numeric_entry
                .as_ref()
                .map(|entry| entry.draft.as_str()),
            Some(expected_draft.as_str())
        );
        reduce_editor_message(
            &mut state,
            EditorMessage::NumericEntry(NumericEntryMessage::Commit {
                target: NumericEntryTarget::Delay,
                draft: raw.to_string(),
            }),
        );
        assert_eq!(params.delay_beats(), expected);
        let expected_history_len = if expected == prior_delay {
            history_before_commit
        } else {
            history_before_commit + 1
        };
        assert_eq!(state.undo_history.len(), expected_history_len);
        assert!(state.numeric_entry.is_none());
    }

    assert_eq!(format_delay_beats_for_ui(0), "0 beats");
    assert_eq!(format_delay_beats_for_ui(1), "1 beat");
    assert_eq!(format_delay_beats_for_ui(2), "2 beats");
    assert_eq!(format_delay_beats_for_ui(MAX_DELAY_BEATS), "32 beats");

    reduce_editor_message(
        &mut state,
        EditorMessage::NumericEntry(NumericEntryMessage::Begin {
            target: NumericEntryTarget::Delay,
        }),
    );
    let history_before_invalid_commit = state.undo_history.len();
    reduce_editor_message(
        &mut state,
        EditorMessage::NumericEntry(NumericEntryMessage::Commit {
            target: NumericEntryTarget::Delay,
            draft: "1.5 beats".to_string(),
        }),
    );
    assert_eq!(params.delay_beats(), MAX_DELAY_BEATS);
    assert!(state.numeric_entry.is_none());
    assert_eq!(
        state.undo_history.len(),
        history_before_invalid_commit,
        "invalid Delay commits must not add history"
    );
}
