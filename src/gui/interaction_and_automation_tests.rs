mod interaction_and_automation_tests {
    use super::*;
    use super::super::{
        find_segment_line_hit_within_for_size, segment_direct_hit_radius,
        segment_near_hit_radius, CurveDragMode, CurveMarqueeSelection, RegionInteractionKind,
        QUICK_SLOT_KEY_PREFIX,
    };
    use crate::curve::MAX_EDITABLE_NODES;
    use crate::curve_presets::quick_slot_seeds;
    use crate::params::with_test_curve_slot_path;
    use toybox::clack_plugin::events::io::EventBuffer;

    fn frame_input(mouse_down: bool) -> InputState {
        frame_input_with_buttons(mouse_down, false)
    }

    fn frame_input_with_buttons(mouse_down: bool, mouse_secondary_down: bool) -> InputState {
        InputState {
            mouse_down,
            mouse_secondary_down,
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        }
    }

    fn with_isolated_global_curve_slots<R>(label: &str, f: impl FnOnce() -> R) -> R {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pump-gui-global-slots-{label}-{}-{stamp}.bin",
            std::process::id()
        ));
        let result = with_test_curve_slot_path(path.clone(), f);
        let _ = std::fs::remove_file(path);
        result
    }

    fn drain_automation_event_count(queue: &AutomationQueue) -> usize {
        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        queue.drain_to_output(&mut output, &mut scratch).attempted
    }

    #[test]
    fn knob_drag_emits_single_begin_and_end_gesture() {
        let params = Arc::new(PumpParams::new());
        let queue = Arc::new(AutomationQueue::default());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::clone(&queue),
            None,
        );

        let _ = state.build_ui(&frame_input(true));
        state.reduce_action(UiAction::KnobChanged {
            key: super::super::MIX_KEY.to_string(),
            value: 0.35,
        });
        state.reduce_action(UiAction::KnobChanged {
            key: super::super::MIX_KEY.to_string(),
            value: 0.55,
        });
        assert_eq!(
            drain_automation_event_count(queue.as_ref()),
            3,
            "drag updates should emit begin + value stream without per-tick gesture end"
        );

        let _ = state.build_ui(&frame_input(false));
        assert_eq!(
            drain_automation_event_count(queue.as_ref()),
            1,
            "pointer release should emit one gesture end"
        );
    }

    #[test]
    fn knob_non_drag_update_emits_single_begin_value_end_triplet() {
        let params = Arc::new(PumpParams::new());
        let queue = Arc::new(AutomationQueue::default());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::clone(&queue),
            None,
        );

        let _ = state.build_ui(&frame_input(false));
        state.reduce_action(UiAction::KnobChanged {
            key: super::super::MIX_KEY.to_string(),
            value: 0.41,
        });
        assert_eq!(
            drain_automation_event_count(queue.as_ref()),
            3,
            "non-drag knob updates should keep begin/value/end semantics"
        );
    }

    #[test]
    fn knob_drag_updates_commit_one_undo_step_on_release() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let before = params.mix();
        let _ = state.build_ui(&frame_input(true));

        state.reduce_action(UiAction::KnobChanged {
            key: super::super::MIX_KEY.to_string(),
            value: 0.26,
        });
        state.reduce_action(UiAction::KnobChanged {
            key: super::super::MIX_KEY.to_string(),
            value: 0.59,
        });
        let _ = state.build_ui(&frame_input(false));
        assert!((params.mix() - 0.59).abs() < 1.0e-6);

        state.reduce_action(UiAction::ButtonPressed {
            key: UNDO_KEY.to_string(),
        });
        assert!(
            (params.mix() - before).abs() < 1.0e-6,
            "one undo should revert the entire knob drag gesture"
        );

        state.reduce_action(UiAction::ButtonPressed {
            key: REDO_KEY.to_string(),
        });
        assert!(
            (params.mix() - 0.59).abs() < 1.0e-6,
            "redo should restore the final drag result in one step"
        );
    }

    #[test]
    fn primary_release_without_region_release_clears_curve_drag_mode() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let before = params.editable_curve_snapshot();
        let start = local_from_node(before.nodes[1]);
        let moved = Point {
            x: start.x + 18,
            y: start.y - 12,
        };

        let _ = state.build_ui(&frame_input(true));
        state.reduce_curve_interaction(RegionInteractionKind::Pressed, start, start, false);
        state.reduce_curve_interaction(RegionInteractionKind::Dragged, moved, moved, false);
        let _ = state.build_ui(&frame_input(false));
        let edited = params.editable_curve_snapshot();
        assert_ne!(edited, before, "drag should modify curve before undo");

        let runtime = state.runtime.lock().expect("runtime lock should succeed");
        assert!(
            runtime.drag_mode.is_none(),
            "global primary release should clear stale curve drag mode"
        );
        drop(runtime);

        state.reduce_action(UiAction::ButtonPressed {
            key: UNDO_KEY.to_string(),
        });
        assert_eq!(
            params.editable_curve_snapshot(),
            before,
            "one undo should revert the drag even when region release event was missed"
        );
    }

    #[test]
    fn max_node_insert_attempt_does_not_push_no_op_curve_revision() {
        let mut nodes = Vec::with_capacity(MAX_EDITABLE_NODES);
        for index in 0..MAX_EDITABLE_NODES {
            nodes.push(CurveNode {
                x: index as f32 / (MAX_EDITABLE_NODES - 1) as f32,
                y: 1.0,
            });
        }
        let dense_curve = EditableCurve {
            nodes,
            segments: vec![CurveSegment { tension: 0.0 }; MAX_EDITABLE_NODES - 1],

        ..EditableCurve::default()};
        let params = Arc::new(PumpParams::new());
        params.set_editable_curve(&dense_curve);
        let before_curve = params.editable_curve_snapshot();
        let before_revision = params.curve_revision();
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );

        let _ = state.build_ui(&frame_input(true));
        let pointer = Point {
            x: (CURVE_W as i32) / 2,
            y: CURVE_H as i32 - 1,
        };
        state.reduce_curve_interaction(RegionInteractionKind::Pressed, pointer, pointer, false);

        assert_eq!(
            params.editable_curve_snapshot(),
            before_curve,
            "insert attempts at max node count should preserve the existing curve"
        );
        assert_eq!(
            params.curve_revision(),
            before_revision,
            "insert no-op must not bump curve revision or create an undo step"
        );
    }

    #[test]
    fn curve_press_drag_release_updates_curve_and_resets_drag_mode() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let initial = params.editable_curve_snapshot();
        let start = local_from_node(initial.nodes[1]);
        state.reduce_curve_interaction(RegionInteractionKind::Pressed, start, start, false);

        let moved = Point {
            x: start.x + 36,
            y: start.y - 18,
        };
        state.reduce_curve_interaction(RegionInteractionKind::Dragged, moved, moved, false);
        let updated = params.editable_curve_snapshot();
        assert_ne!(
            updated.nodes, initial.nodes,
            "dragging a selected node should mutate editable curve nodes"
        );

        state.reduce_curve_interaction(RegionInteractionKind::Released, moved, moved, false);
        let runtime = state.runtime.lock().expect("runtime lock should succeed");
        assert!(
            runtime.drag_mode.is_none(),
            "release should end the active curve drag mode"
        );
    }

    #[test]
    fn command_point_drag_snaps_time_and_releases_to_continuous_movement() {
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

        ..EditableCurve::default()};
        params.set_editable_curve(&curve);
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let size = UiLayoutMetrics::design_space().curve_size;
        let start = local_from_node_for_size(curve.nodes[1], size);
        let target = local_from_node_for_size(CurveNode { x: 0.34, y: 0.7 }, size);

        state.reduce_action(UiAction::RegionInteracted {
            key: CURVE_KEY.to_string(),
            kind: RegionInteractionKind::Pressed,
            local_pointer: start,
            raw_local_pointer: start,
            alt_down: false,
            command_down: false,
            shift_down: false,
        });
        state.reduce_action(UiAction::RegionInteracted {
            key: CURVE_KEY.to_string(),
            kind: RegionInteractionKind::Dragged,
            local_pointer: target,
            raw_local_pointer: target,
            alt_down: false,
            command_down: true,
            shift_down: false,
        });
        let snapped = params.editable_curve_snapshot().nodes[1];
        let expected_snap = snap_curve_time_to_beat_grid(6, size.width as f32, 0.34);
        assert!((snapped.x - expected_snap).abs() < 1.0e-6);

        state.reduce_action(UiAction::RegionInteracted {
            key: CURVE_KEY.to_string(),
            kind: RegionInteractionKind::Dragged,
            local_pointer: target,
            raw_local_pointer: target,
            alt_down: false,
            command_down: false,
            shift_down: false,
        });
        let released = params.editable_curve_snapshot().nodes[1];
        let raw_target = node_from_local_for_size(target, size);
        assert!((released.x - raw_target.x).abs() < 1.0e-6);
        assert!((released.y - snapped.y).abs() < 1.0e-6);
    }

    #[test]
    fn command_empty_canvas_insert_snaps_time_without_snapping_gain() {
        let params = Arc::new(PumpParams::new());
        params.set_sync_division(6.0);
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let size = UiLayoutMetrics::design_space().curve_size;
        let target_node = CurveNode { x: 0.34, y: 0.08 };
        let target = local_from_node_for_size(target_node, size);
        let raw_target = node_from_local_for_size(target, size);

        state.reduce_action(UiAction::RegionInteracted {
            key: CURVE_KEY.to_string(),
            kind: RegionInteractionKind::Pressed,
            local_pointer: target,
            raw_local_pointer: target,
            alt_down: false,
            command_down: true,
            shift_down: false,
        });

        let inserted = params
            .editable_curve_snapshot()
            .nodes
            .into_iter()
            .find(|node| (node.y - raw_target.y).abs() < 1.0e-6)
            .expect("command insertion should preserve the pointer gain");
        assert!((inserted.x - snap_curve_time_to_beat_grid(6, size.width as f32, raw_target.x)).abs() < 1.0e-6);
    }

    #[test]
    fn curve_region_segment_move_requires_command_modifier() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.9 },
                CurveNode { x: 0.25, y: 0.4 },
                CurveNode { x: 0.75, y: 0.4 },
                CurveNode { x: 1.0, y: 0.9 },
            ],
            segments: vec![
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
            ],

        ..EditableCurve::default()};
        let curve_size = UiLayoutMetrics::design_space().curve_size;
        let mut pointer = local_from_node_for_size(CurveNode { x: 0.5, y: 0.4 }, curve_size);
        pointer.y += 6;
        assert_eq!(
            find_segment_line_hit_within_for_size(
                &curve,
                pointer,
                segment_near_hit_radius(curve_size),
                curve_size,
            ),
            Some(1),
            "regression pointer must be inside the segment move radius"
        );
        assert_eq!(
            find_segment_line_hit_within_for_size(
                &curve,
                pointer,
                segment_direct_hit_radius(curve_size),
                curve_size,
            ),
            None,
            "regression pointer must stay outside the direct insertion radius"
        );

        let plain_params = Arc::new(PumpParams::new());
        plain_params.set_editable_curve(&curve);
        let mut plain_state = GuiState::new(
            Arc::clone(&plain_params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        plain_state.reduce_action(UiAction::RegionInteracted {
            key: CURVE_KEY.to_string(),
            kind: RegionInteractionKind::Pressed,
            local_pointer: pointer,
            raw_local_pointer: pointer,
            alt_down: false,
            command_down: false,
            shift_down: false,
        });
        assert!(
            !matches!(
                plain_state
                    .runtime
                    .lock()
                    .expect("runtime lock should succeed")
                    .drag_mode,
                Some(CurveDragMode::MoveSegment { .. })
            ),
            "an unmodified press near a segment must not start grouped movement"
        );

        let command_params = Arc::new(PumpParams::new());
        command_params.set_editable_curve(&curve);
        let mut command_state = GuiState::new(
            Arc::clone(&command_params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        command_state.reduce_action(UiAction::RegionInteracted {
            key: CURVE_KEY.to_string(),
            kind: RegionInteractionKind::Pressed,
            local_pointer: pointer,
            raw_local_pointer: pointer,
            alt_down: false,
            command_down: true,
            shift_down: false,
        });
        assert!(matches!(
            command_state
                .runtime
                .lock()
                .expect("runtime lock should succeed")
                .drag_mode,
            Some(CurveDragMode::MoveSegment { index: 1, .. })
        ));

        let moved = Point {
            x: pointer.x + 20,
            y: pointer.y - 12,
        };
        command_state.reduce_action(UiAction::RegionInteracted {
            key: CURVE_KEY.to_string(),
            kind: RegionInteractionKind::Dragged,
            local_pointer: moved,
            raw_local_pointer: moved,
            alt_down: false,
            command_down: true,
            shift_down: false,
        });
        let translated = command_params.editable_curve_snapshot();
        assert!(translated.nodes[1].x > curve.nodes[1].x);
        assert!(translated.nodes[1].y > curve.nodes[1].y);
        assert!((translated.nodes[2].x - translated.nodes[1].x - 0.5).abs() < 1.0e-6);
        assert!((translated.nodes[2].y - translated.nodes[1].y).abs() < 1.0e-6);

        let canceled_params = Arc::new(PumpParams::new());
        canceled_params.set_editable_curve(&curve);
        let mut canceled_state = GuiState::new(
            Arc::clone(&canceled_params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        canceled_state.reduce_action(UiAction::RegionInteracted {
            key: CURVE_KEY.to_string(),
            kind: RegionInteractionKind::Pressed,
            local_pointer: pointer,
            raw_local_pointer: pointer,
            alt_down: false,
            command_down: true,
            shift_down: false,
        });
        canceled_state.reduce_action(UiAction::RegionInteracted {
            key: CURVE_KEY.to_string(),
            kind: RegionInteractionKind::Dragged,
            local_pointer: moved,
            raw_local_pointer: moved,
            alt_down: false,
            command_down: false,
            shift_down: false,
        });
        assert_eq!(
            canceled_params.editable_curve_snapshot(),
            curve,
            "releasing Command before movement must cancel the grouped drag"
        );
        assert!(
            canceled_state
                .runtime
                .lock()
                .expect("runtime lock should succeed")
                .drag_mode
                .is_none(),
            "a modifier-canceled grouped drag must not remain active"
        );
    }

    #[test]
    fn curve_double_click_deletes_interior_node() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let before = params.editable_curve_snapshot();
        let target = local_from_node(before.nodes[1]);
        state.reduce_curve_interaction(RegionInteractionKind::DoubleClicked, target, target, false);
        let after = params.editable_curve_snapshot();
        assert_eq!(
            after.nodes.len() + 1,
            before.nodes.len(),
            "double-clicking an interior node should remove it"
        );
    }

    #[test]
    fn curve_editor_changed_with_deleted_interior_node_updates_params() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let before = params.editable_curve_snapshot();
        let mut model = toybox::gui::declarative::CurveModel::new(
            before
                .nodes
                .iter()
                .map(|node| toybox::gui::declarative::CurvePoint::new(node.x, node.y))
                .collect(),
            before
                .segments
                .iter()
                .map(|segment| toybox::gui::declarative::CurveSegment::new(segment.tension))
                .collect(),
        );
        model.points.remove(1);
        model.segments.remove(0);

        state.reduce_action(UiAction::CurveEditorChanged {
            key: CURVE_KEY.to_string(),
            model,
        });

        let after = params.editable_curve_snapshot();
        assert_eq!(
            after.nodes.len() + 1,
            before.nodes.len(),
            "curve editor changed action should preserve interior deletion"
        );
    }

    #[test]
    fn curve_drag_updates_commit_one_undo_step_on_release() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let before = params.editable_curve_snapshot();

        let _ = state.build_ui(&frame_input(true));
        let mut model = toybox::gui::declarative::CurveModel::new(
            before
                .nodes
                .iter()
                .map(|node| toybox::gui::declarative::CurvePoint::new(node.x, node.y))
                .collect(),
            before
                .segments
                .iter()
                .map(|segment| toybox::gui::declarative::CurveSegment::new(segment.tension))
                .collect(),
        );
        model.points[1].x = 0.24;
        model.points[1].y = 0.16;
        state.reduce_action(UiAction::CurveEditorChanged {
            key: CURVE_KEY.to_string(),
            model: model.clone(),
        });

        model.points[1].x = 0.31;
        model.points[1].y = 0.29;
        state.reduce_action(UiAction::CurveEditorChanged {
            key: CURVE_KEY.to_string(),
            model,
        });

        let _ = state.build_ui(&frame_input(false));
        let edited = params.editable_curve_snapshot();
        assert_ne!(edited, before, "drag should modify curve before undo");

        state.reduce_action(UiAction::ButtonPressed {
            key: UNDO_KEY.to_string(),
        });
        assert_eq!(
            params.editable_curve_snapshot(),
            before,
            "one undo should revert the entire drag gesture"
        );

        state.reduce_action(UiAction::ButtonPressed {
            key: REDO_KEY.to_string(),
        });
        assert_eq!(
            params.editable_curve_snapshot(),
            edited,
            "redo should restore the final drag result in one step"
        );
    }

    #[test]
    fn region_drag_updates_commit_one_undo_step_on_release() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let before = params.editable_curve_snapshot();
        let start = local_from_node(before.nodes[1]);
        let moved = Point {
            x: start.x + 16,
            y: start.y + 12,
        };

        state.reduce_curve_interaction(RegionInteractionKind::Pressed, start, start, false);
        state.reduce_curve_interaction(RegionInteractionKind::Dragged, moved, moved, false);
        state.reduce_curve_interaction(RegionInteractionKind::Released, moved, moved, false);
        let edited = params.editable_curve_snapshot();
        assert_ne!(edited, before, "drag should modify curve before undo");

        state.reduce_action(UiAction::ButtonPressed {
            key: UNDO_KEY.to_string(),
        });
        assert_eq!(
            params.editable_curve_snapshot(),
            before,
            "one undo should revert the entire region drag gesture"
        );

        state.reduce_action(UiAction::ButtonPressed {
            key: REDO_KEY.to_string(),
        });
        assert_eq!(
            params.editable_curve_snapshot(),
            edited,
            "redo should restore the final region drag result in one step"
        );
    }

    #[test]
    fn secondary_drag_marquee_selects_enclosed_curve_nodes() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let editable = params.editable_curve_snapshot();
        let node_a = local_from_node(editable.nodes[1]);
        let node_b = local_from_node(editable.nodes[2]);
        let start = Point {
            x: node_a.x.min(node_b.x) - 8,
            y: node_a.y.min(node_b.y) - 8,
        };
        let end = Point {
            x: node_a.x.max(node_b.x) + 8,
            y: node_a.y.max(node_b.y) + 8,
        };

        let _ = state.build_ui(&frame_input_with_buttons(false, false));
        state.reduce_action(UiAction::RegionHover {
            key: CURVE_KEY.to_string(),
            hovered: true,
            local_pointer: start,
        });
        state.reduce_curve_interaction(RegionInteractionKind::SecondaryClicked, start, start, false);

        let _ = state.build_ui(&frame_input_with_buttons(false, true));
        state.reduce_action(UiAction::RegionHover {
            key: CURVE_KEY.to_string(),
            hovered: true,
            local_pointer: end,
        });

        let _ = state.build_ui(&frame_input_with_buttons(false, false));
        state.reduce_action(UiAction::RegionHover {
            key: CURVE_KEY.to_string(),
            hovered: true,
            local_pointer: end,
        });

        let runtime = state.runtime.lock().expect("runtime lock should succeed");
        assert_eq!(
            runtime.selected_nodes,
            vec![1, 2],
            "secondary drag marquee should select enclosed interior nodes"
        );
    }

    #[test]
    fn dragging_one_marquee_selected_node_moves_the_whole_selection() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let before = params.editable_curve_snapshot();
        let node_a = local_from_node(before.nodes[1]);
        let node_b = local_from_node(before.nodes[2]);
        let marquee_start = Point {
            x: node_a.x.min(node_b.x) - 8,
            y: node_a.y.min(node_b.y) - 8,
        };
        let marquee_end = Point {
            x: node_a.x.max(node_b.x) + 8,
            y: node_a.y.max(node_b.y) + 8,
        };

        let _ = state.build_ui(&frame_input_with_buttons(false, false));
        state.reduce_action(UiAction::RegionHover {
            key: CURVE_KEY.to_string(),
            hovered: true,
            local_pointer: marquee_start,
        });
        state.reduce_curve_interaction(
            RegionInteractionKind::SecondaryClicked,
            marquee_start,
            marquee_start,
            false,
        );
        let _ = state.build_ui(&frame_input_with_buttons(false, true));
        state.reduce_action(UiAction::RegionHover {
            key: CURVE_KEY.to_string(),
            hovered: true,
            local_pointer: marquee_end,
        });
        let _ = state.build_ui(&frame_input_with_buttons(false, false));
        state.reduce_action(UiAction::RegionHover {
            key: CURVE_KEY.to_string(),
            hovered: true,
            local_pointer: marquee_end,
        });

        let drag_start = local_from_node(before.nodes[1]);
        let drag_end = Point {
            x: drag_start.x + 12,
            y: drag_start.y - 20,
        };
        state.reduce_curve_interaction(RegionInteractionKind::Pressed, drag_start, drag_start, false);
        state.reduce_curve_interaction(RegionInteractionKind::Dragged, drag_end, drag_end, false);
        state.reduce_curve_interaction(RegionInteractionKind::Released, drag_end, drag_end, false);
        let edited = params.editable_curve_snapshot();
        let node1_delta = edited.nodes[1].y - before.nodes[1].y;
        let node2_delta = edited.nodes[2].y - before.nodes[2].y;
        assert!(
            node1_delta.abs() > 1.0e-4 && (node1_delta - node2_delta).abs() <= 1.0e-4,
            "dragging one selected node should move both selected nodes together"
        );

        state.reduce_action(UiAction::ButtonPressed {
            key: UNDO_KEY.to_string(),
        });
        assert_eq!(
            params.editable_curve_snapshot(),
            before,
            "one undo should revert the grouped marquee drag"
        );
    }

    #[test]
    fn curve_press_on_segment_inserts_preview_node() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let before = params.editable_curve_snapshot();
        let pointer_curve_x = 0.25;
        let pointer_curve_y = sample_editable_curve(&before, pointer_curve_x);
        let local = local_from_node(CurveNode {
            x: pointer_curve_x,
            y: pointer_curve_y,
        });

        state.reduce_curve_interaction(RegionInteractionKind::Pressed, local, local, false);
        let after = params.editable_curve_snapshot();
        assert_eq!(
            after.nodes.len(),
            before.nodes.len() + 1,
            "segment press should insert a preview node before drag starts"
        );
    }

    #[test]
    fn drag_back_restores_push_through_deleted_nodes_before_release() {
        let params = Arc::new(PumpParams::new());
        let mut state = GuiState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let before = params.editable_curve_snapshot();
        let moving_node = before.nodes[1];
        let crossed_node = before.nodes[2];

        let start = local_from_node(moving_node);
        state.reduce_curve_interaction(RegionInteractionKind::Pressed, start, start, false);

        let drag_right = local_from_node(CurveNode {
            x: 0.9,
            y: moving_node.y,
        });
        state.reduce_curve_interaction(RegionInteractionKind::Dragged, drag_right, drag_right, false);
        let crossed = params.editable_curve_snapshot();
        assert!(
            crossed.nodes.len() < before.nodes.len(),
            "dragging through an interior node should temporarily remove crossed nodes"
        );

        let drag_back = local_from_node(CurveNode {
            x: 0.18,
            y: moving_node.y,
        });
        state.reduce_curve_interaction(RegionInteractionKind::Dragged, drag_back, drag_back, false);
        let restored = params.editable_curve_snapshot();
        assert_eq!(
            restored.nodes.len(),
            before.nodes.len(),
            "dragging back within the same gesture should restore crossed nodes"
        );
        assert!(
            restored.nodes.iter().any(|node| {
                (node.x - crossed_node.x).abs() <= f32::EPSILON
                    && (node.y - crossed_node.y).abs() <= f32::EPSILON
            }),
            "restored topology should recover the original crossed node values"
        );

        state.reduce_curve_interaction(RegionInteractionKind::Released, drag_back, drag_back, false);
    }

    #[test]
    fn quick_slot_press_loads_curve_and_clears_selection_state() {
        with_isolated_global_curve_slots("load-clears-selection", || {
            let params = Arc::new(PumpParams::new());
            let mut state = GuiState::new(
                Arc::clone(&params),
                Arc::new(GuiStatus::default()),
                Arc::new(AutomationQueue::default()),
                None,
            );
            let slot_curve = quick_slot_seeds()[4].curve.clone();
            assert!(params.set_global_curve_slot_curve(4, &slot_curve));
            let origin_curve = params.editable_curve_snapshot();
            let before_division = params.sync_division();
            {
                let mut runtime = state.runtime.lock().expect("runtime lock should succeed");
                runtime.selected_node = Some(1);
                runtime.selected_nodes = vec![1, 2];
                runtime.drag_mode = Some(CurveDragMode::MoveNode {
                    origin_index: 1,
                    origin_curve,
                    start_pointer: Point { x: 24, y: 24 },
                    dragging: true,
                });
                runtime.marquee_selection = Some(CurveMarqueeSelection {
                    start_pointer: Point { x: 10, y: 10 },
                    current_pointer: Point { x: 30, y: 30 },
                });
            }

            state.reduce_action(UiAction::RegionInteracted {
                key: format!("{QUICK_SLOT_KEY_PREFIX}4"),
                kind: RegionInteractionKind::Pressed,
                local_pointer: Point { x: 0, y: 0 },
                raw_local_pointer: Point { x: 0, y: 0 },
                alt_down: false,
                command_down: false,
                shift_down: false,
            });

            assert_eq!(params.editable_curve_snapshot(), slot_curve);
            assert_eq!(params.sync_division(), before_division);
            let runtime = state.runtime.lock().expect("runtime lock should succeed");
            assert_eq!(runtime.selected_node, None);
            assert!(runtime.selected_nodes.is_empty());
            assert!(runtime.drag_mode.is_none());
            assert!(runtime.marquee_selection.is_none());
            assert_eq!(runtime.loaded_global_curve_slot, Some(4));
        });
    }

    #[test]
    fn quick_slot_press_commits_curve_load_as_one_undo_step() {
        with_isolated_global_curve_slots("load-undo", || {
            let params = Arc::new(PumpParams::new());
            let mut state = GuiState::new(
                Arc::clone(&params),
                Arc::new(GuiStatus::default()),
                Arc::new(AutomationQueue::default()),
                None,
            );
            let slot_curve = quick_slot_seeds()[7].curve.clone();
            assert!(params.set_global_curve_slot_curve(7, &slot_curve));
            let before_curve = params.editable_curve_snapshot();
            let before_division = params.sync_division();

            state.reduce_action(UiAction::RegionInteracted {
                key: format!("{QUICK_SLOT_KEY_PREFIX}7"),
                kind: RegionInteractionKind::Pressed,
                local_pointer: Point { x: 0, y: 0 },
                raw_local_pointer: Point { x: 0, y: 0 },
                alt_down: false,
                command_down: false,
                shift_down: false,
            });
            assert_eq!(params.editable_curve_snapshot(), slot_curve);
            assert_eq!(params.sync_division(), before_division);

            state.reduce_action(UiAction::ButtonPressed {
                key: UNDO_KEY.to_string(),
            });
            assert_eq!(params.editable_curve_snapshot(), before_curve);
            assert_eq!(params.sync_division(), before_division);

            state.reduce_action(UiAction::ButtonPressed {
                key: REDO_KEY.to_string(),
            });
            assert_eq!(params.editable_curve_snapshot(), slot_curve);
            assert_eq!(params.sync_division(), before_division);
        });
    }

    #[test]
    fn quick_slot_command_press_stores_curve_without_changing_live_curve() {
        with_isolated_global_curve_slots("cmd-store", || {
            let params = Arc::new(PumpParams::new());
            let mut state = GuiState::new(
                Arc::clone(&params),
                Arc::new(GuiStatus::default()),
                Arc::new(AutomationQueue::default()),
                None,
            );
            let stored_curve = EditableCurve {
                nodes: vec![
                    CurveNode { x: 0.0, y: 1.0 },
                    CurveNode { x: 0.08, y: 0.04 },
                    CurveNode { x: 0.22, y: 0.68 },
                    CurveNode { x: 1.0, y: 1.0 },
                ],
                segments: vec![
                    CurveSegment { tension: -0.5 },
                    CurveSegment { tension: 0.42 },
                    CurveSegment { tension: -0.04 },
                ],

        ..EditableCurve::default()}
            .normalized();
            params.set_editable_curve(&stored_curve);

            state.reduce_action(UiAction::RegionInteracted {
                key: format!("{QUICK_SLOT_KEY_PREFIX}5"),
                kind: RegionInteractionKind::Pressed,
                local_pointer: Point { x: 0, y: 0 },
                raw_local_pointer: Point { x: 0, y: 0 },
                alt_down: false,
                command_down: true,
                shift_down: false,
            });
            assert_eq!(params.global_curve_slot_curve(5), Some(stored_curve.clone()));
            assert_eq!(params.editable_curve_snapshot(), stored_curve);

            state.reduce_action(UiAction::ButtonPressed {
                key: UNDO_KEY.to_string(),
            });
            assert_eq!(
                params.global_curve_slot_curve(5),
                Some(stored_curve.clone()),
                "global slot stores should not participate in local undo"
            );
            assert_eq!(
                params.editable_curve_snapshot(),
                stored_curve,
                "storing a slot should leave the live curve untouched even after undo"
            );
        });
    }

    #[test]
    fn quick_slot_command_press_does_not_mark_selected_preset_dirty() {
        with_isolated_global_curve_slots("cmd-store-dirty", || {
            let params = Arc::new(PumpParams::new());
            let mut state = GuiState::new(
                Arc::clone(&params),
                Arc::new(GuiStatus::default()),
                Arc::new(AutomationQueue::default()),
                None,
            );

            assert!(!params.current_state_differs_from_selected_preset());
            state.reduce_action(UiAction::RegionInteracted {
                key: format!("{QUICK_SLOT_KEY_PREFIX}2"),
                kind: RegionInteractionKind::Pressed,
                local_pointer: Point { x: 0, y: 0 },
                raw_local_pointer: Point { x: 0, y: 0 },
                alt_down: false,
                command_down: true,
                shift_down: false,
            });
            assert!(
                !params.current_state_differs_from_selected_preset(),
                "slot edits should not participate in the live dirty-state comparison"
            );
        });
    }

    #[test]
    fn empty_quick_slot_press_is_non_destructive_no_op() {
        with_isolated_global_curve_slots("empty-no-op", || {
            let params = Arc::new(PumpParams::new());
            let mut state = GuiState::new(
                Arc::clone(&params),
                Arc::new(GuiStatus::default()),
                Arc::new(AutomationQueue::default()),
                None,
            );
            let before_curve = params.editable_curve_snapshot();

            state.reduce_action(UiAction::RegionInteracted {
                key: format!("{QUICK_SLOT_KEY_PREFIX}1"),
                kind: RegionInteractionKind::Pressed,
                local_pointer: Point { x: 0, y: 0 },
                raw_local_pointer: Point { x: 0, y: 0 },
                alt_down: false,
                command_down: false,
                shift_down: false,
            });

            assert_eq!(params.editable_curve_snapshot(), before_curve);
            let runtime = state.runtime.lock().expect("runtime lock should succeed");
            assert_eq!(runtime.loaded_global_curve_slot, None);
        });
    }

    #[test]
    fn loaded_quick_slot_tracks_curve_deviation_state() {
        with_isolated_global_curve_slots("deviation", || {
            let params = Arc::new(PumpParams::new());
            let state = GuiState::new(
                Arc::clone(&params),
                Arc::new(GuiStatus::default()),
                Arc::new(AutomationQueue::default()),
                None,
            );
            let slot_curve = quick_slot_seeds()[0].curve.clone();
            assert!(params.set_global_curve_slot_curve(0, &slot_curve));
            state.apply_quick_slot_curve(0);

            assert!(!params.current_curve_deviates_from_global_slot(0));
            let mut edited = slot_curve.clone();
            edited.nodes[1].y = (edited.nodes[1].y + 0.1).clamp(0.0, 1.0);
            params.set_editable_curve(&edited);
            assert!(params.current_curve_deviates_from_global_slot(0));
            params.set_editable_curve(&slot_curve);
            assert!(!params.current_curve_deviates_from_global_slot(0));
        });
    }
}
