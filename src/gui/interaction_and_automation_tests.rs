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
}
