    use super::{
        build_version_label, constrained_host_size, curve_beat_grid, curve_gain_references,
        find_deletable_node_hit, find_segment_line_hit_within, local_from_node,
        local_from_node_for_size, move_node_with_push_through, move_segment_translated,
        preferred_window_size, preview_node_on_curve,
        radiant_editor_frame_for_params, recompute_move_node_from_origin_for_size,
        resolve_runtime_controls_slot_widths,
        resolve_vertical_slot_heights, segment_upward_tension_sign,
        tension_delta_from_drag_for_segment, CurveRenderState, GuiState, PumpTheme,
        UiLayoutMetrics, CURVE_H, CURVE_KEY, CURVE_W, DIVISION_KEY, HEADER_EMPTY_SECTION_PERCENT,
        GRID_OVERRIDE_KEY, HEADER_INDICATOR_SECTION_PERCENT, INCOMING_WAVEFORM_KEY,
        METER_STROKE, METER_WIDTH,
        PRESET_DROPDOWN_KEY,
        PRESET_RENAME_BUTTON_KEY, PRESET_RENAME_KEY, PRESET_SAVE_KEY, PRESET_WARNING_STORAGE,
        QUICK_SLOT_KEY_PREFIX, REDO_KEY, SNAP_KEY, UNDO_KEY, TRANSPORT_INDICATOR_SIZE,
        WINDOW_HEIGHT, WINDOW_WIDTH,
    };
    use super::state_impl::{
        curve_beat_grid_commands, curve_gain_reference_label_commands,
        curve_gain_reference_line_commands, gain_reduction_meter_commands,
        incoming_waveform_underlay_commands,
    };
    use crate::curve::{sample_editable_curve, CurveNode, CurveSegment, EditableCurve};
    use crate::params::{
        with_test_persistence_failure, with_test_persistence_path, PumpParams,
        TestPersistenceFailure, GLOBAL_CURVE_SLOT_COUNT, MAX_SYNC_DIVISION,
    };
    use crate::{GuiStatus, GuiTransportTelemetry};
    use std::sync::Arc;
    use toybox::clack_extensions::gui::GuiSize;
    use toybox::clap::automation::AutomationQueue;
    use toybox::clap::gui::InputState;
    use toybox::gui::declarative::{
        measure_checked, ContainerLayout, ContainerLength, CurveEditorSpec, DropdownSpec,
        GridKind, LayoutBox, Length, Node, PanelSpec, RegionInteractionKind, RootScaleMode,
        SurfaceCommand, UiAction, UiSpec,
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
            | Node::TabBar(_)
            | Node::EqAttractorSurface(_)
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
                transport_is_playing: is_playing,
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
                selected_nodes: Vec::new(),
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
    fn attenuation_fill_tracks_the_area_beneath_the_curve() {
        let (commands, curve, theme) = curve_draw_commands_with_transport(0.25, false, false);
        let strips = fill_rects_for_color(&commands, theme.curve_fill);
        let metrics = UiLayoutMetrics::design_space();

        assert!(
            strips.len() >= metrics.curve_size.width.saturating_div(2) as usize,
            "attenuation fill should span the curve width with contiguous strips"
        );
        let midpoint_x = (metrics.curve_size.width / 2) & !1;
        let midpoint = strips
            .iter()
            .find(|(origin, _)| origin.x == midpoint_x as i32)
            .expect("attenuation fill should include the curve midpoint");
        let midpoint_phase = midpoint_x as f32 / (metrics.curve_size.width - 1) as f32;
        let expected_top = local_from_node_for_size(
            CurveNode {
                x: midpoint_phase,
                y: sample_editable_curve(&curve, midpoint_phase),
            },
            metrics.curve_size,
        )
        .y;
        assert_eq!(midpoint.0.y, expected_top);
        assert_eq!(
            midpoint.1.height,
            metrics.curve_size.height.saturating_sub(expected_top as u32),
            "fill should extend from the curve down to the bottom edge"
        );
        assert!(theme.curve_fill.a < theme.curve_line.a);
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
    fn playhead_palette_is_distinct_from_curve_and_editable_node_states() {
        let theme = PumpTheme::main(UiLayoutMetrics::design_space());
        let editable_colors = [
            theme.curve_line,
            theme.curve_line_highlight,
            theme.preview_fill,
            theme.preview_stroke,
            theme.node_fill,
            theme.node_hover_fill,
            theme.node_selected_fill,
            theme.node_stroke,
            theme.node_hover_stroke,
            theme.node_selected_stroke,
            theme.node_hover_ring,
            theme.node_selected_ring,
        ];

        for editable_color in editable_colors {
            assert_ne!(
                theme.playhead_dot_core, editable_color,
                "playhead core must not reuse a curve or editable-node state color"
            );
            assert_ne!(
                theme.playhead_dot_stroke, editable_color,
                "playhead ring must not reuse a curve or editable-node state color"
            );
        }
        assert_ne!(theme.playhead_dot_core, theme.playhead_dot_stroke);
        assert!(
            theme.playhead_dot_glow.a < theme.playhead_dot_core.a,
            "playhead glow should support the indicator without reading as another solid node"
        );
    }

    #[test]
    fn playhead_dot_tracks_curve_sample_at_host_phase() {
        let phase = 0.37;
        let (commands, curve, theme) = curve_draw_commands_with_transport(phase, true, true);
        let core_centers = fill_circle_centers_for_color(&commands, theme.playhead_dot_core);
        assert_eq!(core_centers.len(), 1, "expected one core playhead dot");
        let phase = phase.rem_euclid(1.0);
        let expected = local_from_node_for_size(
            CurveNode {
                x: phase,
                y: sample_editable_curve(&curve, phase).clamp(0.0, 1.0),
            },
            UiLayoutMetrics::design_space().curve_size,
        );
        let dx = (core_centers[0].x - expected.x).abs();
        let dy = (core_centers[0].y - expected.y).abs();
        assert!(
            dx <= 6 && dy <= 2,
            "playhead dot should stay near sampled curve point at host phase (expected {expected:?}, got {:?}, dx={dx}, dy={dy})",
            core_centers[0]
        );
    }

    #[test]
    fn reduction_meter_is_empty_at_zero_db_and_fills_top_down_under_reduction() {
        let metrics = UiLayoutMetrics::design_space();
        let theme = PumpTheme::main(metrics);
        let size = Size {
            width: metrics.meter_panel_width,
            height: 64,
        };
        let unity_commands = gain_reduction_meter_commands(size, theme, 0.0);
        assert!(
            fill_rects_for_color(&unity_commands, theme.meter_fill).is_empty(),
            "gain reduction meter should be empty at unity gain"
        );

        let reduction_db = crate::gui_status::GAIN_REDUCTION_METER_MAX_DB * 0.5;
        let reduced_commands = gain_reduction_meter_commands(size, theme, reduction_db);
        let rects = fill_rects_for_color(&reduced_commands, theme.meter_fill);
        assert_eq!(rects.len(), 1, "expected one gain-reduction fill rect");
        let (fill_origin, fill_size) = rects[0];
        let meter_width = (METER_WIDTH as u32).min(size.width);
        let meter_stroke_u32 = METER_STROKE.max(1) as u32;
        let expected_fill_height =
            (size.height.saturating_sub(meter_stroke_u32 * 2) as f32 * 0.5).round() as u32;
        let expected_fill_origin = Point {
            x: ((size.width - meter_width) / 2 + meter_stroke_u32) as i32,
            y: meter_stroke_u32 as i32,
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
    fn incoming_waveform_toggle_controls_capture_and_empty_underlay_state() {
        let status = Arc::new(GuiStatus::default());
        let mut state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::clone(&status),
            Arc::new(AutomationQueue::default()),
            None,
        );
        assert!(!status.incoming_waveform_enabled());

        state.reduce_action(UiAction::ToggleChanged {
            key: INCOMING_WAVEFORM_KEY.to_string(),
            value: true,
        });
        assert!(status.incoming_waveform_enabled());

        let metrics = UiLayoutMetrics::design_space();
        let theme = PumpTheme::main(metrics);
        let commands = incoming_waveform_underlay_commands(metrics.curve_size, theme, None);
        assert_eq!(commands.len(), 1, "unavailable input should draw only the stable background");
        assert!(matches!(commands[0], SurfaceCommand::FillRect { .. }));

        state.reduce_action(UiAction::ToggleChanged {
            key: INCOMING_WAVEFORM_KEY.to_string(),
            value: false,
        });
        assert!(!status.incoming_waveform_enabled());
    }

    #[test]
    fn incoming_waveform_underlay_is_bounded_and_phase_aligned() {
        let metrics = UiLayoutMetrics::design_space();
        let theme = PumpTheme::main(metrics);
        let mut waveform = [0.0; crate::incoming_waveform::INCOMING_WAVEFORM_BIN_COUNT];
        waveform[0] = 1.0;
        waveform[crate::incoming_waveform::INCOMING_WAVEFORM_BIN_COUNT - 1] = 0.5;

        let commands =
            incoming_waveform_underlay_commands(metrics.curve_size, theme, Some(waveform));
        assert_eq!(
            commands.len(),
            1 + 2 * (crate::incoming_waveform::INCOMING_WAVEFORM_BIN_COUNT - 1)
        );
        let first_line = commands.iter().find_map(|command| match command {
            SurfaceCommand::Line { start, .. } => Some(start),
            _ => None,
        }).expect("waveform should emit line segments");
        let last_line_end = commands.iter().rev().find_map(|command| match command {
            SurfaceCommand::Line { end, .. } => Some(end),
            _ => None,
        }).expect("waveform should end with a line segment");
        assert_eq!(first_line.x, 0);
        assert_eq!(last_line_end.x, metrics.curve_size.width.saturating_sub(1) as i32);
    }

    #[test]
    fn beat_grid_tracks_short_and_long_sync_lengths() {
        let shortest = curve_beat_grid(0, 396.0);
        assert_eq!(shortest, super::CurveBeatGrid::default());

        let eighth_triplet = curve_beat_grid(1, 396.0);
        assert_eq!(eighth_triplet.major, vec![0.5]);
        assert_eq!(eighth_triplet.minor, vec![0.75]);

        let two_bars = curve_beat_grid(7, 396.0);
        assert_eq!(two_bars.major.len(), 7);
        assert!((two_bars.major[0] - 0.125).abs() < 1.0e-6);
        assert!((two_bars.major[6] - 0.875).abs() < 1.0e-6);
        assert_eq!(two_bars.minor.len(), 24);
    }

    #[test]
    fn beat_grid_thins_minor_lines_at_short_widths_without_losing_alignment() {
        let wide = curve_beat_grid(7, 396.0);
        let narrow = curve_beat_grid(7, 64.0);
        assert!(narrow.minor.len() < wide.minor.len());
        assert_eq!(narrow.major, wide.major);
        assert!(narrow
            .minor
            .iter()
            .chain(narrow.major.iter())
            .all(|position| (position * 8.0 * 4.0).fract().abs() < 1.0e-5));
    }

    #[test]
    fn beat_grid_has_stable_empty_behavior_for_unsupported_timing() {
        assert_eq!(
            curve_beat_grid(crate::params::SYNC_DIVISIONS.len(), 396.0),
            super::CurveBeatGrid::default()
        );
        assert_eq!(curve_beat_grid(4, 0.0), super::CurveBeatGrid::default());
    }

    #[test]
    fn beat_grid_commands_span_the_curve_and_distinguish_major_lines() {
        let size = Size {
            width: 200,
            height: 80,
        };
        let theme = PumpTheme::main(UiLayoutMetrics::design_space());
        let commands = curve_beat_grid_commands(size, theme, 7);
        let verticals: Vec<_> = commands
            .iter()
            .filter_map(|command| match command {
                SurfaceCommand::Line { start, end, color } if start.x == end.x => {
                    Some((*start, *end, *color))
                }
                _ => None,
            })
            .collect();
        assert!(!verticals.is_empty());
        assert!(verticals.iter().all(|(start, end, _)| start.y == 0
            && end.y == size.height.saturating_sub(1) as i32));
        assert!(verticals
            .iter()
            .any(|(_, _, color)| *color == theme.curve_grid_emphasis));
        assert!(verticals
            .iter()
            .any(|(_, _, color)| *color == theme.curve_grid_vertical));
    }

    #[test]
    fn gain_references_use_curve_linear_gain_mapping_and_silence_floor() {
        let references = curve_gain_references();
        assert_eq!(
            references.map(|reference| reference.label),
            ["0 dB", "−6 dB", "−12 dB", "−∞"]
        );
        assert!((references[0].gain - crate::dsp::db_to_linear(0.0)).abs() < 1.0e-6);
        assert!((references[1].gain - crate::dsp::db_to_linear(-6.0)).abs() < 1.0e-6);
        assert!((references[2].gain - crate::dsp::db_to_linear(-12.0)).abs() < 1.0e-6);
        assert_eq!(references[3].gain, 0.0);
    }

    #[test]
    fn gain_reference_commands_position_and_label_all_guides() {
        let size = Size {
            width: 200,
            height: 80,
        };
        let metrics = UiLayoutMetrics::design_space();
        let theme = PumpTheme::main(metrics);
        let line_commands = curve_gain_reference_line_commands(size, theme);
        let guide_y_positions: Vec<_> = line_commands
            .iter()
            .filter_map(|command| match command {
                SurfaceCommand::Line { start, end, color }
                    if *color == theme.curve_reference_line && start.y == end.y =>
                {
                    Some(start.y)
                }
                _ => None,
            })
            .collect();
        let label_commands = curve_gain_reference_label_commands(size, theme, metrics.text_scale);
        let labels: Vec<_> = label_commands
            .iter()
            .filter_map(|command| match command {
                SurfaceCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let expected_y_positions: Vec<_> = curve_gain_references()
            .iter()
            .map(|reference| {
                local_from_node_for_size(
                    CurveNode {
                        x: 0.0,
                        y: reference.gain,
                    },
                    size,
                )
                .y
            })
            .collect();

        assert_eq!(guide_y_positions, expected_y_positions);
        assert_eq!(labels, ["0 dB", "-6 dB", "-12 dB", "-INF"]);
        assert_eq!(guide_y_positions[0], 0);
        assert_eq!(guide_y_positions[3], size.height as i32 - 1);
        assert!(line_commands.iter().all(|command| {
            matches!(command, SurfaceCommand::Line { start, end, .. }
                if start.x == 0 && end.x == size.width as i32 - 1)
        }));
        assert!(label_commands
            .iter()
            .all(|command| !matches!(command, SurfaceCommand::Line { .. })));
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
    fn radiant_embedded_gui_surface_renders_at_pump_design_size() {
        use radiant::gui::types::Vector2;

        let frame = radiant_editor_frame_for_params(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Vector2::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
        );

        let stats = frame.paint_plan.stats();
        let version_label = build_version_label();
        assert!(stats.total > 0, "Radiant frame should emit paint primitives");
        assert!(stats.text > 0, "Radiant frame should emit text primitives");
        assert!(frame.paint_plan.primitives.iter().any(|primitive| {
            matches!(primitive, radiant::runtime::PaintPrimitive::Text(text) if text.text.as_str() == version_label)
        }));
        assert!(!frame.paint_plan.primitives.iter().any(|primitive| {
            matches!(primitive, radiant::runtime::PaintPrimitive::Text(text) if text.text.eq_ignore_ascii_case("pump"))
        }));
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
        let (header_h, curve_h, quick_shapes_h, controls_h) =
            resolve_vertical_slot_heights(WINDOW_HEIGHT);
        assert_eq!(header_h, 19);
        assert_eq!(curve_h, 165);
        assert_eq!(quick_shapes_h, 25);
        assert_eq!(controls_h, 73);
        assert_eq!(header_h + curve_h + quick_shapes_h + controls_h, WINDOW_HEIGHT);
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
        let (header_h, curve_h, quick_shapes_h, controls_h) = resolve_vertical_slot_heights(259);
        assert_eq!(header_h + curve_h + quick_shapes_h + controls_h, 259);

        let (knobs_w, dropdown_w) = resolve_runtime_controls_slot_widths(799);
        assert_eq!(knobs_w + dropdown_w, 799);
    }

    #[test]
    fn build_ui_reserves_meter_width_from_curve_editor_extent() {
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
        let metrics = UiLayoutMetrics::design_space();
        assert_eq!(curve_editor_layout, LayoutBox::fill());
        assert!(metrics.curve_size.width < metrics.content_w);
        assert!(metrics.curve_reference_gutter_width > 0);
        assert_eq!(
            metrics.curve_reference_gutter_width
                + metrics.curve_size.width
                + metrics.meter_panel_width,
            metrics.content_w,
            "reference gutter, curve viewport, and meter should consume the full row"
        );
        assert_eq!(
            state.runtime.lock().expect("runtime lock should succeed").curve_size,
            metrics.curve_size
        );
    }

    #[test]
    fn visual_curve_right_edge_targets_wrapped_endpoint() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let metrics = UiLayoutMetrics::design_space();
        let right_endpoint = Point {
            x: metrics.curve_size.width.saturating_sub(1) as i32,
            y: 0,
        };

        state.reduce_curve_interaction(
            RegionInteractionKind::Pressed,
            right_endpoint,
            right_endpoint,
            false,
        );

        let runtime = state.runtime.lock().expect("runtime lock should succeed");
        assert_eq!(runtime.curve_size, metrics.curve_size);
        assert_eq!(
            runtime.selected_node,
            Some(params.editable_curve_snapshot().nodes.len() - 1),
            "the visual right edge must map to phase 1 instead of the pre-meter full width"
        );
    }

    #[test]
    fn build_ui_exposes_snap_checkbox_and_grid_override_controls() {
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

        let snap_region =
            find_region_node(spec.root.content(), SNAP_KEY).expect("snap checkbox should exist");
        let Node::Region(snap_region) = snap_region else {
            panic!("expected snap control node to be a region");
        };
        let theme = PumpTheme::main(UiLayoutMetrics::design_space());
        assert!(matches!(
            snap_region.draw.first(),
            Some(SurfaceCommand::FillRect { color, .. }) if *color == theme.snap_checkbox_bg
        ));
        let grid_dropdown = find_dropdown_spec(spec.root.content(), GRID_OVERRIDE_KEY)
            .expect("grid override dropdown should exist");
        assert_eq!(grid_dropdown.selected, 0, "grid override should default to Auto");
    }

    #[test]
    fn build_ui_uses_effective_grid_for_curve_overlay_and_snap() {
        let params = Arc::new(PumpParams::new());
        params.set_sync_division(4.0);
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        state.reduce_action(UiAction::ToggleChanged {
            key: SNAP_KEY.to_string(),
            value: true,
        });
        state.reduce_action(UiAction::DropdownSelected {
            key: GRID_OVERRIDE_KEY.to_string(),
            index: 3,
        });

        let spec = state.build_ui(&InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        });
        let curve_editor = find_curve_editor_spec(spec.root.content(), CURVE_KEY)
            .expect("curve editor should exist");
        assert_eq!(
            curve_editor.grid.emphasized_verticals,
            vec![0.5],
            "a sub-beat grid override should retain a stable major midpoint"
        );
        assert!(curve_editor.interaction.snap.enabled);
        assert_eq!(
            curve_editor.interaction.snap.vertical_positions,
            vec![0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.0]
        );
        assert_eq!(
            curve_editor.interaction.snap.horizontal_positions,
            vec![0.0, 0.25, 0.5, 0.75, 1.0]
        );
    }

    #[test]
    fn build_ui_temporary_snap_invert_uses_held_s_key() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        state.reduce_action(UiAction::ToggleChanged {
            key: SNAP_KEY.to_string(),
            value: true,
        });

        let mut input = InputState {
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        };
        input.held_shortcut_keys.push('s');

        let spec = state.build_ui(&input);
        let curve_editor = find_curve_editor_spec(spec.root.content(), CURVE_KEY)
            .expect("curve editor should exist");
        assert!(
            !curve_editor.interaction.snap.enabled,
            "holding s should temporarily invert the global snap state"
        );
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
    fn build_ui_root_content_is_four_slot_column() {
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
        assert_eq!(root_grid.children().len(), 4);
        let header_panel = expect_slot_panel(&root_grid.children()[0], "header");
        let _curve_panel = expect_slot_panel(&root_grid.children()[1], "curve");
        let _quick_shapes_panel = expect_slot_panel(&root_grid.children()[2], "quick-shapes");
        let header_grid = match expect_slot_child(header_panel.content(), "header") {
            Node::Grid(grid) => grid,
            other => panic!("expected header row grid in panel, got {other:?}"),
        };
        assert_eq!(header_grid.kind(), GridKind::SlotRow);
        assert_eq!(header_grid.children().len(), 2);

        let controls_panel = expect_slot_panel(&root_grid.children()[3], "controls");
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

        for expected in ["Mix", "Phase", "Output", "dB", "0"] {
            assert!(
                texts.iter().any(|text| text == expected),
                "expected textbox caption `{expected}` in {:?}",
                texts
            );
        }

        let version_label = build_version_label();
        assert!(
            texts.iter().any(|text| text == &version_label),
            "expected build version label `{version_label}` in {:?}",
            texts
        );
        assert!(
            !texts.iter().any(|text| text.eq_ignore_ascii_case("pump")),
            "expected no visible pump label in {:?}",
            texts
        );
    }

    #[test]
    fn build_ui_includes_all_quick_slot_regions() {
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
        let mut keys = Vec::new();
        collect_region_keys(spec.root.content(), &mut keys);

        for index in 0..GLOBAL_CURVE_SLOT_COUNT {
            let expected = format!("{QUICK_SLOT_KEY_PREFIX}{index}");
            assert!(
                keys.iter().any(|key| key == &expected),
                "expected quick-slot region `{expected}` in {:?}",
                keys
            );
        }
    }

    #[test]
    fn build_ui_uses_uniform_widths_for_quick_slot_regions() {
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
        let expected_width = UiLayoutMetrics::design_space().quick_shape_button_w;
        let expected_height = UiLayoutMetrics::design_space().quick_shape_button_h;

        for index in 0..GLOBAL_CURVE_SLOT_COUNT {
            let key = format!("{QUICK_SLOT_KEY_PREFIX}{index}");
            let region =
                find_region_node(spec.root.content(), &key).expect("quick-slot region should exist");
            let Node::Region(region) = region else {
                panic!("expected quick-slot node to be a region");
            };
            assert_eq!(region.layout.width(), Length::Px(expected_width));
            assert_eq!(region.layout.min_width(), Some(expected_width));
            assert_eq!(region.layout.height(), Length::Px(expected_height));
            assert_eq!(region.layout.min_height(), Some(expected_height));
        }
    }

    #[test]
    fn quick_slot_draw_commands_use_store_hover_palette_when_command_is_held() {
        let metrics = UiLayoutMetrics::design_space();
        let theme = PumpTheme::main(metrics);
        let size = Size {
            width: metrics.quick_shape_button_w,
            height: metrics.quick_shape_button_h,
        };
        let curve = crate::curve::default_editable_curve();
        let commands = GuiState::build_quick_slot_draw_commands(
            Some(&curve),
            size,
            theme,
            true,
            false,
            true,
            false,
        );

        assert!(matches!(
            commands.first(),
            Some(SurfaceCommand::FillRect { color, .. })
                if *color == theme.quick_slot_store_hover_bg
        ));
        assert!(matches!(
            commands.get(1),
            Some(SurfaceCommand::StrokeRect { color, .. })
                if *color == theme.quick_slot_outline_store_hover
        ));
    }

    #[test]
    fn quick_slot_draw_commands_use_deviation_palette_when_loaded_curve_differs() {
        let metrics = UiLayoutMetrics::design_space();
        let theme = PumpTheme::main(metrics);
        let size = Size {
            width: metrics.quick_shape_button_w,
            height: metrics.quick_shape_button_h,
        };
        let curve = crate::curve::default_editable_curve();
        let commands = GuiState::build_quick_slot_draw_commands(
            Some(&curve),
            size,
            theme,
            false,
            true,
            false,
            true,
        );

        assert!(matches!(
            commands.first(),
            Some(SurfaceCommand::FillRect { color, .. })
                if *color == theme.quick_slot_deviation_bg
        ));
        assert!(matches!(
            commands.get(1),
            Some(SurfaceCommand::StrokeRect { color, .. })
                if *color == theme.quick_slot_outline_deviation
        ));
    }

    #[test]
    fn dropdown_change_preserves_curve() {
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

        assert_ne!(
            params.sync_division(),
            previous_division,
            "division should still update on dropdown selection"
        );
        assert_eq!(
            params.editable_curve_snapshot(),
            custom_curve,
            "division changes must not mutate the editable curve"
        );
    }

    #[test]
    fn snap_checkbox_draw_commands_light_when_enabled() {
        let metrics = UiLayoutMetrics::design_space();
        let theme = PumpTheme::main(metrics);
        let size = Size {
            width: metrics.button_control_h.saturating_sub(8).max(12),
            height: metrics.button_control_h.saturating_sub(8).max(12),
        };
        let commands = GuiState::build_snap_checkbox_draw_commands(size, theme, true, false);

        assert!(matches!(
            commands.first(),
            Some(SurfaceCommand::FillRect { color, .. })
                if *color == theme.snap_checkbox_active_bg
        ));
        assert!(matches!(
            commands.get(1),
            Some(SurfaceCommand::StrokeRect { color, .. })
                if *color == theme.snap_checkbox_outline_hover
        ));
    }

    #[test]
    fn snap_checkbox_press_updates_editor_local_state_only() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );

        state.reduce_action(UiAction::RegionInteracted {
            key: SNAP_KEY.to_string(),
            kind: RegionInteractionKind::Pressed,
            local_pointer: Point { x: 0, y: 0 },
            raw_local_pointer: Point { x: 0, y: 0 },
            alt_down: false,
            command_down: false,
            shift_down: false,
        });
        let controls = state.snapshot_controls();
        assert!(controls.snap_enabled, "snap checkbox should update GUI runtime state");
        assert_eq!(
            params.editable_curve_snapshot(),
            crate::curve::default_editable_curve(),
            "pressing the snap checkbox should not mutate the stored curve"
        );
    }

    #[test]
    fn grid_override_dropdown_updates_editor_local_state_only() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );

        state.reduce_action(UiAction::DropdownSelected {
            key: GRID_OVERRIDE_KEY.to_string(),
            index: 3,
        });
        let controls = state.snapshot_controls();
        assert_eq!(controls.grid_override, Some(2));
        assert_eq!(
            params.sync_division(),
            crate::params::DEFAULT_SYNC_DIVISION_INDEX,
            "grid override should not change the audio sync division"
        );
    }

    #[test]
    fn undo_and_redo_hotkeys_restore_previous_mix_value() {
        let path = std::env::temp_dir().join(format!(
            "pump-gui-history-persistence-{}",
            std::process::id()
        ));
        with_test_persistence_path(path.clone(), || {
            let params = Arc::new(PumpParams::new());
            let mut state = GuiState::new(
                Arc::clone(&params),
                Arc::new(GuiStatus::default()),
                Arc::new(AutomationQueue::default()),
                None,
            );
            let original_mix = params.mix();

            state.reduce_action(UiAction::KnobChanged {
                key: "mix".to_string(),
                value: 0.17,
            });
            assert!((params.mix() - 0.17).abs() < 1.0e-6);

            with_test_persistence_failure(TestPersistenceFailure::WriteTemporary, || {
                state.reduce_action(UiAction::ButtonPressed {
                    key: UNDO_KEY.to_string(),
                });
            });
            assert!((params.mix() - original_mix).abs() < 1.0e-6);
            assert_eq!(params.preset_persistence_warning(), None);

            with_test_persistence_failure(TestPersistenceFailure::WriteTemporary, || {
                state.reduce_action(UiAction::ButtonPressed {
                    key: REDO_KEY.to_string(),
                });
            });
            assert!((params.mix() - 0.17).abs() < 1.0e-6);
        });
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn failed_preset_undo_remains_available_for_retry() {
        let path = std::env::temp_dir().join(format!(
            "pump-gui-preset-history-retry-{}",
            std::process::id()
        ));
        with_test_persistence_path(path.clone(), || {
            let params = Arc::new(PumpParams::new());
            let mut state = GuiState::new(
                Arc::clone(&params),
                Arc::new(GuiStatus::default()),
                Arc::new(AutomationQueue::default()),
                None,
            );
            state.reduce_action(UiAction::ButtonPressed {
                key: super::PRESET_ADD_KEY.to_string(),
            });
            assert_eq!(params.preset_bank_snapshot().presets.len(), 2);

            with_test_persistence_failure(TestPersistenceFailure::WriteTemporary, || {
                state.reduce_action(UiAction::ButtonPressed {
                    key: UNDO_KEY.to_string(),
                });
            });
            assert_eq!(params.preset_bank_snapshot().presets.len(), 2);
            assert_eq!(
                state
                    .runtime
                    .lock()
                    .expect("runtime lock should succeed")
                    .undo_history
                    .len(),
                1
            );

            state.reduce_action(UiAction::ButtonPressed {
                key: UNDO_KEY.to_string(),
            });
            assert_eq!(params.preset_bank_snapshot().presets.len(), 1);
        });
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn preset_save_overwrites_existing_name_from_rename_draft() {
        let params = Arc::new(PumpParams::new());
        params
            .add_preset_from_current_state()
            .expect("preset insertion should succeed");
        assert!(params.rename_preset(1, "Verse").is_ok());
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
    fn persistence_failure_is_actionable_in_toybox_and_radiant_surfaces() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pump-gui-persistence-warning-{}-{stamp}.bin",
            std::process::id()
        ));

        with_test_persistence_path(path.clone(), || {
            let params = Arc::new(PumpParams::new());
            let mut state = GuiState::new(
                Arc::clone(&params),
                Arc::new(GuiStatus::default()),
                Arc::new(AutomationQueue::default()),
                None,
            );

            with_test_persistence_failure(TestPersistenceFailure::WriteTemporary, || {
                state.reduce_action(UiAction::ButtonPressed {
                    key: super::PRESET_ADD_KEY.to_string(),
                });
            });
            state.reduce_action(UiAction::ButtonPressed {
                key: PRESET_RENAME_BUTTON_KEY.to_string(),
            });

            let presets = state.snapshot_presets();
            assert_eq!(presets.names[presets.selected], PRESET_WARNING_STORAGE);
            assert!(presets.persistence_warning);
            assert!(presets.rename_active);
            assert_eq!(params.preset_bank_snapshot().presets.len(), 1);

            let spec = state.build_ui(&InputState {
                window_size: Size {
                    width: WINDOW_WIDTH,
                    height: WINDOW_HEIGHT,
                },
                ..InputState::default()
            });
            let mut texts = Vec::new();
            collect_textbox_texts(spec.root.content(), &mut texts);
            assert!(texts.iter().any(|text| text == PRESET_WARNING_STORAGE));
            assert!(texts.iter().any(|text| text == "Init"));

            let frame = radiant_editor_frame_for_params(
                Arc::clone(&params),
                Arc::new(GuiStatus::default()),
                radiant::gui::types::Vector2::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
            );
            assert!(frame.paint_plan.primitives.iter().any(|primitive| {
                matches!(
                    primitive,
                    radiant::runtime::PaintPrimitive::Text(text)
                        if text.text.as_str() == PRESET_WARNING_STORAGE
                )
            }));
        });
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn init_preset_is_renamable_from_header_interaction() {
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
        assert!(runtime.preset_rename_active);
        assert_eq!(runtime.preset_name_draft, "Init");
        assert_eq!(runtime.preset_warning_text, None);
        assert_eq!(params.preset_bank_snapshot().presets[0].name, "Init");
    }

    #[test]
    fn init_save_overwrites_without_warning_or_header_relayout() {
        let params = Arc::new(PumpParams::new());
        params.set_mix(0.52);
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        state.reduce_action(UiAction::ButtonPressed {
            key: PRESET_SAVE_KEY.to_string(),
        });

        let bank = params.preset_bank_snapshot();
        assert_eq!(bank.presets.len(), 1);
        assert_eq!(bank.selected, 0);
        assert_eq!(bank.presets[0].name, "Init");
        assert!((bank.presets[0].mix - 0.52).abs() < 1.0e-6);
        {
            let runtime = state.runtime.lock().expect("runtime lock should succeed");
            assert_eq!(runtime.preset_warning_text, None);
        }

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
        assert_eq!(dropdown.selected_option_background_override, None);

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
                transport_is_playing: true,
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
                transport_is_playing: true,
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
                transport_is_playing: true,
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

        let (header_h, curve_h, quick_shapes_h, controls_h) =
            resolve_vertical_slot_heights(WINDOW_HEIGHT);
        let controls_top = header_h
            .saturating_add(curve_h)
            .saturating_add(quick_shapes_h);
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
            | Node::TabBar(_)
            | Node::EqAttractorSurface(_)
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
            | Node::TabBar(_)
            | Node::EqAttractorSurface(_)
            | Node::Region(_)
            | Node::Indicator(_) => {}
        }
    }

    fn collect_region_keys(node: &Node, keys: &mut Vec<String>) {
        match node {
            Node::Slot(slot) => collect_region_keys(slot.child(), keys),
            Node::Region(region) => keys.push(region.key.clone()),
            Node::Panel(panel) => collect_region_keys(panel.content(), keys),
            Node::PaddingBox(padding_box) => collect_region_keys(padding_box.content(), keys),
            Node::AlignBox(align_box) => collect_region_keys(align_box.content(), keys),
            Node::AspectBox(aspect_box) => collect_region_keys(aspect_box.content(), keys),
            Node::Row(flex) | Node::Column(flex) => {
                for child in flex.children() {
                    collect_region_keys(child, keys);
                }
            }
            Node::Grid(grid) => {
                for child in grid.children() {
                    collect_region_keys(child, keys);
                }
            }
            Node::Absolute(absolute) => {
                for child in absolute.children() {
                    collect_region_keys(child.node(), keys);
                }
            }
            Node::Stack(stack) => {
                for child in stack.children() {
                    collect_region_keys(child, keys);
                }
            }
            Node::ScrollView(scroll_view) => collect_region_keys(scroll_view.content(), keys),
            Node::Wrap(wrap) => {
                for child in wrap.children() {
                    collect_region_keys(child, keys);
                }
            }
            Node::SwitchLayout(switch_layout) => {
                for case_entry in switch_layout.cases() {
                    collect_region_keys(case_entry.child(), keys);
                }
                collect_region_keys(switch_layout.fallback(), keys);
            }
            Node::TextBox(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::CurveEditor(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::TabBar(_)
            | Node::EqAttractorSurface(_)
            | Node::Indicator(_) => {}
        }
    }

    fn find_region_node<'a>(node: &'a Node, key: &str) -> Option<&'a Node> {
        match node {
            Node::Slot(slot) => find_region_node(slot.child(), key),
            Node::Region(region) if region.key == key => Some(node),
            Node::Panel(panel) => find_region_node(panel.content(), key),
            Node::PaddingBox(padding_box) => find_region_node(padding_box.content(), key),
            Node::AlignBox(align_box) => find_region_node(align_box.content(), key),
            Node::AspectBox(aspect_box) => find_region_node(aspect_box.content(), key),
            Node::Row(flex) | Node::Column(flex) => {
                flex.children().iter().find_map(|child| find_region_node(child, key))
            }
            Node::Grid(grid) => grid
                .children()
                .iter()
                .find_map(|child| find_region_node(child, key)),
            Node::Absolute(absolute) => absolute
                .children()
                .iter()
                .find_map(|child| find_region_node(child.node(), key)),
            Node::Stack(stack) => stack
                .children()
                .iter()
                .find_map(|child| find_region_node(child, key)),
            Node::ScrollView(scroll_view) => find_region_node(scroll_view.content(), key),
            Node::Wrap(wrap) => wrap
                .children()
                .iter()
                .find_map(|child| find_region_node(child, key)),
            Node::SwitchLayout(switch_layout) => switch_layout
                .cases()
                .iter()
                .find_map(|case_entry| find_region_node(case_entry.child(), key))
                .or_else(|| find_region_node(switch_layout.fallback(), key)),
            Node::Region(_)
            | Node::TextBox(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::CurveEditor(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::TabBar(_)
            | Node::EqAttractorSurface(_)
            | Node::Indicator(_) => None,
        }
    }

    fn find_curve_editor_spec<'a>(node: &'a Node, key: &str) -> Option<&'a CurveEditorSpec> {
        match node {
            Node::Slot(slot) => find_curve_editor_spec(slot.child(), key),
            Node::CurveEditor(curve_editor) if curve_editor.key == key => Some(curve_editor),
            Node::Panel(panel) => find_curve_editor_spec(panel.content(), key),
            Node::PaddingBox(padding_box) => find_curve_editor_spec(padding_box.content(), key),
            Node::AlignBox(align_box) => find_curve_editor_spec(align_box.content(), key),
            Node::AspectBox(aspect_box) => find_curve_editor_spec(aspect_box.content(), key),
            Node::Row(flex) | Node::Column(flex) => {
                flex.children().iter().find_map(|child| find_curve_editor_spec(child, key))
            }
            Node::Grid(grid) => grid
                .children()
                .iter()
                .find_map(|child| find_curve_editor_spec(child, key)),
            Node::Stack(stack) => stack
                .children()
                .iter()
                .find_map(|child| find_curve_editor_spec(child, key)),
            Node::ScrollView(scroll_view) => find_curve_editor_spec(scroll_view.content(), key),
            Node::Wrap(wrap) => wrap
                .children()
                .iter()
                .find_map(|child| find_curve_editor_spec(child, key)),
            Node::SwitchLayout(switch_layout) => switch_layout
                .cases()
                .iter()
                .find_map(|case_entry| find_curve_editor_spec(case_entry.child(), key))
                .or_else(|| find_curve_editor_spec(switch_layout.fallback(), key)),
            Node::CurveEditor(_) => None,
            Node::TextBox(_)
            | Node::Spacer(_)
            | Node::Knob(_)
            | Node::Slider(_)
            | Node::Toggle(_)
            | Node::Button(_)
            | Node::Dropdown(_)
            | Node::TabBar(_)
            | Node::EqAttractorSurface(_)
            | Node::Region(_)
            | Node::Indicator(_)
            | Node::Absolute(_) => None,
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
            | Node::TabBar(_)
            | Node::EqAttractorSurface(_)
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
            | Node::TabBar(_)
            | Node::EqAttractorSurface(_)
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
            | Node::TabBar(_)
            | Node::EqAttractorSurface(_)
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
