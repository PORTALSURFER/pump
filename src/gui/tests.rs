    use super::{
        constrained_host_size, find_deletable_node_hit, find_segment_line_hit_within,
        local_from_node, move_node_with_push_through, move_segment_translated,
        preferred_window_size, preview_node_on_curve, resolve_runtime_controls_slot_widths,
        recompute_move_node_from_origin_for_size,
        resolve_vertical_slot_heights, segment_upward_tension_sign,
        tension_delta_from_drag_for_segment, CurveRenderState, GuiState, PumpTheme,
        UiLayoutMetrics, CURVE_H, CURVE_KEY, CURVE_W, DIVISION_KEY, HEADER_EMPTY_SECTION_PERCENT,
        HEADER_INDICATOR_SECTION_PERCENT, PRESET_DROPDOWN_KEY, PRESET_RENAME_BUTTON_KEY,
        PRESET_RENAME_KEY, PRESET_SAVE_KEY, PRESET_WARNING_INIT, RESET_KEY,
        TRANSPORT_INDICATOR_SIZE, WINDOW_HEIGHT, WINDOW_WIDTH,
    };
    use crate::curve::{sample_editable_curve, CurveNode, CurveSegment, EditableCurve};
    use crate::params::{PumpParams, MAX_SYNC_DIVISION};
    use crate::{GuiStatus, GuiTransportTelemetry};
    use std::sync::Arc;
    use toybox::clack_extensions::gui::GuiSize;
    use toybox::clap::automation::AutomationQueue;
    use toybox::clap::gui::InputState;
    use toybox::gui::declarative::{
        measure_checked, ContainerLayout, ContainerLength, DropdownSpec, GridKind, LayoutBox,
        Node, PanelSpec, RootScaleMode, SurfaceCommand, UiAction, UiSpec,
    };
    use toybox::gui::{render_spec_to_frame, Color, MainPalette, Point, Size};

    fn expect_slot_child<'a>(node: &'a Node, label: &str) -> &'a Node {
        match node {
            Node::Slot(slot) => slot.child(),
            other => panic!("expected {label} slot wrapper, got {other:?}"),
        }
    }

    fn expect_slot_panel<'a>(node: &'a Node, label: &str) -> &'a PanelSpec {
        match expect_slot_child(node, label) {
            Node::Row(row) => match row.children() {
                [child] => match expect_slot_child(child, label) {
                    Node::Panel(panel) => panel,
                    other => panic!("expected {label} row to wrap panel, got {other:?}"),
                },
                _ => panic!("expected {label} row to contain exactly one child"),
            },
            Node::Panel(panel) => panel,
            other => panic!("expected {label} panel (or row wrapper), got {other:?}"),
        }
    }

    fn assert_container_layout_host_derived(layout: ContainerLayout) {
        assert!(matches!(
            layout.width,
            ContainerLength::Auto | ContainerLength::Fill(_)
        ));
        assert!(matches!(
            layout.height,
            ContainerLength::Auto | ContainerLength::Fill(_)
        ));
    }

    fn assert_slot_tree_node(node: &Node) {
        match node {
            Node::Slot(slot) => {
                let child = slot.child();
                assert!(
                    !matches!(child, Node::Slot(_)),
                    "slot child must not be another slot"
                );
                assert_slot_tree_node(child);
            }
            Node::Panel(panel) => {
                assert_container_layout_host_derived(panel.container_layout());
                assert!(matches!(panel.content(), Node::Slot(_)));
                assert_slot_tree_node(panel.content());
            }
            Node::PaddingBox(padding_box) => {
                assert_container_layout_host_derived(padding_box.container_layout());
                assert!(matches!(padding_box.content(), Node::Slot(_)));
                assert_slot_tree_node(padding_box.content());
            }
            Node::AlignBox(align_box) => {
                assert_container_layout_host_derived(align_box.container_layout());
                assert!(matches!(align_box.content(), Node::Slot(_)));
                assert_slot_tree_node(align_box.content());
            }
            Node::AspectBox(aspect_box) => {
                assert_container_layout_host_derived(aspect_box.container_layout());
                assert!(matches!(aspect_box.content(), Node::Slot(_)));
                assert_slot_tree_node(aspect_box.content());
            }
            Node::Row(row) => {
                assert_container_layout_host_derived(row.container_layout());
                for child in row.children() {
                    assert!(matches!(child, Node::Slot(_)));
                    assert_slot_tree_node(child);
                }
            }
            Node::Column(column) => {
                assert_container_layout_host_derived(column.container_layout());
                for child in column.children() {
                    assert!(matches!(child, Node::Slot(_)));
                    assert_slot_tree_node(child);
                }
            }
            Node::Grid(grid) => {
                assert_container_layout_host_derived(grid.container_layout());
                for child in grid.children() {
                    assert!(matches!(child, Node::Slot(_)));
                    assert_slot_tree_node(child);
                }
            }
            Node::Absolute(absolute) => {
                assert_container_layout_host_derived(absolute.container_layout());
                for child in absolute.children() {
                    assert!(matches!(child.node(), Node::Slot(_)));
                    assert_slot_tree_node(child.node());
                }
            }
            Node::Stack(stack) => {
                assert_container_layout_host_derived(stack.container_layout());
                for child in stack.children() {
                    assert!(matches!(child, Node::Slot(_)));
                    assert_slot_tree_node(child);
                }
            }
            Node::ScrollView(scroll_view) => {
                assert_container_layout_host_derived(scroll_view.container_layout());
                assert!(matches!(scroll_view.content(), Node::Slot(_)));
                assert_slot_tree_node(scroll_view.content());
            }
            Node::Wrap(wrap) => {
                assert_container_layout_host_derived(wrap.container_layout());
                for child in wrap.children() {
                    assert!(matches!(child, Node::Slot(_)));
                    assert_slot_tree_node(child);
                }
            }
            Node::SwitchLayout(switch_layout) => {
                assert_container_layout_host_derived(switch_layout.container_layout());
                assert!(matches!(switch_layout.fallback(), Node::Slot(_)));
                assert_slot_tree_node(switch_layout.fallback());
                for case_entry in switch_layout.cases() {
                    assert!(matches!(case_entry.child(), Node::Slot(_)));
                    assert_slot_tree_node(case_entry.child());
                }
            }
            Node::TextBox(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::CurveEditor(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::Region(_)
            | Node::Indicator(_) => {}
        }
    }

    fn assert_emitted_slot_tree_invariants(spec: &UiSpec) {
        let root = spec.root.content();
        assert!(matches!(root, Node::Slot(_)));
        let root_child = expect_slot_child(root, "root");
        assert!(
            matches!(
                root_child,
                Node::Panel(_)
                    | Node::PaddingBox(_)
                    | Node::AlignBox(_)
                    | Node::AspectBox(_)
                    | Node::Row(_)
                    | Node::Column(_)
                    | Node::Grid(_)
                    | Node::Absolute(_)
                    | Node::Stack(_)
                    | Node::ScrollView(_)
                    | Node::Wrap(_)
                    | Node::SwitchLayout(_)
            ),
            "root slot child must be a container"
        );
        assert_slot_tree_node(root_child);
    }

    #[test]
    fn delete_hit_ignores_endpoints_and_targets_interior_nodes() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.3, y: 0.4 },
                CurveNode { x: 0.6, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
            ],
        };

        let near_start = local_from_node(curve.nodes[0]);
        assert_eq!(find_deletable_node_hit(&curve, near_start), None);

        let near_middle = local_from_node(curve.nodes[1]);
        assert_eq!(find_deletable_node_hit(&curve, near_middle), Some(1));
    }

    #[test]
    fn delete_hit_returns_none_outside_node_hit_radius() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.5, y: 0.2 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }, CurveSegment { tension: 0.0 }],
        };

        let far_away = Point { x: 0, y: 0 };
        assert_eq!(find_deletable_node_hit(&curve, far_away), None);
    }

    #[test]
    fn segment_line_hit_detects_nearby_curve_segment() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.3, y: 0.2 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }, CurveSegment { tension: 0.0 }],
        };

        let near_segment = local_from_node(CurveNode { x: 0.2, y: 0.45 });
        assert_eq!(
            find_segment_line_hit_within(&curve, near_segment, 24),
            Some(0)
        );
    }

    #[test]
    fn push_through_drag_consumes_crossed_interior_nodes_only() {
        let mut curve = EditableCurve {
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
        };

        let moved_index =
            move_node_with_push_through(&mut curve, 2, CurveNode { x: 0.95, y: 0.4 }, 0);
        assert_eq!(moved_index, 2);
        assert_eq!(curve.nodes.len(), 4);
        assert_eq!(curve.nodes[0].x, 0.0);
        assert_eq!(curve.nodes[curve.nodes.len() - 1].x, 1.0);
        assert!(curve.nodes.iter().all(|node| node.x <= 1.0));
    }

    #[test]
    fn reversible_push_through_restores_crossed_nodes_within_same_drag() {
        let origin = EditableCurve {
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
        };

        let size = Size {
            width: CURVE_W,
            height: CURVE_H,
        };
        let removed = origin.nodes[3];

        let (crossed_curve, _) = recompute_move_node_from_origin_for_size(
            &origin,
            2,
            CurveNode { x: 0.95, y: 0.4 },
            0,
            size,
        );
        assert_eq!(crossed_curve.nodes.len(), origin.nodes.len() - 1);
        assert!(
            !crossed_curve.nodes.contains(&removed),
            "crossing right should remove crossed interior node"
        );

        let (restored_curve, _) = recompute_move_node_from_origin_for_size(
            &origin,
            2,
            CurveNode { x: 0.55, y: 0.4 },
            0,
            size,
        );
        assert_eq!(restored_curve.nodes.len(), origin.nodes.len());
        assert_eq!(restored_curve.nodes[3], removed);
        assert_eq!(restored_curve.segments, origin.segments);
    }

    #[test]
    fn push_through_threshold_requires_boundary_crossing() {
        let origin = EditableCurve {
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
        };

        let size = Size {
            width: CURVE_W,
            height: CURVE_H,
        };
        let threshold_px = 10;
        let threshold_x = threshold_px as f32 / (size.width.max(2) - 1) as f32;
        let boundary = origin.nodes[3].x;

        let (not_crossed, _) = recompute_move_node_from_origin_for_size(
            &origin,
            2,
            CurveNode {
                x: boundary + threshold_x - 1.0e-3,
                y: 0.4,
            },
            threshold_px,
            size,
        );
        assert_eq!(not_crossed.nodes.len(), origin.nodes.len());

        let (crossed, _) = recompute_move_node_from_origin_for_size(
            &origin,
            2,
            CurveNode {
                x: boundary + threshold_x + 1.0e-3,
                y: 0.4,
            },
            threshold_px,
            size,
        );
        assert_eq!(crossed.nodes.len(), origin.nodes.len() - 1);
    }

    #[test]
    fn wrapped_endpoints_move_together() {
        let mut curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.5, y: 0.25 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }, CurveSegment { tension: 0.0 }],
        };

        move_node_with_push_through(&mut curve, 0, CurveNode { x: 0.0, y: 0.31 }, 10);
        let last_index = curve.nodes.len() - 1;
        assert!((curve.nodes[0].y - curve.nodes[last_index].y).abs() <= f32::EPSILON);
    }

    #[test]
    fn preview_node_snaps_to_curve_value_at_pointer_x() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.5, y: 0.0 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }, CurveSegment { tension: 0.0 }],
        };
        let pointer = local_from_node(CurveNode { x: 0.5, y: 0.9 });
        let preview = preview_node_on_curve(&curve, pointer).expect("preview exists");
        let expected = sample_editable_curve(&curve, preview.x);
        assert!((preview.y - expected).abs() < 1.0e-6);
    }

    #[test]
    fn segment_translation_moves_interior_segment_horizontally() {
        let mut curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.3, y: 0.5 },
                CurveNode { x: 0.6, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
            ],
        };
        move_segment_translated(&mut curve, 1, (0.3, 0.5), (0.6, 0.5), (0.1, 0.1));
        assert!((curve.nodes[1].x - 0.4).abs() < 1.0e-6);
        assert!((curve.nodes[2].x - 0.7).abs() < 1.0e-6);
        assert!((curve.nodes[1].y - 0.6).abs() < 1.0e-6);
        assert!((curve.nodes[2].y - 0.6).abs() < 1.0e-6);
    }

    #[test]
    fn upward_bend_sign_tracks_segment_direction() {
        let rising = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.2 }, CurveNode { x: 1.0, y: 0.8 }],
            segments: vec![CurveSegment { tension: 0.0 }],
        };
        let falling = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.8 }, CurveNode { x: 1.0, y: 0.2 }],
            segments: vec![CurveSegment { tension: 0.0 }],
        };

        assert_eq!(segment_upward_tension_sign(&rising, 0), -1.0);
        assert_eq!(segment_upward_tension_sign(&falling, 0), 1.0);
    }

    #[test]
    fn upward_drag_bends_rising_segment_upward() {
        let mut curve = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.2 }, CurveNode { x: 1.0, y: 0.8 }],
            segments: vec![CurveSegment { tension: 0.0 }],
        };
        let baseline_mid = sample_editable_curve(&curve, 0.5);
        let delta = tension_delta_from_drag_for_segment(
            &curve,
            0,
            Point { x: 0, y: 80 },
            Point { x: 0, y: 40 },
            Size {
                width: CURVE_W,
                height: CURVE_H,
            },
        );
        curve.segments[0].tension = (curve.segments[0].tension + delta).clamp(
            crate::curve::MIN_SEGMENT_TENSION,
            crate::curve::MAX_SEGMENT_TENSION,
        );
        let dragged_mid = sample_editable_curve(&curve, 0.5);
        assert!(
            dragged_mid > baseline_mid,
            "upward drag should move midpoint up for rising segment"
        );
    }

    #[test]
    fn upward_drag_bends_falling_segment_upward() {
        let mut curve = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.8 }, CurveNode { x: 1.0, y: 0.2 }],
            segments: vec![CurveSegment { tension: 0.0 }],
        };
        let baseline_mid = sample_editable_curve(&curve, 0.5);
        let delta = tension_delta_from_drag_for_segment(
            &curve,
            0,
            Point { x: 0, y: 80 },
            Point { x: 0, y: 40 },
            Size {
                width: CURVE_W,
                height: CURVE_H,
            },
        );
        curve.segments[0].tension = (curve.segments[0].tension + delta).clamp(
            crate::curve::MIN_SEGMENT_TENSION,
            crate::curve::MAX_SEGMENT_TENSION,
        );
        let dragged_mid = sample_editable_curve(&curve, 0.5);
        assert!(
            dragged_mid > baseline_mid,
            "upward drag should move midpoint up for falling segment"
        );
    }

    fn curve_draw_commands_with_status(
        phase: f32,
        is_playing: bool,
        has_host_beats_timeline: bool,
        gain: f32,
    ) -> (Vec<SurfaceCommand>, EditableCurve, PumpTheme) {
        let params = Arc::new(PumpParams::new());
        let status = Arc::new(GuiStatus::default());
        status.update(
            phase,
            gain,
            GuiTransportTelemetry {
                is_playing,
                has_host_beats_timeline,
                beat_phase: phase.rem_euclid(1.0),
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        let state = GuiState::new(
            Arc::clone(&params),
            status,
            Arc::new(AutomationQueue::default()),
            None,
        );
        let metrics = UiLayoutMetrics::design_space();
        let theme = PumpTheme::main(metrics);
        let curve = params.editable_curve_snapshot();
        let commands = state.build_curve_draw_commands(
            &curve,
            metrics,
            CurveRenderState {
                selected_node: None,
                hovered_node: None,
                hovered_segment: None,
                preview_node: None,
            },
            &theme,
        );
        (commands, curve, theme)
    }

    fn curve_draw_commands_with_transport(
        phase: f32,
        is_playing: bool,
        has_host_beats_timeline: bool,
    ) -> (Vec<SurfaceCommand>, EditableCurve, PumpTheme) {
        curve_draw_commands_with_status(phase, is_playing, has_host_beats_timeline, 1.0)
    }

    fn fill_circle_centers_for_color(commands: &[SurfaceCommand], color: Color) -> Vec<Point> {
        commands
            .iter()
            .filter_map(|command| match command {
                SurfaceCommand::FillCircle {
                    center,
                    color: command_color,
                    ..
                } if *command_color == color => Some(*center),
                _ => None,
            })
            .collect()
    }

    fn fill_rects_for_color(commands: &[SurfaceCommand], color: Color) -> Vec<(Point, Size)> {
        commands
            .iter()
            .filter_map(|command| match command {
                SurfaceCommand::FillRect {
                    rect,
                    color: command_color,
                } if *command_color == color => Some((rect.origin, rect.size)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn playhead_dot_hidden_when_transport_stopped_without_host_timeline() {
        let (commands, _curve, theme) = curve_draw_commands_with_transport(0.25, false, false);
        assert!(
            fill_circle_centers_for_color(&commands, theme.playhead_dot_glow).is_empty(),
            "glow dot should be hidden when transport is stopped and host timeline is unavailable"
        );
        assert!(
            fill_circle_centers_for_color(&commands, theme.playhead_dot_core).is_empty(),
            "core dot should be hidden when transport is stopped and host timeline is unavailable"
        );
    }

    #[test]
    fn playhead_dot_visible_when_transport_stopped_with_host_timeline() {
        let (commands, _curve, theme) = curve_draw_commands_with_transport(0.25, false, true);
        assert!(
            !fill_circle_centers_for_color(&commands, theme.playhead_dot_glow).is_empty(),
            "glow dot should remain visible when host timeline is available"
        );
        assert!(
            !fill_circle_centers_for_color(&commands, theme.playhead_dot_core).is_empty(),
            "core dot should remain visible when host timeline is available"
        );
    }

    #[test]
    fn playhead_dot_visible_without_host_beats_timeline_when_playing() {
        let (commands, _curve, theme) = curve_draw_commands_with_transport(0.25, true, false);
        assert!(
            !fill_circle_centers_for_color(&commands, theme.playhead_dot_glow).is_empty(),
            "glow dot should remain visible while transport is playing"
        );
        assert!(
            !fill_circle_centers_for_color(&commands, theme.playhead_dot_core).is_empty(),
            "core dot should remain visible while transport is playing"
        );
    }

    #[test]
    fn playhead_dot_visible_when_transport_running_with_beats_timeline() {
        let (commands, _curve, theme) = curve_draw_commands_with_transport(0.25, true, true);
        let glow_centers = fill_circle_centers_for_color(&commands, theme.playhead_dot_glow);
        let core_centers = fill_circle_centers_for_color(&commands, theme.playhead_dot_core);
        assert_eq!(glow_centers.len(), 1, "expected one glow playhead dot");
        assert_eq!(core_centers.len(), 1, "expected one core playhead dot");
        assert_eq!(
            core_centers[0], glow_centers[0],
            "playhead glow and core should share the same center"
        );
    }

    #[test]
    fn playhead_dot_tracks_curve_sample_at_host_phase() {
        let phase = 0.37;
        let (commands, curve, theme) = curve_draw_commands_with_transport(phase, true, true);
        let core_centers = fill_circle_centers_for_color(&commands, theme.playhead_dot_core);
        assert_eq!(core_centers.len(), 1, "expected one core playhead dot");
        let phase = phase.rem_euclid(1.0);
        let expected = local_from_node(CurveNode {
            x: phase,
            y: sample_editable_curve(&curve, phase).clamp(0.0, 1.0),
        });
        let dx = (core_centers[0].x - expected.x).abs();
        let dy = (core_centers[0].y - expected.y).abs();
        assert!(
            dx <= 6 && dy <= 2,
            "playhead dot should stay near sampled curve point at host phase (expected {expected:?}, got {:?}, dx={dx}, dy={dy})",
            core_centers[0]
        );
    }

    #[test]
    fn reduction_meter_is_empty_at_unity_and_fills_top_down_under_reduction() {
        let (unity_commands, _curve, theme) =
            curve_draw_commands_with_status(0.25, true, true, 1.0);
        assert!(
            fill_rects_for_color(&unity_commands, theme.meter_fill).is_empty(),
            "gain reduction meter should be empty at unity gain"
        );

        let reduced_gain = 0.4;
        let (reduced_commands, _curve, theme) =
            curve_draw_commands_with_status(0.25, true, true, reduced_gain);
        let rects = fill_rects_for_color(&reduced_commands, theme.meter_fill);
        assert_eq!(rects.len(), 1, "expected one gain-reduction fill rect");
        let (fill_origin, fill_size) = rects[0];

        let metrics = UiLayoutMetrics::design_space();
        let meter_x_offset = metrics.meter_x_offset.max(0);
        let meter_y_offset = metrics.meter_y_offset.max(0);
        let meter_width = metrics.meter_width.max(1);
        let meter_width_i32 = i32::try_from(meter_width).unwrap_or(i32::MAX);
        let meter_stroke_u32 = metrics.meter_stroke.max(1);
        let meter_stroke_i32 = i32::try_from(meter_stroke_u32).unwrap_or(i32::MAX);
        let meter_height = metrics
            .curve_size
            .height
            .saturating_sub((meter_y_offset.saturating_mul(2)).max(0) as u32);
        let reduction = (1.0 - reduced_gain.clamp(0.0, 1.0)).clamp(0.0, 1.0);
        let expected_fill_height = ((meter_height as f32) * reduction).round() as u32;
        let expected_fill_origin = Point {
            x: metrics.curve_size.width as i32 - meter_x_offset - meter_width_i32
                + meter_stroke_i32,
            y: meter_y_offset + meter_stroke_i32,
        };

        assert_eq!(
            fill_origin, expected_fill_origin,
            "gain-reduction fill should start at top-left inside meter border"
        );
        assert_eq!(
            fill_size.height, expected_fill_height,
            "gain-reduction fill height should match reduction amount"
        );
    }

    #[test]
    fn measured_open_size_is_at_least_default_window_baseline() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let (width, height) = state.measured_open_size();
        assert_eq!(width, WINDOW_WIDTH);
        assert_eq!(height, WINDOW_HEIGHT);
    }

    #[test]
    fn preferred_window_size_tracks_measured_layout() {
        let (preferred_width, preferred_height) = preferred_window_size();
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let (measured_width, measured_height) = state.measured_open_size();
        assert_eq!(preferred_width, measured_width);
        assert_eq!(preferred_height, measured_height);
        assert_eq!(preferred_width, WINDOW_WIDTH);
        assert_eq!(preferred_height, WINDOW_HEIGHT);
    }

    #[test]
    fn constrained_host_size_enforces_baseline_minimums() {
        assert_eq!(
            constrained_host_size(GuiSize {
                width: 1,
                height: 1,
            }),
            GuiSize {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            }
        );
        assert_eq!(
            constrained_host_size(GuiSize {
                width: WINDOW_WIDTH * 2,
                height: 40,
            }),
            GuiSize {
                width: WINDOW_WIDTH * 2,
                height: WINDOW_HEIGHT,
            }
        );
        assert_eq!(
            constrained_host_size(GuiSize {
                width: 64,
                height: WINDOW_HEIGHT * 2,
            }),
            GuiSize {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT * 2,
            }
        );
    }

    #[test]
    fn slot_height_split_matches_expected_ratios() {
        let (header_h, curve_h, controls_h) = resolve_vertical_slot_heights(WINDOW_HEIGHT);
        assert_eq!(header_h, 18);
        assert_eq!(curve_h, 163);
        assert_eq!(controls_h, 77);
        assert_eq!(header_h + curve_h + controls_h, WINDOW_HEIGHT);
    }

    #[test]
    fn bottom_row_split_matches_expected_ratio() {
        let (knobs_w, dropdown_w) = resolve_runtime_controls_slot_widths(WINDOW_WIDTH);
        assert_eq!(knobs_w, 294);
        assert_eq!(dropdown_w, 126);
        assert_eq!(knobs_w + dropdown_w, WINDOW_WIDTH);
    }

    #[test]
    fn runtime_slot_splits_consume_full_parent_extent() {
        let (header_h, curve_h, controls_h) = resolve_vertical_slot_heights(259);
        assert_eq!(header_h + curve_h + controls_h, 259);

        let (knobs_w, dropdown_w) = resolve_runtime_controls_slot_widths(799);
        assert_eq!(knobs_w + dropdown_w, 799);
    }

    #[test]
    fn build_ui_places_curve_editor_at_full_spline_extent() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let spec = state.build_ui(&InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        });
        let curve_editor_layout = find_curve_editor_layout(spec.root.content())
            .expect("curve editor should exist");
        assert_eq!(curve_editor_layout, LayoutBox::fill());
    }

    #[test]
    fn build_ui_keeps_design_sized_root_across_host_resize_sequences() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let input_sizes = [
            Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            Size {
                width: WINDOW_WIDTH * 3,
                height: WINDOW_HEIGHT * 3,
            },
            Size {
                width: WINDOW_WIDTH / 2,
                height: WINDOW_HEIGHT / 2,
            },
            Size {
                width: WINDOW_WIDTH * 2,
                height: WINDOW_HEIGHT * 2,
            },
        ];
        for window_size in input_sizes {
            let input = InputState {
                window_size,
                ..InputState::default()
            };
            let spec = state.build_ui(&input);
            let measured = measure_checked(&spec).expect("measurement should succeed");
            assert_eq!(measured.width, WINDOW_WIDTH);
            assert_eq!(measured.height, WINDOW_HEIGHT);
            assert_eq!(spec.root.scale_mode, RootScaleMode::UniformFit);
            assert_eq!(
                spec.root.design_size_value(),
                Some(Size {
                    width: WINDOW_WIDTH,
                    height: WINDOW_HEIGHT,
                })
            );
        }
    }

    #[test]
    fn build_ui_handles_tiny_window_sizes_without_measurement_errors() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let tiny_sizes = [
            Size {
                width: 1,
                height: 1,
            },
            Size {
                width: 2,
                height: 3,
            },
            Size {
                width: 3,
                height: 2,
            },
            Size {
                width: 8,
                height: 8,
            },
        ];

        for input_size in tiny_sizes {
            let input = InputState {
                window_size: input_size,
                ..InputState::default()
            };
            let spec = state.build_ui(&input);
            let measured = measure_checked(&spec).expect("measurement should succeed");
            assert_eq!(measured.width, WINDOW_WIDTH);
            assert_eq!(measured.height, WINDOW_HEIGHT);
            assert_eq!(spec.root.scale_mode, RootScaleMode::UniformFit);
            assert_eq!(
                spec.root.design_size_value(),
                Some(Size {
                    width: WINDOW_WIDTH,
                    height: WINDOW_HEIGHT,
                })
            );
        }
    }

    #[test]
    fn build_ui_handles_host_resize_jitter_without_layout_regressions() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let jitter_sizes = [
            Size {
                width: 1,
                height: 1,
            },
            Size {
                width: 640,
                height: 360,
            },
            Size {
                width: 2,
                height: 3,
            },
            Size {
                width: 1024,
                height: 256,
            },
            Size {
                width: 3,
                height: 2,
            },
            Size {
                width: 700,
                height: 700,
            },
            Size {
                width: 1,
                height: 1,
            },
            Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
        ];

        for input_size in jitter_sizes {
            let input = InputState {
                window_size: input_size,
                ..InputState::default()
            };
            let spec = state.build_ui(&input);
            let measured = measure_checked(&spec).expect("measurement should succeed");

            assert_eq!(measured.width, WINDOW_WIDTH);
            assert_eq!(measured.height, WINDOW_HEIGHT);
            assert_eq!(spec.root.scale_mode, RootScaleMode::UniformFit);
            assert_eq!(
                spec.root.design_size_value(),
                Some(Size {
                    width: WINDOW_WIDTH,
                    height: WINDOW_HEIGHT,
                })
            );

            let curve_editor_layout = find_curve_editor_layout(spec.root.content())
                .expect("curve editor should exist for all measured sizes");
            assert_eq!(curve_editor_layout, LayoutBox::fill());

            let dropdown_size = find_dropdown_control_size(spec.root.content(), DIVISION_KEY)
                .expect("division dropdown control size should exist for all measured sizes");
            let (_, expected_dropdown_w) = resolve_runtime_controls_slot_widths(WINDOW_WIDTH);
            assert_eq!(dropdown_size.width, expected_dropdown_w);
        }
    }

    #[test]
    fn build_ui_root_content_is_three_slot_column() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let spec = state.build_ui(&InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        });
        let root_grid = match expect_slot_child(spec.root.content(), "root") {
            Node::Grid(grid) => grid,
            other => panic!("expected root content grid, got {other:?}"),
        };
        assert_eq!(root_grid.children().len(), 3);
        let header_panel = expect_slot_panel(&root_grid.children()[0], "header");
        let _curve_panel = expect_slot_panel(&root_grid.children()[1], "curve");
        let header_grid = match expect_slot_child(header_panel.content(), "header") {
            Node::Grid(grid) => grid,
            other => panic!("expected header row grid in panel, got {other:?}"),
        };
        assert_eq!(header_grid.kind(), GridKind::SlotRow);
        assert_eq!(header_grid.children().len(), 2);

        let controls_panel = expect_slot_panel(&root_grid.children()[2], "controls");
        let controls_grid = match expect_slot_child(controls_panel.content(), "controls") {
            Node::Grid(grid) => grid,
            other => panic!("expected controls grid in panel, got {other:?}"),
        };
        assert_eq!(controls_grid.children().len(), 2);
        let _knobs_panel = expect_slot_panel(&controls_grid.children()[0], "knobs");
        let _dropdown_panel = expect_slot_panel(&controls_grid.children()[1], "dropdowns");
    }

    #[test]
    fn build_ui_includes_textboxes_for_control_captions() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let spec = state.build_ui(&InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        });
        let mut texts = Vec::new();
        collect_textbox_texts(spec.root.content(), &mut texts);

        for expected in ["Mix", "Depth", "Phase", "Output"] {
            assert!(
                texts.iter().any(|text| text == expected),
                "expected textbox caption `{expected}` in {:?}",
                texts
            );
        }
    }

    #[test]
    fn dropdown_change_preserves_curve_and_swallows_immediate_reset_press() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let custom_curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.35, y: 0.2 },
                CurveNode { x: 1.0, y: 0.85 },
            ],
            segments: vec![
                CurveSegment { tension: -0.4 },
                CurveSegment { tension: 0.25 },
            ],
        }
        .normalized();
        params.set_editable_curve(&custom_curve);

        let previous_division = params.sync_division();
        state.reduce_action(UiAction::DropdownSelected {
            key: DIVISION_KEY.to_string(),
            index: (previous_division + 1).min(MAX_SYNC_DIVISION as usize),
        });
        state.reduce_action(UiAction::ButtonPressed {
            key: RESET_KEY.to_string(),
        });

        assert_ne!(
            params.sync_division(),
            previous_division,
            "division should still update on dropdown selection"
        );
        assert_eq!(
            params.editable_curve_snapshot(),
            custom_curve,
            "division changes must not reset the editable curve"
        );
    }

    #[test]
    fn second_reset_press_after_dropdown_still_resets_curve() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let custom_curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.4, y: 0.1 },
                CurveNode { x: 1.0, y: 0.9 },
            ],
            segments: vec![
                CurveSegment { tension: -0.5 },
                CurveSegment { tension: 0.4 },
            ],
        }
        .normalized();
        params.set_editable_curve(&custom_curve);

        state.reduce_action(UiAction::DropdownSelected {
            key: DIVISION_KEY.to_string(),
            index: 2,
        });
        state.reduce_action(UiAction::ButtonPressed {
            key: RESET_KEY.to_string(),
        });
        assert_eq!(
            params.editable_curve_snapshot(),
            custom_curve,
            "first reset press after dropdown should be guarded"
        );

        state.reduce_action(UiAction::ButtonPressed {
            key: RESET_KEY.to_string(),
        });
        let reset_curve = params.editable_curve_snapshot();
        assert_eq!(
            reset_curve,
            crate::curve::default_editable_curve(),
            "second reset press should perform an intentional curve reset"
        );
    }

    #[test]
    fn preset_save_overwrites_existing_name_from_rename_draft() {
        let params = Arc::new(PumpParams::new());
        params
            .add_preset_from_current_state()
            .expect("preset insertion should succeed");
        assert!(params.rename_preset(1, "Verse"));
        params
            .load_preset(1)
            .expect("preset selection should succeed");
        params.set_mix(0.19);

        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        state.reduce_action(UiAction::ButtonPressed {
            key: PRESET_RENAME_BUTTON_KEY.to_string(),
        });
        state.reduce_action(UiAction::TextBoxEdited {
            key: PRESET_RENAME_KEY.to_string(),
            text: "verse".to_string(),
        });
        state.reduce_action(UiAction::ButtonPressed {
            key: PRESET_SAVE_KEY.to_string(),
        });

        let bank = params.preset_bank_snapshot();
        assert_eq!(bank.presets.len(), 2);
        assert_eq!(bank.selected, 1);
        assert_eq!(bank.presets[1].name, "Verse");
        assert!((bank.presets[1].mix - 0.19).abs() < 1.0e-6);
    }

    #[test]
    fn preset_save_creates_new_entry_for_new_name() {
        let params = Arc::new(PumpParams::new());
        params
            .add_preset_from_current_state()
            .expect("preset insertion should succeed");
        params.set_mix(0.74);

        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        state.reduce_action(UiAction::ButtonPressed {
            key: PRESET_RENAME_BUTTON_KEY.to_string(),
        });
        state.reduce_action(UiAction::TextBoxEdited {
            key: PRESET_RENAME_KEY.to_string(),
            text: "Hook".to_string(),
        });
        state.reduce_action(UiAction::ButtonPressed {
            key: PRESET_SAVE_KEY.to_string(),
        });

        let bank = params.preset_bank_snapshot();
        assert_eq!(bank.presets.len(), 3);
        assert_eq!(bank.selected, 2);
        assert_eq!(bank.presets[2].name, "Hook");
        assert!((bank.presets[2].mix - 0.74).abs() < 1.0e-6);
    }

    #[test]
    fn init_preset_is_not_renamable_from_header_interaction() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );

        state.reduce_action(UiAction::ButtonPressed {
            key: PRESET_RENAME_BUTTON_KEY.to_string(),
        });

        let runtime = state.runtime.lock().expect("runtime lock should succeed");
        assert!(!runtime.preset_rename_active);
        assert_eq!(runtime.preset_warning_text, Some(PRESET_WARNING_INIT));
        assert_eq!(params.preset_bank_snapshot().presets[0].name, "Init");
    }

    #[test]
    fn init_save_warning_blinks_without_header_relayout() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        state.reduce_action(UiAction::ButtonPressed {
            key: PRESET_SAVE_KEY.to_string(),
        });

        let input = InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        };
        let spec = state.build_ui(&input);
        let dropdown = find_dropdown_spec(spec.root.content(), PRESET_DROPDOWN_KEY)
            .expect("preset dropdown should exist");
        assert_eq!(
            dropdown.selected_option_background_override,
            Some(MainPalette::main().literals)
        );

        let root_grid = match expect_slot_child(spec.root.content(), "root") {
            Node::Grid(grid) => grid,
            other => panic!("expected root slot child grid, got {other:?}"),
        };
        let header_panel = expect_slot_panel(&root_grid.children()[0], "header");
        let header_grid = match expect_slot_child(header_panel.content(), "header") {
            Node::Grid(grid) => grid,
            other => panic!("expected header row grid in panel, got {other:?}"),
        };
        let left_header = expect_slot_child(&header_grid.children()[0], "header-left");
        let left_header_grid = match left_header {
            Node::Grid(grid) => grid,
            Node::Row(row) => match row.children() {
                [child] => match expect_slot_child(child, "header-left-row") {
                    Node::Grid(grid) => grid,
                    other => panic!("expected wrapped header-left grid, got {other:?}"),
                },
                _ => panic!("expected one wrapped header-left child"),
            },
            other => panic!("expected left header content to remain slot-based, got {other:?}"),
        };
        assert_eq!(left_header_grid.kind(), GridKind::SlotRow);
    }

    #[test]
    fn preset_rename_button_enters_rename_mode() {
        let params = Arc::new(PumpParams::new());
        params
            .add_preset_from_current_state()
            .expect("preset insertion should succeed");
        params
            .load_preset(1)
            .expect("preset selection should succeed");
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );

        state.reduce_action(UiAction::ButtonPressed {
            key: PRESET_RENAME_BUTTON_KEY.to_string(),
        });

        let runtime = state.runtime.lock().expect("runtime lock should succeed");
        assert!(runtime.preset_rename_active);
        assert_eq!(runtime.preset_name_draft, "Preset 2");
    }

    #[test]
    fn preset_rename_draft_allows_empty_name_while_editing() {
        let params = Arc::new(PumpParams::new());
        params
            .add_preset_from_current_state()
            .expect("preset insertion should succeed");
        params
            .load_preset(1)
            .expect("preset selection should succeed");
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );

        state.reduce_action(UiAction::ButtonPressed {
            key: PRESET_RENAME_BUTTON_KEY.to_string(),
        });
        state.reduce_action(UiAction::TextBoxEdited {
            key: PRESET_RENAME_KEY.to_string(),
            text: String::new(),
        });

        let snapshot = state.snapshot_presets();
        assert!(snapshot.rename_active);
        assert_eq!(
            snapshot.rename_draft, "",
            "rename draft should remain empty instead of being refilled"
        );

        let runtime = state.runtime.lock().expect("runtime lock should succeed");
        assert_eq!(
            runtime.preset_name_draft, "",
            "runtime draft should preserve user-cleared state"
        );
    }

    #[test]
    fn preset_dropdown_marks_selected_option_with_red_highlight_when_dirty() {
        let params = Arc::new(PumpParams::new());
        let state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let input = InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        };

        let clean_spec = state.build_ui(&input);
        let clean_dropdown = find_dropdown_spec(clean_spec.root.content(), PRESET_DROPDOWN_KEY)
            .expect("preset dropdown should exist");
        assert_eq!(clean_dropdown.selected_option_background_override, None);
        assert!(!state.snapshot_presets().dirty);

        params.set_mix(0.5);
        let dirty_spec = state.build_ui(&input);
        let dirty_dropdown = find_dropdown_spec(dirty_spec.root.content(), PRESET_DROPDOWN_KEY)
            .expect("preset dropdown should exist");
        assert_eq!(
            dirty_dropdown.selected_option_background_override,
            Some(MainPalette::main().literals)
        );
        assert!(state.snapshot_presets().dirty);
    }

    #[test]
    fn header_transport_indicator_reflects_transport_blink_state() {
        let params = Arc::new(PumpParams::new());
        let status = Arc::new(GuiStatus::default());
        let state = GuiState::new(
            params,
            Arc::clone(&status),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let input = InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        };

        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.05,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        let lit_spec = state.build_ui(&input);
        let lit = find_first_indicator_active(lit_spec.root.content())
            .expect("header transport indicator should exist");
        assert!(lit, "indicator should blink on at beat onset");

        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.5,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        let dim_spec = state.build_ui(&input);
        let dim = find_first_indicator_active(dim_spec.root.content())
            .expect("header transport indicator should exist");
        assert!(!dim, "indicator should be off between beat flashes");

        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: false,
                beat_phase: 0.05,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        let no_timeline_spec = state.build_ui(&input);
        let no_timeline = find_first_indicator_active(no_timeline_spec.root.content())
            .expect("header transport indicator should exist");
        assert!(
            no_timeline,
            "indicator should stay lit while playing when host beat timeline is unavailable"
        );
    }

    #[test]
    fn header_transport_indicator_has_expected_fixed_size() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let spec = state.build_ui(&InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        });
        let indicator_layout = find_first_indicator_size(spec.root.content())
            .expect("header transport indicator should exist");
        assert_eq!(
            indicator_layout,
            LayoutBox::fixed(TRANSPORT_INDICATOR_SIZE, TRANSPORT_INDICATOR_SIZE)
                .max(TRANSPORT_INDICATOR_SIZE, TRANSPORT_INDICATOR_SIZE)
        );
    }

    #[test]
    fn header_uses_two_slot_row_with_80_20_split() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let spec = state.build_ui(&InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        });
        let root_grid = match expect_slot_child(spec.root.content(), "root") {
            Node::Grid(grid) => grid,
            other => panic!("expected root content grid, got {other:?}"),
        };
        let header_panel = expect_slot_panel(&root_grid.children()[0], "header");
        let header_grid = match expect_slot_child(header_panel.content(), "header") {
            Node::Grid(grid) => grid,
            other => panic!("expected header row grid in panel, got {other:?}"),
        };
        assert_eq!(header_grid.kind(), GridKind::SlotRow);
        assert_eq!(header_grid.children().len(), 2);
        assert!(
            matches!(
                header_grid.template.columns.as_slice(),
                [
                    toybox::gui::declarative::TrackSize::Fr(left),
                    toybox::gui::declarative::TrackSize::Fr(right)
                ] if *left == HEADER_EMPTY_SECTION_PERCENT as u16
                    && *right == HEADER_INDICATOR_SECTION_PERCENT as u16
            ),
            "expected header slot weights [{}, {}], got {:?}",
            HEADER_EMPTY_SECTION_PERCENT,
            HEADER_INDICATOR_SECTION_PERCENT,
            header_grid.template.columns
        );
    }

    #[test]
    fn emitted_ui_spec_passes_strict_slot_validation() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let sizes = [
            Size {
                width: 1,
                height: 1,
            },
            Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            Size {
                width: WINDOW_WIDTH * 2,
                height: WINDOW_HEIGHT * 2,
            },
        ];

        for size in sizes {
            let spec = state.build_ui(&InputState {
                window_size: size,
                ..InputState::default()
            });
            measure_checked(&spec).expect("emitted tree must pass strict declarative validation");
            assert_emitted_slot_tree_invariants(&spec);
        }
    }

    #[test]
    fn pump_knob_strip_is_borderless() {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let frame = render_spec_to_frame(
            Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            |input| state.build_ui(input),
        )
        .expect("pump frame should render");

        let (header_h, curve_h, controls_h) = resolve_vertical_slot_heights(WINDOW_HEIGHT);
        let controls_top = header_h.saturating_add(curve_h);
        let base_row = controls_top.min(frame.height.saturating_sub(1));
        let (knobs_w, _) = resolve_runtime_controls_slot_widths(WINDOW_WIDTH);
        let border = MainPalette::main().text_primary;

        let end_row = base_row
            .saturating_add(controls_h.saturating_sub(1))
            .min(frame.height.saturating_sub(1));
        let mut best_runs: Vec<(u32, u32)> = Vec::new();
        let mut best_coverage = 0u32;
        for y in base_row..=end_row {
            let runs = color_runs_on_row(
                &frame.pixels,
                frame.width,
                y,
                0,
                knobs_w.saturating_sub(1),
                border,
            );
            let coverage = runs
                .iter()
                .map(|(start, end)| end.saturating_sub(*start).saturating_add(1))
                .sum::<u32>();
            if coverage > best_coverage {
                best_coverage = coverage;
                best_runs = runs;
            }
        }

        let significant_runs: Vec<(u32, u32)> = best_runs
            .into_iter()
            .filter(|(start, end)| end.saturating_sub(*start).saturating_add(1) >= 12)
            .collect();

        assert!(
            significant_runs.is_empty(),
            "expected no knob-border runs in pump knob slot, got {:?}",
            significant_runs
        );
    }

    fn find_curve_editor_layout(node: &Node) -> Option<LayoutBox> {
        match node {
            Node::Slot(slot) => find_curve_editor_layout(slot.child()),
            Node::CurveEditor(curve_editor) if curve_editor.key == CURVE_KEY => {
                Some(curve_editor.layout)
            }
            Node::Panel(panel) => find_curve_editor_layout(panel.content()),
            Node::PaddingBox(padding_box) => find_curve_editor_layout(padding_box.content()),
            Node::AlignBox(align_box) => find_curve_editor_layout(align_box.content()),
            Node::AspectBox(aspect_box) => find_curve_editor_layout(aspect_box.content()),
            Node::Row(flex) | Node::Column(flex) => {
                flex.children().iter().find_map(find_curve_editor_layout)
            }
            Node::Grid(grid) => grid.children().iter().find_map(find_curve_editor_layout),
            Node::Stack(stack) => stack.children().iter().find_map(find_curve_editor_layout),
            Node::ScrollView(scroll_view) => find_curve_editor_layout(scroll_view.content()),
            Node::Wrap(wrap) => wrap.children().iter().find_map(find_curve_editor_layout),
            Node::SwitchLayout(switch_layout) => switch_layout
                .cases()
                .iter()
                .find_map(|case_entry| find_curve_editor_layout(case_entry.child()))
                .or_else(|| find_curve_editor_layout(switch_layout.fallback())),
            Node::CurveEditor(_) => None,
            Node::Region(_) => None,
            Node::TextBox(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::Indicator(_)
            | Node::Absolute(_) => None,
        }
    }

    fn collect_textbox_texts(node: &Node, texts: &mut Vec<String>) {
        match node {
            Node::Slot(slot) => collect_textbox_texts(slot.child(), texts),
            Node::Panel(panel) => collect_textbox_texts(panel.content(), texts),
            Node::PaddingBox(padding_box) => collect_textbox_texts(padding_box.content(), texts),
            Node::AlignBox(align_box) => collect_textbox_texts(align_box.content(), texts),
            Node::AspectBox(aspect_box) => collect_textbox_texts(aspect_box.content(), texts),
            Node::Row(flex) | Node::Column(flex) => {
                for child in flex.children() {
                    collect_textbox_texts(child, texts);
                }
            }
            Node::Grid(grid) => {
                for child in grid.children() {
                    collect_textbox_texts(child, texts);
                }
            }
            Node::Absolute(absolute) => {
                for child in absolute.children() {
                    collect_textbox_texts(child.node(), texts);
                }
            }
            Node::Stack(stack) => {
                for child in stack.children() {
                    collect_textbox_texts(child, texts);
                }
            }
            Node::ScrollView(scroll_view) => collect_textbox_texts(scroll_view.content(), texts),
            Node::Wrap(wrap) => {
                for child in wrap.children() {
                    collect_textbox_texts(child, texts);
                }
            }
            Node::SwitchLayout(switch_layout) => {
                for case_entry in switch_layout.cases() {
                    collect_textbox_texts(case_entry.child(), texts);
                }
                collect_textbox_texts(switch_layout.fallback(), texts);
            }
            Node::TextBox(text_box) => texts.push(text_box.text.clone()),
            Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::CurveEditor(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::Region(_)
            | Node::Indicator(_) => {}
        }
    }

    fn find_dropdown_control_size(node: &Node, key: &str) -> Option<Size> {
        find_dropdown_spec(node, key).and_then(|dropdown| dropdown.control_size)
    }

    fn find_dropdown_spec<'a>(node: &'a Node, key: &str) -> Option<&'a DropdownSpec> {
        match node {
            Node::Slot(slot) => find_dropdown_spec(slot.child(), key),
            Node::Dropdown(dropdown) if dropdown.key == key => Some(dropdown),
            Node::Panel(panel) => find_dropdown_spec(panel.content(), key),
            Node::PaddingBox(padding_box) => find_dropdown_spec(padding_box.content(), key),
            Node::AlignBox(align_box) => find_dropdown_spec(align_box.content(), key),
            Node::AspectBox(aspect_box) => find_dropdown_spec(aspect_box.content(), key),
            Node::Row(flex) | Node::Column(flex) => flex
                .children()
                .iter()
                .find_map(|child| find_dropdown_spec(child, key)),
            Node::Grid(grid) => grid
                .children()
                .iter()
                .find_map(|child| find_dropdown_spec(child, key)),
            Node::Stack(stack) => stack
                .children()
                .iter()
                .find_map(|child| find_dropdown_spec(child, key)),
            Node::ScrollView(scroll_view) => find_dropdown_spec(scroll_view.content(), key),
            Node::Wrap(wrap) => wrap
                .children()
                .iter()
                .find_map(|child| find_dropdown_spec(child, key)),
            Node::SwitchLayout(switch_layout) => switch_layout
                .cases()
                .iter()
                .find_map(|case_entry| find_dropdown_spec(case_entry.child(), key))
                .or_else(|| find_dropdown_spec(switch_layout.fallback(), key)),
            Node::Dropdown(_) => None,
            Node::TextBox(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::CurveEditor(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Region(_)
            | Node::Indicator(_)
            | Node::Absolute(_) => None,
        }
    }

    fn find_first_indicator_active(node: &Node) -> Option<bool> {
        match node {
            Node::Slot(slot) => find_first_indicator_active(slot.child()),
            Node::Indicator(indicator) => Some(indicator.active),
            Node::Panel(panel) => find_first_indicator_active(panel.content()),
            Node::PaddingBox(padding_box) => find_first_indicator_active(padding_box.content()),
            Node::AlignBox(align_box) => find_first_indicator_active(align_box.content()),
            Node::AspectBox(aspect_box) => find_first_indicator_active(aspect_box.content()),
            Node::Row(flex) | Node::Column(flex) => {
                flex.children().iter().find_map(find_first_indicator_active)
            }
            Node::Grid(grid) => grid.children().iter().find_map(find_first_indicator_active),
            Node::Stack(stack) => stack
                .children()
                .iter()
                .find_map(find_first_indicator_active),
            Node::ScrollView(scroll_view) => find_first_indicator_active(scroll_view.content()),
            Node::Wrap(wrap) => wrap.children().iter().find_map(find_first_indicator_active),
            Node::SwitchLayout(switch_layout) => switch_layout
                .cases()
                .iter()
                .find_map(|case_entry| find_first_indicator_active(case_entry.child()))
                .or_else(|| find_first_indicator_active(switch_layout.fallback())),
            Node::Absolute(absolute) => absolute
                .children()
                .iter()
                .find_map(|child| find_first_indicator_active(child.node())),
            Node::TextBox(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::CurveEditor(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::Region(_) => None,
        }
    }

    fn find_first_indicator_size(node: &Node) -> Option<LayoutBox> {
        match node {
            Node::Slot(slot) => find_first_indicator_size(slot.child()),
            Node::Indicator(indicator) => Some(indicator.layout),
            Node::Panel(panel) => find_first_indicator_size(panel.content()),
            Node::PaddingBox(padding_box) => find_first_indicator_size(padding_box.content()),
            Node::AlignBox(align_box) => find_first_indicator_size(align_box.content()),
            Node::AspectBox(aspect_box) => find_first_indicator_size(aspect_box.content()),
            Node::Row(flex) | Node::Column(flex) => {
                flex.children().iter().find_map(find_first_indicator_size)
            }
            Node::Grid(grid) => grid.children().iter().find_map(find_first_indicator_size),
            Node::Stack(stack) => stack.children().iter().find_map(find_first_indicator_size),
            Node::ScrollView(scroll_view) => find_first_indicator_size(scroll_view.content()),
            Node::Wrap(wrap) => wrap.children().iter().find_map(find_first_indicator_size),
            Node::SwitchLayout(switch_layout) => switch_layout
                .cases()
                .iter()
                .find_map(|case_entry| find_first_indicator_size(case_entry.child()))
                .or_else(|| find_first_indicator_size(switch_layout.fallback())),
            Node::Absolute(absolute) => absolute
                .children()
                .iter()
                .find_map(|child| find_first_indicator_size(child.node())),
            Node::TextBox(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::CurveEditor(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::Region(_) => None,
        }
    }

    fn color_runs_on_row(
        pixels: &[u8],
        frame_width: u32,
        y: u32,
        x_start: u32,
        x_end: u32,
        color: toybox::gui::Color,
    ) -> Vec<(u32, u32)> {
        if frame_width == 0 || pixels.is_empty() || x_start > x_end {
            return Vec::new();
        }
        let mut runs = Vec::new();
        let mut active_start: Option<u32> = None;
        for x in x_start..=x_end {
            let idx =
                ((y.saturating_mul(frame_width).saturating_add(x)).saturating_mul(4)) as usize;
            if idx + 3 >= pixels.len() {
                break;
            }
            let matches = pixels[idx] == color.r
                && pixels[idx + 1] == color.g
                && pixels[idx + 2] == color.b
                && pixels[idx + 3] != 0;
            match (active_start, matches) {
                (None, true) => active_start = Some(x),
                (Some(start), false) => {
                    runs.push((start, x.saturating_sub(1)));
                    active_start = None;
                }
                _ => {}
            }
        }
        if let Some(start) = active_start {
            runs.push((start, x_end));
        }
        runs
    }
