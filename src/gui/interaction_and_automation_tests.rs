mod interaction_and_automation_tests {
    use super::*;
    use super::super::RegionInteractionKind;
    use toybox::clack_plugin::events::io::EventBuffer;

    fn frame_input(mouse_down: bool) -> InputState {
        InputState {
            mouse_down,
            window_size: Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            ..InputState::default()
        }
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
}
